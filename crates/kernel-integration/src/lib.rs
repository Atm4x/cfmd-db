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
use std::collections::BTreeMap;

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedWritableCompilation {
    pub coordinates: PreparedRelWritableCoordinates,
    pub compilation: RelWritableCompilation,
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
    Ok(PreparedWritableCompilation {
        coordinates,
        compilation,
    })
}

fn synthesize_filter_source_rewrite_for_commit<I: Clone>(
    plan: &RelWritableViewPlan,
    source: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
) -> Result<kernel_query::PreparedRelationRewrite<I>, RelRewriteLiftError> {
    use std::collections::BTreeSet;

    if source_spec.id != plan.rewrite_spec {
        return Err(RelRewriteLiftError::SourceRewriteSpecMismatch {
            expected: plan.rewrite_spec,
            actual: source_spec.id,
        });
    }
    let allowed = BTreeSet::from([
        RelWritableObligation::PredicateAdmissibility,
        RelWritableObligation::DtcGuardNoImpact,
        RelWritableObligation::VmfInvariantClosure,
    ]);
    if !plan.required_obligations.is_subset(&allowed)
        || plan.stages.is_empty()
        || !plan.stages.iter().all(|stage| {
            matches!(
                stage,
                RelRewriteLiftStage::FilterEqConst { .. }
                    | RelRewriteLiftStage::FilterEqColumns { .. }
            )
        })
    {
        return Err(RelRewriteLiftError::UnresolvedObligations(
            plan.required_obligations.clone(),
        ));
    }

    let scan = kernel_query::RelExpr::Scan(plan.owner_relation);
    let old_owner = scan.evaluate(source, semantic, registry)?;
    let old_view = plan.query.evaluate(source, semantic, registry)?;
    let requested_endpoint = requested_view.apply(&old_view);
    let complement = kernel_query::RelExpr::Difference {
        left: Box::new(scan.clone()),
        right: Box::new(plan.query.clone()),
    }
    .evaluate(source, semantic, registry)?;
    let owner_type = scan.typecheck(semantic, registry)?;
    let mut reconstructed_rows = requested_endpoint.rows().to_vec();
    reconstructed_rows.extend(complement.rows().iter().cloned());
    let reconstructed = match &owner_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(reconstructed_rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows: reconstructed_rows,
            column_equivalences: column_equivalences.clone(),
        },
    };
    let delta = kernel_query::RelationDelta::between_values(
        &old_owner,
        &reconstructed,
        owner_type,
        semantic,
        registry,
    )?;
    let normalized_endpoint = delta.apply_to_value(old_owner.clone(), semantic, registry)?;
    let mut candidate_model = source.clone();
    candidate_model
        .relations
        .insert(plan.owner_relation, normalized_endpoint.into_rows());
    let actual_view = plan.query.evaluate(&candidate_model, semantic, registry)?;
    let view_type = plan.query.typecheck(semantic, registry)?;
    if !kernel_query::RelationDelta::between_values(
        &actual_view,
        &requested_endpoint,
        view_type,
        semantic,
        registry,
    )?
    .is_empty()
    {
        return Err(RelRewriteLiftError::RequestedViewInadmissible);
    }
    Ok(delta.prepare_relation_rewrite(
        &old_owner,
        semantic,
        registry,
        source_spec,
        requested_view.explicit_inputs.clone(),
    )?)
}

fn synthesize_bijective_project_source_rewrite_for_commit<I: Clone>(
    plan: &RelWritableViewPlan,
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
    if !plan.required_obligations.is_empty()
        || plan.stages.is_empty()
        || !plan
            .stages
            .iter()
            .all(|stage| matches!(stage, RelRewriteLiftStage::Project { .. }))
    {
        return Err(RelRewriteLiftError::UnresolvedObligations(
            plan.required_obligations.clone(),
        ));
    }

    let scan = kernel_query::RelExpr::Scan(plan.owner_relation);
    let old_owner = scan.evaluate(source, semantic, registry)?;
    let old_view = plan.query.evaluate(source, semantic, registry)?;
    let requested_endpoint = requested_view.apply(&old_view);
    let owner_type = scan.typecheck(semantic, registry)?;

    let mut rows = requested_endpoint.rows().to_vec();
    for stage in plan.stages.iter().rev() {
        let RelRewriteLiftStage::Project { columns } = stage else {
            unreachable!("project-only stage predicate checked above");
        };
        if columns.len() != owner_type.columns.len() {
            return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
        }
        let mut seen = vec![false; columns.len()];
        for &source_column in columns {
            if source_column >= columns.len() || seen[source_column] {
                return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
            }
            seen[source_column] = true;
        }
        for row in &mut rows {
            if row.len() != columns.len() {
                return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
            }
            let projected = row.clone();
            for (view_column, &source_column) in columns.iter().enumerate() {
                row[source_column] = projected[view_column].clone();
            }
        }
    }

    let reconstructed = match &owner_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.clone(),
        },
    };
    let delta = kernel_query::RelationDelta::between_values(
        &old_owner,
        &reconstructed,
        owner_type,
        semantic,
        registry,
    )?;
    let normalized_endpoint = delta.apply_to_value(old_owner.clone(), semantic, registry)?;
    let mut candidate_model = source.clone();
    candidate_model
        .relations
        .insert(plan.owner_relation, normalized_endpoint.into_rows());
    let actual_view = plan.query.evaluate(&candidate_model, semantic, registry)?;
    let view_type = plan.query.typecheck(semantic, registry)?;
    if !kernel_query::RelationDelta::between_values(
        &actual_view,
        &requested_endpoint,
        view_type,
        semantic,
        registry,
    )?
    .is_empty()
    {
        return Err(RelRewriteLiftError::RequestedViewInadmissible);
    }
    Ok(delta.prepare_relation_rewrite(
        &old_owner,
        semantic,
        registry,
        source_spec,
        requested_view.explicit_inputs.clone(),
    )?)
}

fn project_owner_row(
    plan: &RelWritableViewPlan,
    row: &[kernel_model::Value],
) -> Result<kernel_query::Row, RelRewriteLiftError> {
    let mut projected = row.to_vec();
    for stage in &plan.stages {
        let RelRewriteLiftStage::Project { columns } = stage else {
            return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
        };
        let previous = projected;
        projected = columns
            .iter()
            .map(|&column| {
                previous
                    .get(column)
                    .cloned()
                    .ok_or(RelRewriteLiftError::CandidateGenerationUnsupported)
            })
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(projected)
}

fn rows_equal_for_type(
    left: &[kernel_model::Value],
    right: &[kernel_model::Value],
    relation_type: &kernel_query::RelType,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RelRewriteLiftError> {
    let equivalences = match &relation_type.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    };
    if left.len() != right.len() || left.len() != equivalences.len() {
        return Ok(false);
    }
    for ((left, right), &equivalence) in left.iter().zip(right).zip(equivalences) {
        if !registry
            .equivalent(semantic, equivalence, left, right)
            .map_err(kernel_query::RelQueryError::from)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn reconstruct_lossy_project_deletion(
    plan: &RelWritableViewPlan,
    old_owner: &RelationValue,
    owner_type: &kernel_query::RelType,
    view_type: &kernel_query::RelType,
    removed_rows: &[kernel_query::Row],
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelRewriteLiftError> {
    let view_is_set = matches!(
        view_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    let mut remaining = old_owner.rows().to_vec();
    for removed_view in removed_rows {
        let mut matching = Vec::new();
        for (index, source_row) in remaining.iter().enumerate() {
            let projected = project_owner_row(plan, source_row)?;
            if rows_equal_for_type(&projected, removed_view, view_type, semantic, registry)? {
                matching.push(index);
            }
        }
        let Some(&first) = matching.first() else {
            return Err(RelRewriteLiftError::RequestedViewInadmissible);
        };
        if view_is_set {
            for index in matching.into_iter().rev() {
                remaining.remove(index);
            }
            continue;
        }
        for &index in matching.iter().skip(1) {
            if !rows_equal_for_type(
                &remaining[first],
                &remaining[index],
                owner_type,
                semantic,
                registry,
            )? {
                return Err(RelRewriteLiftError::ProjectionPreimageAmbiguous);
            }
        }
        remaining.remove(first);
    }
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
    owner_type: &'a kernel_query::RelType,
    view_type: &'a kernel_query::RelType,
    semantic: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
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
    let visible_columns = owner_projection_columns(plan, context.owner_type.columns.len())?;
    let visible_set = visible_columns
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let hidden_columns = (0..context.owner_type.columns.len())
        .filter(|column| !visible_set.contains(column))
        .collect::<Vec<_>>();
    let before = coordinates
        .relation_determinant_morphism(
            plan.owner_relation,
            old_owner,
            &visible_columns,
            &hidden_columns,
            context.semantic,
            context.registry,
        )
        .map_err(|_| RelRewriteLiftError::DeterminantEvidenceUnavailable)?
        .ok_or(RelRewriteLiftError::ProjectionPreimageAmbiguous)?;
    let mut constructed_new_domain = false;
    for inserted_view in inserted_rows {
        let mut representative = None;
        for source_row in old_owner.rows() {
            let projected = project_owner_row(plan, source_row)?;
            if !rows_equal_for_type(
                &projected,
                inserted_view,
                context.view_type,
                context.semantic,
                context.registry,
            )? {
                continue;
            }
            representative = Some(source_row);
            break;
        }
        if let Some(representative) = representative {
            debug_assert!(!before.materialized_mapping().is_empty());
            reconstructed_rows.push(representative.clone());
            continue;
        }
        let Some(constructor) = constructor else {
            return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
        };
        reconstructed_rows.push(construct_project_owner_row(
            plan,
            inserted_view,
            context.owner_type.columns.len(),
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
                source: &visible_columns,
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
    projected_row: &[kernel_model::Value],
    owner_arity: usize,
    constructor: &FixedHiddenProjectInsertConstructor,
) -> Result<kernel_query::Row, RelRewriteLiftError> {
    if constructor.owner_relation != plan.owner_relation {
        return Err(RelRewriteLiftError::ProjectConstructorOwnerMismatch);
    }
    if constructor.rewrite_spec != plan.rewrite_spec {
        return Err(RelRewriteLiftError::ProjectConstructorRewriteSpecMismatch);
    }
    let visible_columns = owner_projection_columns(plan, owner_arity)?;
    if projected_row.len() != visible_columns.len() {
        return Err(RelRewriteLiftError::RequestedViewInadmissible);
    }
    let visible_set = visible_columns
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    for &column in constructor.hidden_values.keys() {
        if column >= owner_arity {
            return Err(RelRewriteLiftError::ProjectConstructorColumnOutOfBounds {
                column,
                arity: owner_arity,
            });
        }
        if visible_set.contains(&column) {
            return Err(RelRewriteLiftError::ProjectConstructorOverridesVisibleColumn { column });
        }
    }
    let mut owner_row = vec![None; owner_arity];
    for (&column, value) in visible_columns.iter().zip(projected_row) {
        owner_row[column] = Some(value.clone());
    }
    for (column, slot) in owner_row.iter_mut().enumerate() {
        if slot.is_some() {
            continue;
        }
        let Some(value) = constructor.hidden_values.get(&column) else {
            return Err(RelRewriteLiftError::ProjectConstructorMissingHiddenColumn { column });
        };
        *slot = Some(value.clone());
    }
    Ok(owner_row.into_iter().map(Option::unwrap).collect())
}

struct LossyProjectSynthesisContext<'a> {
    semantic: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
    source_spec: &'a kernel_change::RewriteSpec,
    constructor: Option<&'a FixedHiddenProjectInsertConstructor>,
}

fn owner_projection_columns(
    plan: &RelWritableViewPlan,
    owner_arity: usize,
) -> Result<Vec<usize>, RelRewriteLiftError> {
    let mut projected = (0..owner_arity).collect::<Vec<_>>();
    for stage in &plan.stages {
        let RelRewriteLiftStage::Project { columns } = stage else {
            return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
        };
        projected = columns
            .iter()
            .map(|&column| {
                projected
                    .get(column)
                    .copied()
                    .ok_or(RelRewriteLiftError::CandidateGenerationUnsupported)
            })
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(projected)
}

fn synthesize_lossy_project_source_rewrite_with_coordinates<I: Clone>(
    plan: &RelWritableViewPlan,
    source: &kernel_model::FiniteModel,
    requested_view: &PreparedRewrite<RelationValue, I>,
    coordinates: &mut PreparedRelWritableCoordinates,
    context: &LossyProjectSynthesisContext<'_>,
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
    if plan.stages.is_empty()
        || !plan
            .stages
            .iter()
            .all(|stage| matches!(stage, RelRewriteLiftStage::Project { .. }))
        || plan.required_obligations.iter().any(|obligation| {
            !matches!(
                obligation,
                RelWritableObligation::PreserveHiddenColumnsComplement { .. }
                    | RelWritableObligation::HiddenColumnConstructorForInsert { .. }
                    | RelWritableObligation::ProjectionNoSemanticCollapse
            )
        })
    {
        return Err(RelRewriteLiftError::UnresolvedObligations(
            plan.required_obligations.clone(),
        ));
    }
    let scan = kernel_query::RelExpr::Scan(plan.owner_relation);
    let old_owner = scan.evaluate(source, semantic, registry)?;
    let owner_type = scan.typecheck(semantic, registry)?;
    let old_view = plan.query.evaluate(source, semantic, registry)?;
    let view_type = plan.query.typecheck(semantic, registry)?;
    let requested_endpoint = requested_view.apply(&old_view);
    let view_delta = kernel_query::RelationDelta::between_values(
        &old_view,
        &requested_endpoint,
        view_type.clone(),
        semantic,
        registry,
    )?;
    let mut reconstructed = reconstruct_lossy_project_deletion(
        plan,
        &old_owner,
        &owner_type,
        &view_type,
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
            owner_type: &owner_type,
            view_type: &view_type,
            semantic,
            registry,
        },
        coordinates,
        context.constructor,
    )?;
    let delta = kernel_query::RelationDelta::between_values(
        &old_owner,
        &reconstructed,
        owner_type,
        semantic,
        registry,
    )?;
    let normalized_endpoint = delta.apply_to_value(old_owner.clone(), semantic, registry)?;
    let mut candidate_model = source.clone();
    candidate_model
        .relations
        .insert(plan.owner_relation, normalized_endpoint.into_rows());
    let actual_view = plan.query.evaluate(&candidate_model, semantic, registry)?;
    if !kernel_query::RelationDelta::between_values(
        &actual_view,
        &requested_endpoint,
        view_type,
        semantic,
        registry,
    )?
    .is_empty()
    {
        return Err(RelRewriteLiftError::RequestedViewInadmissible);
    }
    Ok(delta.prepare_relation_rewrite(
        &old_owner,
        semantic,
        registry,
        source_spec,
        requested_view.explicit_inputs.clone(),
    )?)
}

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
    synthesize_lossy_project_source_rewrite_with_coordinates(
        plan,
        source,
        requested_view,
        &mut coordinates,
        &LossyProjectSynthesisContext {
            semantic,
            registry,
            source_spec,
            constructor,
        },
    )
}

#[derive(Clone)]
struct DirectJoinSection {
    owner_is_left: bool,
    owner_query: kernel_query::RelExpr,
    full_row_owner_query: kernel_query::RelExpr,
    owner_project_stages: Vec<Vec<usize>>,
    owner_lift_stages: Vec<RelRewriteLiftStage>,
    lookup_query: kernel_query::RelExpr,
    owner_column: usize,
    lookup_column: usize,
    equivalence: kernel_types::SemanticId,
}

#[derive(Clone)]
struct OwnerJoinPipeline {
    full_row_query: kernel_query::RelExpr,
    project_stages: Vec<Vec<usize>>,
    lift_stages: Vec<RelRewriteLiftStage>,
}

fn owner_join_pipeline(
    query: &kernel_query::RelExpr,
    owner_relation: kernel_types::SemanticId,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<OwnerJoinPipeline, RelRewriteLiftError> {
    match query {
        kernel_query::RelExpr::Scan(relation) if *relation == owner_relation => {
            Ok(OwnerJoinPipeline {
                full_row_query: query.clone(),
                project_stages: Vec::new(),
                lift_stages: Vec::new(),
            })
        }
        kernel_query::RelExpr::FilterEqConst {
            input,
            column,
            equivalence,
            ..
        } => {
            let mut pipeline = owner_join_pipeline(input, owner_relation, semantic, registry)?;
            if !pipeline.project_stages.is_empty() {
                return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
            }
            pipeline.full_row_query = query.clone();
            pipeline
                .lift_stages
                .push(RelRewriteLiftStage::FilterEqConst {
                    column: *column,
                    equivalence: *equivalence,
                });
            Ok(pipeline)
        }
        kernel_query::RelExpr::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => {
            let mut pipeline = owner_join_pipeline(input, owner_relation, semantic, registry)?;
            if !pipeline.project_stages.is_empty() {
                return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
            }
            pipeline.full_row_query = query.clone();
            pipeline
                .lift_stages
                .push(RelRewriteLiftStage::FilterEqColumns {
                    left_column: *left_column,
                    right_column: *right_column,
                    equivalence: *equivalence,
                });
            Ok(pipeline)
        }
        kernel_query::RelExpr::Project { input, columns } => {
            let mut pipeline = owner_join_pipeline(input, owner_relation, semantic, registry)?;
            let input_arity = input.typecheck(semantic, registry)?.columns.len();
            let mut seen = vec![false; input_arity];
            for &column in columns {
                if column >= input_arity || seen[column] {
                    return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
                }
                seen[column] = true;
            }
            pipeline.project_stages.push(columns.clone());
            pipeline.lift_stages.push(RelRewriteLiftStage::Project {
                columns: columns.clone(),
            });
            Ok(pipeline)
        }
        _ => Err(RelRewriteLiftError::CandidateGenerationUnsupported),
    }
}

fn direct_join_section(
    plan: &RelWritableViewPlan,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<DirectJoinSection, RelRewriteLiftError> {
    let (left, right, left_column, right_column, equivalence) = match &plan.query {
        kernel_query::RelExpr::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => (
            left.as_ref(),
            right.as_ref(),
            *left_column,
            *right_column,
            *equivalence,
        ),
        _ => return Err(RelRewriteLiftError::CandidateGenerationUnsupported),
    };
    let section = if !right.scan_relations().contains(&plan.owner_relation) {
        let pipeline = owner_join_pipeline(left, plan.owner_relation, semantic, registry)?;
        DirectJoinSection {
            owner_is_left: true,
            owner_query: left.clone(),
            full_row_owner_query: pipeline.full_row_query,
            owner_project_stages: pipeline.project_stages,
            owner_lift_stages: pipeline.lift_stages,
            lookup_query: right.clone(),
            owner_column: left_column,
            lookup_column: right_column,
            equivalence,
        }
    } else if !left.scan_relations().contains(&plan.owner_relation) {
        let pipeline = owner_join_pipeline(right, plan.owner_relation, semantic, registry)?;
        DirectJoinSection {
            owner_is_left: false,
            owner_query: right.clone(),
            full_row_owner_query: pipeline.full_row_query,
            owner_project_stages: pipeline.project_stages,
            owner_lift_stages: pipeline.lift_stages,
            lookup_query: left.clone(),
            owner_column: right_column,
            lookup_column: left_column,
            equivalence,
        }
    } else {
        return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
    };
    let Some((last, prefix)) = plan.stages.split_last() else {
        return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
    };
    if prefix == section.owner_lift_stages.as_slice()
        && matches!(
            last,
            RelRewriteLiftStage::JoinOwnerSide {
                owner_is_left,
                owner_column,
                lookup_column,
                equivalence,
            } if *owner_is_left == section.owner_is_left
                && *owner_column == section.owner_column
                && *lookup_column == section.lookup_column
                && *equivalence == section.equivalence
        )
    {
        Ok(section)
    } else {
        Err(RelRewriteLiftError::CandidateGenerationUnsupported)
    }
}

fn merge_owner_sections(
    accepted: RelationValue,
    rejected: &RelationValue,
    owner_type: &kernel_query::RelType,
) -> RelationValue {
    let mut rows = accepted.into_rows();
    rows.extend(rejected.rows().iter().cloned());
    match &owner_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.clone(),
        },
    }
}

fn direct_join_obligations_supported(plan: &RelWritableViewPlan) -> bool {
    let mut determinants = 0;
    plan.required_obligations
        .iter()
        .all(|obligation| match obligation {
            RelWritableObligation::PreserveHiddenColumnsComplement { .. }
            | RelWritableObligation::HiddenColumnConstructorForInsert { .. }
            | RelWritableObligation::ProjectionNoSemanticCollapse
            | RelWritableObligation::PredicateAdmissibility
            | RelWritableObligation::DtcGuardNoImpact
            | RelWritableObligation::VmfInvariantClosure => true,
            RelWritableObligation::JoinLookupDeterminant { .. } => {
                determinants += 1;
                determinants <= 1
            }
        })
}

fn validate_direct_lookup_key_uniqueness(
    lookup_query: &kernel_query::RelExpr,
    lookup_column: usize,
    equivalence: kernel_types::SemanticId,
    source: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(), RelRewriteLiftError> {
    let lookup_value = lookup_query.evaluate(source, semantic, registry)?;
    for row in lookup_value.rows() {
        let key = row
            .get(lookup_column)
            .ok_or(kernel_query::RelQueryError::ColumnOutOfBounds)?;
        let key_fiber = kernel_query::RelExpr::FilterEqConst {
            input: Box::new(lookup_query.clone()),
            column: lookup_column,
            value: key.clone(),
            equivalence,
        }
        .evaluate(source, semantic, registry)?;
        if key_fiber.rows().len() != 1 {
            return Err(RelRewriteLiftError::JoinLookupKeyNotUnique);
        }
    }
    Ok(())
}

fn direct_join_reconstructed_owner(
    requested_endpoint: &RelationValue,
    unmatched: &RelationValue,
    visible_owner_type: &kernel_query::RelType,
    lookup_arity: usize,
    owner_is_left: bool,
) -> Result<RelationValue, RelRewriteLiftError> {
    let owner_arity = visible_owner_type.columns.len();
    let mut rows = Vec::with_capacity(requested_endpoint.rows().len() + unmatched.rows().len());
    for row in requested_endpoint.rows() {
        if row.len() != owner_arity + lookup_arity {
            return Err(RelRewriteLiftError::RequestedViewInadmissible);
        }
        rows.push(if owner_is_left {
            row[..owner_arity].to_vec()
        } else {
            row[lookup_arity..].to_vec()
        });
    }
    rows.extend(unmatched.rows().iter().cloned());
    Ok(match &visible_owner_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.clone(),
        },
    })
}

fn invert_bijective_project_stages(
    value: RelationValue,
    project_stages: &[Vec<usize>],
    owner_type: &kernel_query::RelType,
) -> Result<RelationValue, RelRewriteLiftError> {
    let mut rows = value.into_rows();
    for columns in project_stages.iter().rev() {
        for row in &mut rows {
            if row.len() != columns.len() {
                return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
            }
            let projected = row.clone();
            for (view_column, &source_column) in columns.iter().enumerate() {
                row[source_column] = projected[view_column].clone();
            }
        }
    }
    Ok(match &owner_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.clone(),
        },
    })
}

fn projection_columns_from_stages(
    project_stages: &[Vec<usize>],
    owner_arity: usize,
) -> Result<Vec<usize>, RelRewriteLiftError> {
    let mut projected = (0..owner_arity).collect::<Vec<_>>();
    for columns in project_stages {
        projected = columns
            .iter()
            .map(|&column| {
                projected
                    .get(column)
                    .copied()
                    .ok_or(RelRewriteLiftError::CandidateGenerationUnsupported)
            })
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(projected)
}

fn project_row_through_stages(
    row: &[kernel_model::Value],
    project_stages: &[Vec<usize>],
) -> Result<kernel_query::Row, RelRewriteLiftError> {
    let mut projected = row.to_vec();
    for columns in project_stages {
        projected = columns
            .iter()
            .map(|&column| {
                projected
                    .get(column)
                    .cloned()
                    .ok_or(RelRewriteLiftError::CandidateGenerationUnsupported)
            })
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(projected)
}

struct LossyJoinProjectContext<'a> {
    owner_relation: kernel_types::SemanticId,
    rewrite_spec: kernel_change::RewriteSpecId,
    owner_type: &'a kernel_query::RelType,
    visible_owner_type: &'a kernel_query::RelType,
    project_stages: &'a [Vec<usize>],
    semantic: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
    constructor: Option<&'a FixedHiddenProjectInsertConstructor>,
}

fn remove_lossy_join_project_rows(
    rows: &mut Vec<kernel_query::Row>,
    removed_rows: &[kernel_query::Row],
    context: &LossyJoinProjectContext<'_>,
) -> Result<(), RelRewriteLiftError> {
    let view_is_set = matches!(
        context.visible_owner_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    for removed_view in removed_rows {
        let mut matching = Vec::new();
        for (index, source_row) in rows.iter().enumerate() {
            let projected = project_row_through_stages(source_row, context.project_stages)?;
            if rows_equal_for_type(
                &projected,
                removed_view,
                context.visible_owner_type,
                context.semantic,
                context.registry,
            )? {
                matching.push(index);
            }
        }
        let Some(&first) = matching.first() else {
            return Err(RelRewriteLiftError::RequestedViewInadmissible);
        };
        if view_is_set {
            for index in matching.into_iter().rev() {
                rows.remove(index);
            }
            continue;
        }
        for &index in matching.iter().skip(1) {
            if !rows_equal_for_type(
                &rows[first],
                &rows[index],
                context.owner_type,
                context.semantic,
                context.registry,
            )? {
                return Err(RelRewriteLiftError::ProjectionPreimageAmbiguous);
            }
        }
        rows.remove(first);
    }
    Ok(())
}

fn extend_lossy_join_project_rows(
    rows: &mut Vec<kernel_query::Row>,
    inserted_rows: &[kernel_query::Row],
    old_full_section: &RelationValue,
    visible_columns: &[usize],
    coordinates: &mut PreparedRelWritableCoordinates,
    context: &LossyJoinProjectContext<'_>,
) -> Result<bool, RelRewriteLiftError> {
    let visible_set = visible_columns
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let hidden_columns = (0..context.owner_type.columns.len())
        .filter(|column| !visible_set.contains(column))
        .collect::<Vec<_>>();
    let before = coordinates
        .relation_determinant_morphism(
            context.owner_relation,
            old_full_section,
            visible_columns,
            &hidden_columns,
            context.semantic,
            context.registry,
        )
        .map_err(|_| RelRewriteLiftError::DeterminantEvidenceUnavailable)?
        .ok_or(RelRewriteLiftError::ProjectionPreimageAmbiguous)?;
    let mut constructed_new_domain = false;
    for inserted_view in inserted_rows {
        let representative = old_full_section.rows().iter().find_map(|source_row| {
            let projected = project_row_through_stages(source_row, context.project_stages).ok()?;
            rows_equal_for_type(
                &projected,
                inserted_view,
                context.visible_owner_type,
                context.semantic,
                context.registry,
            )
            .ok()?
            .then(|| source_row.clone())
        });
        if let Some(representative) = representative {
            debug_assert!(!before.materialized_mapping().is_empty());
            rows.push(representative);
            continue;
        }
        let Some(constructor) = context.constructor else {
            return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
        };
        rows.push(construct_owner_row_from_projection(
            inserted_view,
            visible_columns,
            context.owner_type.columns.len(),
            context.owner_relation,
            context.rewrite_spec,
            constructor,
        )?);
        constructed_new_domain = true;
    }
    Ok(constructed_new_domain)
}

fn reconstruct_lossy_join_owner_section(
    old_full_section: &RelationValue,
    requested_visible_section: &RelationValue,
    coordinates: &mut PreparedRelWritableCoordinates,
    context: &LossyJoinProjectContext<'_>,
) -> Result<RelationValue, RelRewriteLiftError> {
    let view_delta = kernel_query::RelationDelta::between_values(
        &project_relation_value(old_full_section, context)?,
        requested_visible_section,
        context.visible_owner_type.clone(),
        context.semantic,
        context.registry,
    )?;
    let mut rows = old_full_section.rows().to_vec();
    remove_lossy_join_project_rows(&mut rows, &view_delta.removed, context)?;
    let visible_columns =
        projection_columns_from_stages(context.project_stages, context.owner_type.columns.len())?;
    let constructed_new_domain = extend_lossy_join_project_rows(
        &mut rows,
        &view_delta.inserted,
        old_full_section,
        &visible_columns,
        coordinates,
        context,
    )?;
    let reconstructed = match &context.owner_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.clone(),
        },
    };
    let visible_set = visible_columns
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let hidden_columns = (0..context.owner_type.columns.len())
        .filter(|column| !visible_set.contains(column))
        .collect::<Vec<_>>();
    let revalidated = coordinates
        .revalidate_relation_determinant(
            context.owner_relation,
            old_full_section,
            &reconstructed,
            RelationDeterminantColumns {
                source: &visible_columns,
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
    Ok(reconstructed)
}

fn project_relation_value(
    value: &RelationValue,
    context: &LossyJoinProjectContext<'_>,
) -> Result<RelationValue, RelRewriteLiftError> {
    let rows = value
        .rows()
        .iter()
        .map(|row| project_row_through_stages(row, context.project_stages))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(match &context.visible_owner_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.clone(),
        },
    })
}

fn construct_owner_row_from_projection(
    projected_row: &[kernel_model::Value],
    visible_columns: &[usize],
    owner_arity: usize,
    owner_relation: kernel_types::SemanticId,
    rewrite_spec: kernel_change::RewriteSpecId,
    constructor: &FixedHiddenProjectInsertConstructor,
) -> Result<kernel_query::Row, RelRewriteLiftError> {
    if constructor.owner_relation != owner_relation {
        return Err(RelRewriteLiftError::ProjectConstructorOwnerMismatch);
    }
    if constructor.rewrite_spec != rewrite_spec {
        return Err(RelRewriteLiftError::ProjectConstructorRewriteSpecMismatch);
    }
    if projected_row.len() != visible_columns.len() {
        return Err(RelRewriteLiftError::RequestedViewInadmissible);
    }
    let visible_set = visible_columns
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    for &column in constructor.hidden_values.keys() {
        if column >= owner_arity {
            return Err(RelRewriteLiftError::ProjectConstructorColumnOutOfBounds {
                column,
                arity: owner_arity,
            });
        }
        if visible_set.contains(&column) {
            return Err(RelRewriteLiftError::ProjectConstructorOverridesVisibleColumn { column });
        }
    }
    let mut owner_row = vec![None; owner_arity];
    for (&column, value) in visible_columns.iter().zip(projected_row) {
        owner_row[column] = Some(value.clone());
    }
    for (column, slot) in owner_row.iter_mut().enumerate() {
        if slot.is_some() {
            continue;
        }
        let Some(value) = constructor.hidden_values.get(&column) else {
            return Err(RelRewriteLiftError::ProjectConstructorMissingHiddenColumn { column });
        };
        *slot = Some(value.clone());
    }
    Ok(owner_row.into_iter().map(Option::unwrap).collect())
}

#[cfg(test)]
fn synthesize_direct_scan_join_source_rewrite_for_commit<I: Clone>(
    plan: &RelWritableViewPlan,
    source: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
) -> Result<kernel_query::PreparedRelationRewrite<I>, RelRewriteLiftError> {
    synthesize_direct_scan_join_source_rewrite_with_constructor_for_commit(
        plan,
        source,
        semantic,
        registry,
        source_spec,
        requested_view,
        None,
    )
}

fn reconstruct_join_owner_candidate<I: Clone>(
    plan: &RelWritableViewPlan,
    source: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    requested_view: &PreparedRewrite<RelationValue, I>,
    project_constructor: Option<&FixedHiddenProjectInsertConstructor>,
) -> Result<(RelationValue, RelationValue, RelationValue), RelRewriteLiftError> {
    let section = direct_join_section(plan, semantic, registry)?;
    let owner_scan = kernel_query::RelExpr::Scan(plan.owner_relation);
    validate_direct_lookup_key_uniqueness(
        &section.lookup_query,
        section.lookup_column,
        section.equivalence,
        source,
        semantic,
        registry,
    )?;
    let owner_type = owner_scan.typecheck(semantic, registry)?;
    let visible_owner_type = section.owner_query.typecheck(semantic, registry)?;
    let lookup_arity = section
        .lookup_query
        .typecheck(semantic, registry)?
        .columns
        .len();
    let old_owner = owner_scan.evaluate(source, semantic, registry)?;
    let old_view = plan.query.evaluate(source, semantic, registry)?;
    let requested_endpoint = requested_view.apply(&old_view);
    let unmatched = kernel_query::RelExpr::AntiJoin {
        left: Box::new(section.owner_query.clone()),
        right: Box::new(section.lookup_query.clone()),
        left_column: section.owner_column,
        right_column: section.lookup_column,
        equivalence: section.equivalence,
    }
    .evaluate(source, semantic, registry)?;
    let visible = direct_join_reconstructed_owner(
        &requested_endpoint,
        &unmatched,
        &visible_owner_type,
        lookup_arity,
        section.owner_is_left,
    )?;
    let accepted = if section
        .owner_project_stages
        .iter()
        .all(|columns| columns.len() == owner_type.columns.len())
    {
        invert_bijective_project_stages(visible, &section.owner_project_stages, &owner_type)?
    } else {
        let old_full_section = section
            .full_row_owner_query
            .evaluate(source, semantic, registry)?;
        let prepared = kernel_plan::prepare_baseline(plan.query.clone(), semantic, registry)
            .map_err(|_| RelRewriteLiftError::DeterminantEvidenceUnavailable)?;
        let mut coordinates = prepared
            .writable_coordinates(registry)
            .map_err(|_| RelRewriteLiftError::DeterminantEvidenceUnavailable)?;
        reconstruct_lossy_join_owner_section(
            &old_full_section,
            &visible,
            &mut coordinates,
            &LossyJoinProjectContext {
                owner_relation: plan.owner_relation,
                rewrite_spec: plan.rewrite_spec,
                owner_type: &owner_type,
                visible_owner_type: &visible_owner_type,
                project_stages: &section.owner_project_stages,
                semantic,
                registry,
                constructor: project_constructor,
            },
        )?
    };
    let rejected = kernel_query::RelExpr::Difference {
        left: Box::new(owner_scan),
        right: Box::new(section.full_row_owner_query),
    }
    .evaluate(source, semantic, registry)?;
    Ok((
        old_owner,
        requested_endpoint,
        merge_owner_sections(accepted, &rejected, &owner_type),
    ))
}

fn synthesize_direct_scan_join_source_rewrite_with_constructor_for_commit<I: Clone>(
    plan: &RelWritableViewPlan,
    source: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
    project_constructor: Option<&FixedHiddenProjectInsertConstructor>,
) -> Result<kernel_query::PreparedRelationRewrite<I>, RelRewriteLiftError> {
    if source_spec.id != plan.rewrite_spec {
        return Err(RelRewriteLiftError::SourceRewriteSpecMismatch {
            expected: plan.rewrite_spec,
            actual: source_spec.id,
        });
    }
    if !direct_join_obligations_supported(plan) {
        return Err(RelRewriteLiftError::UnresolvedObligations(
            plan.required_obligations.clone(),
        ));
    }
    let (old_owner, requested_endpoint, reconstructed) = reconstruct_join_owner_candidate(
        plan,
        source,
        semantic,
        registry,
        requested_view,
        project_constructor,
    )?;
    let owner_type =
        kernel_query::RelExpr::Scan(plan.owner_relation).typecheck(semantic, registry)?;
    let delta = kernel_query::RelationDelta::between_values(
        &old_owner,
        &reconstructed,
        owner_type,
        semantic,
        registry,
    )?;
    let normalized_endpoint = delta.apply_to_value(old_owner.clone(), semantic, registry)?;
    let mut candidate_model = source.clone();
    candidate_model
        .relations
        .insert(plan.owner_relation, normalized_endpoint.into_rows());
    let actual_view = plan.query.evaluate(&candidate_model, semantic, registry)?;
    let view_type = plan.query.typecheck(semantic, registry)?;
    if !kernel_query::RelationDelta::between_values(
        &actual_view,
        &requested_endpoint,
        view_type,
        semantic,
        registry,
    )?
    .is_empty()
    {
        return Err(RelRewriteLiftError::RequestedViewInadmissible);
    }
    Ok(delta.prepare_relation_rewrite(
        &old_owner,
        semantic,
        registry,
        source_spec,
        requested_view.explicit_inputs.clone(),
    )?)
}

/// Executes the relational writable-view boundary without creating a second
/// publication path. Lift classification runs against one immutable runtime
/// snapshot; only the unique case is handed to the existing durable
/// `RelationRewriteExact` path, which re-derives the target and re-runs DTC, VMF
/// and freshness validation before publication.
pub fn commit_unique_relational_view_rewrite<I: Clone + PartialEq>(
    runtime: &DurableRuntime,
    transaction_id: ClientTransactionId,
    target_revision: RevisionId,
    plan: &RelWritableViewPlan,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
) -> Result<WritableViewCommitOutcome, WritableViewCommitError> {
    commit_unique_relational_view_rewrite_inner(
        runtime,
        transaction_id,
        target_revision,
        plan,
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
    plan: &RelWritableViewPlan,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
    constructor: &FixedHiddenProjectInsertConstructor,
) -> Result<WritableViewCommitOutcome, WritableViewCommitError> {
    commit_unique_relational_view_rewrite_inner(
        runtime,
        transaction_id,
        target_revision,
        plan,
        source_spec,
        requested_view,
        Some(constructor),
    )
}

fn commit_unique_relational_view_rewrite_inner<I: Clone + PartialEq>(
    runtime: &DurableRuntime,
    transaction_id: ClientTransactionId,
    target_revision: RevisionId,
    plan: &RelWritableViewPlan,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
    project_constructor: Option<&FixedHiddenProjectInsertConstructor>,
) -> Result<WritableViewCommitOutcome, WritableViewCommitError> {
    let snapshot = runtime
        .snapshot()
        .map_err(DurableRuntimeCommitError::from)?;
    let source_revision = snapshot.revision().id();
    let registry = runtime.semantic_registry();
    let unique = match plan.synthesize_identity_source_rewrite(
        &snapshot.revision().state().model,
        snapshot.revision().semantic_context(),
        registry,
        source_spec,
        requested_view,
    ) {
        Ok(unique) => unique,
        Err(
            RelRewriteLiftError::CandidateGenerationUnsupported
            | RelRewriteLiftError::UnresolvedObligations(_),
        ) => match synthesize_bijective_project_source_rewrite_for_commit(
            plan,
            &snapshot.revision().state().model,
            snapshot.revision().semantic_context(),
            registry,
            source_spec,
            requested_view,
        ) {
            Ok(unique) => unique,
            Err(
                RelRewriteLiftError::CandidateGenerationUnsupported
                | RelRewriteLiftError::UnresolvedObligations(_),
            ) => match synthesize_lossy_project_deletion_source_rewrite_for_commit(
                plan,
                &snapshot.revision().state().model,
                snapshot.revision().semantic_context(),
                registry,
                source_spec,
                requested_view,
                project_constructor,
            ) {
                Ok(unique) => unique,
                Err(
                    RelRewriteLiftError::CandidateGenerationUnsupported
                    | RelRewriteLiftError::UnresolvedObligations(_),
                ) => match synthesize_direct_scan_join_source_rewrite_with_constructor_for_commit(
                    plan,
                    &snapshot.revision().state().model,
                    snapshot.revision().semantic_context(),
                    registry,
                    source_spec,
                    requested_view,
                    project_constructor,
                ) {
                    Ok(unique) => unique,
                    Err(
                        RelRewriteLiftError::CandidateGenerationUnsupported
                        | RelRewriteLiftError::UnresolvedObligations(_),
                    ) => synthesize_filter_source_rewrite_for_commit(
                        plan,
                        &snapshot.revision().state().model,
                        snapshot.revision().semantic_context(),
                        registry,
                        source_spec,
                        requested_view,
                    )?,
                    Err(error) => return Err(error.into()),
                },
                Err(error) => return Err(error.into()),
            },
            Err(error) => return Err(error.into()),
        },
        Err(error) => return Err(error.into()),
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
            footprint: kernel_change::RewriteFootprint::default(),
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
        let lifted = super::synthesize_filter_source_rewrite_for_commit(
            &plan,
            &model,
            &context,
            &registry,
            &spec,
            &keep_alpha,
        )
        .unwrap();
        assert!(lifted.delta.is_empty());

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
                &plan, &model, &context, &registry, &spec, &bad_view,
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
            footprint: kernel_change::RewriteFootprint::default(),
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
        let lifted = super::synthesize_bijective_project_source_rewrite_for_commit(
            &plan, &model, &context, &registry, &spec, &requested,
        )
        .unwrap();
        let old_owner = RelExpr::Scan(relation)
            .evaluate(&model, &context, &registry)
            .unwrap();
        assert_eq!(
            lifted.rewrite.apply(&old_owner),
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
            footprint: kernel_change::RewriteFootprint::default(),
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
            lifted.delta.removed,
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
            footprint: kernel_change::RewriteFootprint::default(),
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
            lifted.delta.inserted,
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
            footprint: kernel_change::RewriteFootprint::default(),
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
            lifted.delta.inserted,
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
            footprint: kernel_change::RewriteFootprint::default(),
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
        let actual_owner = lifted.rewrite.apply(&old_owner);
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
        let actual_owner = lifted.rewrite.apply(&old_owner);
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
        let actual_owner = lifted.rewrite.apply(&old_owner);
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
            lifted.delta.inserted,
            vec![vec![
                Value::Text("b".into()),
                Value::Text("hidden-new".into())
            ]]
        );
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
            footprint: kernel_change::RewriteFootprint::default(),
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
        let actual_owner = lifted.rewrite.apply(&old_owner);
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
