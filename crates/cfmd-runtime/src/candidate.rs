use std::sync::Arc;

use crate::{
    Error, ErrorKind, GroupKey, Object, ObjectPredicate, Plan, Projection, Query, Relation,
    RelationId, RelationQuery, RelationResult, Result, RevisionId, Row, TypedQuery,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CandidateDerivedEffects {
    normalized_rows_removed: usize,
    orphan_entities_deleted: usize,
}

impl CandidateDerivedEffects {
    /// Rows removed by lifecycle/normalization beyond the Plan's explicit row removals.
    #[must_use]
    pub const fn normalized_rows_removed(self) -> usize {
        self.normalized_rows_removed
    }

    /// Owned target objects deleted only because their final owner set became empty.
    #[must_use]
    pub const fn orphan_entities_deleted(self) -> usize {
        self.orphan_entities_deleted
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.normalized_rows_removed == 0 && self.orphan_entities_deleted == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidatePreview {
    source_revision: RevisionId,
    target_revision: RevisionId,
    effects: CandidateEffects,
    derived: CandidateDerivedEffects,
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
    pub const fn derived(self) -> CandidateDerivedEffects {
        self.derived
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
/// used by publication. A Candidate is inspectable future state only; publication belongs to the
/// originating [`crate::Database`] / [`crate::SessionDatabase`] control surface.
#[derive(Debug, Clone)]
pub struct Candidate {
    plan: Plan,
    target: Arc<kernel_revision::Revision>,
    changes: Vec<RelationChange>,
}

fn relation_len(
    relations: &kernel_model::RelationStore,
    relation: kernel_types::SemanticId,
) -> usize {
    relations
        .get(&relation)
        .map_or(0, kernel_model::SharedRelationRows::len)
}

fn expected_relation_len(plan: &Plan, relation: RelationId) -> usize {
    let source = relation_len(
        &plan.source.revision().state().model.relations,
        relation.into(),
    );
    let Some(mutation) = plan.mutations.get(&relation) else {
        return source;
    };
    source
        .saturating_add(mutation.inserted.len())
        .saturating_sub(mutation.removed.len())
}

fn derived_effects(plan: &Plan, target: &kernel_revision::Revision) -> CandidateDerivedEffects {
    let source_relations = &plan.source.revision().state().model.relations;
    let target_relations = &target.state().model.relations;
    let mut normalized_rows_removed = 0usize;

    let relation_ids = source_relations
        .keys()
        .chain(target_relations.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    for relation in relation_ids {
        let public = RelationId::new(relation.raw());
        let expected = expected_relation_len(plan, public);
        let actual = relation_len(target_relations, relation);
        normalized_rows_removed =
            normalized_rows_removed.saturating_add(expected.saturating_sub(actual));
    }

    let mut orphan_entities_deleted = 0usize;
    for contract in plan.owned_relations.values() {
        if contract.orphan_policy != crate::OrphanPolicy::DeleteIfUnowned {
            continue;
        }
        let expected = expected_relation_len(plan, contract.target_relation);
        let actual = relation_len(target_relations, contract.target_relation.into());
        orphan_entities_deleted =
            orphan_entities_deleted.saturating_add(expected.saturating_sub(actual));
    }

    CandidateDerivedEffects {
        normalized_rows_removed,
        orphan_entities_deleted,
    }
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
            derived: derived_effects(&self.plan, &self.target),
            diagnostics: self.diagnostics(),
        }
    }

    pub fn execute(&self, query: &Query) -> Result<RelationResult> {
        let prepared = query
            .inner
            .prepare(self.target.semantic_context(), &self.plan.registry)
            .map_err(|error| crate::query::query_error_at(query, &error))?;
        let footprint = prepared
            .read_footprint()
            .map_err(|error| crate::query::query_error_at(query, &error))?;
        self.plan.authority.require_read_footprint(&footprint)?;
        prepared
            .evaluate(
                &self.target.state().model,
                self.target.semantic_context(),
                &self.plan.registry,
            )
            .map(Into::into)
            .map_err(|error| crate::query::query_error_at(query, &error))
    }

    pub fn objects<E: Object>(&self) -> Result<CandidateObjectSet<E>> {
        self.plan.authority.require_read_entry()?;
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

    #[track_caller]
    #[must_use]
    pub fn top<F, V>(&self, k: usize, field: F) -> CandidateObjectQuery<E>
    where
        F: FnOnce(&E::Proxy) -> crate::Field<E, V>,
        V: crate::OrderedObjectValue,
    {
        self.query().top(k, field)
    }

    #[track_caller]
    #[must_use]
    pub fn bottom<F, V>(&self, k: usize, field: F) -> CandidateObjectQuery<E>
    where
        F: FnOnce(&E::Proxy) -> crate::Field<E, V>,
        V: crate::OrderedObjectValue,
    {
        self.query().bottom(k, field)
    }

    #[must_use]
    pub fn select<F, P>(&self, projection: F) -> CandidateProjectionQuery<E, P>
    where
        F: FnOnce(&E::Proxy) -> P,
        P: Projection<E>,
    {
        self.query().select(projection)
    }

    #[must_use]
    pub fn group_by<F, G>(&self, key: F) -> CandidateObjectGroupQuery<E, G>
    where
        F: FnOnce(&E::Proxy) -> G,
        G: GroupKey<E>,
    {
        self.query().group_by(key)
    }

    pub fn all(&self) -> Result<Vec<E>> {
        self.query().all()
    }

    pub fn count(&self) -> Result<usize> {
        self.query().count()
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

    #[track_caller]
    #[must_use]
    pub fn top<F, V>(mut self, k: usize, field: F) -> Self
    where
        F: FnOnce(&E::Proxy) -> crate::Field<E, V>,
        V: crate::OrderedObjectValue,
    {
        let proxy = E::proxy(self.relation.clone());
        self.inner = self.inner.top(field(&proxy), k);
        self
    }

    #[track_caller]
    #[must_use]
    pub fn bottom<F, V>(mut self, k: usize, field: F) -> Self
    where
        F: FnOnce(&E::Proxy) -> crate::Field<E, V>,
        V: crate::OrderedObjectValue,
    {
        let proxy = E::proxy(self.relation.clone());
        self.inner = self.inner.bottom(field(&proxy), k);
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

    #[must_use]
    pub fn group_by<F, G>(self, key: F) -> CandidateObjectGroupQuery<E, G>
    where
        F: FnOnce(&E::Proxy) -> G,
        G: GroupKey<E>,
    {
        let proxy = E::proxy(self.relation.clone());
        let key = key(&proxy);
        let error = (!key.belongs_to(self.relation.id())).then(|| {
            Error::new(
                ErrorKind::InvalidPlan,
                "candidate group key belongs to a different relation handle",
            )
        });
        CandidateObjectGroupQuery {
            candidate: self.candidate,
            relation: self.relation,
            inner: self.inner.raw(),
            key,
            error,
        }
    }

    pub fn all(&self) -> Result<Vec<E>> {
        let result = self.candidate.execute(&self.inner.clone().raw())?;
        result.rows().iter().map(E::from_row).collect()
    }

    pub fn count(&self) -> Result<usize> {
        candidate_exact_count(&self.candidate, self.inner.clone().raw())
    }

    pub fn first_or_none(&self) -> Result<Option<E>> {
        let mut values = self.all()?;
        Ok(values.drain(..).next())
    }

    pub fn one_or_none(&self) -> Result<Option<E>> {
        match self.count()? {
            0 => Ok(None),
            1 => {
                let mut values = self.all()?;
                Ok(values.pop())
            }
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
pub struct CandidateObjectGroupQuery<E: Object, G: GroupKey<E>> {
    candidate: Candidate,
    relation: Relation<E>,
    inner: Query,
    key: G,
    error: Option<Error>,
}

impl<E: Object, G: GroupKey<E>> CandidateObjectGroupQuery<E, G> {
    #[must_use]
    pub fn count(self) -> CandidateGroupedAggregateQuery<G::Output, usize> {
        let key_width = self.key.columns().len();
        let query = self.inner.clone().group_count(
            self.key.columns(),
            self.key.equivalences(),
            crate::object::count_equivalence_id(),
        );
        CandidateGroupedAggregateQuery {
            candidate: self.candidate,
            inner: query,
            decode: crate::object::decode_group_count_row::<E, G>,
            aggregate_column: key_width,
            ordering: crate::object::count_ordering_id(),
            error: self.error,
        }
    }

    #[must_use]
    pub fn sum<F>(self, value: F) -> CandidateGroupedAggregateQuery<G::Output, f64>
    where
        F: FnOnce(&E::Proxy) -> crate::Field<E, f64>,
    {
        let proxy = E::proxy(self.relation.clone());
        let value = value(&proxy);
        let error = self.error.or_else(|| {
            (value.relation_id() != self.relation.id()).then(|| {
                Error::new(
                    ErrorKind::InvalidPlan,
                    "candidate group aggregate field belongs to a different relation handle",
                )
            })
        });
        let key_width = self.key.columns().len();
        let query = self.inner.clone().group_exact_f64_sum(
            self.key.columns(),
            self.key.equivalences(),
            value.column(),
            value.equivalence(),
        );
        CandidateGroupedAggregateQuery {
            candidate: self.candidate,
            inner: query,
            decode: crate::object::decode_group_sum_row::<E, G>,
            aggregate_column: key_width,
            ordering: crate::object::exact_f64_sum_ordering_id(),
            error,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CandidateGroupedAggregateQuery<K, A> {
    candidate: Candidate,
    inner: Query,
    decode: fn(&Row) -> Result<(K, A)>,
    aggregate_column: usize,
    ordering: crate::OrderingId,
    error: Option<Error>,
}

impl<K, A> CandidateGroupedAggregateQuery<K, A> {
    #[track_caller]
    #[must_use]
    pub fn top(mut self, k: usize) -> Self {
        self.inner = self.inner.top_k_with_ties(
            self.aggregate_column,
            self.ordering,
            crate::OrderDirection::Descending,
            k,
        );
        self
    }

    #[track_caller]
    #[must_use]
    pub fn bottom(mut self, k: usize) -> Self {
        self.inner = self.inner.top_k_with_ties(
            self.aggregate_column,
            self.ordering,
            crate::OrderDirection::Ascending,
            k,
        );
        self
    }

    pub fn all(&self) -> Result<Vec<(K, A)>> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        self.candidate
            .execute(&self.inner)?
            .rows()
            .iter()
            .map(self.decode)
            .collect()
    }

    #[must_use]
    pub const fn node_id(&self) -> crate::QueryNodeId {
        self.inner.node_id()
    }

    #[must_use]
    pub fn source(&self) -> crate::QuerySource {
        self.inner.source()
    }
}

#[derive(Debug, Clone)]
pub struct CandidateProjectionQuery<E: Object, P: Projection<E>> {
    candidate: Candidate,
    inner: TypedQuery<E, P>,
}

impl<E: Object, P: Projection<E>> CandidateProjectionQuery<E, P> {
    #[must_use]
    pub fn distinct(mut self) -> Self {
        self.inner = self.inner.distinct();
        self
    }

    pub fn all(&self) -> Result<Vec<P::Output>> {
        let result = self.candidate.execute(self.inner.raw())?;
        self.inner.decode_result(&result)
    }

    pub fn count(&self) -> Result<usize> {
        candidate_exact_count(&self.candidate, self.inner.raw().clone())
    }

    pub fn first_or_none(&self) -> Result<Option<P::Output>> {
        let mut values = self.all()?;
        Ok(values.drain(..).next())
    }

    pub fn one_or_none(&self) -> Result<Option<P::Output>> {
        match self.count()? {
            0 => Ok(None),
            1 => {
                let mut values = self.all()?;
                Ok(values.pop())
            }
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

fn candidate_exact_count(candidate: &Candidate, query: Query) -> Result<usize> {
    let query = query.count(crate::object::count_equivalence_id());
    let result = candidate.execute(&query)?;
    let [row] = result.rows() else {
        return Err(Error::new(
            ErrorKind::InvariantViolation,
            "exact candidate count aggregate did not return exactly one row",
        ));
    };
    let [crate::Value::I64(count)] = row.as_slice() else {
        return Err(Error::new(
            ErrorKind::InvariantViolation,
            "exact candidate count aggregate returned an invalid row shape",
        ));
    };
    usize::try_from(*count).map_err(|_| {
        Error::new(
            ErrorKind::InvariantViolation,
            "exact candidate count aggregate cannot be represented as usize",
        )
    })
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
