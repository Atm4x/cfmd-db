fn semantic_statistics_advisor_selection(
    store: &PhysicalStore,
) -> SemanticStatisticsAdvisorSelection {
    let current_memory = store
        .artifact_memory_report()
        .total_estimated_retained_bytes;
    let replaceable_managed_bytes = saturating_usize_sum(
        store
            .semantic_statistics
            .iter()
            .filter(|(binding, _)| {
                store
                    .advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::SemanticStatistics((*binding).clone()))
            })
            .map(|(_, state)| semantic_statistics_estimated_retained_bytes(state)),
    );
    let fixed_estimated_bytes = current_memory.saturating_sub(replaceable_managed_bytes);
    let report = SemanticStatisticsAdvisorReport {
        fixed_estimated_bytes,
        total_estimated_bytes_after: fixed_estimated_bytes,
        ..SemanticStatisticsAdvisorReport::default()
    };
    SemanticStatisticsAdvisorSelection {
        selected: BTreeSet::new(),
        prepared_states: Vec::new(),
        managed_estimated_bytes: 0,
        report,
    }
}

