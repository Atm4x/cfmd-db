pub(super) fn prepare_semantic_quotient_program(
    plan: &Plan,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<PreparedSemanticQuotientProgram>, RelQueryError> {
    let mut leaf_count = 0;
    let mut predicates = Vec::new();
    let Some(_) = flatten_multiway_join_shape(plan, context, &mut leaf_count, &mut predicates)?
    else {
        return Ok(None);
    };
    if leaf_count < 3 {
        return Ok(None);
    }
    let specs =
        semantic_quotient_specs(&predicates, context, registry).map_err(|error| match error {
            PhysicalExecutionError::Query(query) => query,
            _ => RelQueryError::InconsistentIncrementalDelta,
        })?;
    let gyo_order = quotient_hypergraph_search_order(leaf_count, &specs);
    let (hypergraph_order, search_certificate) = if let Some(order) = gyo_order {
        (
            Some(order),
            Some(SemanticQuotientSearchCertificate::GyoAcyclic),
        )
    } else if leaf_count > 8 {
        (
            quotient_hypergraph_cyclic_search_order(leaf_count, &specs),
            Some(SemanticQuotientSearchCertificate::BoundedCyclic),
        )
    } else {
        (None, None)
    };
    if leaf_count > 8 && hypergraph_order.is_none() {
        return Ok(None);
    }
    Ok(Some(PreparedSemanticQuotientProgram {
        specs,
        hypergraph_order,
        search_certificate,
    }))
}

pub(super) fn prepare_anchor_pullback_program(
    plan: &Plan,
    context: &kernel_schema::SemanticContext,
) -> Result<Option<PreparedAnchorPullbackProgram>, RelQueryError> {
    let mut leaf_count = 0;
    let mut predicates = Vec::new();
    let Some(_) = flatten_multiway_join_shape(plan, context, &mut leaf_count, &mut predicates)?
    else {
        return Ok(None);
    };
    if leaf_count < 3 || predicates.is_empty() {
        return Ok(None);
    }
    Ok(Some(PreparedAnchorPullbackProgram {
        leaf_count,
        predicates,
    }))
}

