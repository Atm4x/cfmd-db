// HOSTILE[P178][ACTIVE][CLEAN]: transparent scan recognition belongs to Plan shape inspection,
// not to the join executor that happens to consume it.
pub(super) fn transparent_direct_scan_relation(
    plan: &Plan,
    store: &PhysicalStore,
) -> Option<(SemanticId, LayoutBinding)> {
    match plan {
        Plan::Project { input, columns } => {
            let (relation, layout) = transparent_direct_scan_relation(input, store)?;
            let width = native_column_count(&store.installed(relation, layout).ok()?.data);
            (columns.len() == width
                && columns
                    .iter()
                    .copied()
                    .enumerate()
                    .all(|(column, projected)| column == projected))
            .then_some((relation, layout))
        }
        Plan::Scan { relation, layout } => Some((*relation, *layout)),
        _ => None,
    }
}
