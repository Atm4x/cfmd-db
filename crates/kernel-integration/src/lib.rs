mod certification;
mod join;
mod preimage;
mod projection;

use certification::{OwnerCertificationContext, certify_owner_candidate};
use join::{
    PreparedDirectJoinLift,
    synthesize_direct_scan_join_source_rewrite_with_constructor_for_commit_prepared,
};
#[cfg(test)]
use join::{
    synthesize_direct_scan_join_source_rewrite_for_commit,
    synthesize_direct_scan_join_source_rewrite_with_constructor_for_commit,
    validate_direct_lookup_key_uniqueness,
};
use kernel_change::PreparedRewrite;
use kernel_lens::{
    RelRewriteLiftError, RelRewriteLiftStage, RelWritableBindingError, RelWritableCompilation,
    RelWritableCompileContext, RelWritableObligation, RelWritableViewPlan,
};
use kernel_plan::{
    DerivedRelationRewriteTransitionRequest, DurableRuntime, DurableRuntimeCommitError,
    DurableRuntimeCommitOutcome, PreparedPlan, PreparedRelWritableCoordinates,
    RelationDeterminantColumns, RevisionRelationRewrite, WritableCoordinatePrepareError,
};
use kernel_query::RelationValue;
use kernel_types::{ClientTransactionId, RevisionId};
use preimage::GammaPreimageCatalog;
use projection::PreparedProjectionPath;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub enum WritableViewCommitError {
    Lift(RelRewriteLiftError),
    Runtime(DurableRuntimeCommitError),
}

impl From<RelRewriteLiftError> for WritableViewCommitError {
    fn from(value: RelRewriteLiftError) -> Self {
        Self::Lift(value)
    }
}

impl From<DurableRuntimeCommitError> for WritableViewCommitError {
    fn from(value: DurableRuntimeCommitError) -> Self {
        Self::Runtime(value)
    }
}

#[derive(Debug)]
pub enum WritableViewCommitOutcome {
    Durable(DurableRuntimeCommitOutcome),
}

/// Explicit constructor for genuinely-new rows inserted through a lossy
/// projection. Only hidden owner columns are supplied here; visible columns
/// always come from the requested projected row. The constructor is pinned to
/// the same owner relation and Rewrite family as the writable plan so it cannot
/// silently act as a cross-view default policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedHiddenProjectInsertConstructor {
    pub owner_relation: kernel_types::SemanticId,
    pub rewrite_spec: kernel_change::RewriteSpecId,
    pub hidden_values: BTreeMap<usize, kernel_model::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedWritableCompilationError {
    Coordinates(WritableCoordinatePrepareError),
    Bindings(RelWritableBindingError),
    Determinants(kernel_semantics::anchor_pullback::AnchorPullbackError),
    Lift(RelRewriteLiftError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedWritableCompilation {
    coordinates: PreparedRelWritableCoordinates,
    compilation: RelWritableCompilation,
    strategy: PreparedRelWritableLiftStrategy,
}

impl PreparedWritableCompilation {
    #[must_use]
    pub const fn coordinates(&self) -> &PreparedRelWritableCoordinates {
        &self.coordinates
    }

    #[must_use]
    pub const fn compilation(&self) -> &RelWritableCompilation {
        &self.compilation
    }
}

fn prepared_writable_plan(
    compilation: &RelWritableCompilation,
) -> Result<&RelWritableViewPlan, RelRewriteLiftError> {
    match compilation {
        RelWritableCompilation::Writable(plan)
        | RelWritableCompilation::Conditional { plan, .. } => Ok(plan),
        RelWritableCompilation::ReadOnly(_) => {
            Err(RelRewriteLiftError::CandidateGenerationUnsupported)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedLiftBase {
    owner_query: kernel_query::PreparedRelExpr,
    view_query: kernel_query::PreparedRelExpr,
}

impl PreparedLiftBase {
    fn prepare(
        plan: &RelWritableViewPlan,
        semantic: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelRewriteLiftError> {
        Ok(Self {
            owner_query: kernel_query::RelExpr::Scan(plan.owner_relation)
                .prepare(semantic, registry)?,
            view_query: plan.query.prepare(semantic, registry)?,
        })
    }

    fn owner_type(&self) -> &kernel_query::RelType {
        self.owner_query.result_type()
    }

    fn view_type(&self) -> &kernel_query::RelType {
        self.view_query.result_type()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedIdentityLift {
    base: PreparedLiftBase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedProjectLift {
    base: PreparedLiftBase,
    projection: PreparedProjectionPath,
}

impl PreparedProjectLift {
    fn prepare(
        plan: &RelWritableViewPlan,
        semantic: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelRewriteLiftError> {
        let base = PreparedLiftBase::prepare(plan, semantic, registry)?;
        let projection = PreparedProjectionPath::from_output_origins(
            &plan.output_origins,
            plan.owner_relation,
            base.owner_type().columns.len(),
        )?;
        Ok(Self { base, projection })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedFilterLift {
    base: PreparedLiftBase,
    complement_query: kernel_query::PreparedRelExpr,
}

impl PreparedFilterLift {
    fn prepare(
        plan: &RelWritableViewPlan,
        semantic: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelRewriteLiftError> {
        let base = PreparedLiftBase::prepare(plan, semantic, registry)?;
        let complement_query = kernel_query::RelExpr::Difference {
            left: Box::new(kernel_query::RelExpr::Scan(plan.owner_relation)),
            right: Box::new(plan.query.clone()),
        }
        .prepare(semantic, registry)?;
        Ok(Self {
            base,
            complement_query,
        })
    }
}

fn synthesize_identity_source_rewrite_for_commit<I: Clone>(
    plan: &RelWritableViewPlan,
    prepared: &PreparedIdentityLift,
    source: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
) -> Result<kernel_query::PreparedRelationRewrite<I>, RelRewriteLiftError> {
    if source_spec.id != plan.rewrite_spec {
        return Err(RelRewriteLiftError::SourceRewriteSpecMismatch {
            expected: plan.rewrite_spec,
            actual: source_spec.id,
        });
    }
    let old_owner = prepared
        .base
        .owner_query
        .evaluate(source, semantic, registry)?;
    let requested_endpoint = requested_view.apply(&old_owner);
    let delta = kernel_query::RelationDelta::between_values(
        &old_owner,
        &requested_endpoint,
        prepared.base.owner_type().clone(),
        semantic,
        registry,
    )?;
    Ok(delta.prepare_relation_rewrite(
        plan.owner_relation,
        &old_owner,
        semantic,
        registry,
        source_spec,
        requested_view.explicit_inputs.clone(),
    )?)
}

/// Compiles a writable relational query directly from the same pinned
/// `PreparedPlan` that owns its executable semantics. Callers no longer
/// assemble relation-column observable IDs by hand.
pub fn compile_prepared_relational_writable_query(
    prepared: &PreparedPlan,
    owner_relation: kernel_types::SemanticId,
    rewrite_spec: kernel_change::RewriteSpecId,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<PreparedWritableCompilation, PreparedWritableCompilationError> {
    let coordinates = prepared
        .writable_coordinates(registry)
        .map_err(PreparedWritableCompilationError::Coordinates)?;
    let bindings = kernel_lens::RelWritableColumnBindings::from_catalog(
        prepared.semantic_context(),
        coordinates.catalog(),
        coordinates.relation_columns().clone(),
    )
    .map_err(PreparedWritableCompilationError::Bindings)?;
    let determinant_theory = coordinates
        .determinant_theory(Vec::new())
        .map_err(PreparedWritableCompilationError::Determinants)?;
    let context = RelWritableCompileContext {
        owner_relation,
        rewrite_spec,
        relation_columns: &bindings,
        determinant_theory: Some(&determinant_theory),
    };
    let compilation = kernel_lens::compile_rel_writable_query(prepared.logical(), &context)
        .map_err(PreparedWritableCompilationError::Determinants)?;
    let strategy = match &compilation {
        RelWritableCompilation::Writable(plan)
        | RelWritableCompilation::Conditional { plan, .. } => {
            prepare_rel_writable_lift_strategy(plan, prepared.semantic_context(), registry)
                .map_err(PreparedWritableCompilationError::Lift)?
        }
        RelWritableCompilation::ReadOnly(_) => PreparedRelWritableLiftStrategy::ReadOnly,
    };
    Ok(PreparedWritableCompilation {
        coordinates,
        compilation,
        strategy,
    })
}

fn synthesize_filter_source_rewrite_for_commit<I: Clone>(
    plan: &RelWritableViewPlan,
    prepared: &PreparedFilterLift,
    source: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
) -> Result<kernel_query::PreparedRelationRewrite<I>, RelRewriteLiftError> {
    if source_spec.id != plan.rewrite_spec {
        return Err(RelRewriteLiftError::SourceRewriteSpecMismatch {
            expected: plan.rewrite_spec,
            actual: source_spec.id,
        });
    }
    let old_owner = prepared
        .base
        .owner_query
        .evaluate(source, semantic, registry)?;
    let old_view = prepared
        .base
        .view_query
        .evaluate(source, semantic, registry)?;
    let requested_endpoint = requested_view.apply(&old_view);
    let complement = prepared
        .complement_query
        .evaluate(source, semantic, registry)?;
    let mut reconstructed_rows = requested_endpoint.rows().to_vec();
    reconstructed_rows.extend(complement.rows().iter().cloned());
    let reconstructed = match &prepared.base.owner_type().semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(reconstructed_rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows: reconstructed_rows,
            column_equivalences: column_equivalences.clone(),
        },
    };
    certify_owner_candidate(
        plan,
        requested_view,
        &old_owner,
        &requested_endpoint,
        &reconstructed,
        &OwnerCertificationContext {
            source,
            semantic,
            registry,
            source_spec,
            owner_type: prepared.base.owner_type(),
            view_query: &prepared.base.view_query,
        },
    )
}

fn synthesize_bijective_project_source_rewrite_for_commit<I: Clone>(
    plan: &RelWritableViewPlan,
    prepared: &PreparedProjectLift,
    source: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
) -> Result<kernel_query::PreparedRelationRewrite<I>, RelRewriteLiftError> {
    if source_spec.id != plan.rewrite_spec {
        return Err(RelRewriteLiftError::SourceRewriteSpecMismatch {
            expected: plan.rewrite_spec,
            actual: source_spec.id,
        });
    }
    let old_owner = prepared
        .base
        .owner_query
        .evaluate(source, semantic, registry)?;
    let old_view = prepared
        .base
        .view_query
        .evaluate(source, semantic, registry)?;
    let requested_endpoint = requested_view.apply(&old_view);
    let rows = requested_endpoint
        .rows()
        .iter()
        .map(|row| prepared.projection.invert_bijective_row(row))
        .collect::<Result<Vec<_>, _>>()?;

    let reconstructed = match &prepared.base.owner_type().semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.clone(),
        },
    };
    certify_owner_candidate(
        plan,
        requested_view,
        &old_owner,
        &requested_endpoint,
        &reconstructed,
        &OwnerCertificationContext {
            source,
            semantic,
            registry,
            source_spec,
            owner_type: prepared.base.owner_type(),
            view_query: &prepared.base.view_query,
        },
    )
}

fn reconstruct_lossy_project_deletion(
    old_owner: &RelationValue,
    owner_type: &kernel_query::RelType,
    view_type: &kernel_query::RelType,
    removed_rows: &[kernel_query::Row],
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    catalog: &mut GammaPreimageCatalog,
) -> Result<RelationValue, RelRewriteLiftError> {
    let view_is_set = matches!(
        view_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    let mut removed = vec![false; old_owner.rows().len()];
    for removed_view in removed_rows {
        let class =
            GammaPreimageCatalog::class_for_view_row(removed_view, view_type, semantic, registry)?;
        catalog.remove_requested(&class, &mut removed, view_is_set)?;
    }
    let remaining = old_owner
        .rows()
        .iter()
        .zip(removed)
        .filter(|(_, removed)| !*removed)
        .map(|(row, _)| row.clone())
        .collect::<Vec<_>>();
    Ok(match &owner_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(remaining),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows: remaining,
            column_equivalences: column_equivalences.clone(),
        },
    })
}

struct RuntimeProjectDeterminantContext<'a> {
    projection: &'a PreparedProjectionPath,
    owner_type: &'a kernel_query::RelType,
    view_type: &'a kernel_query::RelType,
    semantic: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
    preimages: &'a GammaPreimageCatalog,
}

fn extend_lossy_bag_project_from_runtime_determinant(
    plan: &RelWritableViewPlan,
    old_owner: &RelationValue,
    reconstructed: &mut RelationValue,
    inserted_rows: &[kernel_query::Row],
    context: &RuntimeProjectDeterminantContext<'_>,
    coordinates: &mut PreparedRelWritableCoordinates,
    constructor: Option<&FixedHiddenProjectInsertConstructor>,
) -> Result<(), RelRewriteLiftError> {
    if inserted_rows.is_empty() {
        return Ok(());
    }
    if constructor.is_none()
        && (!matches!(
            context.owner_type.semantics,
            kernel_schema::RelationSemantics::Bag { .. }
        ) || !matches!(
            context.view_type.semantics,
            kernel_schema::RelationSemantics::Bag { .. }
        ))
    {
        return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
    }
    let reconstructed_rows = match reconstructed {
        RelationValue::Bag(rows) | RelationValue::Set { rows, .. } => rows,
    };
    let visible_columns = context.projection.visible_columns();
    let hidden_columns = context.projection.hidden_columns();
    let before = coordinates
        .relation_determinant_morphism(
            plan.owner_relation,
            old_owner,
            visible_columns,
            &hidden_columns,
            context.semantic,
            context.registry,
        )
        .map_err(|_| RelRewriteLiftError::DeterminantEvidenceUnavailable)?
        .ok_or(RelRewriteLiftError::ProjectionPreimageAmbiguous)?;
    let mut constructed_new_domain = false;
    for inserted_view in inserted_rows {
        let class = GammaPreimageCatalog::class_for_view_row(
            inserted_view,
            context.view_type,
            context.semantic,
            context.registry,
        )?;
        if let Some(representative) = context.preimages.representative(&class, old_owner.rows()) {
            debug_assert!(!before.materialized_mapping().is_empty());
            reconstructed_rows.push(representative.clone());
            continue;
        }
        let Some(constructor) = constructor else {
            return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
        };
        reconstructed_rows.push(construct_project_owner_row(
            plan,
            context.projection,
            inserted_view,
            constructor,
        )?);
        constructed_new_domain = true;
    }

    let revalidated = coordinates
        .revalidate_relation_determinant(
            plan.owner_relation,
            old_owner,
            reconstructed,
            RelationDeterminantColumns {
                source: visible_columns,
                target: &hidden_columns,
            },
            context.semantic,
            context.registry,
        )
        .map_err(|_| RelRewriteLiftError::DeterminantEvidenceUnavailable)?
        .ok_or(RelRewriteLiftError::ProjectionPreimageAmbiguous)?;
    if !revalidated.shared_domain_images_are_stable()
        || (!constructed_new_domain && !revalidated.after_domain_is_covered_by_before())
    {
        return Err(RelRewriteLiftError::DeterminantEvidenceUnavailable);
    }
    Ok(())
}

fn construct_project_owner_row(
    plan: &RelWritableViewPlan,
    projection: &PreparedProjectionPath,
    projected_row: &[kernel_model::Value],
    constructor: &FixedHiddenProjectInsertConstructor,
) -> Result<kernel_query::Row, RelRewriteLiftError> {
    if constructor.owner_relation != plan.owner_relation {
        return Err(RelRewriteLiftError::ProjectConstructorOwnerMismatch);
    }
    if constructor.rewrite_spec != plan.rewrite_spec {
        return Err(RelRewriteLiftError::ProjectConstructorRewriteSpecMismatch);
    }
    projection.inflate_with_hidden(projected_row, &constructor.hidden_values)
}

struct PreparedLiftContext<'a> {
    semantic: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
    source_spec: &'a kernel_change::RewriteSpec,
    constructor: Option<&'a FixedHiddenProjectInsertConstructor>,
}

fn reconstruct_lossy_project_preimage(
    projection: &PreparedProjectionPath,
    old_owner: &RelationValue,
    owner_type: &kernel_query::RelType,
    view_type: &kernel_query::RelType,
    removed_rows: &[kernel_query::Row],
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(RelationValue, GammaPreimageCatalog), RelRewriteLiftError> {
    let mut preimages = GammaPreimageCatalog::build(
        old_owner.rows(),
        owner_type,
        view_type,
        semantic,
        registry,
        |row| projection.project_row(row),
    )?;
    let reconstructed = reconstruct_lossy_project_deletion(
        old_owner,
        owner_type,
        view_type,
        removed_rows,
        semantic,
        registry,
        &mut preimages,
    )?;
    Ok((reconstructed, preimages))
}

fn synthesize_lossy_project_source_rewrite_with_coordinates<I: Clone>(
    plan: &RelWritableViewPlan,
    prepared: &PreparedProjectLift,
    source: &kernel_model::FiniteModel,
    requested_view: &PreparedRewrite<RelationValue, I>,
    coordinates: &mut PreparedRelWritableCoordinates,
    context: &PreparedLiftContext<'_>,
) -> Result<kernel_query::PreparedRelationRewrite<I>, RelRewriteLiftError> {
    let semantic = context.semantic;
    let registry = context.registry;
    let source_spec = context.source_spec;
    if source_spec.id != plan.rewrite_spec {
        return Err(RelRewriteLiftError::SourceRewriteSpecMismatch {
            expected: plan.rewrite_spec,
            actual: source_spec.id,
        });
    }
    let old_owner = prepared
        .base
        .owner_query
        .evaluate(source, semantic, registry)?;
    let owner_type = prepared.base.owner_type();
    let old_view = prepared
        .base
        .view_query
        .evaluate(source, semantic, registry)?;
    let view_type = prepared.base.view_type();
    let requested_endpoint = requested_view.apply(&old_view);
    let view_delta = kernel_query::RelationDelta::between_values(
        &old_view,
        &requested_endpoint,
        view_type.clone(),
        semantic,
        registry,
    )?;
    let (mut reconstructed, preimages) = reconstruct_lossy_project_preimage(
        &prepared.projection,
        &old_owner,
        owner_type,
        view_type,
        &view_delta.removed,
        semantic,
        registry,
    )?;
    extend_lossy_bag_project_from_runtime_determinant(
        plan,
        &old_owner,
        &mut reconstructed,
        &view_delta.inserted,
        &RuntimeProjectDeterminantContext {
            projection: &prepared.projection,
            owner_type,
            view_type,
            semantic,
            registry,
            preimages: &preimages,
        },
        coordinates,
        context.constructor,
    )?;
    certify_owner_candidate(
        plan,
        requested_view,
        &old_owner,
        &requested_endpoint,
        &reconstructed,
        &OwnerCertificationContext {
            source,
            semantic,
            registry,
            source_spec,
            owner_type: prepared.base.owner_type(),
            view_query: &prepared.base.view_query,
        },
    )
}

#[cfg(test)]
fn synthesize_lossy_project_deletion_source_rewrite_for_commit<I: Clone>(
    plan: &RelWritableViewPlan,
    source: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
    constructor: Option<&FixedHiddenProjectInsertConstructor>,
) -> Result<kernel_query::PreparedRelationRewrite<I>, RelRewriteLiftError> {
    let prepared = kernel_plan::prepare_baseline(plan.query.clone(), semantic, registry)
        .map_err(|_| RelRewriteLiftError::DeterminantEvidenceUnavailable)?;
    let mut coordinates = prepared
        .writable_coordinates(registry)
        .map_err(|_| RelRewriteLiftError::DeterminantEvidenceUnavailable)?;
    let project = PreparedProjectLift::prepare(plan, semantic, registry)?;
    synthesize_lossy_project_source_rewrite_with_coordinates(
        plan,
        &project,
        source,
        requested_view,
        &mut coordinates,
        &PreparedLiftContext {
            semantic,
            registry,
            source_spec,
            constructor,
        },
    )
}

/// Executes the relational writable-view boundary without creating a second
/// publication path. Lift classification runs against one immutable runtime
/// snapshot; only the unique case is handed to the existing durable
/// `RelationRewrite` path, which re-derives the target and re-runs DTC, VMF
/// and freshness validation before publication.
pub fn commit_unique_relational_view_rewrite<I: Clone + PartialEq>(
    runtime: &DurableRuntime,
    transaction_id: ClientTransactionId,
    target_revision: RevisionId,
    prepared: &mut PreparedWritableCompilation,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
) -> Result<WritableViewCommitOutcome, WritableViewCommitError> {
    commit_unique_relational_view_rewrite_inner(
        runtime,
        transaction_id,
        target_revision,
        prepared,
        source_spec,
        requested_view,
        None,
    )
}

/// Same publication boundary as [`commit_unique_relational_view_rewrite`], but
/// with an explicit fixed hidden-column constructor for genuinely-new values of
/// a lossy projection. The constructor is used only when the requested visible
/// Γ-class has no source preimage; existing classes still use determinant-
/// revalidated source representatives.
pub fn commit_unique_relational_view_rewrite_with_project_constructor<I: Clone + PartialEq>(
    runtime: &DurableRuntime,
    transaction_id: ClientTransactionId,
    target_revision: RevisionId,
    prepared: &mut PreparedWritableCompilation,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
    constructor: &FixedHiddenProjectInsertConstructor,
) -> Result<WritableViewCommitOutcome, WritableViewCommitError> {
    commit_unique_relational_view_rewrite_inner(
        runtime,
        transaction_id,
        target_revision,
        prepared,
        source_spec,
        requested_view,
        Some(constructor),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreparedRelWritableLiftStrategy {
    ReadOnly,
    Identity(Box<PreparedIdentityLift>),
    BijectiveProject(Box<PreparedProjectLift>),
    LossyProject(Box<PreparedProjectLift>),
    JoinOwnerSide(Box<PreparedDirectJoinLift>),
    Filter(Box<PreparedFilterLift>),
}

fn prepare_rel_writable_lift_strategy(
    plan: &RelWritableViewPlan,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<PreparedRelWritableLiftStrategy, RelRewriteLiftError> {
    if plan.stages.is_empty() {
        if !plan.required_obligations.is_empty()
            || !matches!(plan.query, kernel_query::RelExpr::Scan(relation) if relation == plan.owner_relation)
        {
            return Err(RelRewriteLiftError::UnresolvedObligations(
                plan.required_obligations.clone(),
            ));
        }
        return Ok(PreparedRelWritableLiftStrategy::Identity(Box::new(
            PreparedIdentityLift {
                base: PreparedLiftBase::prepare(plan, semantic, registry)?,
            },
        )));
    }
    if plan
        .stages
        .iter()
        .any(|stage| matches!(stage, RelRewriteLiftStage::JoinOwnerSide { .. }))
    {
        return Ok(PreparedRelWritableLiftStrategy::JoinOwnerSide(Box::new(
            PreparedDirectJoinLift::prepare(plan, semantic, registry)?,
        )));
    }
    if plan
        .stages
        .iter()
        .all(|stage| matches!(stage, RelRewriteLiftStage::Project { .. }))
    {
        let prepared = Box::new(PreparedProjectLift::prepare(plan, semantic, registry)?);
        return Ok(if plan.required_obligations.is_empty() {
            if !prepared.projection.is_bijective() {
                return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
            }
            PreparedRelWritableLiftStrategy::BijectiveProject(prepared)
        } else {
            if plan.required_obligations.iter().any(|obligation| {
                !matches!(
                    obligation,
                    RelWritableObligation::PreserveHiddenColumnsComplement { .. }
                        | RelWritableObligation::HiddenColumnConstructorForInsert { .. }
                        | RelWritableObligation::ProjectionNoSemanticCollapse
                )
            }) {
                return Err(RelRewriteLiftError::UnresolvedObligations(
                    plan.required_obligations.clone(),
                ));
            }
            PreparedRelWritableLiftStrategy::LossyProject(prepared)
        });
    }
    if plan.stages.iter().all(|stage| {
        matches!(
            stage,
            RelRewriteLiftStage::FilterEqConst { .. } | RelRewriteLiftStage::FilterEqColumns { .. }
        )
    }) {
        let allowed = BTreeSet::from([
            RelWritableObligation::PredicateAdmissibility,
            RelWritableObligation::DtcGuardNoImpact,
            RelWritableObligation::VmfInvariantClosure,
        ]);
        if !plan.required_obligations.is_subset(&allowed) {
            return Err(RelRewriteLiftError::UnresolvedObligations(
                plan.required_obligations.clone(),
            ));
        }
        return Ok(PreparedRelWritableLiftStrategy::Filter(Box::new(
            PreparedFilterLift::prepare(plan, semantic, registry)?,
        )));
    }
    Err(RelRewriteLiftError::CandidateGenerationUnsupported)
}

fn commit_unique_relational_view_rewrite_inner<I: Clone + PartialEq>(
    runtime: &DurableRuntime,
    transaction_id: ClientTransactionId,
    target_revision: RevisionId,
    prepared: &mut PreparedWritableCompilation,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
    project_constructor: Option<&FixedHiddenProjectInsertConstructor>,
) -> Result<WritableViewCommitOutcome, WritableViewCommitError> {
    let (coordinates, compilation, strategy) = (
        &mut prepared.coordinates,
        &prepared.compilation,
        &prepared.strategy,
    );
    let plan = prepared_writable_plan(compilation)?;
    let snapshot = runtime
        .snapshot()
        .map_err(DurableRuntimeCommitError::from)?;
    let source_revision = snapshot.revision().id();
    let registry = runtime.semantic_registry();
    let model = &snapshot.revision().state().model;
    let semantic = snapshot.revision().semantic_context();
    let unique = match strategy {
        PreparedRelWritableLiftStrategy::ReadOnly => {
            return Err(RelRewriteLiftError::CandidateGenerationUnsupported.into());
        }
        PreparedRelWritableLiftStrategy::Identity(identity) => {
            synthesize_identity_source_rewrite_for_commit(
                plan,
                identity,
                model,
                semantic,
                registry,
                source_spec,
                requested_view,
            )?
        }
        PreparedRelWritableLiftStrategy::BijectiveProject(project) => {
            synthesize_bijective_project_source_rewrite_for_commit(
                plan,
                project,
                model,
                semantic,
                registry,
                source_spec,
                requested_view,
            )?
        }
        PreparedRelWritableLiftStrategy::LossyProject(project) => {
            synthesize_lossy_project_source_rewrite_with_coordinates(
                plan,
                project,
                model,
                requested_view,
                coordinates,
                &PreparedLiftContext {
                    semantic,
                    registry,
                    source_spec,
                    constructor: project_constructor,
                },
            )?
        }
        PreparedRelWritableLiftStrategy::JoinOwnerSide(join) => {
            synthesize_direct_scan_join_source_rewrite_with_constructor_for_commit_prepared(
                plan,
                join,
                model,
                requested_view,
                coordinates,
                &PreparedLiftContext {
                    semantic,
                    registry,
                    source_spec,
                    constructor: project_constructor,
                },
            )?
        }
        PreparedRelWritableLiftStrategy::Filter(filter) => {
            synthesize_filter_source_rewrite_for_commit(
                plan,
                filter,
                model,
                semantic,
                registry,
                source_spec,
                requested_view,
            )?
        }
    };
    drop(snapshot);

    let relation_rewrite = RevisionRelationRewrite {
        relation: plan.owner_relation,
        rewrite: &unique,
    };
    let rewrites = [relation_rewrite];
    let outcome = runtime.commit_derived_relation_rewrites(
        transaction_id,
        &DerivedRelationRewriteTransitionRequest {
            source_revision,
            target_revision,
            rewrites: &rewrites,
        },
    )?;
    Ok(WritableViewCommitOutcome::Durable(outcome))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use kernel_change::{Change, SeqSplice};
    use kernel_identity::IdentityTransport;
    use kernel_lifecycle::{LifecycleFact, LifecycleIntent};
    use kernel_model::{DatabaseState, FiniteModel, Value};
    use kernel_plan::{
        LayoutBinding, LayoutFamily, LayoutId, PhysicalCatalog, Plan as PhysicalPlan,
        RelationDeterminantColumns, prepare_baseline, prepare_with_catalog,
    };
    use kernel_proof::{Cell, Plan, Predicate, RewriteCertificate, check_rewrite};
    use kernel_query::{ExactQuery, Expr, RelExpr, check_derivative_law, derivative_seq_splice};
    use kernel_retention::{ErasureDomain, Tracked, select};
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, SemanticRegistry};
    use kernel_types::{EntityId, SchemaRevisionId, SemanticEnvId, SemanticId};
    use storage_memory::MemoryStore;

    fn id(raw: u128) -> EntityId {
        EntityId::new(raw)
    }

    fn context_with_explicit_text_equality() -> (SemanticContext, SemanticRegistry) {
        let text_eq = SemanticId::new(500);
        let set_type = SemanticId::new(501);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_type(
                set_type,
                TypeExpr::Set {
                    element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    equivalence: text_eq,
                },
            )
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: SemanticId::new(502),
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, digest);
        (
            SemanticContext {
                schema,
                environment,
            },
            registry,
        )
    }

    fn assert_lifecycle_merge(context: &SemanticContext, registry: &SemanticRegistry) {
        let mut state = DatabaseState::default();
        state.lifecycle.entities.extend([id(1), id(2), id(3)]);
        state.lifecycle.roots.extend([id(1), id(3)]);
        state
            .lifecycle
            .keeps_alive
            .insert(id(1), BTreeSet::from([id(2)]));

        let mut store = MemoryStore::with_semantic_registry(registry.clone());
        let base = store.bootstrap(context, state).unwrap();
        let mut left = LifecycleIntent::default();
        left.set(
            LifecycleFact::KeepsAlive {
                parent: id(1),
                child: id(2),
            },
            false,
        );
        let left_revision = store.commit_lifecycle(base, context, left).unwrap();

        let mut right = LifecycleIntent::default();
        right.set(
            LifecycleFact::KeepsAlive {
                parent: id(3),
                child: id(2),
            },
            true,
        );
        let right_revision = store.commit_lifecycle(base, context, right).unwrap();
        let merged = store
            .merge_lifecycle_branches(left_revision, right_revision, context)
            .unwrap();
        assert!(
            store
                .revision(merged)
                .unwrap()
                .state()
                .lifecycle
                .entities
                .contains(&id(2))
        );
    }

    fn assert_fine_delta_law() {
        let query = ExactQuery::new(Expr::SeqLength(Box::new(Expr::Input)));
        let old = Value::Seq(vec![Value::I64(1), Value::I64(2)]);
        let splice = SeqSplice {
            start: 1,
            delete_count: 0,
            insert: vec![Value::I64(3)],
        };
        let fine = derivative_seq_splice(&query, &old, &splice)
            .unwrap()
            .unwrap();
        let Value::Seq(old_values) = &old else {
            unreachable!();
        };
        let next = Value::Seq(splice.apply(old_values).unwrap());
        assert!(check_derivative_law(
            &query,
            &old,
            &Change::Replace(next),
            &fine
        ));
    }

    fn assert_identity_transport() {
        let source = BTreeSet::from([id(10), id(11)]);
        let target = BTreeSet::from([id(20), id(21)]);
        let transport = IdentityTransport::new(
            &source,
            &target,
            BTreeMap::from([(id(10), id(21)), (id(11), id(20))]),
        )
        .unwrap();
        assert_eq!(transport.inverse().transport(id(21)), Some(id(10)));
    }

    fn assert_implicit_retention_flow() {
        let secret = ErasureDomain(SemanticId::new(900));
        let derived = select(
            Tracked::protected(true, secret),
            Tracked::public(1_i64),
            Tracked::public(0_i64),
        );
        assert!(derived.label().contains(secret));
    }

    fn assert_relational_plan_lowering(context: &SemanticContext, registry: &SemanticRegistry) {
        let relation = SemanticId::new(502);
        let text_eq = SemanticId::new(500);
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text("Alpha".into()),
            equivalence: text_eq,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into())],
                vec![Value::Text("Beta".into())],
            ],
        );
        let mut catalog = PhysicalCatalog::default();
        let layout = LayoutBinding {
            id: LayoutId(77),
            family: LayoutFamily::Columnar,
        };
        catalog.bind_relation(relation, layout);
        let prepared = prepare_with_catalog(query.clone(), context, registry, &catalog).unwrap();
        assert_eq!(
            prepared
                .reference_execute(&model, context, registry)
                .unwrap(),
            query.evaluate(&model, context, registry).unwrap()
        );
        let PhysicalPlan::FilterEqConst { input, .. } = prepared.physical() else {
            unreachable!();
        };
        assert!(matches!(
            input.as_ref(),
            PhysicalPlan::Scan {
                relation: actual,
                layout: actual_layout,
                ..
            } if *actual == relation && *actual_layout == layout
        ));
    }

    #[test]
    fn prepared_plan_owns_distinct_writable_coordinates_for_shared_equivalence_columns() {
        let equivalence = SemanticId::new(980);
        let relation = SemanticId::new(981);
        let mut schema = Schema::new(SchemaRevisionId::new(4));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![equivalence, equivalence],
                },
            })
            .unwrap();
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(4));
        environment.pin_module(equivalence, digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let prepared = prepare_baseline(RelExpr::Scan(relation), &context, &registry).unwrap();
        let mut compiled = super::compile_prepared_relational_writable_query(
            &prepared,
            relation,
            kernel_change::RewriteSpecId(SemanticId::new(982)),
            &registry,
        )
        .unwrap();
        let columns = compiled.coordinates.relation_columns()[&relation].clone();
        assert_eq!(columns.len(), 2);
        assert_ne!(columns[0], columns[1]);
        assert!(matches!(
            compiled.compilation,
            kernel_lens::RelWritableCompilation::Writable(_)
        ));
        let value = kernel_query::RelationValue::Bag(vec![
            vec![Value::Text("a".into()), Value::Text("x".into())],
            vec![Value::Text("b".into()), Value::Text("y".into())],
        ]);
        let measure = compiled
            .coordinates
            .relation_measure(relation, &value, &context, &registry)
            .unwrap();
        assert!(
            measure
                .determinant_morphism(
                    compiled.coordinates.catalog(),
                    vec![columns[0]],
                    vec![columns[1]],
                )
                .unwrap()
                .is_some()
        );

        assert_revalidated_determinant_boundaries(
            &mut compiled,
            relation,
            &value,
            &context,
            &registry,
        );
    }

    fn assert_revalidated_determinant_boundaries(
        compiled: &mut super::PreparedWritableCompilation,
        relation: SemanticId,
        value: &kernel_query::RelationValue,
        context: &SemanticContext,
        registry: &SemanticRegistry,
    ) {
        let stable_after = kernel_query::RelationValue::Bag(vec![
            vec![Value::Text("a".into()), Value::Text("x".into())],
            vec![Value::Text("b".into()), Value::Text("y".into())],
            vec![Value::Text("a".into()), Value::Text("x".into())],
        ]);
        let stable = compiled
            .coordinates
            .revalidate_relation_determinant(
                relation,
                value,
                &stable_after,
                RelationDeterminantColumns {
                    source: &[0],
                    target: &[1],
                },
                context,
                registry,
            )
            .unwrap()
            .unwrap();
        assert!(stable.after_domain_is_covered_by_before());
        assert!(stable.shared_domain_images_are_stable());

        let remapped_after = kernel_query::RelationValue::Bag(vec![
            vec![Value::Text("a".into()), Value::Text("z".into())],
            vec![Value::Text("b".into()), Value::Text("y".into())],
        ]);
        let remapped = compiled
            .coordinates
            .revalidate_relation_determinant(
                relation,
                value,
                &remapped_after,
                RelationDeterminantColumns {
                    source: &[0],
                    target: &[1],
                },
                context,
                registry,
            )
            .unwrap()
            .unwrap();
        assert!(remapped.after_domain_is_covered_by_before());
        assert!(!remapped.shared_domain_images_are_stable());

        let extended_after = kernel_query::RelationValue::Bag(vec![
            vec![Value::Text("a".into()), Value::Text("x".into())],
            vec![Value::Text("b".into()), Value::Text("y".into())],
            vec![Value::Text("c".into()), Value::Text("z".into())],
        ]);
        let extended = compiled
            .coordinates
            .revalidate_relation_determinant(
                relation,
                value,
                &extended_after,
                RelationDeterminantColumns {
                    source: &[0],
                    target: &[1],
                },
                context,
                registry,
            )
            .unwrap()
            .unwrap();
        assert!(!extended.after_domain_is_covered_by_before());
    }

    fn assert_plan_certificate() {
        let first = Predicate::EqI64 {
            column: 0,
            value: 1,
        };
        let second = Predicate::EqI64 {
            column: 1,
            value: 2,
        };
        let before = Plan::Filter {
            predicate: second.clone(),
            input: Box::new(Plan::Filter {
                predicate: first.clone(),
                input: Box::new(Plan::Input),
            }),
        };
        let after = Plan::Filter {
            predicate: Predicate::And(Box::new(first), Box::new(second)),
            input: Box::new(Plan::Input),
        };
        assert!(check_rewrite(
            &before,
            &after,
            RewriteCertificate::FuseNestedFilters
        ));
        let input = vec![vec![Cell::I64(1), Cell::I64(2)]];
        assert_eq!(before.execute(&input), after.execute(&input));
    }

    #[test]
    fn filtered_write_section_preserves_rejected_complement_and_rejects_bad_view() {
        let (context, registry) = context_with_explicit_text_equality();
        let relation = SemanticId::new(502);
        let text_eq = SemanticId::new(500);
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text("Alpha".into()),
            equivalence: text_eq,
        };
        let spec = kernel_change::RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(930)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(931)),
            footprint: kernel_change::RewriteFootprint::opaque_relation(relation),
        };
        let plan = kernel_lens::RelWritableViewPlan {
            query: query.clone(),
            owner_relation: relation,
            rewrite_spec: spec.id,
            output_origins: Vec::new(),
            stages: vec![kernel_lens::RelRewriteLiftStage::FilterEqConst {
                column: 0,
                equivalence: text_eq,
            }],
            required_obligations: BTreeSet::from([
                kernel_lens::RelWritableObligation::PredicateAdmissibility,
                kernel_lens::RelWritableObligation::DtcGuardNoImpact,
                kernel_lens::RelWritableObligation::VmfInvariantClosure,
            ]),
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into())],
                vec![Value::Text("Beta".into())],
            ],
        );
        let keep_alpha = kernel_change::PreparedRewrite {
            spec: kernel_change::RewriteSpecId(SemanticId::new(940)),
            explicit_inputs: Vec::<Value>::new(),
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![Value::Text("Alpha".into())],
            ])),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(941)),
        };
        let prepared_filter =
            super::PreparedFilterLift::prepare(&plan, &context, &registry).unwrap();
        let lifted = super::synthesize_filter_source_rewrite_for_commit(
            &plan,
            &prepared_filter,
            &model,
            &context,
            &registry,
            &spec,
            &keep_alpha,
        )
        .unwrap();
        assert!(lifted.delta().is_empty());

        let bad_view = kernel_change::PreparedRewrite {
            spec: kernel_change::RewriteSpecId(SemanticId::new(942)),
            explicit_inputs: Vec::<Value>::new(),
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![Value::Text("Beta".into())],
            ])),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(943)),
        };
        assert!(matches!(
            super::synthesize_filter_source_rewrite_for_commit(
                &plan,
                &prepared_filter,
                &model,
                &context,
                &registry,
                &spec,
                &bad_view,
            ),
            Err(kernel_lens::RelRewriteLiftError::RequestedViewInadmissible)
        ));
    }

    #[test]
    fn full_column_project_write_section_inverts_coordinate_permutation() {
        let text_eq = SemanticId::new(960);
        let relation = SemanticId::new(961);
        let mut schema = Schema::new(SchemaRevisionId::new(2));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, text_eq],
                },
            })
            .unwrap();
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2));
        environment.pin_module(text_eq, digest);
        let context = SemanticContext {
            schema,
            environment,
        };

        let spec = kernel_change::RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(962)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(963)),
            footprint: kernel_change::RewriteFootprint::opaque_relation(relation),
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![1, 0],
        };
        let plan = kernel_lens::RelWritableViewPlan {
            query,
            owner_relation: relation,
            rewrite_spec: spec.id,
            output_origins: Vec::new(),
            stages: vec![kernel_lens::RelRewriteLiftStage::Project {
                columns: vec![1, 0],
            }],
            required_obligations: BTreeSet::new(),
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![vec![
                Value::Text("left".into()),
                Value::Text("right".into()),
            ]],
        );
        let requested = kernel_change::PreparedRewrite {
            spec: kernel_change::RewriteSpecId(SemanticId::new(964)),
            explicit_inputs: Vec::<Value>::new(),
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![Value::Text("RIGHT2".into()), Value::Text("LEFT2".into())],
            ])),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(965)),
        };
        let prepared_project = super::PreparedProjectLift {
            base: super::PreparedLiftBase::prepare(&plan, &context, &registry).unwrap(),
            projection: super::PreparedProjectionPath::from_stage_columns(&[vec![1, 0]], 2)
                .unwrap(),
        };
        let lifted = super::synthesize_bijective_project_source_rewrite_for_commit(
            &plan,
            &prepared_project,
            &model,
            &context,
            &registry,
            &spec,
            &requested,
        )
        .unwrap();
        let old_owner = RelExpr::Scan(relation)
            .evaluate(&model, &context, &registry)
            .unwrap();
        assert_eq!(
            lifted.apply_structural(&old_owner, &registry).unwrap(),
            kernel_query::RelationValue::Bag(vec![vec![
                Value::Text("LEFT2".into()),
                Value::Text("RIGHT2".into()),
            ]])
        );
    }

    #[test]
    fn lossy_project_deletion_preserves_hidden_complement_and_rejects_ambiguous_bag_preimage() {
        let text_eq = SemanticId::new(966);
        let relation = SemanticId::new(967);
        let mut schema = Schema::new(SchemaRevisionId::new(6));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, text_eq],
                },
            })
            .unwrap();
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(6));
        environment.pin_module(text_eq, digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let spec = kernel_change::RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(968)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(969)),
            footprint: kernel_change::RewriteFootprint::opaque_relation(relation),
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let prepared = prepare_baseline(query, &context, &registry).unwrap();
        let compiled = super::compile_prepared_relational_writable_query(
            &prepared, relation, spec.id, &registry,
        )
        .unwrap();
        let kernel_lens::RelWritableCompilation::Conditional { plan, .. } = compiled.compilation
        else {
            panic!("lossy project must remain conditional")
        };
        let requested = kernel_change::PreparedRewrite {
            spec: kernel_change::RewriteSpecId(SemanticId::new(970)),
            explicit_inputs: Vec::<Value>::new(),
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![Value::Text("b".into())],
            ])),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(971)),
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("a".into()), Value::Text("hidden-a".into())],
                vec![Value::Text("b".into()), Value::Text("hidden-b".into())],
            ],
        );
        let lifted = super::synthesize_lossy_project_deletion_source_rewrite_for_commit(
            &plan, &model, &context, &registry, &spec, &requested, None,
        )
        .unwrap();
        assert_eq!(
            lifted.delta().removed,
            vec![vec![
                Value::Text("a".into()),
                Value::Text("hidden-a".into())
            ]]
        );

        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("a".into()), Value::Text("hidden-1".into())],
                vec![Value::Text("a".into()), Value::Text("hidden-2".into())],
            ],
        );
        let keep_one_a = kernel_change::PreparedRewrite {
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![Value::Text("a".into())],
            ])),
            ..requested
        };
        assert!(matches!(
            super::synthesize_lossy_project_deletion_source_rewrite_for_commit(
                &plan,
                &model,
                &context,
                &registry,
                &spec,
                &keep_one_a,
                None,
            ),
            Err(kernel_lens::RelRewriteLiftError::ProjectionPreimageAmbiguous)
        ));
    }

    fn assert_unseen_lossy_project_insert_rejected(
        plan: &kernel_lens::RelWritableViewPlan,
        model: &FiniteModel,
        context: &SemanticContext,
        registry: &SemanticRegistry,
        spec: &kernel_change::RewriteSpec,
        requested: kernel_change::PreparedRewrite<kernel_query::RelationValue, Value>,
    ) {
        let unseen = kernel_change::PreparedRewrite {
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![Value::Text("b".into())],
            ])),
            ..requested
        };
        assert!(matches!(
            super::synthesize_lossy_project_deletion_source_rewrite_for_commit(
                plan, model, context, registry, spec, &unseen, None,
            ),
            Err(kernel_lens::RelRewriteLiftError::CandidateGenerationUnsupported)
        ));
    }

    #[test]
    fn lossy_bag_project_can_grow_existing_class_only_when_hidden_preimage_is_unique() {
        let text_eq = SemanticId::new(9720);
        let relation = SemanticId::new(9721);
        let mut schema = Schema::new(SchemaRevisionId::new(62));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, text_eq],
                },
            })
            .unwrap();
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(62));
        environment.pin_module(text_eq, digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let spec = kernel_change::RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(9722)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(9723)),
            footprint: kernel_change::RewriteFootprint::opaque_relation(relation),
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let prepared = prepare_baseline(query, &context, &registry).unwrap();
        let compiled = super::compile_prepared_relational_writable_query(
            &prepared, relation, spec.id, &registry,
        )
        .unwrap();
        let kernel_lens::RelWritableCompilation::Conditional { plan, .. } = compiled.compilation
        else {
            panic!("lossy project must remain conditional")
        };
        let requested = kernel_change::PreparedRewrite {
            spec: kernel_change::RewriteSpecId(SemanticId::new(9724)),
            explicit_inputs: Vec::<Value>::new(),
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![Value::Text("a".into())],
                vec![Value::Text("a".into())],
            ])),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(9725)),
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![vec![Value::Text("a".into()), Value::Text("hidden".into())]],
        );
        let lifted = super::synthesize_lossy_project_deletion_source_rewrite_for_commit(
            &plan, &model, &context, &registry, &spec, &requested, None,
        )
        .unwrap();
        assert_eq!(
            lifted.delta().inserted,
            vec![vec![Value::Text("a".into()), Value::Text("hidden".into())]]
        );

        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("a".into()), Value::Text("h1".into())],
                vec![Value::Text("a".into()), Value::Text("h2".into())],
            ],
        );
        let grow_to_three = kernel_change::PreparedRewrite {
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![Value::Text("a".into())],
                vec![Value::Text("a".into())],
                vec![Value::Text("a".into())],
            ])),
            ..requested.clone()
        };
        assert!(matches!(
            super::synthesize_lossy_project_deletion_source_rewrite_for_commit(
                &plan,
                &model,
                &context,
                &registry,
                &spec,
                &grow_to_three,
                None,
            ),
            Err(kernel_lens::RelRewriteLiftError::ProjectionPreimageAmbiguous)
        ));

        model.relations.insert(
            relation,
            vec![vec![Value::Text("a".into()), Value::Text("hidden".into())]],
        );
        assert_unseen_lossy_project_insert_rejected(
            &plan, &model, &context, &registry, &spec, requested,
        );
    }

    struct LossyProjectConstructorFixture {
        context: SemanticContext,
        registry: SemanticRegistry,
        spec: kernel_change::RewriteSpec,
        plan: kernel_lens::RelWritableViewPlan,
        model: FiniteModel,
        requested: kernel_change::PreparedRewrite<kernel_query::RelationValue, Value>,
        relation: SemanticId,
    }

    fn lossy_project_constructor_fixture() -> LossyProjectConstructorFixture {
        let (text_eq, relation) = (SemanticId::new(9730), SemanticId::new(9731));
        let mut schema = Schema::new(SchemaRevisionId::new(63));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, text_eq],
                },
            })
            .unwrap();
        let mut registry = SemanticRegistry::default();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(63));
        environment.pin_module(
            text_eq,
            registry.install_equivalence(EquivalenceModule::TextExact),
        );
        let context = SemanticContext {
            schema,
            environment,
        };
        let spec = kernel_change::RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(9732)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(9733)),
            footprint: kernel_change::RewriteFootprint::opaque_relation(relation),
        };
        let prepared = prepare_baseline(
            RelExpr::Project {
                input: Box::new(RelExpr::Scan(relation)),
                columns: vec![0],
            },
            &context,
            &registry,
        )
        .unwrap();
        let kernel_lens::RelWritableCompilation::Conditional { plan, .. } =
            super::compile_prepared_relational_writable_query(
                &prepared, relation, spec.id, &registry,
            )
            .unwrap()
            .compilation
        else {
            panic!("lossy project must remain conditional")
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![vec![
                Value::Text("a".into()),
                Value::Text("hidden-a".into()),
            ]],
        );
        let requested = kernel_change::PreparedRewrite {
            spec: kernel_change::RewriteSpecId(SemanticId::new(9734)),
            explicit_inputs: Vec::<Value>::new(),
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![Value::Text("a".into())],
                vec![Value::Text("b".into())],
            ])),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(9735)),
        };
        LossyProjectConstructorFixture {
            context,
            registry,
            spec,
            plan,
            model,
            requested,
            relation,
        }
    }

    #[test]
    fn fixed_hidden_constructor_admits_unseen_lossy_project_class_without_guessing() {
        let fixture = lossy_project_constructor_fixture();
        let constructor = super::FixedHiddenProjectInsertConstructor {
            owner_relation: fixture.relation,
            rewrite_spec: fixture.spec.id,
            hidden_values: BTreeMap::from([(1, Value::Text("hidden-new".into()))]),
        };
        let lifted = super::synthesize_lossy_project_deletion_source_rewrite_for_commit(
            &fixture.plan,
            &fixture.model,
            &fixture.context,
            &fixture.registry,
            &fixture.spec,
            &fixture.requested,
            Some(&constructor),
        )
        .unwrap();
        assert_eq!(
            lifted.delta().inserted,
            vec![vec![
                Value::Text("b".into()),
                Value::Text("hidden-new".into())
            ]]
        );

        let visible_override = super::FixedHiddenProjectInsertConstructor {
            hidden_values: BTreeMap::from([(0, Value::Text("forbidden".into()))]),
            ..constructor
        };
        assert!(matches!(
            super::synthesize_lossy_project_deletion_source_rewrite_for_commit(
                &fixture.plan,
                &fixture.model,
                &fixture.context,
                &fixture.registry,
                &fixture.spec,
                &fixture.requested,
                Some(&visible_override),
            ),
            Err(
                kernel_lens::RelRewriteLiftError::ProjectConstructorOverridesVisibleColumn {
                    column: 0
                }
            )
        ));
    }

    struct DirectJoinFixture {
        context: SemanticContext,
        registry: SemanticRegistry,
        spec: kernel_change::RewriteSpec,
        plan: kernel_lens::RelWritableViewPlan,
        model: FiniteModel,
        requested: kernel_change::PreparedRewrite<kernel_query::RelationValue, Value>,
        owner: SemanticId,
        lookup: SemanticId,
    }

    fn direct_join_requested() -> kernel_change::PreparedRewrite<kernel_query::RelationValue, Value>
    {
        kernel_change::PreparedRewrite {
            spec: kernel_change::RewriteSpecId(SemanticId::new(975)),
            explicit_inputs: Vec::<Value>::new(),
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![
                    Value::Text("a".into()),
                    Value::Text("new".into()),
                    Value::Text("a".into()),
                    Value::Text("label".into()),
                ],
            ])),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(976)),
        }
    }

    fn direct_join_fixture() -> DirectJoinFixture {
        let text_eq = SemanticId::new(970);
        let owner = SemanticId::new(971);
        let lookup = SemanticId::new(972);
        let mut schema = Schema::new(SchemaRevisionId::new(3));
        for (id, semantics) in [
            (
                owner,
                RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, text_eq],
                },
            ),
            (
                lookup,
                RelationSemantics::Set {
                    column_equivalences: vec![text_eq, text_eq],
                },
            ),
        ] {
            schema
                .define_relation(RelationDef {
                    id,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::Text),
                        TypeExpr::Scalar(ScalarType::Text),
                    ],
                    semantics,
                })
                .unwrap();
        }
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(3));
        environment.pin_module(text_eq, digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let spec = kernel_change::RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(973)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(974)),
            footprint: kernel_change::RewriteFootprint::opaque_relation(owner),
        };
        let plan = kernel_lens::RelWritableViewPlan {
            query: RelExpr::JoinEq {
                left: Box::new(RelExpr::Scan(owner)),
                right: Box::new(RelExpr::Scan(lookup)),
                left_column: 0,
                right_column: 0,
                equivalence: text_eq,
            },
            owner_relation: owner,
            rewrite_spec: spec.id,
            output_origins: Vec::new(),
            stages: vec![kernel_lens::RelRewriteLiftStage::JoinOwnerSide {
                owner_is_left: true,
                owner_column: 0,
                lookup_column: 0,
                equivalence: text_eq,
            }],
            required_obligations: BTreeSet::from([
                kernel_lens::RelWritableObligation::DtcGuardNoImpact,
                kernel_lens::RelWritableObligation::VmfInvariantClosure,
                kernel_lens::RelWritableObligation::JoinLookupDeterminant {
                    join_key: kernel_types::RevisionObservableId::new(1),
                    lookup_outputs: BTreeSet::new(),
                },
            ]),
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            owner,
            vec![
                vec![Value::Text("a".into()), Value::Text("old".into())],
                vec![Value::Text("z".into()), Value::Text("hidden".into())],
            ],
        );
        model.relations.insert(
            lookup,
            vec![vec![Value::Text("a".into()), Value::Text("label".into())]],
        );
        let requested = direct_join_requested();
        DirectJoinFixture {
            context,
            registry,
            spec,
            plan,
            model,
            requested,
            owner,
            lookup,
        }
    }

    #[test]
    fn direct_owner_join_write_section_preserves_unmatched_complement_and_checks_lookup_uniqueness()
    {
        let mut fixture = direct_join_fixture();
        let lifted = super::synthesize_direct_scan_join_source_rewrite_for_commit(
            &fixture.plan,
            &fixture.model,
            &fixture.context,
            &fixture.registry,
            &fixture.spec,
            &fixture.requested,
        )
        .unwrap();
        let old_owner = RelExpr::Scan(fixture.owner)
            .evaluate(&fixture.model, &fixture.context, &fixture.registry)
            .unwrap();
        let actual_owner = lifted
            .apply_structural(&old_owner, &fixture.registry)
            .unwrap();
        assert_eq!(actual_owner.rows().len(), 2);
        assert!(
            actual_owner
                .rows()
                .contains(&vec![Value::Text("a".into()), Value::Text("new".into())])
        );
        assert!(
            actual_owner
                .rows()
                .contains(&vec![Value::Text("z".into()), Value::Text("hidden".into())])
        );

        let forged_lookup_view = kernel_change::PreparedRewrite {
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![
                    Value::Text("a".into()),
                    Value::Text("new".into()),
                    Value::Text("a".into()),
                    Value::Text("forged".into()),
                ],
            ])),
            ..fixture.requested.clone()
        };
        assert!(matches!(
            super::synthesize_direct_scan_join_source_rewrite_for_commit(
                &fixture.plan,
                &fixture.model,
                &fixture.context,
                &fixture.registry,
                &fixture.spec,
                &forged_lookup_view,
            ),
            Err(kernel_lens::RelRewriteLiftError::RequestedViewInadmissible)
        ));

        fixture.model.relations.insert(
            fixture.lookup,
            vec![
                vec![Value::Text("a".into()), Value::Text("label".into())],
                vec![Value::Text("a".into()), Value::Text("other".into())],
            ],
        );
        assert!(matches!(
            super::synthesize_direct_scan_join_source_rewrite_for_commit(
                &fixture.plan,
                &fixture.model,
                &fixture.context,
                &fixture.registry,
                &fixture.spec,
                &fixture.requested,
            ),
            Err(kernel_lens::RelRewriteLiftError::JoinLookupKeyNotUnique)
        ));
    }

    #[test]
    fn filtered_owner_join_preserves_rejected_owner_complement() {
        let mut fixture = direct_join_fixture();
        let text_eq = SemanticId::new(970);
        let owner_filter = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(fixture.owner)),
            column: 0,
            value: Value::Text("a".into()),
            equivalence: text_eq,
        };
        fixture.plan.query = RelExpr::JoinEq {
            left: Box::new(owner_filter),
            right: Box::new(RelExpr::Scan(fixture.lookup)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        fixture.plan.stages.insert(
            0,
            kernel_lens::RelRewriteLiftStage::FilterEqConst {
                column: 0,
                equivalence: text_eq,
            },
        );
        fixture
            .plan
            .required_obligations
            .insert(kernel_lens::RelWritableObligation::PredicateAdmissibility);

        let lifted = super::synthesize_direct_scan_join_source_rewrite_for_commit(
            &fixture.plan,
            &fixture.model,
            &fixture.context,
            &fixture.registry,
            &fixture.spec,
            &fixture.requested,
        )
        .unwrap();
        let old_owner = RelExpr::Scan(fixture.owner)
            .evaluate(&fixture.model, &fixture.context, &fixture.registry)
            .unwrap();
        let actual_owner = lifted
            .apply_structural(&old_owner, &fixture.registry)
            .unwrap();
        assert!(
            actual_owner
                .rows()
                .contains(&vec![Value::Text("a".into()), Value::Text("new".into())])
        );
        assert!(
            actual_owner
                .rows()
                .contains(&vec![Value::Text("z".into()), Value::Text("hidden".into())])
        );
    }

    #[test]
    fn bijective_project_over_filtered_owner_join_inverts_coordinates_and_preserves_rejected_rows()
    {
        let fixture = direct_join_fixture();
        let text_eq = SemanticId::new(970);
        let owner_query = RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(fixture.owner)),
                column: 0,
                value: Value::Text("a".into()),
                equivalence: text_eq,
            }),
            columns: vec![1, 0],
        };
        let query = RelExpr::JoinEq {
            left: Box::new(owner_query),
            right: Box::new(RelExpr::Scan(fixture.lookup)),
            left_column: 1,
            right_column: 0,
            equivalence: text_eq,
        };
        let prepared = prepare_baseline(query, &fixture.context, &fixture.registry).unwrap();
        let compiled = super::compile_prepared_relational_writable_query(
            &prepared,
            fixture.owner,
            fixture.spec.id,
            &fixture.registry,
        )
        .unwrap();
        let plan = match compiled.compilation {
            kernel_lens::RelWritableCompilation::Conditional { plan, .. }
            | kernel_lens::RelWritableCompilation::Writable(plan) => plan,
            other @ kernel_lens::RelWritableCompilation::ReadOnly(_) => {
                panic!("bijective projected owner join must compile writable: {other:?}")
            }
        };
        let requested = kernel_change::PreparedRewrite {
            spec: kernel_change::RewriteSpecId(SemanticId::new(977)),
            explicit_inputs: Vec::<Value>::new(),
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![
                    Value::Text("new".into()),
                    Value::Text("a".into()),
                    Value::Text("a".into()),
                    Value::Text("label".into()),
                ],
            ])),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(978)),
        };

        let lifted = super::synthesize_direct_scan_join_source_rewrite_for_commit(
            &plan,
            &fixture.model,
            &fixture.context,
            &fixture.registry,
            &fixture.spec,
            &requested,
        )
        .unwrap();
        let old_owner = RelExpr::Scan(fixture.owner)
            .evaluate(&fixture.model, &fixture.context, &fixture.registry)
            .unwrap();
        let actual_owner = lifted
            .apply_structural(&old_owner, &fixture.registry)
            .unwrap();
        assert!(
            actual_owner
                .rows()
                .contains(&vec![Value::Text("a".into()), Value::Text("new".into())])
        );
        assert!(
            actual_owner
                .rows()
                .contains(&vec![Value::Text("z".into()), Value::Text("hidden".into())])
        );
    }

    #[test]
    fn lossy_project_owner_join_uses_explicit_constructor_for_unseen_visible_class() {
        let mut fixture = direct_join_fixture();
        let text_eq = SemanticId::new(970);
        fixture
            .model
            .relations
            .get_mut(&fixture.lookup)
            .unwrap()
            .push(vec![Value::Text("b".into()), Value::Text("label-b".into())]);
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Scan(fixture.owner)),
                columns: vec![0],
            }),
            right: Box::new(RelExpr::Scan(fixture.lookup)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let prepared = prepare_baseline(query, &fixture.context, &fixture.registry).unwrap();
        let compiled = super::compile_prepared_relational_writable_query(
            &prepared,
            fixture.owner,
            fixture.spec.id,
            &fixture.registry,
        )
        .unwrap();
        let plan = match compiled.compilation {
            kernel_lens::RelWritableCompilation::Conditional { plan, .. }
            | kernel_lens::RelWritableCompilation::Writable(plan) => plan,
            other @ kernel_lens::RelWritableCompilation::ReadOnly(_) => {
                panic!("lossy projected owner join must compile conditionally: {other:?}")
            }
        };
        let requested = kernel_change::PreparedRewrite {
            spec: kernel_change::RewriteSpecId(SemanticId::new(979)),
            explicit_inputs: Vec::<Value>::new(),
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![
                    Value::Text("a".into()),
                    Value::Text("a".into()),
                    Value::Text("label".into()),
                ],
                vec![
                    Value::Text("b".into()),
                    Value::Text("b".into()),
                    Value::Text("label-b".into()),
                ],
            ])),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(980)),
        };
        let constructor = super::FixedHiddenProjectInsertConstructor {
            owner_relation: fixture.owner,
            rewrite_spec: fixture.spec.id,
            hidden_values: BTreeMap::from([(1, Value::Text("hidden-new".into()))]),
        };

        assert!(matches!(
            super::synthesize_direct_scan_join_source_rewrite_with_constructor_for_commit(
                &plan,
                &fixture.model,
                &fixture.context,
                &fixture.registry,
                &fixture.spec,
                &requested,
                None,
            ),
            Err(kernel_lens::RelRewriteLiftError::CandidateGenerationUnsupported)
        ));

        let lifted = super::synthesize_direct_scan_join_source_rewrite_with_constructor_for_commit(
            &plan,
            &fixture.model,
            &fixture.context,
            &fixture.registry,
            &fixture.spec,
            &requested,
            Some(&constructor),
        )
        .unwrap();
        assert_eq!(
            lifted.delta().inserted,
            vec![vec![
                Value::Text("b".into()),
                Value::Text("hidden-new".into())
            ]]
        );
    }

    #[test]
    fn lookup_uniqueness_is_checked_once_by_gamma_key_not_raw_value() {
        let equivalence = SemanticId::new(9_794_001);
        let relation = SemanticId::new(9_794_002);
        let mut schema = Schema::new(SchemaRevisionId::new(96));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![equivalence],
                },
            })
            .unwrap();
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(96));
        environment.pin_module(equivalence, digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into())],
                vec![Value::Text("alpha".into())],
            ],
        );
        let prepared_lookup = RelExpr::Scan(relation)
            .prepare(&context, &registry)
            .unwrap();
        let error = super::validate_direct_lookup_key_uniqueness(
            &prepared_lookup,
            0,
            equivalence,
            &model,
            &context,
            &registry,
        )
        .unwrap_err();
        assert_eq!(
            error,
            kernel_lens::RelRewriteLiftError::JoinLookupKeyNotUnique
        );
    }

    #[test]
    fn gamma_preimage_catalog_keys_projected_rows_by_declared_equivalence() {
        let equivalence = SemanticId::new(9_795_001);
        let schema = Schema::new(SchemaRevisionId::new(97));
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(97));
        environment.pin_module(equivalence, digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let owner_type = kernel_query::RelType {
            columns: vec![
                TypeExpr::Scalar(ScalarType::Text),
                TypeExpr::Scalar(ScalarType::Text),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence, equivalence],
            },
        };
        let projected_type = kernel_query::RelType {
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        };
        let rows = vec![
            vec![Value::Text("Alpha".into()), Value::Text("x".into())],
            vec![Value::Text("alpha".into()), Value::Text("x".into())],
        ];
        let catalog = super::GammaPreimageCatalog::build(
            &rows,
            &owner_type,
            &projected_type,
            &context,
            &registry,
            |row| Ok(vec![row[0].clone()]),
        )
        .unwrap();
        let class = super::GammaPreimageCatalog::class_for_view_row(
            &[Value::Text("ALPHA".into())],
            &projected_type,
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(catalog.representative(&class, &rows), Some(&rows[0]));
    }

    #[test]
    fn gamma_preimage_catalog_projects_owner_once_for_reused_delta_lookups() {
        use std::cell::Cell;

        let fixture = direct_join_fixture();
        let owner_type = RelExpr::Scan(fixture.owner)
            .typecheck(&fixture.context, &fixture.registry)
            .unwrap();
        let text_eq = match &owner_type.semantics {
            RelationSemantics::Bag {
                column_equivalences,
            }
            | RelationSemantics::Set {
                column_equivalences,
            } => column_equivalences[0],
        };
        let projected_type = kernel_query::RelType {
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![text_eq],
            },
        };
        let owner_rows = fixture
            .model
            .relations
            .materialize_owned(&fixture.owner)
            .unwrap();
        let projection_calls = Cell::new(0_usize);
        let catalog = super::GammaPreimageCatalog::build(
            &owner_rows,
            &owner_type,
            &projected_type,
            &fixture.context,
            &fixture.registry,
            |row| {
                projection_calls.set(projection_calls.get() + 1);
                Ok(vec![row[0].clone()])
            },
        )
        .unwrap();
        assert_eq!(projection_calls.get(), owner_rows.len());

        for _ in 0..128 {
            let class = super::GammaPreimageCatalog::class_for_view_row(
                &[Value::Text("a".into())],
                &projected_type,
                &fixture.context,
                &fixture.registry,
            )
            .unwrap();
            assert_eq!(
                catalog.representative(&class, &owner_rows),
                Some(&owner_rows[0])
            );
        }
        assert_eq!(projection_calls.get(), owner_rows.len());
    }

    #[test]
    fn lossy_join_project_representative_lookup_propagates_semantic_failure() {
        let fixture = direct_join_fixture();
        let owner_type = RelExpr::Scan(fixture.owner)
            .typecheck(&fixture.context, &fixture.registry)
            .unwrap();
        let unavailable_equivalence = SemanticId::new(9_790_001);
        let visible_owner_type = kernel_query::RelType {
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![unavailable_equivalence],
            },
        };
        let old_full_section = kernel_query::RelationValue::Bag(
            fixture
                .model
                .relations
                .materialize_owned(&fixture.owner)
                .unwrap(),
        );
        let rows = old_full_section.rows().to_vec();
        let project_stages = vec![vec![0]];
        let projection = super::PreparedProjectionPath::from_stage_columns(
            &project_stages,
            owner_type.columns.len(),
        )
        .unwrap();

        let error = super::GammaPreimageCatalog::build(
            old_full_section.rows(),
            &owner_type,
            &visible_owner_type,
            &fixture.context,
            &fixture.registry,
            |row| projection.project_row(row),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            kernel_lens::RelRewriteLiftError::Query(kernel_query::RelQueryError::Semantic(
                kernel_semantics::SemanticError::WrongModuleKind(id)
            )) if id == unavailable_equivalence
        ));
        assert_eq!(rows, old_full_section.rows());
    }

    fn filtered_lookup_join_fixture() -> DirectJoinFixture {
        let text_eq = SemanticId::new(990);
        let owner = SemanticId::new(991);
        let lookup = SemanticId::new(992);
        let mut schema = Schema::new(SchemaRevisionId::new(5));
        for (id, arity) in [(owner, 2_usize), (lookup, 3_usize)] {
            schema
                .define_relation(RelationDef {
                    id,
                    columns: vec![TypeExpr::Scalar(ScalarType::Text); arity],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![text_eq; arity],
                    },
                })
                .unwrap();
        }
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(5));
        environment.pin_module(text_eq, digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let lookup_query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(lookup)),
            column: 2,
            value: Value::Text("yes".into()),
            equivalence: text_eq,
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(owner)),
            right: Box::new(lookup_query),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let prepared = prepare_baseline(query, &context, &registry).unwrap();
        let spec = kernel_change::RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(993)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(994)),
            footprint: kernel_change::RewriteFootprint::opaque_relation(owner),
        };
        let compiled =
            super::compile_prepared_relational_writable_query(&prepared, owner, spec.id, &registry)
                .unwrap();
        let kernel_lens::RelWritableCompilation::Conditional { plan, .. } = compiled.compilation
        else {
            panic!(
                "filtered lookup join must remain conditional until runtime uniqueness is checked"
            )
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            owner,
            vec![
                vec![Value::Text("a".into()), Value::Text("old".into())],
                vec![Value::Text("z".into()), Value::Text("hidden".into())],
            ],
        );
        model.relations.insert(
            lookup,
            vec![
                vec![
                    Value::Text("a".into()),
                    Value::Text("label".into()),
                    Value::Text("yes".into()),
                ],
                vec![
                    Value::Text("a".into()),
                    Value::Text("inactive".into()),
                    Value::Text("no".into()),
                ],
            ],
        );
        let requested = kernel_change::PreparedRewrite {
            spec: kernel_change::RewriteSpecId(SemanticId::new(995)),
            explicit_inputs: Vec::<Value>::new(),
            effect: kernel_change::RewriteEffect::Replace(kernel_query::RelationValue::Bag(vec![
                vec![
                    Value::Text("a".into()),
                    Value::Text("new".into()),
                    Value::Text("a".into()),
                    Value::Text("label".into()),
                    Value::Text("yes".into()),
                ],
            ])),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(996)),
        };
        DirectJoinFixture {
            context,
            registry,
            spec,
            plan,
            model,
            requested,
            owner,
            lookup,
        }
    }

    #[test]
    fn owner_join_accepts_owner_free_filtered_lookup_when_runtime_key_fiber_is_unique() {
        let mut fixture = filtered_lookup_join_fixture();
        let lifted = super::synthesize_direct_scan_join_source_rewrite_for_commit(
            &fixture.plan,
            &fixture.model,
            &fixture.context,
            &fixture.registry,
            &fixture.spec,
            &fixture.requested,
        )
        .unwrap();
        let old_owner = RelExpr::Scan(fixture.owner)
            .evaluate(&fixture.model, &fixture.context, &fixture.registry)
            .unwrap();
        let actual_owner = lifted
            .apply_structural(&old_owner, &fixture.registry)
            .unwrap();
        assert!(
            actual_owner
                .rows()
                .contains(&vec![Value::Text("a".into()), Value::Text("new".into())])
        );
        assert!(
            actual_owner
                .rows()
                .contains(&vec![Value::Text("z".into()), Value::Text("hidden".into())])
        );

        fixture
            .model
            .relations
            .get_mut(&fixture.lookup)
            .unwrap()
            .push(vec![
                Value::Text("a".into()),
                Value::Text("duplicate".into()),
                Value::Text("yes".into()),
            ]);
        assert!(matches!(
            super::synthesize_direct_scan_join_source_rewrite_for_commit(
                &fixture.plan,
                &fixture.model,
                &fixture.context,
                &fixture.registry,
                &fixture.spec,
                &fixture.requested,
            ),
            Err(kernel_lens::RelRewriteLiftError::JoinLookupKeyNotUnique)
        ));
    }

    #[test]
    fn vertical_slice_preserves_semantics_across_layers() {
        let (context, registry) = context_with_explicit_text_equality();
        context.validate().unwrap();
        assert_lifecycle_merge(&context, &registry);
        assert_fine_delta_law();
        assert_identity_transport();
        assert_implicit_retention_flow();
        assert_relational_plan_lowering(&context, &registry);
        assert_plan_certificate();
    }
}
