fn observable_atom_advice_scores(
    store: &PhysicalStore,
    workload: &[SemanticIndexWorkloadSample],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<BTreeMap<SemanticIndexBinding, ObservableAtomAdviceScore>, PhysicalExecutionError> {
    let mut scores = BTreeMap::<SemanticIndexBinding, ObservableAtomAdviceScore>::new();
    for sample in workload {
        if sample.expected_executions == 0 {
            continue;
        }
        for observation in
            semantic_index_advice_observations(&sample.plan, store, context, registry)?
        {
            let entry = scores
                .entry(observation.binding.clone())
                .or_insert_with(|| ObservableAtomAdviceScore {
                    gross_savings: 0,
                    key_cells: observation.key_cells,
                    estimated_bytes: observation.estimated_bytes,
                });
            entry.gross_savings = entry.gross_savings.saturating_add(
                observation
                    .savings_per_execution
                    .saturating_mul(sample.expected_executions as u128),
            );
            entry.key_cells = entry.key_cells.max(observation.key_cells);
            entry.estimated_bytes = entry.estimated_bytes.max(observation.estimated_bytes);
        }
    }
    Ok(scores)
}
