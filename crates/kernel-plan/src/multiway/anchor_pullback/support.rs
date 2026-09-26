pub(super) fn semantic_quotient_support_binding(
    leaves: &[MultiwayJoinLeaf],
    program: &PreparedSemanticQuotientProgram,
) -> SemanticQuotientSupportBinding {
    SemanticQuotientSupportBinding {
        leaves: leaves
            .iter()
            .map(|leaf| SemanticQuotientPhysicalLeaf { relation: leaf.relation, layout: leaf.layout, width: leaf.width })
            .collect(),
        specs: program
            .specs
            .iter()
            .map(|(equivalence, endpoints)| {
                (*equivalence, endpoints.iter().map(|endpoint| SemanticQuotientEndpoint { leaf: endpoint.leaf, column: endpoint.column }).collect())
            })
            .collect(),
    }
}

pub(super) fn prepared_quotient_acceleration_available(
    plan: &Plan,
    program: &PreparedSemanticQuotientProgram,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, PhysicalExecutionError> {
    let mut leaves = Vec::new();
    let mut predicates = Vec::new();
    let Some(_) = flatten_multiway_join_tree(plan, store, &mut leaves, &mut predicates)? else {
        return Ok(false);
    };
    let support_binding = semantic_quotient_support_binding(&leaves, program);
    if store.semantic_quotient_support(&support_binding).is_some() {
        return Ok(true);
    }
    for (equivalence, endpoints) in &program.specs {
        for endpoint in endpoints {
            let Some(leaf) = leaves.get(endpoint.leaf) else {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            };
            let binding = SemanticIndexBinding::single(
                leaf.relation,
                leaf.layout,
                endpoint.column,
                *equivalence,
            );
            if store.has_semantic_quotient_capability(&binding, context, registry)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}


