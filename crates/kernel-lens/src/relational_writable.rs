use std::collections::{BTreeMap, BTreeSet};

use kernel_change::PreparedRewrite;
#[cfg(test)]
use kernel_change::RewriteEffect;
#[cfg(test)]
use kernel_model::Value;
use kernel_types::{RevisionObservableId, SemanticId};

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
            self.owner_relation,
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
            if candidate.rewrite().spec() != self.rewrite_spec {
                return Err(RelRewriteLiftError::CandidateRewriteSpecMismatch {
                    expected: self.rewrite_spec,
                    actual: candidate.rewrite().spec(),
                });
            }
            if candidate.delta().result_type != owner_type {
                return Err(RelRewriteLiftError::CandidateDeltaTypeMismatch);
            }
            let delta_endpoint = candidate.apply_structural(&old_owner, registry)?;

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

#[derive(Debug)]
enum RelWritableAnalysisError {
    Writability(RelWritabilityFailure),
    Determinant(kernel_semantics::anchor_pullback::AnchorPullbackError),
}

impl From<RelWritabilityFailure> for RelWritableAnalysisError {
    fn from(value: RelWritabilityFailure) -> Self {
        Self::Writability(value)
    }
}

impl From<kernel_semantics::anchor_pullback::AnchorPullbackError> for RelWritableAnalysisError {
    fn from(value: kernel_semantics::anchor_pullback::AnchorPullbackError) -> Self {
        Self::Determinant(value)
    }
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
        Err(RelWritableAnalysisError::Writability(reason)) => {
            return Ok(RelWritableCompilation::ReadOnly(reason));
        }
        Err(RelWritableAnalysisError::Determinant(error)) => return Err(error),
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
) -> Result<RelWritableAnalysis, RelWritableAnalysisError> {
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
        RelExpr::FilterOrderConst { .. }
        | RelExpr::Difference { .. }
        | RelExpr::AntiJoin { .. }
        | RelExpr::Distinct { .. }
        | RelExpr::Group { .. }
        | RelExpr::TopKWithTies { .. }
        | RelExpr::PromoteToBag(_) => Err(RelWritabilityFailure::UnsupportedOperator.into()),
    }
}

fn analyze_rel_scan(
    relation: SemanticId,
    context: &RelWritableCompileContext<'_>,
) -> Result<RelWritableAnalysis, RelWritableAnalysisError> {
    let Some(columns) = context.relation_columns.columns.get(&relation) else {
        return Err(RelWritabilityFailure::MissingObservableBinding { relation }.into());
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
) -> Result<RelWritableAnalysis, RelWritableAnalysisError> {
    let mut analysis = analyze_writable_rel(input, context)?;
    let mut seen = BTreeSet::new();
    for &column in columns {
        ensure_column(column, analysis.output_origins.len())?;
        if !seen.insert(column) {
            return Err(RelWritabilityFailure::DuplicateProjectionColumn(column).into());
        }
    }
    let previous = analysis.output_origins.clone();
    let projected = columns
        .iter()
        .map(|&column| previous[column])
        .collect::<Vec<_>>();
    if analysis.owner_present {
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
        )?;
        // A duplicate-free projection that retains every input column is a pure
        // coordinate permutation. It is injective on rows under both Set and Bag
        // semantics, so there is no collapse/complement obligation to discharge.
        if columns.len() != previous.len() {
            analysis
                .obligations
                .insert(RelWritableObligation::ProjectionNoSemanticCollapse);
        }
        analysis.stages.push(RelRewriteLiftStage::Project {
            columns: columns.to_vec(),
        });
    }
    analysis.output_origins = projected;
    Ok(analysis)
}

fn add_projection_obligations(
    obligations: &mut BTreeSet<RelWritableObligation>,
    context: &RelWritableCompileContext<'_>,
    visible_owner: &BTreeSet<RevisionObservableId>,
    hidden_owner: BTreeSet<RevisionObservableId>,
) -> Result<(), kernel_semantics::anchor_pullback::AnchorPullbackError> {
    if hidden_owner.is_empty()
        || determinant_covers(context.determinant_theory, visible_owner, &hidden_owner)?
    {
        return Ok(());
    }
    obligations.insert(RelWritableObligation::PreserveHiddenColumnsComplement {
        relation: context.owner_relation,
        hidden: hidden_owner.clone(),
    });
    obligations.insert(RelWritableObligation::HiddenColumnConstructorForInsert {
        relation: context.owner_relation,
        hidden: hidden_owner,
    });
    Ok(())
}

fn analyze_rel_filter_const(
    input: &kernel_query::RelExpr,
    column: usize,
    equivalence: SemanticId,
    context: &RelWritableCompileContext<'_>,
) -> Result<RelWritableAnalysis, RelWritableAnalysisError> {
    let mut analysis = analyze_writable_rel(input, context)?;
    ensure_column(column, analysis.output_origins.len())?;
    if analysis.owner_present {
        add_guard_obligations(&mut analysis.obligations);
        analysis.stages.push(RelRewriteLiftStage::FilterEqConst {
            column,
            equivalence,
        });
    }
    Ok(analysis)
}

fn analyze_rel_filter_columns(
    input: &kernel_query::RelExpr,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    context: &RelWritableCompileContext<'_>,
) -> Result<RelWritableAnalysis, RelWritableAnalysisError> {
    let mut analysis = analyze_writable_rel(input, context)?;
    ensure_column(left_column, analysis.output_origins.len())?;
    ensure_column(right_column, analysis.output_origins.len())?;
    if analysis.owner_present {
        add_guard_obligations(&mut analysis.obligations);
        analysis.stages.push(RelRewriteLiftStage::FilterEqColumns {
            left_column,
            right_column,
            equivalence,
        });
    }
    Ok(analysis)
}

fn analyze_rel_join(
    left: &kernel_query::RelExpr,
    right: &kernel_query::RelExpr,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    context: &RelWritableCompileContext<'_>,
) -> Result<RelWritableAnalysis, RelWritableAnalysisError> {
    let left_analysis = analyze_writable_rel(left, context)?;
    let right_analysis = analyze_writable_rel(right, context)?;
    if left_analysis.owner_present && right_analysis.owner_present {
        return Err(RelWritabilityFailure::OwnerRelationReferencedOnBothJoinSides.into());
    }
    if !left_analysis.owner_present && !right_analysis.owner_present {
        ensure_column(left_column, left_analysis.output_origins.len())?;
        ensure_column(right_column, right_analysis.output_origins.len())?;
        let mut output_origins = left_analysis.output_origins;
        output_origins.extend(right_analysis.output_origins);
        return Ok(RelWritableAnalysis {
            owner_present: false,
            output_origins,
            stages: Vec::new(),
            obligations: BTreeSet::new(),
        });
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
    add_join_obligations(&mut owner.obligations, &lookup, lookup_column, context)?;
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
) -> Result<(), kernel_semantics::anchor_pullback::AnchorPullbackError> {
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
    )? {
        return Ok(());
    }
    obligations.insert(RelWritableObligation::JoinLookupDeterminant {
        join_key: lookup_key,
        lookup_outputs,
    });
    Ok(())
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
    if seed.is_superset(targets) {
        return Ok(true);
    }
    let Some(theory) = theory else {
        return Ok(false);
    };
    Ok(theory.closure(seed)?.is_superset(targets))
}

#[cfg(test)]
mod relational_writable_tests {
    use super::*;
    use kernel_change::RewriteSpecId;
    use kernel_query::RelExpr;

    const OWNER: SemanticId = SemanticId(800);
    const LOOKUP: SemanticId = SemanticId(801);
    const EQ: SemanticId = SemanticId(802);
    const LOOKUP_AUX: SemanticId = SemanticId(803);

    fn observable(id: u128) -> RevisionObservableId {
        RevisionObservableId::new(id)
    }

    fn bindings() -> BTreeMap<SemanticId, Vec<RevisionObservableId>> {
        BTreeMap::from([
            (OWNER, vec![observable(1), observable(2), observable(3)]),
            (LOOKUP, vec![observable(10), observable(11)]),
            (LOOKUP_AUX, vec![observable(20)]),
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
    fn determinant_coordinate_failure_is_not_downgraded_to_conditional_writability() {
        use kernel_schema::{Schema, SemanticContext, SemanticEnvironment};
        use kernel_semantics::{EquivalenceModule, SemanticRegistry};
        use kernel_types::{SchemaRevisionId, SemanticEnvId};

        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(80));
        environment.pin_module(EQ, digest);
        let semantic = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(80)),
            environment,
        };
        let mut catalog =
            kernel_semantics::observable::RevisionObservableCatalog::new(&semantic).unwrap();
        let first = catalog
            .register_equivalence(&registry, &semantic, EQ)
            .unwrap();
        let second = catalog
            .register_equivalence_coordinate(&registry, &semantic, EQ, 1)
            .unwrap();
        let third = catalog
            .register_equivalence_coordinate(&registry, &semantic, EQ, 2)
            .unwrap();
        let theory = kernel_semantics::anchor_pullback::DeterminantTheory::new(
            &catalog,
            [first],
            Vec::new(),
        )
        .unwrap();
        let bindings = RelWritableColumnBindings {
            columns: BTreeMap::from([(OWNER, vec![first, second, third])]),
        };
        let compile_context = RelWritableCompileContext {
            owner_relation: OWNER,
            rewrite_spec: RewriteSpecId(SemanticId(900)),
            relation_columns: &bindings,
            determinant_theory: Some(&theory),
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(OWNER)),
            columns: vec![0, 1],
        };
        assert_eq!(
            compile_rel_writable_query(&query, &compile_context),
            Err(kernel_semantics::anchor_pullback::AnchorPullbackError::UnknownCoordinate(second))
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
    fn lookup_projection_does_not_create_owner_preimage_obligations() {
        let bindings = RelWritableColumnBindings {
            columns: bindings(),
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(OWNER)),
            right: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Scan(LOOKUP)),
                columns: vec![0],
            }),
            left_column: 0,
            right_column: 0,
            equivalence: EQ,
        };
        let RelWritableCompilation::Conditional { plan, obligations } =
            compile_rel_writable_query(&query, &context(&bindings)).unwrap()
        else {
            panic!("join guards remain conditional")
        };
        assert!(!obligations.contains(&RelWritableObligation::ProjectionNoSemanticCollapse));
        assert!(!obligations.contains(&RelWritableObligation::PredicateAdmissibility));
        assert!(!obligations.iter().any(|obligation| matches!(
            obligation,
            RelWritableObligation::JoinLookupDeterminant { .. }
        )));
        assert_eq!(plan.stages.len(), 1);
        assert!(matches!(
            plan.stages[0],
            RelRewriteLiftStage::JoinOwnerSide { .. }
        ));
    }

    #[test]
    fn lookup_only_join_subtree_is_observational_not_a_second_write_owner() {
        let bindings = RelWritableColumnBindings {
            columns: bindings(),
        };
        let lookup = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(LOOKUP)),
            right: Box::new(RelExpr::Scan(LOOKUP_AUX)),
            left_column: 0,
            right_column: 0,
            equivalence: EQ,
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(OWNER)),
            right: Box::new(lookup),
            left_column: 0,
            right_column: 0,
            equivalence: EQ,
        };
        let compiled = compile_rel_writable_query(&query, &context(&bindings)).unwrap();
        let plan = match compiled {
            RelWritableCompilation::Writable(plan)
            | RelWritableCompilation::Conditional { plan, .. } => plan,
            RelWritableCompilation::ReadOnly(reason) => {
                panic!("lookup-only join subtree must remain readable: {reason:?}")
            }
        };
        assert_eq!(plan.output_origins.len(), 6);
        assert_eq!(plan.stages.len(), 1);
        assert!(matches!(
            plan.stages[0],
            RelRewriteLiftStage::JoinOwnerSide { .. }
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
            footprint: kernel_change::RewriteFootprint::opaque_relation(OWNER),
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
        assert_eq!(source.rewrite().spec(), spec.id);
        assert_eq!(source.rewrite().law_set(), spec.law_set);
        assert_eq!(source.rewrite().explicit_inputs(), vec![Value::I64(2)]);
        assert_eq!(source.delta().removed, vec![vec![Value::I64(1)]]);
        assert_eq!(source.delta().inserted, vec![vec![Value::I64(2)]]);
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
            .prepare_relation_rewrite(
                OWNER,
                &old,
                &semantic,
                &registry,
                &spec,
                vec![Value::I64(2)],
            )
            .unwrap();
        let requested = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(990)),
            explicit_inputs: Vec::<Value>::new(),
            effect: RewriteEffect::Replace(candidate.apply_structural(&old, &registry).unwrap()),
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
            .prepare_relation_rewrite(
                OWNER,
                &old,
                &semantic,
                &registry,
                &spec,
                vec![Value::I64(10)],
            )
            .unwrap();
        let second = delta
            .prepare_relation_rewrite(
                OWNER,
                &old,
                &semantic,
                &registry,
                &spec,
                vec![Value::I64(20)],
            )
            .unwrap();
        let requested = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(992)),
            explicit_inputs: Vec::<Value>::new(),
            effect: RewriteEffect::Replace(first.apply_structural(&old, &registry).unwrap()),
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
    fn finite_lift_classifier_rejects_candidate_prepared_against_wrong_gamma_base() {
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
        let wrong_old =
            kernel_query::RelationValue::Bag(vec![vec![Value::I64(1)], vec![Value::I64(99)]]);
        let candidate = delta
            .prepare_relation_rewrite(
                OWNER,
                &wrong_old,
                &semantic,
                &registry,
                &spec,
                Vec::<Value>::new(),
            )
            .unwrap();
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
                [candidate],
            ),
            Err(RelRewriteLiftError::Query(
                kernel_query::RelQueryError::StructuralRewriteBaseMismatch,
            ))
        );
    }
}
