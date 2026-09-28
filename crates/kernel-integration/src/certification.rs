use kernel_change::{PreparedRewrite, RewriteSpec};
use kernel_lens::{RelRewriteLiftError, RelWritableViewPlan};
use kernel_query::{PreparedRelationRewrite, RelationDelta, RelationValue};

pub(crate) struct OwnerCertificationContext<'a> {
    pub(crate) source: &'a kernel_model::FiniteModel,
    pub(crate) semantic: &'a kernel_schema::SemanticContext,
    pub(crate) registry: &'a kernel_semantics::SemanticRegistry,
    pub(crate) source_spec: &'a RewriteSpec,
    pub(crate) owner_type: &'a kernel_query::RelType,
    pub(crate) view_query: &'a kernel_query::PreparedRelExpr,
}

/// Single fail-closed postcondition boundary for every writable relational lift.
///
/// Strategies only reconstruct an owner candidate. This owner verifies that the
/// normalized owner delta re-evaluates to the exact requested view endpoint and
/// only then prepares the authoritative relation Rewrite.
pub(crate) fn certify_owner_candidate<I: Clone>(
    plan: &RelWritableViewPlan,
    requested_view: &PreparedRewrite<RelationValue, I>,
    old_owner: &RelationValue,
    requested_endpoint: &RelationValue,
    reconstructed: &RelationValue,
    context: &OwnerCertificationContext<'_>,
) -> Result<PreparedRelationRewrite<I>, RelRewriteLiftError> {
    let delta = RelationDelta::between_values(
        old_owner,
        reconstructed,
        context.owner_type.clone(),
        context.semantic,
        context.registry,
    )?;
    let normalized_endpoint =
        delta.apply_to_value(old_owner.clone(), context.semantic, context.registry)?;
    let mut candidate_model = context.source.clone();
    candidate_model
        .relations
        .insert(plan.owner_relation, normalized_endpoint.into_rows());
    let actual_view =
        context
            .view_query
            .evaluate(&candidate_model, context.semantic, context.registry)?;
    if !RelationDelta::between_values(
        &actual_view,
        requested_endpoint,
        context.view_query.result_type().clone(),
        context.semantic,
        context.registry,
    )?
    .is_empty()
    {
        return Err(RelRewriteLiftError::RequestedViewInadmissible);
    }
    Ok(delta.prepare_relation_rewrite(
        plan.owner_relation,
        old_owner,
        context.semantic,
        context.registry,
        context.source_spec,
        requested_view.explicit_inputs.clone(),
    )?)
}
