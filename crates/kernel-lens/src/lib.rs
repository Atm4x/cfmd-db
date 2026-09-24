use std::collections::{BTreeMap, BTreeSet};

use kernel_change::{FineChange, FineChangeKind, PreparedRewrite, RewriteEffect};
use kernel_model::Value;
use kernel_types::{RevisionObservableId, SemanticId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LensError {
    TypeMismatch,
    FieldMissing(SemanticId),
    ComplementMismatch,
}

/// Explicit dependent complement captured while projecting a source value.
/// Complements contain logical values only; physical row/index handles cannot
/// enter this representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LensComplement {
    Identity,
    ProductRemainder {
        field: SemanticId,
        remainder: BTreeMap<SemanticId, Value>,
    },
    Compose {
        outer: Box<Self>,
        inner: Box<Self>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LensExpr {
    Identity,
    ProductField(SemanticId),
    Compose(Box<Self>, Box<Self>),
}

impl LensExpr {
    pub fn split(&self, source: &Value) -> Result<(Value, LensComplement), LensError> {
        match self {
            Self::Identity => Ok((source.clone(), LensComplement::Identity)),
            Self::ProductField(field) => {
                let Value::Product(fields) = source else {
                    return Err(LensError::TypeMismatch);
                };
                let view = fields
                    .get(field)
                    .cloned()
                    .ok_or(LensError::FieldMissing(*field))?;
                let mut remainder = fields.clone();
                remainder.remove(field);
                Ok((
                    view,
                    LensComplement::ProductRemainder {
                        field: *field,
                        remainder,
                    },
                ))
            }
            Self::Compose(outer, inner) => {
                let (middle, outer_complement) = outer.split(source)?;
                let (view, inner_complement) = inner.split(&middle)?;
                Ok((
                    view,
                    LensComplement::Compose {
                        outer: Box::new(outer_complement),
                        inner: Box::new(inner_complement),
                    },
                ))
            }
        }
    }

    pub fn restore(&self, view: &Value, complement: &LensComplement) -> Result<Value, LensError> {
        match (self, complement) {
            (Self::Identity, LensComplement::Identity) => Ok(view.clone()),
            (
                Self::ProductField(expected_field),
                LensComplement::ProductRemainder { field, remainder },
            ) if expected_field == field => {
                let mut fields = remainder.clone();
                fields.insert(*field, view.clone());
                Ok(Value::Product(fields))
            }
            (
                Self::Compose(outer, inner),
                LensComplement::Compose {
                    outer: outer_complement,
                    inner: inner_complement,
                },
            ) => {
                let middle = inner.restore(view, inner_complement)?;
                outer.restore(&middle, outer_complement)
            }
            _ => Err(LensError::ComplementMismatch),
        }
    }

    pub fn get(&self, source: &Value) -> Result<Value, LensError> {
        self.split(source).map(|(view, _)| view)
    }

    pub fn complement(&self, source: &Value) -> Result<LensComplement, LensError> {
        self.split(source).map(|(_, complement)| complement)
    }

    pub fn put(&self, source: &Value, view: &Value) -> Result<Value, LensError> {
        let (_, complement) = self.split(source)?;
        self.restore(view, &complement)
    }

    /// Lifts an intent-bearing rewrite on the view back to an intent-bearing
    /// rewrite on the source. Rewrite identity/law identity are preserved;
    /// only the extensional effect is re-derived through the complement.
    pub fn lift_rewrite<I: Clone>(
        &self,
        source: &Value,
        rewrite: &PreparedRewrite<Value, I>,
    ) -> Result<PreparedRewrite<Value, I>, LensError> {
        let (old_view, complement) = self.split(source)?;
        let new_view = rewrite.apply(&old_view);
        let new_source = self.restore(&new_view, &complement)?;
        let effect = match self {
            Self::Identity => rewrite.effect.clone(),
            Self::ProductField(_) | Self::Compose(_, _) => {
                RewriteEffect::Fine(FineChange::new(FineChangeKind::Product, new_source))
            }
        };
        Ok(PreparedRewrite {
            spec: rewrite.spec,
            explicit_inputs: rewrite.explicit_inputs.clone(),
            effect,
            law_set: rewrite.law_set,
        })
    }

    pub fn check_get_put(&self, source: &Value) -> Result<bool, LensError> {
        let view = self.get(source)?;
        Ok(self.put(source, &view)? == *source)
    }

    pub fn check_put_get(&self, source: &Value, view: &Value) -> Result<bool, LensError> {
        let updated = self.put(source, view)?;
        Ok(self.get(&updated)? == *view)
    }

    pub fn check_put_put(
        &self,
        source: &Value,
        first: &Value,
        second: &Value,
    ) -> Result<bool, LensError> {
        let after_first = self.put(source, first)?;
        let after_both = self.put(&after_first, second)?;
        Ok(after_both == self.put(source, second)?)
    }

    pub fn check_complement_round_trip(&self, source: &Value) -> Result<bool, LensError> {
        let (view, complement) = self.split(source)?;
        Ok(self.restore(&view, &complement)? == *source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_change::{RewriteLawSetId, RewriteSpecId};

    const LEFT: SemanticId = SemanticId(1);
    const NESTED: SemanticId = SemanticId(2);
    const RIGHT: SemanticId = SemanticId(3);
    const NAME: SemanticId = SemanticId(4);
    const FLAG: SemanticId = SemanticId(5);

    fn source() -> Value {
        Value::Product(BTreeMap::from([
            (LEFT, Value::I64(10)),
            (
                NESTED,
                Value::Product(BTreeMap::from([
                    (NAME, Value::Text("name".into())),
                    (FLAG, Value::Bool(true)),
                ])),
            ),
            (RIGHT, Value::I64(30)),
        ]))
    }

    #[test]
    fn product_field_lens_preserves_explicit_complement_and_laws() {
        let lens = LensExpr::ProductField(NESTED);
        let source = source();
        let new_view = Value::Product(BTreeMap::from([
            (NAME, Value::Text("new".into())),
            (FLAG, Value::Bool(false)),
        ]));
        assert_eq!(lens.check_complement_round_trip(&source), Ok(true));
        assert_eq!(lens.check_get_put(&source), Ok(true));
        assert_eq!(lens.check_put_get(&source, &new_view), Ok(true));
        assert_eq!(
            lens.check_put_put(
                &source,
                &Value::Product(BTreeMap::from([
                    (NAME, Value::Text("first".into())),
                    (FLAG, Value::Bool(true)),
                ])),
                &new_view,
            ),
            Ok(true)
        );
        let updated = lens.put(&source, &new_view).unwrap();
        let Value::Product(fields) = updated else {
            unreachable!();
        };
        assert_eq!(fields[&LEFT], Value::I64(10));
        assert_eq!(fields[&RIGHT], Value::I64(30));
    }

    #[test]
    fn composed_lens_composes_dependent_complements() {
        let lens = LensExpr::Compose(
            Box::new(LensExpr::ProductField(NESTED)),
            Box::new(LensExpr::ProductField(NAME)),
        );
        let source = source();
        let new_name = Value::Text("renamed".into());
        assert_eq!(lens.check_complement_round_trip(&source), Ok(true));
        assert_eq!(lens.check_get_put(&source), Ok(true));
        assert_eq!(lens.check_put_get(&source, &new_name), Ok(true));
        let (_, complement) = lens.split(&source).unwrap();
        assert!(matches!(complement, LensComplement::Compose { .. }));
    }

    #[test]
    fn complement_from_another_lens_is_rejected() {
        let source = source();
        let lens = LensExpr::ProductField(NESTED);
        let wrong = LensExpr::ProductField(LEFT).complement(&source).unwrap();
        assert_eq!(
            lens.restore(&Value::I64(9), &wrong),
            Err(LensError::ComplementMismatch)
        );
    }

    #[test]
    fn view_rewrite_lifts_to_source_without_losing_intent_identity() {
        let lens = LensExpr::Compose(
            Box::new(LensExpr::ProductField(NESTED)),
            Box::new(LensExpr::ProductField(NAME)),
        );
        let source = source();
        let rewrite = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(100)),
            explicit_inputs: vec![Value::Text("requested".into())],
            effect: RewriteEffect::Fine(FineChange::new(
                FineChangeKind::Scalar,
                Value::Text("renamed".into()),
            )),
            law_set: RewriteLawSetId(SemanticId(101)),
        };
        let lifted = lens.lift_rewrite(&source, &rewrite).unwrap();
        assert_eq!(lifted.spec, rewrite.spec);
        assert_eq!(lifted.law_set, rewrite.law_set);
        let updated = lifted.apply(&source);
        assert_eq!(lens.get(&updated), Ok(Value::Text("renamed".into())));
        let Value::Product(root) = updated else {
            unreachable!()
        };
        assert_eq!(root[&LEFT], Value::I64(10));
        assert_eq!(root[&RIGHT], Value::I64(30));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritabilityFailure {
    ConstantHasNoSourceOwner,
    DerivedOperatorHasNoCertifiedLift,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritableCompilation {
    Writable(WritableViewPlan),
    ReadOnly(WritabilityFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritableViewPlan {
    pub query: kernel_query::ExactQuery,
    pub rewrite_spec: kernel_change::RewriteSpecId,
    pub lens: LensExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritableExecutionError {
    RewriteFamilyMismatch {
        expected: kernel_change::RewriteSpecId,
        actual: kernel_change::RewriteSpecId,
    },
    Lens(LensError),
}

impl From<LensError> for WritableExecutionError {
    fn from(value: LensError) -> Self {
        Self::Lens(value)
    }
}

impl WritableViewPlan {
    pub fn lift_rewrite<I: Clone>(
        &self,
        source: &Value,
        rewrite: &PreparedRewrite<Value, I>,
    ) -> Result<PreparedRewrite<Value, I>, WritableExecutionError> {
        if rewrite.spec != self.rewrite_spec {
            return Err(WritableExecutionError::RewriteFamilyMismatch {
                expected: self.rewrite_spec,
                actual: rewrite.spec,
            });
        }
        self.lens.lift_rewrite(source, rewrite).map_err(Into::into)
    }
}

/// Structural writability compiler for the first certified fragment.
///
/// It intentionally accepts only identity and product-field projection chains.
/// Derived arithmetic/branching/aggregate operators remain read-only until an
/// explicit certified lift is provided; the compiler never selects an
/// arbitrary source preimage.
#[must_use]
pub fn compile_writable_query(
    query: &kernel_query::ExactQuery,
    rewrite_spec: kernel_change::RewriteSpecId,
) -> WritableCompilation {
    match compile_writable_expr(query.root()) {
        Ok(lens) => WritableCompilation::Writable(WritableViewPlan {
            query: query.clone(),
            rewrite_spec,
            lens,
        }),
        Err(reason) => WritableCompilation::ReadOnly(reason),
    }
}

fn compile_writable_expr(expr: &kernel_query::Expr) -> Result<LensExpr, WritabilityFailure> {
    match expr {
        kernel_query::Expr::Input => Ok(LensExpr::Identity),
        kernel_query::Expr::ProductField { input, field } => {
            let outer = compile_writable_expr(input)?;
            Ok(LensExpr::Compose(
                Box::new(outer),
                Box::new(LensExpr::ProductField(*field)),
            ))
        }
        kernel_query::Expr::Const(_) | kernel_query::Expr::TypedConst { .. } => {
            Err(WritabilityFailure::ConstantHasNoSourceOwner)
        }
        kernel_query::Expr::SeqLength(_)
        | kernel_query::Expr::SeqSumI64(_)
        | kernel_query::Expr::AddI64(_, _)
        | kernel_query::Expr::If { .. } => {
            Err(WritabilityFailure::DerivedOperatorHasNoCertifiedLift)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelColumnOrigin {
    pub relation: SemanticId,
    pub column: usize,
    pub observable: RevisionObservableId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelRewriteLiftStage {
    Project {
        columns: Vec<usize>,
    },
    FilterEqConst {
        column: usize,
        equivalence: SemanticId,
    },
    FilterEqColumns {
        left_column: usize,
        right_column: usize,
        equivalence: SemanticId,
    },
    JoinOwnerSide {
        owner_is_left: bool,
        owner_column: usize,
        lookup_column: usize,
        equivalence: SemanticId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RelWritableObligation {
    PreserveHiddenColumnsComplement {
        relation: SemanticId,
        hidden: BTreeSet<RevisionObservableId>,
    },
    HiddenColumnConstructorForInsert {
        relation: SemanticId,
        hidden: BTreeSet<RevisionObservableId>,
    },
    ProjectionNoSemanticCollapse,
    PredicateAdmissibility,
    DtcGuardNoImpact,
    VmfInvariantClosure,
    JoinLookupDeterminant {
        join_key: RevisionObservableId,
        lookup_outputs: BTreeSet<RevisionObservableId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelWritabilityFailure {
    OwnerRelationNotReferenced,
    OwnerRelationReferencedOnBothJoinSides,
    MissingObservableBinding { relation: SemanticId },
    ColumnOutOfBounds { column: usize, arity: usize },
    DuplicateProjectionColumn(usize),
    UnsupportedOperator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelWritableBindingError {
    UnknownRelation(SemanticId),
    ArityMismatch {
        relation: SemanticId,
        expected: usize,
        actual: usize,
    },
    UnknownObservable(RevisionObservableId),
    ObservableEquivalenceMismatch {
        relation: SemanticId,
        column: usize,
        expected: SemanticId,
        actual: SemanticId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelWritableColumnBindings {
    columns: BTreeMap<SemanticId, Vec<RevisionObservableId>>,
}

impl RelWritableColumnBindings {
    pub fn from_catalog(
        semantic: &kernel_schema::SemanticContext,
        catalog: &kernel_semantics::observable::RevisionObservableCatalog,
        columns: BTreeMap<SemanticId, Vec<RevisionObservableId>>,
    ) -> Result<Self, RelWritableBindingError> {
        for (&relation, observables) in &columns {
            let definition = semantic
                .schema
                .relation(relation)
                .ok_or(RelWritableBindingError::UnknownRelation(relation))?;
            let equivalences = match &definition.semantics {
                kernel_schema::RelationSemantics::Set {
                    column_equivalences,
                }
                | kernel_schema::RelationSemantics::Bag {
                    column_equivalences,
                } => column_equivalences,
            };
            if equivalences.len() != observables.len() {
                return Err(RelWritableBindingError::ArityMismatch {
                    relation,
                    expected: equivalences.len(),
                    actual: observables.len(),
                });
            }
            for (column, (&expected, &observable)) in
                equivalences.iter().zip(observables).enumerate()
            {
                let actual = match catalog.definition(observable) {
                    Ok(
                        kernel_semantics::observable::SemanticObservableDefinition::PinnedEquivalence(
                            equivalence,
                        )
                        | kernel_semantics::observable::SemanticObservableDefinition::PinnedEquivalenceCoordinate {
                            equivalence,
                            ..
                        },
                    ) => *equivalence,
                    Ok(kernel_semantics::observable::SemanticObservableDefinition::Product(_)) => {
                        return Err(RelWritableBindingError::UnknownObservable(observable));
                    }
                    Err(_) => return Err(RelWritableBindingError::UnknownObservable(observable)),
                };
                if actual != expected {
                    return Err(RelWritableBindingError::ObservableEquivalenceMismatch {
                        relation,
                        column,
                        expected,
                        actual,
                    });
                }
            }
        }
        Ok(Self { columns })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelWritableViewPlan {
    pub query: kernel_query::RelExpr,
    pub owner_relation: SemanticId,
    pub rewrite_spec: kernel_change::RewriteSpecId,
    pub output_origins: Vec<RelColumnOrigin>,
    pub stages: Vec<RelRewriteLiftStage>,
    pub required_obligations: BTreeSet<RelWritableObligation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelWritableCompilation {
    Writable(RelWritableViewPlan),
    Conditional {
        plan: RelWritableViewPlan,
        obligations: BTreeSet<RelWritableObligation>,
    },
    ReadOnly(RelWritabilityFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelRewriteLiftError {
    CandidateRewriteSpecMismatch {
        expected: kernel_change::RewriteSpecId,
        actual: kernel_change::RewriteSpecId,
    },
    CandidateDeltaTypeMismatch,
    CandidateEffectMismatch,
    CandidateGenerationUnsupported,
    DeterminantEvidenceUnavailable,
    JoinLookupKeyNotUnique,
    ProjectionPreimageAmbiguous,
    ProjectConstructorOwnerMismatch,
    ProjectConstructorRewriteSpecMismatch,
    ProjectConstructorColumnOutOfBounds {
        column: usize,
        arity: usize,
    },
    ProjectConstructorMissingHiddenColumn {
        column: usize,
    },
    ProjectConstructorOverridesVisibleColumn {
        column: usize,
    },
    RequestedViewInadmissible,
    SourceRewriteSpecMismatch {
        expected: kernel_change::RewriteSpecId,
        actual: kernel_change::RewriteSpecId,
    },
    UnresolvedObligations(BTreeSet<RelWritableObligation>),
    Query(kernel_query::RelQueryError),
}

impl From<kernel_query::RelQueryError> for RelRewriteLiftError {
    fn from(value: kernel_query::RelQueryError) -> Self {
        Self::Query(value)
    }
}

/// Exact finite classification of source Rewrite candidates for one requested
/// view Rewrite. Ambiguity is surfaced with two intent-bearing witnesses;
/// candidates are never collapsed merely because their current endpoint is the
/// same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelRewriteLiftClassification<I> {
    Impossible,
    Unique(Box<kernel_query::PreparedRelationRewrite<I>>),
    Ambiguous {
        first: Box<kernel_query::PreparedRelationRewrite<I>>,
        second: Box<kernel_query::PreparedRelationRewrite<I>>,
    },
}

impl RelWritableViewPlan {
    /// Synthesizes the complete unique source Rewrite for the identity relational
    /// section (`Scan(owner)`). More lossy stages remain unavailable until their
    /// complement/determinant certificates can construct a complete lift fiber.
    pub fn synthesize_identity_source_rewrite<I: Clone>(
        &self,
        source: &kernel_model::FiniteModel,
        semantic: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        source_spec: &kernel_change::RewriteSpec,
        requested_view: &PreparedRewrite<kernel_query::RelationValue, I>,
    ) -> Result<kernel_query::PreparedRelationRewrite<I>, RelRewriteLiftError> {
        if !self.required_obligations.is_empty() {
            return Err(RelRewriteLiftError::UnresolvedObligations(
                self.required_obligations.clone(),
            ));
        }
        if source_spec.id != self.rewrite_spec {
            return Err(RelRewriteLiftError::SourceRewriteSpecMismatch {
                expected: self.rewrite_spec,
                actual: source_spec.id,
            });
        }
        if !self.stages.is_empty()
            || !matches!(self.query, kernel_query::RelExpr::Scan(relation) if relation == self.owner_relation)
        {
            return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
        }
        let old_owner = kernel_query::RelExpr::Scan(self.owner_relation)
            .evaluate(source, semantic, registry)?;
        let requested_endpoint = requested_view.apply(&old_owner);
        let owner_type =
            kernel_query::RelExpr::Scan(self.owner_relation).typecheck(semantic, registry)?;
        let delta = kernel_query::RelationDelta::between_values(
            &old_owner,
            &requested_endpoint,
            owner_type,
            semantic,
            registry,
        )?;
        Ok(delta.prepare_relation_rewrite(
            &old_owner,
            semantic,
            registry,
            source_spec,
            requested_view.explicit_inputs.clone(),
        )?)
    }

    pub fn classify_source_rewrite_candidates<I: Clone + PartialEq>(
        &self,
        source: &kernel_model::FiniteModel,
        semantic: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        requested_view: &PreparedRewrite<kernel_query::RelationValue, I>,
        candidates: impl IntoIterator<Item = kernel_query::PreparedRelationRewrite<I>>,
    ) -> Result<RelRewriteLiftClassification<I>, RelRewriteLiftError> {
        if !self.required_obligations.is_empty() {
            return Err(RelRewriteLiftError::UnresolvedObligations(
                self.required_obligations.clone(),
            ));
        }
        let prepared_query = self.query.prepare(semantic, registry)?;
        let view_type = prepared_query.result_type().clone();
        let old_view = prepared_query.evaluate(source, semantic, registry)?;
        let requested_endpoint = requested_view.apply(&old_view);
        let old_owner = kernel_query::RelExpr::Scan(self.owner_relation)
            .evaluate(source, semantic, registry)?;
        let owner_type = kernel_query::RelExpr::Scan(self.owner_relation)
            .prepare(semantic, registry)?
            .result_type()
            .clone();

        let mut matches = Vec::new();
        for candidate in candidates {
            if candidate.rewrite.spec != self.rewrite_spec {
                return Err(RelRewriteLiftError::CandidateRewriteSpecMismatch {
                    expected: self.rewrite_spec,
                    actual: candidate.rewrite.spec,
                });
            }
            if candidate.delta.result_type != owner_type {
                return Err(RelRewriteLiftError::CandidateDeltaTypeMismatch);
            }
            let delta_endpoint =
                candidate
                    .delta
                    .apply_to_value(old_owner.clone(), semantic, registry)?;
            if candidate.rewrite.apply(&old_owner) != delta_endpoint {
                return Err(RelRewriteLiftError::CandidateEffectMismatch);
            }

            let mut next_model = source.clone();
            next_model
                .relations
                .insert(self.owner_relation, delta_endpoint.into_rows());
            let actual_view = prepared_query.evaluate(&next_model, semantic, registry)?;
            let residual = kernel_query::RelationDelta::between_values(
                &actual_view,
                &requested_endpoint,
                view_type.clone(),
                semantic,
                registry,
            )?;
            if !residual.is_empty() {
                continue;
            }

            // Exact represented Rewrite identity is deliberately stronger than
            // endpoint equality. This is conservative until a certified
            // Rewrite-equivalence quotient is first-class.
            if matches
                .iter()
                .any(|existing: &kernel_query::PreparedRelationRewrite<I>| existing == &candidate)
            {
                continue;
            }
            matches.push(candidate);
            if matches.len() == 2 {
                return Ok(RelRewriteLiftClassification::Ambiguous {
                    first: Box::new(matches.remove(0)),
                    second: Box::new(matches.remove(0)),
                });
            }
        }

        Ok(match matches.pop() {
            Some(candidate) => RelRewriteLiftClassification::Unique(Box::new(candidate)),
            None => RelRewriteLiftClassification::Impossible,
        })
    }
}

pub struct RelWritableCompileContext<'a> {
    pub owner_relation: SemanticId,
    pub rewrite_spec: kernel_change::RewriteSpecId,
    pub relation_columns: &'a RelWritableColumnBindings,
    pub determinant_theory: Option<&'a kernel_semantics::anchor_pullback::DeterminantTheory>,
}

#[derive(Debug, Clone)]
struct RelWritableAnalysis {
    owner_present: bool,
    output_origins: Vec<RelColumnOrigin>,
    stages: Vec<RelRewriteLiftStage>,
    obligations: BTreeSet<RelWritableObligation>,
}

/// Compiles the exact relational write fragment with one explicit source owner.
/// The compiler never guesses a preimage: information loss, predicate guards,
/// invariant closure, and lookup-side determinant facts remain typed obligations.
pub fn compile_rel_writable_query(
    query: &kernel_query::RelExpr,
    context: &RelWritableCompileContext<'_>,
) -> Result<RelWritableCompilation, kernel_semantics::anchor_pullback::AnchorPullbackError> {
    let analysis = match analyze_writable_rel(query, context) {
        Ok(analysis) => analysis,
        Err(reason) => return Ok(RelWritableCompilation::ReadOnly(reason)),
    };
    if !analysis.owner_present {
        return Ok(RelWritableCompilation::ReadOnly(
            RelWritabilityFailure::OwnerRelationNotReferenced,
        ));
    }
    let plan = RelWritableViewPlan {
        query: query.clone(),
        owner_relation: context.owner_relation,
        rewrite_spec: context.rewrite_spec,
        output_origins: analysis.output_origins,
        stages: analysis.stages,
        required_obligations: analysis.obligations.clone(),
    };
    if analysis.obligations.is_empty() {
        Ok(RelWritableCompilation::Writable(plan))
    } else {
        Ok(RelWritableCompilation::Conditional {
            plan,
            obligations: analysis.obligations,
        })
    }
}

fn analyze_writable_rel(
    query: &kernel_query::RelExpr,
    context: &RelWritableCompileContext<'_>,
) -> Result<RelWritableAnalysis, RelWritabilityFailure> {
    use kernel_query::RelExpr;
    match query {
        RelExpr::Scan(relation) => analyze_rel_scan(*relation, context),
        RelExpr::Project { input, columns } => analyze_rel_project(input, columns, context),
        RelExpr::FilterEqConst {
            input,
            column,
            equivalence,
            ..
        } => analyze_rel_filter_const(input, *column, *equivalence, context),
        RelExpr::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => analyze_rel_filter_columns(input, *left_column, *right_column, *equivalence, context),
        RelExpr::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => analyze_rel_join(
            left,
            right,
            *left_column,
            *right_column,
            *equivalence,
            context,
        ),
        RelExpr::Difference { .. }
        | RelExpr::AntiJoin { .. }
        | RelExpr::Distinct { .. }
        | RelExpr::Group { .. }
        | RelExpr::TopKWithTies { .. }
        | RelExpr::PromoteToBag(_) => Err(RelWritabilityFailure::UnsupportedOperator),
    }
}

fn analyze_rel_scan(
    relation: SemanticId,
    context: &RelWritableCompileContext<'_>,
) -> Result<RelWritableAnalysis, RelWritabilityFailure> {
    let Some(columns) = context.relation_columns.columns.get(&relation) else {
        return Err(RelWritabilityFailure::MissingObservableBinding { relation });
    };
    Ok(RelWritableAnalysis {
        owner_present: relation == context.owner_relation,
        output_origins: columns
            .iter()
            .enumerate()
            .map(|(column, &observable)| RelColumnOrigin {
                relation,
                column,
                observable,
            })
            .collect(),
        stages: Vec::new(),
        obligations: BTreeSet::new(),
    })
}

fn analyze_rel_project(
    input: &kernel_query::RelExpr,
    columns: &[usize],
    context: &RelWritableCompileContext<'_>,
) -> Result<RelWritableAnalysis, RelWritabilityFailure> {
    let mut analysis = analyze_writable_rel(input, context)?;
    let mut seen = BTreeSet::new();
    for &column in columns {
        ensure_column(column, analysis.output_origins.len())?;
        if !seen.insert(column) {
            return Err(RelWritabilityFailure::DuplicateProjectionColumn(column));
        }
    }
    let previous = analysis.output_origins.clone();
    let projected = columns
        .iter()
        .map(|&column| previous[column])
        .collect::<Vec<_>>();
    let visible_owner = projected
        .iter()
        .filter(|origin| origin.relation == context.owner_relation)
        .map(|origin| origin.observable)
        .collect::<BTreeSet<_>>();
    let hidden_owner = previous
        .iter()
        .filter(|origin| origin.relation == context.owner_relation)
        .map(|origin| origin.observable)
        .filter(|observable| !visible_owner.contains(observable))
        .collect::<BTreeSet<_>>();
    add_projection_obligations(
        &mut analysis.obligations,
        context,
        &visible_owner,
        hidden_owner,
    );
    // A duplicate-free projection that retains every input column is a pure
    // coordinate permutation. It is injective on rows under both Set and Bag
    // semantics, so there is no collapse/complement obligation to discharge.
    if columns.len() != previous.len() {
        analysis
            .obligations
            .insert(RelWritableObligation::ProjectionNoSemanticCollapse);
    }
    analysis.output_origins = projected;
    analysis.stages.push(RelRewriteLiftStage::Project {
        columns: columns.to_vec(),
    });
    Ok(analysis)
}

fn add_projection_obligations(
    obligations: &mut BTreeSet<RelWritableObligation>,
    context: &RelWritableCompileContext<'_>,
    visible_owner: &BTreeSet<RevisionObservableId>,
    hidden_owner: BTreeSet<RevisionObservableId>,
) {
    if hidden_owner.is_empty()
        || determinant_covers(context.determinant_theory, visible_owner, &hidden_owner)
            .unwrap_or(false)
    {
        return;
    }
    obligations.insert(RelWritableObligation::PreserveHiddenColumnsComplement {
        relation: context.owner_relation,
        hidden: hidden_owner.clone(),
    });
    obligations.insert(RelWritableObligation::HiddenColumnConstructorForInsert {
        relation: context.owner_relation,
        hidden: hidden_owner,
    });
}

fn analyze_rel_filter_const(
    input: &kernel_query::RelExpr,
    column: usize,
    equivalence: SemanticId,
    context: &RelWritableCompileContext<'_>,
) -> Result<RelWritableAnalysis, RelWritabilityFailure> {
    let mut analysis = analyze_writable_rel(input, context)?;
    ensure_column(column, analysis.output_origins.len())?;
    add_guard_obligations(&mut analysis.obligations);
    analysis.stages.push(RelRewriteLiftStage::FilterEqConst {
        column,
        equivalence,
    });
    Ok(analysis)
}

fn analyze_rel_filter_columns(
    input: &kernel_query::RelExpr,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    context: &RelWritableCompileContext<'_>,
) -> Result<RelWritableAnalysis, RelWritabilityFailure> {
    let mut analysis = analyze_writable_rel(input, context)?;
    ensure_column(left_column, analysis.output_origins.len())?;
    ensure_column(right_column, analysis.output_origins.len())?;
    add_guard_obligations(&mut analysis.obligations);
    analysis.stages.push(RelRewriteLiftStage::FilterEqColumns {
        left_column,
        right_column,
        equivalence,
    });
    Ok(analysis)
}

fn analyze_rel_join(
    left: &kernel_query::RelExpr,
    right: &kernel_query::RelExpr,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    context: &RelWritableCompileContext<'_>,
) -> Result<RelWritableAnalysis, RelWritabilityFailure> {
    let left_analysis = analyze_writable_rel(left, context)?;
    let right_analysis = analyze_writable_rel(right, context)?;
    if left_analysis.owner_present && right_analysis.owner_present {
        return Err(RelWritabilityFailure::OwnerRelationReferencedOnBothJoinSides);
    }
    if !left_analysis.owner_present && !right_analysis.owner_present {
        return Err(RelWritabilityFailure::OwnerRelationNotReferenced);
    }
    let (mut owner, lookup, owner_column, lookup_column, owner_is_left) =
        if left_analysis.owner_present {
            (
                left_analysis,
                right_analysis,
                left_column,
                right_column,
                true,
            )
        } else {
            (
                right_analysis,
                left_analysis,
                right_column,
                left_column,
                false,
            )
        };
    ensure_column(owner_column, owner.output_origins.len())?;
    ensure_column(lookup_column, lookup.output_origins.len())?;
    add_join_obligations(&mut owner.obligations, &lookup, lookup_column, context);
    let output_origins = join_output_origins(&owner, &lookup, owner_is_left);
    owner.obligations.extend(lookup.obligations);
    owner
        .obligations
        .insert(RelWritableObligation::DtcGuardNoImpact);
    owner
        .obligations
        .insert(RelWritableObligation::VmfInvariantClosure);
    owner.output_origins = output_origins;
    owner.stages.push(RelRewriteLiftStage::JoinOwnerSide {
        owner_is_left,
        owner_column,
        lookup_column,
        equivalence,
    });
    Ok(owner)
}

fn add_join_obligations(
    obligations: &mut BTreeSet<RelWritableObligation>,
    lookup: &RelWritableAnalysis,
    lookup_column: usize,
    context: &RelWritableCompileContext<'_>,
) {
    let lookup_key = lookup.output_origins[lookup_column].observable;
    let lookup_outputs = lookup
        .output_origins
        .iter()
        .map(|origin| origin.observable)
        .collect::<BTreeSet<_>>();
    if determinant_covers(
        context.determinant_theory,
        &BTreeSet::from([lookup_key]),
        &lookup_outputs,
    )
    .unwrap_or(false)
    {
        return;
    }
    obligations.insert(RelWritableObligation::JoinLookupDeterminant {
        join_key: lookup_key,
        lookup_outputs,
    });
}

fn join_output_origins(
    owner: &RelWritableAnalysis,
    lookup: &RelWritableAnalysis,
    owner_is_left: bool,
) -> Vec<RelColumnOrigin> {
    let mut outputs = if owner_is_left {
        owner.output_origins.clone()
    } else {
        lookup.output_origins.clone()
    };
    if owner_is_left {
        outputs.extend(lookup.output_origins.iter().copied());
    } else {
        outputs.extend(owner.output_origins.iter().copied());
    }
    outputs
}

fn add_guard_obligations(obligations: &mut BTreeSet<RelWritableObligation>) {
    obligations.insert(RelWritableObligation::PredicateAdmissibility);
    obligations.insert(RelWritableObligation::DtcGuardNoImpact);
    obligations.insert(RelWritableObligation::VmfInvariantClosure);
}

fn ensure_column(column: usize, arity: usize) -> Result<(), RelWritabilityFailure> {
    if column < arity {
        Ok(())
    } else {
        Err(RelWritabilityFailure::ColumnOutOfBounds { column, arity })
    }
}

fn determinant_covers(
    theory: Option<&kernel_semantics::anchor_pullback::DeterminantTheory>,
    seed: &BTreeSet<RevisionObservableId>,
    targets: &BTreeSet<RevisionObservableId>,
) -> Result<bool, kernel_semantics::anchor_pullback::AnchorPullbackError> {
    let Some(theory) = theory else {
        return Ok(false);
    };
    Ok(theory.closure(seed)?.is_superset(targets))
}

#[cfg(test)]
mod writable_tests {
    use super::*;
    use kernel_change::{RewriteLawSetId, RewriteSpecId};
    use kernel_query::{ExactQuery, Expr};

    #[test]
    fn identity_and_product_field_chain_compile_to_writable_plan() {
        let query = ExactQuery::new(Expr::ProductField {
            input: Box::new(Expr::Input),
            field: SemanticId(2),
        });
        let spec = RewriteSpecId(SemanticId(200));
        let WritableCompilation::Writable(plan) = compile_writable_query(&query, spec) else {
            panic!("product field must be structurally writable");
        };
        assert_eq!(plan.rewrite_spec, spec);
        let source = source_for_writable_test();
        let rewrite: PreparedRewrite<Value, Value> = PreparedRewrite {
            spec,
            explicit_inputs: vec![],
            effect: RewriteEffect::Fine(FineChange::new(FineChangeKind::Scalar, Value::I64(99))),
            law_set: RewriteLawSetId(SemanticId(201)),
        };
        let lifted = plan.lift_rewrite(&source, &rewrite).unwrap();
        let updated = lifted.apply(&source);
        assert_eq!(query.evaluate(&updated), Ok(Value::I64(99)));
    }

    #[test]
    fn derived_query_is_read_only_instead_of_guessing_a_preimage() {
        let query = ExactQuery::new(Expr::SeqLength(Box::new(Expr::Input)));
        assert_eq!(
            compile_writable_query(&query, RewriteSpecId(SemanticId(210))),
            WritableCompilation::ReadOnly(WritabilityFailure::DerivedOperatorHasNoCertifiedLift)
        );
    }

    #[test]
    fn writable_plan_rejects_wrong_rewrite_family() {
        let query = ExactQuery::new(Expr::Input);
        let WritableCompilation::Writable(plan) =
            compile_writable_query(&query, RewriteSpecId(SemanticId(220)))
        else {
            unreachable!();
        };
        let source = Value::I64(1);
        let rewrite = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(221)),
            explicit_inputs: Vec::<Value>::new(),
            effect: RewriteEffect::Replace(Value::I64(2)),
            law_set: RewriteLawSetId(SemanticId(222)),
        };
        assert!(matches!(
            plan.lift_rewrite(&source, &rewrite),
            Err(WritableExecutionError::RewriteFamilyMismatch { .. })
        ));
    }

    fn source_for_writable_test() -> Value {
        Value::Product(BTreeMap::from([
            (SemanticId(1), Value::I64(10)),
            (SemanticId(2), Value::I64(20)),
            (SemanticId(3), Value::I64(30)),
        ]))
    }
}

#[cfg(test)]
mod relational_writable_tests {
    use super::*;
    use kernel_change::RewriteSpecId;
    use kernel_query::RelExpr;

    const OWNER: SemanticId = SemanticId(800);
    const LOOKUP: SemanticId = SemanticId(801);
    const EQ: SemanticId = SemanticId(802);

    fn observable(id: u128) -> RevisionObservableId {
        RevisionObservableId::new(id)
    }

    fn bindings() -> BTreeMap<SemanticId, Vec<RevisionObservableId>> {
        BTreeMap::from([
            (OWNER, vec![observable(1), observable(2), observable(3)]),
            (LOOKUP, vec![observable(10), observable(11)]),
        ])
    }

    fn context(bindings: &RelWritableColumnBindings) -> RelWritableCompileContext<'_> {
        RelWritableCompileContext {
            owner_relation: OWNER,
            rewrite_spec: RewriteSpecId(SemanticId(900)),
            relation_columns: bindings,
            determinant_theory: None,
        }
    }

    #[test]
    fn scan_owner_is_unconditionally_writable() {
        let bindings = RelWritableColumnBindings {
            columns: bindings(),
        };
        let compiled =
            compile_rel_writable_query(&RelExpr::Scan(OWNER), &context(&bindings)).unwrap();
        let RelWritableCompilation::Writable(plan) = compiled else {
            panic!("owner scan must preserve identity without extra obligations")
        };
        assert_eq!(plan.output_origins.len(), 3);
        assert!(plan.stages.is_empty());
    }

    #[test]
    fn projection_filter_exposes_hidden_and_dynamic_obligations() {
        let bindings = RelWritableColumnBindings {
            columns: bindings(),
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(OWNER)),
                column: 0,
                value: Value::I64(7),
                equivalence: EQ,
            }),
            columns: vec![0, 1],
        };
        let RelWritableCompilation::Conditional { plan, obligations } =
            compile_rel_writable_query(&query, &context(&bindings)).unwrap()
        else {
            panic!("lossy filtered projection must be conditional")
        };
        assert_eq!(plan.output_origins.len(), 2);
        assert!(obligations.contains(&RelWritableObligation::PredicateAdmissibility));
        assert!(obligations.contains(&RelWritableObligation::DtcGuardNoImpact));
        assert!(obligations.contains(&RelWritableObligation::VmfInvariantClosure));
        assert!(obligations.contains(&RelWritableObligation::ProjectionNoSemanticCollapse));
        assert!(
            obligations.contains(&RelWritableObligation::PreserveHiddenColumnsComplement {
                relation: OWNER,
                hidden: BTreeSet::from([observable(3)]),
            })
        );
        assert!(
            obligations.contains(&RelWritableObligation::HiddenColumnConstructorForInsert {
                relation: OWNER,
                hidden: BTreeSet::from([observable(3)]),
            })
        );
    }

    #[test]
    fn full_column_projection_permutation_is_unconditionally_writable() {
        let bindings = RelWritableColumnBindings {
            columns: bindings(),
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(OWNER)),
            columns: vec![2, 0, 1],
        };
        let compiled = compile_rel_writable_query(&query, &context(&bindings)).unwrap();
        let RelWritableCompilation::Writable(plan) = compiled else {
            panic!("full-column permutation must be a lossless writable projection")
        };
        assert!(plan.required_obligations.is_empty());
        assert_eq!(
            plan.stages,
            vec![RelRewriteLiftStage::Project {
                columns: vec![2, 0, 1],
            }]
        );
    }

    #[test]
    fn owner_side_join_never_guesses_lookup_preimage() {
        let bindings = RelWritableColumnBindings {
            columns: bindings(),
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(OWNER)),
            right: Box::new(RelExpr::Scan(LOOKUP)),
            left_column: 0,
            right_column: 0,
            equivalence: EQ,
        };
        let RelWritableCompilation::Conditional { plan, obligations } =
            compile_rel_writable_query(&query, &context(&bindings)).unwrap()
        else {
            panic!("join without determinant certificate must be conditional")
        };
        assert_eq!(plan.output_origins.len(), 5);
        assert!(
            obligations.contains(&RelWritableObligation::JoinLookupDeterminant {
                join_key: observable(10),
                lookup_outputs: BTreeSet::from([observable(10), observable(11)]),
            })
        );
        assert!(matches!(
            plan.stages.last(),
            Some(RelRewriteLiftStage::JoinOwnerSide {
                owner_is_left: true,
                ..
            })
        ));
    }

    #[test]
    fn join_with_owner_on_both_sides_is_read_only() {
        let bindings = RelWritableColumnBindings {
            columns: bindings(),
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(OWNER)),
            right: Box::new(RelExpr::Scan(OWNER)),
            left_column: 0,
            right_column: 0,
            equivalence: EQ,
        };
        assert_eq!(
            compile_rel_writable_query(&query, &context(&bindings)).unwrap(),
            RelWritableCompilation::ReadOnly(
                RelWritabilityFailure::OwnerRelationReferencedOnBothJoinSides
            )
        );
    }

    #[test]
    fn distinct_requires_explicit_action_policy() {
        let bindings = RelWritableColumnBindings {
            columns: bindings(),
        };
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(OWNER)),
            column_equivalences: vec![EQ, EQ, EQ],
        };
        assert_eq!(
            compile_rel_writable_query(&query, &context(&bindings)).unwrap(),
            RelWritableCompilation::ReadOnly(RelWritabilityFailure::UnsupportedOperator)
        );
    }

    fn lift_runtime() -> (
        kernel_schema::SemanticContext,
        kernel_semantics::SemanticRegistry,
        kernel_model::FiniteModel,
        RelWritableViewPlan,
        kernel_change::RewriteSpec,
    ) {
        use kernel_schema::{
            RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext,
            SemanticEnvironment, TypeExpr,
        };
        use kernel_semantics::{EquivalenceModule, SemanticRegistry};
        use kernel_types::{SchemaRevisionId, SemanticEnvId};

        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(EQ, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: OWNER,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![EQ],
                },
            })
            .unwrap();
        let semantic = SemanticContext {
            schema,
            environment,
        };
        let mut model = kernel_model::FiniteModel::default();
        model.relations.insert(OWNER, vec![vec![Value::I64(1)]]);
        let spec = kernel_change::RewriteSpec {
            id: RewriteSpecId(SemanticId(900)),
            law_set: kernel_change::RewriteLawSetId(SemanticId(901)),
            footprint: kernel_change::RewriteFootprint::default(),
        };
        let plan = RelWritableViewPlan {
            query: RelExpr::Scan(OWNER),
            owner_relation: OWNER,
            rewrite_spec: spec.id,
            output_origins: Vec::new(),
            stages: Vec::new(),
            required_obligations: BTreeSet::new(),
        };
        (semantic, registry, model, plan, spec)
    }

    #[test]
    fn identity_source_rewrite_synthesis_is_complete_and_delta_authoritative() {
        let (semantic, registry, model, plan, spec) = lift_runtime();
        let requested = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(996)),
            explicit_inputs: vec![Value::I64(2)],
            effect: RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![vec![
                Value::I64(2),
            ]])),
            law_set: kernel_change::RewriteLawSetId(SemanticId(997)),
        };
        let source = plan
            .synthesize_identity_source_rewrite(&model, &semantic, &registry, &spec, &requested)
            .unwrap();
        assert_eq!(source.rewrite.spec, spec.id);
        assert_eq!(source.rewrite.law_set, spec.law_set);
        assert_eq!(source.rewrite.explicit_inputs, vec![Value::I64(2)]);
        assert_eq!(source.delta.removed, vec![vec![Value::I64(1)]]);
        assert_eq!(source.delta.inserted, vec![vec![Value::I64(2)]]);
    }

    #[test]
    fn conditional_plan_cannot_synthesize_before_obligations_are_discharged() {
        let bindings = RelWritableColumnBindings {
            columns: bindings(),
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(OWNER)),
            columns: vec![0],
        };
        let RelWritableCompilation::Conditional { plan, obligations } =
            compile_rel_writable_query(&query, &context(&bindings)).unwrap()
        else {
            panic!("lossy projection must remain conditional")
        };
        let (semantic, registry, model, _, spec) = lift_runtime();
        let requested = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(998)),
            explicit_inputs: Vec::<Value>::new(),
            effect: RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![])),
            law_set: kernel_change::RewriteLawSetId(SemanticId(999)),
        };
        assert_eq!(
            plan.synthesize_identity_source_rewrite(
                &model, &semantic, &registry, &spec, &requested,
            ),
            Err(RelRewriteLiftError::UnresolvedObligations(obligations))
        );
    }

    #[test]
    fn finite_lift_classifier_returns_unique_exact_preimage() {
        let (semantic, registry, model, plan, spec) = lift_runtime();
        let old = RelExpr::Scan(OWNER)
            .evaluate(&model, &semantic, &registry)
            .unwrap();
        let delta = kernel_query::RelationDelta {
            inserted: vec![vec![Value::I64(2)]],
            removed: vec![vec![Value::I64(1)]],
            result_type: RelExpr::Scan(OWNER)
                .typecheck(&semantic, &registry)
                .unwrap(),
        };
        let candidate = delta
            .prepare_relation_rewrite(&old, &semantic, &registry, &spec, vec![Value::I64(2)])
            .unwrap();
        let requested = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(990)),
            explicit_inputs: Vec::<Value>::new(),
            effect: RewriteEffect::Replace(candidate.rewrite.apply(&old)),
            law_set: kernel_change::RewriteLawSetId(SemanticId(991)),
        };
        assert_eq!(
            plan.classify_source_rewrite_candidates(
                &model,
                &semantic,
                &registry,
                &requested,
                [candidate.clone()],
            )
            .unwrap(),
            RelRewriteLiftClassification::Unique(Box::new(candidate))
        );
    }

    #[test]
    fn finite_lift_classifier_preserves_intent_ambiguity_at_same_endpoint() {
        let (semantic, registry, model, plan, spec) = lift_runtime();
        let old = RelExpr::Scan(OWNER)
            .evaluate(&model, &semantic, &registry)
            .unwrap();
        let delta = kernel_query::RelationDelta {
            inserted: vec![vec![Value::I64(2)]],
            removed: vec![vec![Value::I64(1)]],
            result_type: RelExpr::Scan(OWNER)
                .typecheck(&semantic, &registry)
                .unwrap(),
        };
        let first = delta
            .prepare_relation_rewrite(&old, &semantic, &registry, &spec, vec![Value::I64(10)])
            .unwrap();
        let second = delta
            .prepare_relation_rewrite(&old, &semantic, &registry, &spec, vec![Value::I64(20)])
            .unwrap();
        let requested = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(992)),
            explicit_inputs: Vec::<Value>::new(),
            effect: RewriteEffect::Replace(first.rewrite.apply(&old)),
            law_set: kernel_change::RewriteLawSetId(SemanticId(993)),
        };
        assert!(matches!(
            plan.classify_source_rewrite_candidates(
                &model,
                &semantic,
                &registry,
                &requested,
                [first, second],
            )
            .unwrap(),
            RelRewriteLiftClassification::Ambiguous { .. }
        ));
    }

    #[test]
    fn finite_lift_classifier_rejects_forged_delta_effect_pair() {
        let (semantic, registry, model, plan, spec) = lift_runtime();
        let old = RelExpr::Scan(OWNER)
            .evaluate(&model, &semantic, &registry)
            .unwrap();
        let delta = kernel_query::RelationDelta {
            inserted: vec![vec![Value::I64(2)]],
            removed: vec![vec![Value::I64(1)]],
            result_type: RelExpr::Scan(OWNER)
                .typecheck(&semantic, &registry)
                .unwrap(),
        };
        let mut forged = delta
            .prepare_relation_rewrite(&old, &semantic, &registry, &spec, Vec::<Value>::new())
            .unwrap();
        forged.rewrite.effect = RewriteEffect::Replace(old.clone());
        let requested = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(994)),
            explicit_inputs: Vec::<Value>::new(),
            effect: RewriteEffect::Replace(old),
            law_set: kernel_change::RewriteLawSetId(SemanticId(995)),
        };
        assert_eq!(
            plan.classify_source_rewrite_candidates(
                &model,
                &semantic,
                &registry,
                &requested,
                [forged],
            ),
            Err(RelRewriteLiftError::CandidateEffectMismatch)
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LensSpecId(pub SemanticId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticManifestId(pub SemanticId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArchiveProofId(pub SemanticId);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplementCapsule {
    pub source_schema: kernel_types::SchemaRevisionId,
    pub target_schema: kernel_types::SchemaRevisionId,
    pub lens_spec: LensSpecId,
    pub semantic_pins: SemanticManifestId,
    pub encoding_version: u32,
    pub complement: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplementRetention {
    Forever,
    UntilRevision(kernel_types::RevisionId),
    UntilEpoch(u64),
    ExternalArchive(ArchiveProofId),
    Forget,
}

impl ComplementRetention {
    #[must_use]
    pub const fn requires_local_storage(self) -> bool {
        matches!(
            self,
            Self::Forever | Self::UntilRevision(_) | Self::UntilEpoch(_)
        )
    }

    #[must_use]
    pub const fn is_explicit_forget(self) -> bool {
        matches!(self, Self::Forget)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MigrationComplementChain {
    steps: Vec<(ComplementCapsule, ComplementRetention)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationChainError {
    SchemaDiscontinuity,
}

impl MigrationComplementChain {
    pub fn push(
        &mut self,
        capsule: ComplementCapsule,
        retention: ComplementRetention,
    ) -> Result<(), MigrationChainError> {
        if let Some((previous, _)) = self.steps.last()
            && previous.target_schema != capsule.source_schema
        {
            return Err(MigrationChainError::SchemaDiscontinuity);
        }
        self.steps.push((capsule, retention));
        Ok(())
    }

    #[must_use]
    pub fn steps(&self) -> &[(ComplementCapsule, ComplementRetention)] {
        &self.steps
    }
}

#[cfg(test)]
mod migration_complement_tests {
    use super::*;
    use kernel_types::{RevisionId, SchemaRevisionId};

    fn capsule(source: u64, target: u64, payload: i64) -> ComplementCapsule {
        ComplementCapsule {
            source_schema: SchemaRevisionId(source),
            target_schema: SchemaRevisionId(target),
            lens_spec: LensSpecId(SemanticId(300)),
            semantic_pins: SemanticManifestId(SemanticId(301)),
            encoding_version: 1,
            complement: Value::I64(payload),
        }
    }

    #[test]
    fn migration_chain_requires_schema_continuity() {
        let mut chain = MigrationComplementChain::default();
        chain
            .push(capsule(1, 2, 10), ComplementRetention::Forever)
            .unwrap();
        assert_eq!(
            chain.push(capsule(3, 4, 20), ComplementRetention::Forever),
            Err(MigrationChainError::SchemaDiscontinuity)
        );
        chain
            .push(
                capsule(2, 3, 30),
                ComplementRetention::UntilRevision(RevisionId(9)),
            )
            .unwrap();
        assert_eq!(chain.steps().len(), 2);
    }

    #[test]
    fn forget_and_external_archive_are_explicit_nonlocal_retention() {
        assert!(ComplementRetention::Forget.is_explicit_forget());
        assert!(!ComplementRetention::Forget.requires_local_storage());
        assert!(
            !ComplementRetention::ExternalArchive(ArchiveProofId(SemanticId(1)))
                .requires_local_storage()
        );
        assert!(ComplementRetention::Forever.requires_local_storage());
    }
}
