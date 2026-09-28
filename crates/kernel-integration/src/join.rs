use super::{
    FixedHiddenProjectInsertConstructor, GammaPreimageCatalog, OwnerCertificationContext,
    PreparedLiftBase, PreparedLiftContext, PreparedProjectionPath, certify_owner_candidate,
};
use kernel_change::PreparedRewrite;
use kernel_lens::{
    RelRewriteLiftError, RelRewriteLiftStage, RelWritableObligation, RelWritableViewPlan,
};
use kernel_plan::{PreparedRelWritableCoordinates, RelationDeterminantColumns};
use kernel_query::RelationValue;
use std::collections::BTreeSet;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedDirectJoinLift {
    base: PreparedLiftBase,
    owner_is_left: bool,
    full_row_owner_query: kernel_query::PreparedRelExpr,
    lookup_query: kernel_query::PreparedRelExpr,
    unmatched_query: kernel_query::PreparedRelExpr,
    rejected_query: kernel_query::PreparedRelExpr,
    lookup_column: usize,
    equivalence: kernel_types::SemanticId,
    visible_owner_type: kernel_query::RelType,
    lookup_arity: usize,
    projection: PreparedProjectionPath,
}

impl PreparedDirectJoinLift {
    pub(super) fn prepare(
        plan: &RelWritableViewPlan,
        semantic: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelRewriteLiftError> {
        if !direct_join_obligations_supported(plan) {
            return Err(RelRewriteLiftError::UnresolvedObligations(
                plan.required_obligations.clone(),
            ));
        }
        let section = direct_join_section(plan, semantic, registry)?;
        let base = PreparedLiftBase::prepare(plan, semantic, registry)?;
        let owner_scan = kernel_query::RelExpr::Scan(plan.owner_relation);
        let visible_owner_query = section.owner_query.prepare(semantic, registry)?;
        let visible_owner_type = visible_owner_query.result_type().clone();
        let lookup_query = section.lookup_query.prepare(semantic, registry)?;
        let lookup_arity = lookup_query.result_type().columns.len();
        let projection = PreparedProjectionPath::from_stage_columns(
            &section.owner_project_stages,
            base.owner_type().columns.len(),
        )?;
        let unmatched_query = kernel_query::RelExpr::AntiJoin {
            left: Box::new(section.owner_query),
            right: Box::new(section.lookup_query.clone()),
            left_column: section.owner_column,
            right_column: section.lookup_column,
            equivalence: section.equivalence,
        };
        let rejected_query = kernel_query::RelExpr::Difference {
            left: Box::new(owner_scan),
            right: Box::new(section.full_row_owner_query.clone()),
        };
        Ok(Self {
            base,
            owner_is_left: section.owner_is_left,
            full_row_owner_query: section.full_row_owner_query.prepare(semantic, registry)?,
            lookup_query,
            unmatched_query: unmatched_query.prepare(semantic, registry)?,
            rejected_query: rejected_query.prepare(semantic, registry)?,
            lookup_column: section.lookup_column,
            equivalence: section.equivalence,
            visible_owner_type,
            lookup_arity,
            projection,
        })
    }
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

pub(super) fn validate_direct_lookup_key_uniqueness(
    lookup_query: &kernel_query::PreparedRelExpr,
    lookup_column: usize,
    equivalence: kernel_types::SemanticId,
    source: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(), RelRewriteLiftError> {
    let lookup_value = lookup_query.evaluate(source, semantic, registry)?;
    let mut observed = BTreeSet::new();
    for row in lookup_value.rows() {
        let key = row
            .get(lookup_column)
            .ok_or(kernel_query::RelQueryError::ColumnOutOfBounds)?;
        let canonical = registry
            .canonical_equivalence_key(semantic, equivalence, key)
            .map_err(kernel_query::RelQueryError::from)?;
        if !observed.insert(canonical) {
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

struct LossyJoinProjectContext<'a> {
    owner_relation: kernel_types::SemanticId,
    rewrite_spec: kernel_change::RewriteSpecId,
    owner_type: &'a kernel_query::RelType,
    visible_owner_type: &'a kernel_query::RelType,
    projection: &'a PreparedProjectionPath,
    semantic: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
    constructor: Option<&'a FixedHiddenProjectInsertConstructor>,
}

fn remove_lossy_join_project_rows(
    rows: &[kernel_query::Row],
    removed_rows: &[kernel_query::Row],
    context: &LossyJoinProjectContext<'_>,
    catalog: &mut GammaPreimageCatalog,
) -> Result<Vec<kernel_query::Row>, RelRewriteLiftError> {
    let view_is_set = matches!(
        context.visible_owner_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    let mut removed = vec![false; rows.len()];
    for removed_view in removed_rows {
        let class = GammaPreimageCatalog::class_for_view_row(
            removed_view,
            context.visible_owner_type,
            context.semantic,
            context.registry,
        )?;
        catalog.remove_requested(&class, &mut removed, view_is_set)?;
    }
    Ok(rows
        .iter()
        .zip(removed)
        .filter(|(_, removed)| !*removed)
        .map(|(row, _)| row.clone())
        .collect())
}

fn extend_lossy_join_project_rows(
    rows: &mut Vec<kernel_query::Row>,
    inserted_rows: &[kernel_query::Row],
    old_full_section: &RelationValue,
    coordinates: &mut PreparedRelWritableCoordinates,
    context: &LossyJoinProjectContext<'_>,
    catalog: &GammaPreimageCatalog,
) -> Result<bool, RelRewriteLiftError> {
    let visible_columns = context.projection.visible_columns();
    let hidden_columns = context.projection.hidden_columns();
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
        let class = GammaPreimageCatalog::class_for_view_row(
            inserted_view,
            context.visible_owner_type,
            context.semantic,
            context.registry,
        )?;
        if let Some(representative) = catalog.representative(&class, old_full_section.rows()) {
            debug_assert!(!before.materialized_mapping().is_empty());
            rows.push(representative.clone());
            continue;
        }
        let Some(constructor) = context.constructor else {
            return Err(RelRewriteLiftError::CandidateGenerationUnsupported);
        };
        rows.push(construct_owner_row_from_projection(
            inserted_view,
            context.projection,
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
    let mut preimages = GammaPreimageCatalog::build(
        old_full_section.rows(),
        context.owner_type,
        context.visible_owner_type,
        context.semantic,
        context.registry,
        |row| context.projection.project_row(row),
    )?;
    let mut rows = remove_lossy_join_project_rows(
        old_full_section.rows(),
        &view_delta.removed,
        context,
        &mut preimages,
    )?;
    let visible_columns = context.projection.visible_columns();
    let constructed_new_domain = extend_lossy_join_project_rows(
        &mut rows,
        &view_delta.inserted,
        old_full_section,
        coordinates,
        context,
        &preimages,
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
    let hidden_columns = context.projection.hidden_columns();
    let revalidated = coordinates
        .revalidate_relation_determinant(
            context.owner_relation,
            old_full_section,
            &reconstructed,
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
    Ok(reconstructed)
}

fn project_relation_value(
    value: &RelationValue,
    context: &LossyJoinProjectContext<'_>,
) -> Result<RelationValue, RelRewriteLiftError> {
    let rows = value
        .rows()
        .iter()
        .map(|row| context.projection.project_row(row))
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
    projection: &PreparedProjectionPath,
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
    projection.inflate_with_hidden(projected_row, &constructor.hidden_values)
}

#[cfg(test)]
pub(super) fn synthesize_direct_scan_join_source_rewrite_for_commit<I: Clone>(
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
    join: &PreparedDirectJoinLift,
    source: &kernel_model::FiniteModel,
    requested_view: &PreparedRewrite<RelationValue, I>,
    coordinates: &mut PreparedRelWritableCoordinates,
    context: &PreparedLiftContext<'_>,
) -> Result<(RelationValue, RelationValue, RelationValue), RelRewriteLiftError> {
    let semantic = context.semantic;
    let registry = context.registry;
    validate_direct_lookup_key_uniqueness(
        &join.lookup_query,
        join.lookup_column,
        join.equivalence,
        source,
        semantic,
        registry,
    )?;
    let old_owner = join.base.owner_query.evaluate(source, semantic, registry)?;
    let old_view = join.base.view_query.evaluate(source, semantic, registry)?;
    let requested_endpoint = requested_view.apply(&old_view);
    let unmatched = join.unmatched_query.evaluate(source, semantic, registry)?;
    let visible = direct_join_reconstructed_owner(
        &requested_endpoint,
        &unmatched,
        &join.visible_owner_type,
        join.lookup_arity,
        join.owner_is_left,
    )?;
    let accepted = if join.projection.is_bijective() {
        join.projection
            .invert_bijective_value(&visible, join.base.owner_type())?
    } else {
        let old_full_section = join
            .full_row_owner_query
            .evaluate(source, semantic, registry)?;
        reconstruct_lossy_join_owner_section(
            &old_full_section,
            &visible,
            coordinates,
            &LossyJoinProjectContext {
                owner_relation: plan.owner_relation,
                rewrite_spec: plan.rewrite_spec,
                owner_type: join.base.owner_type(),
                visible_owner_type: &join.visible_owner_type,
                projection: &join.projection,
                semantic,
                registry,
                constructor: context.constructor,
            },
        )?
    };
    let rejected = join.rejected_query.evaluate(source, semantic, registry)?;
    Ok((
        old_owner,
        requested_endpoint,
        merge_owner_sections(accepted, &rejected, join.base.owner_type()),
    ))
}

pub(super) fn synthesize_direct_scan_join_source_rewrite_with_constructor_for_commit_prepared<
    I: Clone,
>(
    plan: &RelWritableViewPlan,
    join: &PreparedDirectJoinLift,
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
    let (old_owner, requested_endpoint, reconstructed) =
        reconstruct_join_owner_candidate(plan, join, source, requested_view, coordinates, context)?;
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
            owner_type: join.base.owner_type(),
            view_query: &join.base.view_query,
        },
    )
}

#[cfg(test)]
pub(super) fn synthesize_direct_scan_join_source_rewrite_with_constructor_for_commit<I: Clone>(
    plan: &RelWritableViewPlan,
    source: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    source_spec: &kernel_change::RewriteSpec,
    requested_view: &PreparedRewrite<RelationValue, I>,
    project_constructor: Option<&FixedHiddenProjectInsertConstructor>,
) -> Result<kernel_query::PreparedRelationRewrite<I>, RelRewriteLiftError> {
    let prepared = kernel_plan::prepare_baseline(plan.query.clone(), semantic, registry)
        .map_err(|_| RelRewriteLiftError::DeterminantEvidenceUnavailable)?;
    let mut coordinates = prepared
        .writable_coordinates(registry)
        .map_err(|_| RelRewriteLiftError::DeterminantEvidenceUnavailable)?;
    let join = PreparedDirectJoinLift::prepare(plan, semantic, registry)?;
    synthesize_direct_scan_join_source_rewrite_with_constructor_for_commit_prepared(
        plan,
        &join,
        source,
        requested_view,
        &mut coordinates,
        &PreparedLiftContext {
            semantic,
            registry,
            source_spec,
            constructor: project_constructor,
        },
    )
}
