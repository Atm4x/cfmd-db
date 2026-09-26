/// Reconstructible maintenance acceleration for the semantic quotient support
/// state. Atom ids are stable for the lifetime of this materialization and are
/// keyed by `(leaf, StableRowHandle)`, including the handle generation.
#[derive(Debug, Clone)]
struct SemanticQuotientBfcMaintenance {
    atoms_by_handle: Vec<Arc<StableSemanticQuotientAtomDirectory>>,
     row_leaf_by_atom: PersistentPhysicalVec<Option<usize>>,
    group_atoms:
        PersistentOrdMap<SemanticQuotientGroupAtomKey, kernel_grounded_closure::GroundedAtomId>,
     class_rules_by_atom: PersistentPhysicalVec<Option<kernel_grounded_closure::GroundedRuleId>>,
     row_rules: Vec<Vec<Vec<StableSemanticQuotientRuleDirectory>>>,
    maintainer: kernel_grounded_closure::BipolarSupportMaintenance,
     last_work: kernel_grounded_closure::GroundedWorkStats,
}

type SemanticQuotientGroupAtomKey = (usize, usize, kernel_semantics::CanonicalEqKey);

#[derive(Debug)]
enum PendingSemanticQuotientRuleDirectory {
    Class {
        atom: kernel_grounded_closure::GroundedAtomId,
    },
    Row {
        constraint: usize,
        dependent_leaf: usize,
        handle: PhysicalRowId,
        supporter_leaf: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct StableSemanticQuotientAtomDirectory {
    by_slot: PersistentPhysicalVec<Option<(u64, kernel_grounded_closure::GroundedAtomId)>>,
}

impl StableSemanticQuotientAtomDirectory {
    fn get(&self, handle: PhysicalRowId) -> Option<kernel_grounded_closure::GroundedAtomId> {
        self.by_slot
            .get(handle.slot)
            .copied()
            .flatten()
            .and_then(|(generation, atom)| (generation == handle.generation).then_some(atom))
    }

    fn insert(
        &mut self,
        handle: PhysicalRowId,
        atom: kernel_grounded_closure::GroundedAtomId,
    ) -> Result<(), PhysicalExecutionError> {
        while self.by_slot.len() <= handle.slot {
            self.by_slot.push(None);
        }
        if self.by_slot[handle.slot].is_some() {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
        self.by_slot
            .set(handle.slot, Some((handle.generation, atom)));
        Ok(())
    }

    fn remove(&mut self, handle: PhysicalRowId) -> Result<(), PhysicalExecutionError> {
        let Some((generation, _)) = self.by_slot.get(handle.slot).copied().flatten() else {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        };
        if generation != handle.generation {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
        self.by_slot.set(handle.slot, None);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
 struct StableSemanticQuotientRuleDirectory {
    by_slot: PersistentPhysicalVec<Option<(u64, kernel_grounded_closure::GroundedRuleId)>>,
}

impl StableSemanticQuotientRuleDirectory {
    fn insert(
        &mut self,
        handle: PhysicalRowId,
        rule: kernel_grounded_closure::GroundedRuleId,
    ) -> Result<(), PhysicalExecutionError> {
        while self.by_slot.len() <= handle.slot {
            self.by_slot.push(None);
        }
        if self.by_slot[handle.slot].is_some() {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
        self.by_slot
            .set(handle.slot, Some((handle.generation, rule)));
        Ok(())
    }

    fn remove(
        &mut self,
        handle: PhysicalRowId,
    ) -> Result<kernel_grounded_closure::GroundedRuleId, PhysicalExecutionError> {
        let Some((generation, rule)) = self.by_slot.get(handle.slot).copied().flatten() else {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        };
        if generation != handle.generation {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
        self.by_slot.set(handle.slot, None);
        Ok(rule)
    }
}

type SemanticQuotientRowRuleDirectories = Vec<Vec<Vec<StableSemanticQuotientRuleDirectory>>>;
type SemanticQuotientRuleDirectories = (
    PersistentPhysicalVec<Option<kernel_grounded_closure::GroundedRuleId>>,
    SemanticQuotientRowRuleDirectories,
);

struct SemanticQuotientBfcPatchStage<'a> {
    structural: &'a mut kernel_grounded_closure::BipolarSupportStructuralPatch,
    pending_rules: &'a mut Vec<PendingSemanticQuotientRuleDirectory>,
    add_unavailable: &'a mut BTreeSet<kernel_grounded_closure::GroundedAtomId>,
    remove_unavailable: &'a mut BTreeSet<kernel_grounded_closure::GroundedAtomId>,
    atom_count: &'a mut usize,
}

impl SemanticQuotientBfcMaintenance {
    fn stage_row_atom_delta(
        &mut self,
        leaf: usize,
        removed: &[PhysicalRowId],
        inserted: &[PhysicalRowId],
        structural: &mut kernel_grounded_closure::BipolarSupportStructuralPatch,
        add_unavailable: &mut BTreeSet<kernel_grounded_closure::GroundedAtomId>,
        atom_count: &mut usize,
    ) -> Result<(), PhysicalExecutionError> {
        for handle in removed {
            let atom = self.atoms_by_handle[leaf]
                .get(*handle)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            add_unavailable.insert(atom);
            Arc::make_mut(&mut self.atoms_by_handle[leaf]).remove(*handle)?;
        }
        for handle in inserted {
            let atom = kernel_grounded_closure::GroundedAtomId::new(*atom_count);
            *atom_count = atom_count.saturating_add(1);
            structural.append_atoms = structural.append_atoms.saturating_add(1);
            self.class_rules_by_atom.push(None);
            self.row_leaf_by_atom.push(Some(leaf));
            Arc::make_mut(&mut self.atoms_by_handle[leaf]).insert(*handle, atom)?;
        }
        Ok(())
    }

    fn stage_removed_row_rules(
        &mut self,
        constraint_index: usize,
        leaf: usize,
        constraint_leaves: &[usize],
        removed: &[PhysicalRowId],
        old_key_by_handle: &BTreeMap<PhysicalRowId, Option<kernel_semantics::CanonicalEqKey>>,
        structural: &mut kernel_grounded_closure::BipolarSupportStructuralPatch,
    ) -> Result<(), PhysicalExecutionError> {
        for handle in removed {
            if !old_key_by_handle.get(handle).is_some_and(Option::is_some) {
                continue;
            }
            for &supporter_leaf in constraint_leaves {
                if supporter_leaf == leaf {
                    continue;
                }
                let rule =
                    self.row_rules[constraint_index][leaf][supporter_leaf].remove(*handle)?;
                structural.disable_requirements.push(rule);
            }
        }
        Ok(())
    }

    fn ensure_group_atoms_for_keys(
        &mut self,
        coordinate: (usize, usize),
        constraint_leaves: &[usize],
        stable_keys: &[Arc<StableSemanticQuotientKeyState>],
        next_keys: &StableSemanticQuotientKeyState,
        touched_keys: &BTreeSet<kernel_semantics::CanonicalEqKey>,
        stage: &mut SemanticQuotientBfcPatchStage<'_>,
    ) {
        let (constraint_index, leaf) = coordinate;
        for key in touched_keys {
            for (position, &candidate_leaf) in constraint_leaves.iter().enumerate() {
                let group_key = (constraint_index, candidate_leaf, key.clone());
                if self.group_atoms.contains_key(&group_key) {
                    continue;
                }
                let atom = kernel_grounded_closure::GroundedAtomId::new(*stage.atom_count);
                *stage.atom_count = stage.atom_count.saturating_add(1);
                stage.structural.append_atoms = stage.structural.append_atoms.saturating_add(1);
                self.class_rules_by_atom.push(None);
                self.row_leaf_by_atom.push(None);
                self.group_atoms.insert(group_key, atom);
                let active = if candidate_leaf == leaf {
                    next_keys.contains_key(key)
                } else {
                    stable_keys[position].contains_key(key)
                };
                if !active {
                    stage.add_unavailable.insert(atom);
                }
            }
        }
    }

    fn stage_class_rules(
        &mut self,
        coordinate: (usize, usize),
        old_keys: &StableSemanticQuotientKeyState,
        next_rows: &StableSemanticQuotientRows,
        next_keys: &StableSemanticQuotientKeyState,
        touched_keys: &BTreeSet<kernel_semantics::CanonicalEqKey>,
        stage: &mut SemanticQuotientBfcPatchStage<'_>,
    ) -> Result<(), PhysicalExecutionError> {
        let (constraint_index, leaf) = coordinate;
        for key in touched_keys {
            let group_atom = self
                .group_atoms
                .get(&(constraint_index, leaf, key.clone()))
                .copied()
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let old_active = old_keys.contains_key(key);
            let new_active = next_keys.contains_key(key);
            if old_active {
                let rule = self
                    .class_rules_by_atom
                    .get(group_atom.index())
                    .copied()
                    .flatten()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                stage.structural.disable_requirements.push(rule);
                self.class_rules_by_atom.set(group_atom.index(), None);
            }
            if new_active {
                let supporters = next_keys
                    .live_ordinals(key)
                    .map(|ordinal| next_rows.by_ordinal.get(ordinal).copied().flatten())
                    .collect::<Option<Vec<_>>>()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                    .into_iter()
                    .map(|handle| self.atoms_by_handle[leaf].get(handle))
                    .collect::<Option<Vec<_>>>()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                stage.structural.append_requirements.push(
                    kernel_grounded_closure::BipolarSupportRequirement::new(group_atom, supporters),
                );
                stage
                    .pending_rules
                    .push(PendingSemanticQuotientRuleDirectory::Class { atom: group_atom });
            }
            if old_active && !new_active {
                stage.add_unavailable.insert(group_atom);
            } else if !old_active && new_active {
                stage.remove_unavailable.insert(group_atom);
            }
        }
        Ok(())
    }

    fn stage_inserted_row_rules(
        &mut self,
        constraint_index: usize,
        leaf: usize,
        constraint_leaves: &[usize],
        inserted: &[PhysicalRowId],
        new_key_by_handle: &BTreeMap<PhysicalRowId, Option<kernel_semantics::CanonicalEqKey>>,
        stage: &mut SemanticQuotientBfcPatchStage<'_>,
    ) -> Result<(), PhysicalExecutionError> {
        for handle in inserted {
            let dependent = self.atoms_by_handle[leaf]
                .get(*handle)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let Some(Some(key)) = new_key_by_handle.get(handle) else {
                stage.add_unavailable.insert(dependent);
                continue;
            };
            for &supporter_leaf in constraint_leaves {
                if supporter_leaf == leaf {
                    continue;
                }
                let supporter = self
                    .group_atoms
                    .get(&(constraint_index, supporter_leaf, key.clone()))
                    .copied()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                stage.structural.append_requirements.push(
                    kernel_grounded_closure::BipolarSupportRequirement::new(dependent, [supporter]),
                );
                stage
                    .pending_rules
                    .push(PendingSemanticQuotientRuleDirectory::Row {
                        constraint: constraint_index,
                        dependent_leaf: leaf,
                        handle: *handle,
                        supporter_leaf,
                    });
            }
        }
        Ok(())
    }

    fn install_appended_rule_ids(
        &mut self,
        pending_rules: Vec<PendingSemanticQuotientRuleDirectory>,
        appended: Vec<kernel_grounded_closure::GroundedRuleId>,
    ) -> Result<(), PhysicalExecutionError> {
        if appended.len() != pending_rules.len() {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
        for (pending, rule) in pending_rules.into_iter().zip(appended) {
            match pending {
                PendingSemanticQuotientRuleDirectory::Class { atom } => {
                    self.class_rules_by_atom.set(atom.index(), Some(rule));
                }
                PendingSemanticQuotientRuleDirectory::Row {
                    constraint,
                    dependent_leaf,
                    handle,
                    supporter_leaf,
                } => self.row_rules[constraint][dependent_leaf][supporter_leaf]
                    .insert(handle, rule)?,
            }
        }
        Ok(())
    }
}

