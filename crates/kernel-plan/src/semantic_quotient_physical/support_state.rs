#[derive(Debug, Clone)]
pub(super) struct MaterializedSemanticQuotientSupportState {
    pub(super) binding: SemanticQuotientSupportBinding,
    key_binding: kernel_semantic_index::SemanticIndexBinding,
    stable_rows: Vec<Arc<StableSemanticQuotientRows>>,
    stable_keys: Vec<Vec<Arc<StableSemanticQuotientKeyState>>>,
    constraint_leaves: Vec<Vec<usize>>,
    pub(super) base_masks: Vec<Arc<Vec<u64>>>,
    dense_projection: OnceLock<Arc<Vec<SemanticQuotientConstraint>>>,
    bfc: SemanticQuotientBfcMaintenance,
}

impl PartialEq for MaterializedSemanticQuotientSupportState {
    fn eq(&self, other: &Self) -> bool {
        self.binding == other.binding
            && self.key_binding == other.key_binding
            && self.constraint_leaves == other.constraint_leaves
            && self.base_masks == other.base_masks
            && self.stable_rows.len() == other.stable_rows.len()
            && self
                .stable_rows
                .iter()
                .zip(&other.stable_rows)
                .all(|(left, right)| left.logically_equals(right))
            && self.materialize_dense_constraints().ok()
                == other.materialize_dense_constraints().ok()
    }
}

impl Eq for MaterializedSemanticQuotientSupportState {}

fn stable_quotient_rows_estimated_retained_bytes(rows: &StableSemanticQuotientRows) -> usize {
    std::mem::size_of::<StableSemanticQuotientRows>()
        .saturating_add(rows.by_ordinal.estimated_heap_bytes())
        .saturating_add(rows.ordinal_by_slot.estimated_heap_bytes())
}

fn dense_quotient_constraints_estimated_retained_bytes(
    constraints: &[SemanticQuotientConstraint],
    canonical_eq_key_heap_bytes: fn(&kernel_semantics::CanonicalEqKey) -> usize,
) -> usize {
    let mut bytes = constraints
        .len()
        .saturating_mul(std::mem::size_of::<SemanticQuotientConstraint>());
    for constraint in constraints {
        bytes = bytes.saturating_add(
            constraint
                .leaves
                .capacity()
                .saturating_mul(std::mem::size_of::<SemanticQuotientLeaf>()),
        );
        for leaf in &constraint.leaves {
            bytes = bytes.saturating_add(
                leaf.keys
                    .capacity()
                    .saturating_mul(std::mem::size_of::<
                        Option<kernel_semantics::CanonicalEqKey>,
                    >()),
            );
            for key in leaf.keys.iter().flatten() {
                bytes = bytes.saturating_add(canonical_eq_key_heap_bytes(key));
            }
            for (key, ordinals) in &leaf.buckets {
                bytes = bytes
                    .saturating_add(canonical_eq_key_heap_bytes(key))
                    .saturating_add(
                        ordinals
                            .capacity()
                            .saturating_mul(std::mem::size_of::<u64>()),
                    );
            }
            for key in leaf.live_rows_by_key.keys() {
                bytes = bytes.saturating_add(canonical_eq_key_heap_bytes(key));
            }
        }
        for support in [
            &constraint.key_leaf_support,
            &constraint.live_key_leaf_support,
        ] {
            for key in support.keys() {
                bytes = bytes.saturating_add(canonical_eq_key_heap_bytes(key));
            }
        }
    }
    bytes
}

impl MaterializedSemanticQuotientSupportState {
    // HOSTILE[P176][ACTIVE][CLEAN]: retained-memory accounting is owner-local; advisor supplies
    // only the generic CanonicalEqKey heap sizer instead of traversing QCN representation fields.
    pub(super) fn estimated_retained_bytes(
        &self,
        canonical_eq_key_heap_bytes: fn(&kernel_semantics::CanonicalEqKey) -> usize,
    ) -> usize {
        let mut bytes = std::mem::size_of::<Self>()
            .saturating_add(
                self.binding
                    .leaves
                    .capacity()
                    .saturating_mul(std::mem::size_of::<(SemanticId, LayoutBinding, usize)>()),
            )
            .saturating_add(
                self.binding
                    .specs
                    .capacity()
                    .saturating_mul(std::mem::size_of::<(
                        SemanticId,
                        Vec<SemanticQuotientEndpoint>,
                    )>()),
            );
        for (_, refs) in &self.binding.specs {
            bytes = bytes.saturating_add(
                refs.capacity()
                    .saturating_mul(std::mem::size_of::<SemanticQuotientEndpoint>()),
            );
        }
        for rows in &self.stable_rows {
            bytes = bytes.saturating_add(stable_quotient_rows_estimated_retained_bytes(rows));
        }
        for key_states in &self.stable_keys {
            bytes = bytes.saturating_add(
                key_states
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Arc<StableSemanticQuotientKeyState>>()),
            );
            for state in key_states {
                bytes = bytes
                    .saturating_add(std::mem::size_of::<StableSemanticQuotientKeyState>())
                    .saturating_add(state.by_ordinal.estimated_heap_bytes())
                    .saturating_add(state.bucket_storage_estimated_heap_bytes())
                    .saturating_add(state.bucket_position_by_ordinal.estimated_heap_bytes())
                    .saturating_add(state.bucket_id_by_key.estimated_heap_bytes());
                for key in state.bucket_id_by_key.keys() {
                    bytes = bytes
                        .saturating_add(std::mem::size_of::<kernel_semantics::CanonicalEqKey>())
                        .saturating_add(canonical_eq_key_heap_bytes(key))
                        .saturating_add(std::mem::size_of::<usize>());
                }
            }
        }
        for leaves in &self.constraint_leaves {
            bytes = bytes
                .saturating_add(std::mem::size_of::<Vec<usize>>())
                .saturating_add(
                    leaves
                        .capacity()
                        .saturating_mul(std::mem::size_of::<usize>()),
                );
        }
        for mask in &self.base_masks {
            bytes = bytes
                .saturating_add(std::mem::size_of::<Vec<u64>>())
                .saturating_add(mask.capacity().saturating_mul(std::mem::size_of::<u64>()));
        }
        if let Some(projection) = self.dense_projection.get() {
            bytes = bytes.saturating_add(dense_quotient_constraints_estimated_retained_bytes(
                projection.as_slice(),
                canonical_eq_key_heap_bytes,
            ));
        }
        bytes = bytes
            .saturating_add(std::mem::size_of::<SemanticQuotientBfcMaintenance>())
            .saturating_add(self.bfc.maintainer.estimated_retained_bytes())
            .saturating_add(self.bfc.group_atoms.estimated_heap_bytes());
        for ((_, _, key), _) in &self.bfc.group_atoms {
            bytes = bytes.saturating_add(canonical_eq_key_heap_bytes(key));
        }
        for atoms in &self.bfc.atoms_by_handle {
            bytes = bytes
                .saturating_add(std::mem::size_of::<StableSemanticQuotientAtomDirectory>())
                .saturating_add(atoms.by_slot.estimated_heap_bytes());
        }
        bytes
    }
}

