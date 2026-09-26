use super::*;

pub(crate) fn cyclic_prefix_fixture_for_test() -> (
    [SemanticQuotientConstraint; 2],
    [Vec<u64>; 3],
    Vec<usize>,
    Vec<usize>,
) {
    let q1 = SemanticId::new(1_720);
    let q2 = SemanticId::new(1_721);
    let a = (0..64)
        .map(kernel_semantics::CanonicalEqKey::I64)
        .collect::<Vec<_>>();
    let b = (100..164)
        .map(kernel_semantics::CanonicalEqKey::I64)
        .collect::<Vec<_>>();
    let mut cache = SemanticQuotientKeyCache::new();
    cache.insert((0, 0, q1), a.clone());
    cache.insert(
        (2, 0, q1),
        (0..4_096).map(|row| a[row / 64].clone()).collect(),
    );
    cache.insert((1, 0, q2), b.clone());
    cache.insert(
        (2, 1, q2),
        (0..4_096).map(|row| b[row % 64].clone()).collect(),
    );
    let q1_leaves = vec![
        quotient_leaf_keys(0, &[0], q1, 64, &cache).unwrap(),
        quotient_leaf_keys(2, &[0], q1, 4_096, &cache).unwrap(),
    ];
    let q2_leaves = vec![
        quotient_leaf_keys(1, &[0], q2, 64, &cache).unwrap(),
        quotient_leaf_keys(2, &[1], q2, 4_096, &cache).unwrap(),
    ];
    let q1_support = quotient_key_leaf_support(&q1_leaves);
    let q2_support = quotient_key_leaf_support(&q2_leaves);
    let constraints = [
        SemanticQuotientConstraint {
            leaves: q1_leaves,
            key_leaf_support: q1_support.clone(),
            live_key_leaf_support: q1_support,
        },
        SemanticQuotientConstraint {
            leaves: q2_leaves,
            key_leaf_support: q2_support.clone(),
            live_key_leaf_support: q2_support,
        },
    ];
    let masks = [
        full_ordinal_mask(64),
        full_ordinal_mask(64),
        full_ordinal_mask(4_096),
    ];
    (constraints, masks, vec![64, 64, 4_096], vec![0, 1, 2])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportCowSharingForTest {
    pub(crate) group_atoms: bool,
    pub(crate) atom_maps: Vec<bool>,
    pub(crate) base_masks: Vec<bool>,
    pub(crate) stable_rows: Vec<bool>,
}

pub(crate) fn support_rows_match_dense_for_test(
    state: &MaterializedSemanticQuotientSupportState,
    handles: &[Vec<PhysicalRowId>],
) -> bool {
    state.stable_rows.len() == handles.len()
        && state
            .stable_rows
            .iter()
            .zip(handles)
            .all(|(maintained, current)| maintained.matches_dense(current))
}

pub(crate) fn support_cow_sharing_for_test(
    old: &MaterializedSemanticQuotientSupportState,
    new: &MaterializedSemanticQuotientSupportState,
) -> SupportCowSharingForTest {
    SupportCowSharingForTest {
        group_atoms: old.bfc.group_atoms.shares_root_with(&new.bfc.group_atoms),
        atom_maps: old
            .bfc
            .atoms_by_handle
            .iter()
            .zip(&new.bfc.atoms_by_handle)
            .map(|(left, right)| Arc::ptr_eq(left, right))
            .collect(),
        base_masks: old
            .base_masks
            .iter()
            .zip(&new.base_masks)
            .map(|(left, right)| Arc::ptr_eq(left, right))
            .collect(),
        stable_rows: old
            .stable_rows
            .iter()
            .zip(&new.stable_rows)
            .map(|(left, right)| Arc::ptr_eq(left, right))
            .collect(),
    }
}

pub(crate) fn support_stable_row_for_test(
    state: &MaterializedSemanticQuotientSupportState,
    leaf: usize,
) -> (usize, usize, Vec<PhysicalRowId>) {
    let rows = &state.stable_rows[leaf];
    (rows.live_count, rows.by_ordinal.len(), rows.dense_handles())
}

pub(crate) fn support_contains_stable_key_for_test(
    state: &MaterializedSemanticQuotientSupportState,
    key: &kernel_semantics::CanonicalEqKey,
) -> bool {
    state
        .stable_keys
        .iter()
        .flat_map(|constraint| constraint.iter())
        .any(|keys| keys.bucket_id_by_key.contains_key(key))
}

pub(crate) fn support_group_contains_key_for_test(
    state: &MaterializedSemanticQuotientSupportState,
    key: &kernel_semantics::CanonicalEqKey,
) -> bool {
    state
        .bfc
        .group_atoms
        .keys()
        .any(|(_, _, candidate)| candidate == key)
}

pub(crate) fn support_group_atoms_share_root_for_test(
    old: &MaterializedSemanticQuotientSupportState,
    new: &MaterializedSemanticQuotientSupportState,
) -> bool {
    old.bfc.group_atoms.shares_root_with(&new.bfc.group_atoms)
}

pub(crate) fn support_atom_for_test(
    state: &MaterializedSemanticQuotientSupportState,
    leaf: usize,
    handle: PhysicalRowId,
) -> Option<kernel_grounded_closure::GroundedAtomId> {
    state.bfc.atoms_by_handle[leaf].get(handle)
}

pub(crate) fn support_bfc_work_for_test(
    state: &MaterializedSemanticQuotientSupportState,
) -> (usize, usize) {
    (
        state.bfc.last_work.affected_atoms,
        state.bfc.maintainer.atom_count(),
    )
}

pub(crate) fn support_binding_for_test(
    state: &MaterializedSemanticQuotientSupportState,
) -> SemanticQuotientSupportBinding {
    state.binding.clone()
}

pub(crate) fn reset_dense_projections_for_test() {
    SEMANTIC_QUOTIENT_DENSE_PROJECTIONS.with(|count| count.set(0));
}

pub(crate) fn dense_projections_for_test() -> usize {
    SEMANTIC_QUOTIENT_DENSE_PROJECTIONS.with(std::cell::Cell::get)
}

pub(crate) fn full_bfc_compiles_for_test() -> usize {
    SEMANTIC_QUOTIENT_FULL_BFC_COMPILES.with(std::cell::Cell::get)
}
