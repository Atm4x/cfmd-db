#[derive(Debug)]
struct SemanticQuotientComponentRefreshLeafPatch {
    leaf: usize,
    stable_rows: StableSemanticQuotientRows,
    removed_handles: Vec<PhysicalRowId>,
    inserted_handles: Vec<PhysicalRowId>,
    stable_constraint_keys: Vec<(usize, usize, StableSemanticQuotientKeyState)>,
}

impl MaterializedSemanticQuotientSupportState {
    pub(super) fn matches_dense_handles(&self, handles: &[Vec<PhysicalRowId>]) -> bool {
        self.stable_rows.len() == handles.len()
            && self
                .stable_rows
                .iter()
                .zip(handles)
                .all(|(maintained, current)| maintained.matches_dense(current))
    }

    pub(super) fn compatible_with(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, PhysicalExecutionError> {
        let dependencies = semantic_key_dependencies_for_equivalences(
            self.binding
                .specs
                .iter()
                .map(|(equivalence, _)| *equivalence),
            context,
            registry,
        )?;
        Ok(self.key_binding.is_valid_for(context, &dependencies))
    }

    pub(super) fn supports_relation(&self, relation: SemanticId, layout: LayoutBinding) -> bool {
        self.binding
            .leaves
            .iter()
            .any(|candidate| {
                candidate.relation == relation && candidate.layout.id == layout.id
            })
    }

    pub(super) fn materialize_dense_constraints(
        &self,
    ) -> Result<Arc<Vec<SemanticQuotientConstraint>>, PhysicalExecutionError> {
        if let Some(cached) = self.dense_projection.get() {
            return Ok(cached.clone());
        }
        #[cfg(test)]
        SEMANTIC_QUOTIENT_DENSE_PROJECTIONS.with(|count| count.set(count.get().saturating_add(1)));
        let mut constraints = Vec::with_capacity(self.constraint_leaves.len());
        for (constraint_index, leaves) in self.constraint_leaves.iter().enumerate() {
            let key_states = self
                .stable_keys
                .get(constraint_index)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if key_states.len() != leaves.len() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            let mut dense_leaves = Vec::with_capacity(leaves.len());
            for (position, &leaf) in leaves.iter().enumerate() {
                let rows = self
                    .stable_rows
                    .get(leaf)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let key_state = key_states[position].as_ref();
                let mut keys = Vec::with_capacity(rows.live_count);
                for ordinal in 0..rows.by_ordinal.len() {
                    if rows.by_ordinal.get(ordinal).copied().flatten().is_some() {
                        keys.push(key_state.by_ordinal.get(ordinal).cloned().flatten());
                    }
                }
                if keys.len() != rows.live_count {
                    return Err(RelQueryError::InconsistentIncrementalDelta.into());
                }
                let mut dense_leaf = SemanticQuotientLeaf {
                    leaf,
                    keys,
                    buckets: BTreeMap::new(),
                    live_rows_by_key: BTreeMap::new(),
                };
                rebuild_quotient_leaf_buckets(&mut dense_leaf);
                dense_leaves.push(dense_leaf);
            }
            let key_leaf_support = quotient_key_leaf_support(&dense_leaves);
            constraints.push(SemanticQuotientConstraint {
                leaves: dense_leaves,
                key_leaf_support,
                live_key_leaf_support: BTreeMap::new(),
            });
        }
        let mask_refs = self
            .base_masks
            .iter()
            .map(|mask| mask.as_slice())
            .collect::<Vec<_>>();
        for constraint in &mut constraints {
            reset_constraint_live_support(constraint, &mask_refs);
        }
        let built = Arc::new(constraints);
        let _ = self.dense_projection.set(built.clone());
        Ok(self.dense_projection.get().cloned().unwrap_or(built))
    }

    fn apply_bfc_structural_refresh(
        &mut self,
        patches: Vec<SemanticQuotientComponentRefreshLeafPatch>,
    ) -> bool {
        if patches.is_empty() {
            return false;
        }
        self.dense_projection.take();
        self.apply_bfc_structural_refresh_inner(patches).is_ok()
    }

    fn apply_bfc_structural_refresh_inner(
        &mut self,
        patches: Vec<SemanticQuotientComponentRefreshLeafPatch>,
    ) -> Result<(), PhysicalExecutionError> {
        let mut structural = kernel_grounded_closure::BipolarSupportStructuralPatch::default();
        let mut pending_rules = Vec::new();
        let mut projection_leaves = BTreeSet::new();
        let mut add_unavailable = BTreeSet::new();
        let mut remove_unavailable = BTreeSet::new();
        let mut atom_count = self.bfc.maintainer.atom_count();

        {
            let mut stage = SemanticQuotientBfcPatchStage {
                structural: &mut structural,
                pending_rules: &mut pending_rules,
                add_unavailable: &mut add_unavailable,
                remove_unavailable: &mut remove_unavailable,
                atom_count: &mut atom_count,
            };
            for patch in patches {
                projection_leaves.insert(patch.leaf);
                self.bfc.stage_row_atom_delta(
                    patch.leaf,
                    &patch.removed_handles,
                    &patch.inserted_handles,
                    stage.structural,
                    stage.add_unavailable,
                    stage.atom_count,
                )?;
                for (constraint_index, _, _) in &patch.stable_constraint_keys {
                    self.stage_bfc_constraint_refresh(*constraint_index, &patch, &mut stage)?;
                }
                self.stable_rows[patch.leaf] = Arc::new(patch.stable_rows);
            }
        }

        structural.add_unavailable = add_unavailable.into_iter().collect();
        structural.remove_unavailable = remove_unavailable.into_iter().collect();
        let outcome = self
            .bfc
            .maintainer
            .apply_structural_patch_tracked(structural)
            .map_err(|_| RelQueryError::InconsistentIncrementalDelta)?;
        self.bfc
            .install_appended_rule_ids(pending_rules, outcome.appended_requirements)?;
        self.publish_bfc_certificate(outcome.work, projection_leaves, &outcome.changed_atoms)
    }

    fn stage_bfc_constraint_refresh(
        &mut self,
        constraint_index: usize,
        patch: &SemanticQuotientComponentRefreshLeafPatch,
        stage: &mut SemanticQuotientBfcPatchStage<'_>,
    ) -> Result<(), PhysicalExecutionError> {
        let constraint_leaves = self.constraint_leaves[constraint_index].clone();
        let leaf_index = constraint_leaves
            .iter()
            .position(|candidate| *candidate == patch.leaf)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let old_stable_keys = self.stable_keys[constraint_index][leaf_index].clone();
        let (_, _, next_stable_keys) = patch
            .stable_constraint_keys
            .iter()
            .find(|(candidate, position, _)| {
                *candidate == constraint_index && *position == leaf_index
            })
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let old_key_by_handle = patch
            .removed_handles
            .iter()
            .copied()
            .map(|handle| {
                let ordinal = self.stable_rows[patch.leaf]
                    .ordinal_for(handle)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                Ok((handle, old_stable_keys.key_at(ordinal).cloned()))
            })
            .collect::<Result<BTreeMap<_, _>, PhysicalExecutionError>>()?;
        let new_key_by_handle = patch
            .inserted_handles
            .iter()
            .copied()
            .map(|handle| {
                let ordinal = patch
                    .stable_rows
                    .ordinal_for(handle)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                Ok((handle, next_stable_keys.key_at(ordinal).cloned()))
            })
            .collect::<Result<BTreeMap<_, _>, PhysicalExecutionError>>()?;
        let touched_keys = patch
            .removed_handles
            .iter()
            .filter_map(|handle| old_key_by_handle.get(handle).and_then(Clone::clone))
            .chain(
                patch
                    .inserted_handles
                    .iter()
                    .filter_map(|handle| new_key_by_handle.get(handle).and_then(Clone::clone)),
            )
            .collect::<BTreeSet<_>>();

        self.bfc.stage_removed_row_rules(
            constraint_index,
            patch.leaf,
            &constraint_leaves,
            &patch.removed_handles,
            &old_key_by_handle,
            stage.structural,
        )?;
        self.bfc.ensure_group_atoms_for_keys(
            (constraint_index, patch.leaf),
            &constraint_leaves,
            &self.stable_keys[constraint_index],
            next_stable_keys,
            &touched_keys,
            stage,
        );
        self.bfc.stage_class_rules(
            (constraint_index, patch.leaf),
            old_stable_keys.as_ref(),
            &patch.stable_rows,
            next_stable_keys,
            &touched_keys,
            stage,
        )?;
        self.bfc.stage_inserted_row_rules(
            constraint_index,
            patch.leaf,
            &constraint_leaves,
            &patch.inserted_handles,
            &new_key_by_handle,
            stage,
        )?;
        self.stable_keys[constraint_index][leaf_index] = Arc::new(next_stable_keys.clone());
        Ok(())
    }

    fn publish_bfc_certificate(
        &mut self,
        work: kernel_grounded_closure::GroundedWorkStats,
        projection_leaves: BTreeSet<usize>,
        changed_atoms: &[kernel_grounded_closure::GroundedAtomId],
    ) -> Result<(), PhysicalExecutionError> {
        let mut affected_leaves = projection_leaves;
        for atom in changed_atoms {
            if let Some(Some(leaf)) = self.bfc.row_leaf_by_atom.get(atom.index()).copied() {
                affected_leaves.insert(leaf);
            }
        }

        for &leaf in &affected_leaves {
            let mask = semantic_quotient_mask_from_stable_bfc(
                self.stable_rows[leaf].as_ref(),
                self.bfc.atoms_by_handle[leaf].as_ref(),
                self.bfc.maintainer.certificate(),
            )?;
            self.base_masks[leaf] = Arc::new(mask);
        }
        self.bfc.last_work = work;
        Ok(())
    }
}

#[cfg(test)]
fn full_ordinal_mask(row_count: usize) -> Vec<u64> {
    let words = row_count.div_ceil(64);
    let mut mask = vec![u64::MAX; words];
    if let Some(last) = mask.last_mut() {
        let used = row_count % 64;
        if used != 0 {
            *last = (1_u64 << used) - 1;
        }
    }
    mask
}

fn mask_contains(mask: &[u64], ordinal: usize) -> bool {
    mask.get(ordinal / 64)
        .is_some_and(|word| word & (1_u64 << (ordinal % 64)) != 0)
}

#[cfg(test)]
fn mask_clear(mask: &mut [u64], ordinal: usize) -> bool {
    let Some(word) = mask.get_mut(ordinal / 64) else {
        return false;
    };
    let bit = 1_u64 << (ordinal % 64);
    let changed = *word & bit != 0;
    *word &= !bit;
    changed
}

pub(super) fn mask_intersect_in_place(target: &mut [u64], other: &[u64]) {
    for (target, other) in target.iter_mut().zip(other) {
        *target &= *other;
    }
}

pub(super) fn masks_intersect(left: &[u64], right: &[u64]) -> bool {
    left.iter()
        .zip(right)
        .any(|(left, right)| left & right != 0)
}

fn rebuild_quotient_leaf_buckets(leaf: &mut SemanticQuotientLeaf) {
    leaf.buckets.clear();
    let words = leaf.keys.len().div_ceil(64);
    for (ordinal, key) in leaf.keys.iter().enumerate() {
        let Some(key) = key else {
            continue;
        };
        let bucket = leaf
            .buckets
            .entry(key.clone())
            .or_insert_with(|| vec![0; words]);
        bucket[ordinal / 64] |= 1_u64 << (ordinal % 64);
    }
}

fn quotient_live_row_counts(
    leaf: &SemanticQuotientLeaf,
    mask: &[u64],
) -> BTreeMap<kernel_semantics::CanonicalEqKey, usize> {
    let mut counts = BTreeMap::new();
    for (ordinal, key) in leaf.keys.iter().enumerate() {
        if !mask_contains(mask, ordinal) {
            continue;
        }
        if let Some(key) = key {
            *counts.entry(key.clone()).or_insert(0) += 1;
        }
    }
    counts
}

fn reset_constraint_live_support(constraint: &mut SemanticQuotientConstraint, masks: &[&[u64]]) {
    let mut support = BTreeMap::new();
    for leaf in &mut constraint.leaves {
        leaf.live_rows_by_key = quotient_live_row_counts(leaf, masks[leaf.leaf]);
        for key in leaf.live_rows_by_key.keys() {
            *support.entry(key.clone()).or_insert(0) += 1;
        }
    }
    constraint.live_key_leaf_support = support;
}

#[cfg(test)]
fn remove_live_ordinal_from_constraint(
    constraint: &mut SemanticQuotientConstraint,
    leaf_index: usize,
    ordinal: usize,
) {
    let Some(quotient_leaf) = constraint
        .leaves
        .iter_mut()
        .find(|candidate| candidate.leaf == leaf_index)
    else {
        return;
    };
    let Some(key) = quotient_leaf
        .keys
        .get(ordinal)
        .and_then(Option::as_ref)
        .cloned()
    else {
        return;
    };
    let Some(row_count) = quotient_leaf.live_rows_by_key.get_mut(&key) else {
        return;
    };
    *row_count = row_count.saturating_sub(1);
    if *row_count != 0 {
        return;
    }
    quotient_leaf.live_rows_by_key.remove(&key);
    if let Some(support) = constraint.live_key_leaf_support.get_mut(&key) {
        *support = support.saturating_sub(1);
        if *support == 0 {
            constraint.live_key_leaf_support.remove(&key);
        }
    }
}

#[cfg(test)]
fn constraint_viable_keys(
    constraint: &SemanticQuotientConstraint,
) -> BTreeSet<kernel_semantics::CanonicalEqKey> {
    let required = constraint.leaves.len();
    constraint
        .live_key_leaf_support
        .iter()
        .filter(|(key, support)| {
            **support == required && constraint.key_leaf_support.get(*key) == Some(&required)
        })
        .map(|(key, _)| key.clone())
        .collect()
}

#[cfg(test)]
fn quotient_constraints_by_leaf(
    leaf_count: usize,
    constraints: &[SemanticQuotientConstraint],
) -> Vec<Vec<usize>> {
    let mut constraints_by_leaf = vec![Vec::new(); leaf_count];
    for (constraint_index, constraint) in constraints.iter().enumerate() {
        for leaf in &constraint.leaves {
            constraints_by_leaf[leaf.leaf].push(constraint_index);
        }
    }
    constraints_by_leaf
}

#[cfg(test)]
fn propagate_quotient_support_deletions(
    handles: &[Vec<PhysicalRowId>],
    constraints: &mut [SemanticQuotientConstraint],
    masks: &mut [Vec<u64>],
    changed_leaves: &[usize],
) {
    let constraints_by_leaf = quotient_constraints_by_leaf(handles.len(), constraints);
    let mut queued = vec![false; constraints.len()];
    let mut queue = std::collections::VecDeque::new();
    for leaf in changed_leaves {
        for constraint in &constraints_by_leaf[*leaf] {
            if !queued[*constraint] {
                queued[*constraint] = true;
                queue.push_back(*constraint);
            }
        }
    }
    while let Some(constraint_index) = queue.pop_front() {
        queued[constraint_index] = false;
        let viable_keys = constraint_viable_keys(&constraints[constraint_index]);
        let constraint_leaves = constraints[constraint_index]
            .leaves
            .iter()
            .map(|leaf| (leaf.leaf, leaf.keys.clone()))
            .collect::<Vec<_>>();
        for (leaf_index, keys) in constraint_leaves {
            let mut leaf_changed = false;
            for (ordinal, key) in keys.iter().enumerate() {
                if mask_contains(&masks[leaf_index], ordinal)
                    && key.as_ref().is_none_or(|key| !viable_keys.contains(key))
                {
                    for dependent in &constraints_by_leaf[leaf_index] {
                        remove_live_ordinal_from_constraint(
                            &mut constraints[*dependent],
                            leaf_index,
                            ordinal,
                        );
                    }
                    leaf_changed |= mask_clear(&mut masks[leaf_index], ordinal);
                }
            }
            if leaf_changed {
                for dependent in &constraints_by_leaf[leaf_index] {
                    if !queued[*dependent] {
                        queued[*dependent] = true;
                        queue.push_back(*dependent);
                    }
                }
            }
        }
    }
}

fn prepare_semantic_quotient_component_refresh_patches(
    state: &MaterializedSemanticQuotientSupportState,
    leaf_updates: &[(usize, &PhysicalRelationDelta)],
    store: &dyn SemanticQuotientStoreView,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<Vec<SemanticQuotientComponentRefreshLeafPatch>>, PhysicalExecutionError> {
    if !state.compatible_with(context, registry)? || leaf_updates.is_empty() {
        return Ok(None);
    }
    let mut patches = Vec::with_capacity(leaf_updates.len());
    for (leaf, delta) in leaf_updates {
        let Some(current_stable_rows) = state.stable_rows.get(*leaf) else {
            return Ok(None);
        };
        let (mut next_stable_rows, inserted_handles) = current_stable_rows.apply_delta(delta)?;
        let Some(binding_leaf) = state.binding.leaves.get(*leaf) else {
            return Ok(None);
        };
        let relation = binding_leaf.relation;
        let layout = binding_leaf.layout;
        let mut stable_constraint_keys = Vec::new();
        for (constraint_index, constraint_leaves) in state.constraint_leaves.iter().enumerate() {
            let Some(leaf_position) = constraint_leaves
                .iter()
                .position(|candidate| *candidate == *leaf)
            else {
                continue;
            };
            let Some((equivalence, endpoints)) = state.binding.specs.get(constraint_index) else {
                return Ok(None);
            };
            let columns = endpoints
                .iter()
                .filter(|endpoint| endpoint.leaf == *leaf)
                .map(|endpoint| endpoint.column)
                .collect::<Vec<_>>();
            if columns.is_empty() {
                return Ok(None);
            }
            let mut inserted_keys = Vec::with_capacity(inserted_handles.len());
            for row_id in &inserted_handles {
                let mut row_key = None;
                let mut consistent = true;
                for column in &columns {
                    let binding =
                        SemanticIndexBinding::single(relation, layout, *column, *equivalence);
                    let Some(key) =
                        store.semantic_quotient_single_key(&binding, *row_id, context, registry)?
                    else {
                        return Ok(None);
                    };
                    if row_key.as_ref().is_some_and(|current| current != &key) {
                        consistent = false;
                        break;
                    }
                    row_key = Some(key);
                }
                inserted_keys.push((*row_id, consistent.then_some(row_key).flatten()));
            }
            let Some(next_stable_keys) = advance_stable_semantic_quotient_keys(
                state.stable_keys[constraint_index][leaf_position].as_ref(),
                current_stable_rows,
                &next_stable_rows,
                delta,
                &inserted_keys,
            ) else {
                return Ok(None);
            };
            stable_constraint_keys.push((constraint_index, leaf_position, next_stable_keys));
        }
        if next_stable_rows.should_compact() {
            next_stable_rows = compact_semantic_quotient_leaf_state(
                &next_stable_rows,
                &mut stable_constraint_keys,
            )?;
        }
        patches.push(SemanticQuotientComponentRefreshLeafPatch {
            leaf: *leaf,
            stable_rows: next_stable_rows,
            removed_handles: delta.removed.iter().map(|(handle, _)| *handle).collect(),
            inserted_handles,
            stable_constraint_keys,
        });
    }
    Ok(Some(patches))
}

pub(super) fn prepare_semantic_quotient_component_refresh(
    state: &MaterializedSemanticQuotientSupportState,
    leaf_updates: &[(usize, &PhysicalRelationDelta)],
    store: &dyn SemanticQuotientStoreView,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<
    Option<impl FnOnce(&mut MaterializedSemanticQuotientSupportState) -> bool + use<>>,
    PhysicalExecutionError,
> {
    // HOSTILE[P176][ACTIVE][CLEAN]: patch representation remains private while storage keeps the
    // original prepare-before-mutable-borrow / Arc::make_mut COW boundary via this opaque action.
    let Some(patches) = prepare_semantic_quotient_component_refresh_patches(
        state,
        leaf_updates,
        store,
        context,
        registry,
    )? else {
        return Ok(None);
    };
    Ok(Some(move |state: &mut MaterializedSemanticQuotientSupportState| {
        state.apply_bfc_structural_refresh(patches)
    }))
}


