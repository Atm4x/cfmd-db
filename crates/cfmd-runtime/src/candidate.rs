use std::sync::Arc;

use crate::{
    CommitOutcome, Error, ErrorKind, Object, ObjectPredicate, Plan, Projection, Query, Relation,
    RelationId, RelationQuery, RelationResult, Result, RevisionId, Row, TransactionId, TypedQuery,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationChange {
    relation: RelationId,
    inserted: Vec<Row>,
    removed: Vec<Row>,
}

impl RelationChange {
    #[must_use]
    pub const fn relation(&self) -> RelationId {
        self.relation
    }

    #[must_use]
    pub fn inserted(&self) -> &[Row] {
        &self.inserted
    }

    #[must_use]
    pub fn removed(&self) -> &[Row] {
        &self.removed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateReadiness {
    Ready,
    Stale { current_revision: RevisionId },
    RuntimeClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateEffects {
    touched_relations: usize,
    inserted_rows: usize,
    removed_rows: usize,
    entity_relations: usize,
    lifecycle_changed: bool,
}

impl CandidateEffects {
    #[must_use]
    pub const fn touched_relations(self) -> usize {
        self.touched_relations
    }
    #[must_use]
    pub const fn inserted_rows(self) -> usize {
        self.inserted_rows
    }
    #[must_use]
    pub const fn removed_rows(self) -> usize {
        self.removed_rows
    }
    #[must_use]
    pub const fn entity_relations(self) -> usize {
        self.entity_relations
    }
    #[must_use]
    pub const fn changes_lifecycle(self) -> bool {
        self.lifecycle_changed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateDiagnostics {
    readiness: CandidateReadiness,
}

impl CandidateDiagnostics {
    #[must_use]
    pub const fn invariants_validated(self) -> bool {
        true
    }
    #[must_use]
    pub const fn readiness(self) -> CandidateReadiness {
        self.readiness
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidatePreview {
    source_revision: RevisionId,
    target_revision: RevisionId,
    effects: CandidateEffects,
    diagnostics: CandidateDiagnostics,
}

impl CandidatePreview {
    #[must_use]
    pub const fn source_revision(self) -> RevisionId {
        self.source_revision
    }
    #[must_use]
    pub const fn target_revision(self) -> RevisionId {
        self.target_revision
    }
    #[must_use]
    pub const fn effects(self) -> CandidateEffects {
        self.effects
    }
    #[must_use]
    pub const fn diagnostics(self) -> CandidateDiagnostics {
        self.diagnostics
    }
}

/// Exact proposed future derived from one immutable `Plan`.
///
/// Candidate owns no second database authority: its target Revision is derived from the Plan's
/// pinned source snapshot and exact deltas, then validated by the same kernel revision machinery
/// used by commit. Nothing is published until `commit` succeeds.
#[derive(Debug, Clone)]
pub struct Candidate {
    plan: Plan,
    target: Arc<kernel_revision::Revision>,
    changes: Vec<RelationChange>,
}

impl Candidate {
    pub(crate) fn from_plan(plan: &Plan) -> Result<Self> {
        if plan.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "cannot derive a candidate from an empty plan",
            ));
        }
        let target = crate::runtime::build_plan_target(plan)?;
        let changes = plan
            .mutations
            .iter()
            .map(|(relation, mutation)| RelationChange {
                relation: *relation,
                inserted: mutation.inserted.clone(),
                removed: mutation.removed.clone(),
            })
            .collect();
        Ok(Self {
            plan: plan.clone(),
            target: Arc::new(target),
            changes,
        })
    }

    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.plan.base_revision
    }

    #[must_use]
    pub fn revision(&self) -> RevisionId {
        self.target.id().into()
    }

    #[must_use]
    pub fn changes(&self) -> &[RelationChange] {
        &self.changes
    }

    #[must_use]
    pub fn effects(&self) -> CandidateEffects {
        CandidateEffects {
            touched_relations: self.changes.len(),
            inserted_rows: self
                .changes
                .iter()
                .map(|change| change.inserted.len())
                .sum(),
            removed_rows: self.changes.iter().map(|change| change.removed.len()).sum(),
            entity_relations: self.plan.object_contracts.len(),
            lifecycle_changed: !self.plan.object_contracts.is_empty()
                || self.plan.model_delta.as_ref().is_some_and(|delta| {
                    !delta.lifecycle_entities_inserted.is_empty()
                        || !delta.lifecycle_entities_removed.is_empty()
                        || !delta.lifecycle_roots_inserted.is_empty()
                        || !delta.lifecycle_roots_removed.is_empty()
                        || !delta.lifecycle_keeps_alive.is_empty()
                }),
        }
    }

    #[must_use]
    pub fn diagnostics(&self) -> CandidateDiagnostics {
        let readiness = match self.plan.runtime.upgrade() {
            None => CandidateReadiness::RuntimeClosed,
            Some(runtime) => match runtime.snapshot() {
                Ok(snapshot)
                    if RevisionId::from(snapshot.revision().id()) == self.source_revision() =>
                {
                    CandidateReadiness::Ready
                }
                Ok(snapshot) => CandidateReadiness::Stale {
                    current_revision: snapshot.revision().id().into(),
                },
                Err(_) => CandidateReadiness::RuntimeClosed,
            },
        };
        CandidateDiagnostics { readiness }
    }

    #[must_use]
    pub fn preview(&self) -> CandidatePreview {
        CandidatePreview {
            source_revision: self.source_revision(),
            target_revision: self.revision(),
            effects: self.effects(),
            diagnostics: self.diagnostics(),
        }
    }

    pub fn execute(&self, query: &Query) -> Result<RelationResult> {
        self.plan.authority.require(crate::Permission::Read)?;
        let prepared = query
            .inner
            .prepare(self.target.semantic_context(), &self.plan.registry)
            .map_err(|error| crate::query::query_error(&error))?;
        prepared
            .evaluate(
                &self.target.state().model,
                self.target.semantic_context(),
                &self.plan.registry,
            )
            .map(Into::into)
            .map_err(|error| crate::query::query_error(&error))
    }

    pub fn objects<E: Object>(&self) -> Result<CandidateObjectSet<E>> {
        self.plan.authority.require(crate::Permission::Read)?;
        let schema = crate::SchemaView::from_kernel(&self.target.semantic_context().schema);
        let relation_schema = schema.relation(E::relation_id()).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidSchema,
                format!("candidate schema has no object relation {}", E::KEY),
            )
        })?;
        let relation = Relation::from_schema(relation_schema);
        CandidateObjectSet::new(self.clone(), relation)
    }

    pub fn commit(&self, transaction: TransactionId) -> Result<CommitOutcome> {
        {
            let runtime = self.plan.runtime.upgrade().ok_or_else(|| {
                Error::new(
                    ErrorKind::Recovery,
                    "candidate database runtime is no longer open",
                )
            })?;
            crate::runtime::commit_bound_plan(&runtime, &self.plan, transaction)
        }
    }
}

#[derive(Debug, Clone)]
pub struct CandidateObjectSet<E: Object> {
    candidate: Candidate,
    relation: Relation<E>,
}

impl<E: Object> CandidateObjectSet<E> {
    fn new(candidate: Candidate, relation: Relation<E>) -> Result<Self> {
        let fields = E::fields();
        if relation.width() != fields.len() || !E::accepts(relation.column_types()) {
            return Err(Error::new(
                ErrorKind::TypeMismatch,
                format!(
                    "candidate relation for object {} does not match its Rust shape",
                    E::KEY
                ),
            ));
        }
        Ok(Self {
            candidate,
            relation,
        })
    }

    #[must_use]
    pub fn query(&self) -> CandidateObjectQuery<E> {
        CandidateObjectQuery {
            candidate: self.candidate.clone(),
            relation: self.relation.clone(),
            inner: self.relation.query(),
        }
    }

    #[must_use]
    pub fn where_<F, P>(&self, predicate: F) -> CandidateObjectQuery<E>
    where
        F: FnOnce(&E::Proxy) -> P,
        P: ObjectPredicate<E>,
    {
        self.query().where_(predicate)
    }

    pub fn all(&self) -> Result<Vec<E>> {
        self.query().all()
    }

    pub fn get(&self, id: crate::Id<E>) -> Result<Option<E>> {
        identity_query(&self.relation, self.query(), id)?.one_or_none()
    }

    pub fn require(&self, id: crate::Id<E>) -> Result<E> {
        self.get(id)?.ok_or_else(|| {
            Error::new(
                ErrorKind::NotFound,
                format!(
                    "{} identity {} was not found in candidate",
                    E::KEY,
                    id.raw()
                ),
            )
        })
    }
}

#[derive(Debug, Clone)]
pub struct CandidateObjectQuery<E: Object> {
    candidate: Candidate,
    relation: Relation<E>,
    inner: RelationQuery<E>,
}

impl<E: Object> CandidateObjectQuery<E> {
    #[must_use]
    pub fn where_<F, P>(mut self, predicate: F) -> Self
    where
        F: FnOnce(&E::Proxy) -> P,
        P: ObjectPredicate<E>,
    {
        let proxy = E::proxy(self.relation.clone());
        self.inner = self.inner.filter(predicate(&proxy));
        self
    }

    #[must_use]
    pub fn select<F, P>(self, projection: F) -> CandidateProjectionQuery<E, P>
    where
        F: FnOnce(&E::Proxy) -> P,
        P: Projection<E>,
    {
        let proxy = E::proxy(self.relation.clone());
        CandidateProjectionQuery {
            candidate: self.candidate,
            inner: self.inner.select(projection(&proxy)),
        }
    }

    pub fn all(&self) -> Result<Vec<E>> {
        let result = self.candidate.execute(&self.inner.clone().raw())?;
        result.rows().iter().map(E::from_row).collect()
    }

    pub fn first_or_none(&self) -> Result<Option<E>> {
        let mut values = self.all()?;
        Ok(values.drain(..).next())
    }

    pub fn one_or_none(&self) -> Result<Option<E>> {
        let mut values = self.all()?;
        match values.len() {
            0 => Ok(None),
            1 => Ok(values.pop()),
            count => Err(Error::new(
                ErrorKind::Cardinality,
                format!("expected at most one candidate object, query returned {count}"),
            )),
        }
    }

    pub fn one(&self) -> Result<E> {
        self.one_or_none()?.ok_or_else(|| {
            Error::new(
                ErrorKind::Cardinality,
                "expected exactly one candidate object, query returned none",
            )
        })
    }
}

#[derive(Debug, Clone)]
pub struct CandidateProjectionQuery<E: Object, P: Projection<E>> {
    candidate: Candidate,
    inner: TypedQuery<E, P>,
}

impl<E: Object, P: Projection<E>> CandidateProjectionQuery<E, P> {
    pub fn all(&self) -> Result<Vec<P::Output>> {
        let result = self.candidate.execute(self.inner.raw())?;
        self.inner.decode_result(&result)
    }

    pub fn first_or_none(&self) -> Result<Option<P::Output>> {
        let mut values = self.all()?;
        Ok(values.drain(..).next())
    }

    pub fn one_or_none(&self) -> Result<Option<P::Output>> {
        let mut values = self.all()?;
        match values.len() {
            0 => Ok(None),
            1 => Ok(values.pop()),
            count => Err(Error::new(
                ErrorKind::Cardinality,
                format!("expected at most one candidate projection, query returned {count}"),
            )),
        }
    }

    pub fn one(&self) -> Result<P::Output> {
        self.one_or_none()?.ok_or_else(|| {
            Error::new(
                ErrorKind::Cardinality,
                "expected exactly one candidate projection, query returned none",
            )
        })
    }
}

fn identity_query<E: Object>(
    relation: &Relation<E>,
    query: CandidateObjectQuery<E>,
    id: crate::Id<E>,
) -> Result<CandidateObjectQuery<E>> {
    let column = E::identity_column().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidSchema,
            format!("object {} has no identity field", E::KEY),
        )
    })?;
    let equivalence = relation.equivalence_at(column).ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidSchema,
            format!("object {} identity has no equivalence", E::KEY),
        )
    })?;
    let field = crate::Field::<E, crate::Id<E>>::__from_parts(relation.id(), column, equivalence);
    Ok(CandidateObjectQuery {
        inner: query.inner.filter(field.eq(id)),
        ..query
    })
}
