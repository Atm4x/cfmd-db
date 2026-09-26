#[cfg(test)]
thread_local! {
     static SEMANTIC_QUOTIENT_FULL_BFC_COMPILES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
     static SEMANTIC_QUOTIENT_DENSE_PROJECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn compile_semantic_quotient_bfc_program(
    handles: &[&[PhysicalRowId]],
    constraints: &[&SemanticQuotientConstraint],
    atoms_by_handle: &[&StableSemanticQuotientAtomDirectory],
    group_atoms: &BTreeMap<SemanticQuotientGroupAtomKey, kernel_grounded_closure::GroundedAtomId>,
    atom_count: usize,
) -> Result<kernel_grounded_closure::BipolarSupportProgram, PhysicalExecutionError> {
    #[cfg(test)]
    SEMANTIC_QUOTIENT_FULL_BFC_COMPILES.with(|count| count.set(count.get().saturating_add(1)));

    if handles.len() != atoms_by_handle.len() {
        return Err(RelQueryError::InconsistentIncrementalDelta.into());
    }
    let mut active_row_atoms = BTreeSet::new();
    for (leaf, rows) in handles.iter().enumerate() {
        for handle in *rows {
            let atom = semantic_quotient_atom_for_handle(atoms_by_handle, leaf, *handle)?;
            if atom.index() >= atom_count {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            active_row_atoms.insert(atom);
        }
    }

    // Factor each semantic equivalence class through one stable auxiliary atom:
    //
    //     class_atom(c, leaf, key) <- OR(rows in that class)
    //     row                        <- class_atom(other leaf, same key)
    //
    // The unfactored representation repeated the whole supporter bucket in
    // every dependent row requirement, which is Θ(m²) for a duplicate-heavy
    // class of size m.  The auxiliary atom is exact under the same greatest-
    // support semantics and stores the bucket only once.
    let mut active_group_atoms = BTreeSet::new();
    for (constraint_index, constraint) in constraints.iter().enumerate() {
        for quotient_leaf in &constraint.leaves {
            for key in quotient_leaf.buckets.keys() {
                let group_key = (constraint_index, quotient_leaf.leaf, key.clone());
                let atom = group_atoms
                    .get(&group_key)
                    .copied()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                active_group_atoms.insert(atom);
            }
        }
    }

    let mut unavailable = (0..atom_count)
        .map(kernel_grounded_closure::GroundedAtomId::new)
        .filter(|atom| !active_row_atoms.contains(atom) && !active_group_atoms.contains(atom))
        .collect::<BTreeSet<_>>();
    let mut requirements = Vec::new();
    append_semantic_quotient_class_requirements(
        handles,
        constraints,
        atoms_by_handle,
        group_atoms,
        &mut requirements,
    )?;
    for (constraint_index, constraint) in constraints.iter().enumerate() {
        for quotient_leaf in &constraint.leaves {
            let dependent_handles = handles
                .get(quotient_leaf.leaf)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if dependent_handles.len() != quotient_leaf.keys.len() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            for (ordinal, key) in quotient_leaf.keys.iter().enumerate() {
                let dependent = semantic_quotient_atom_for_handle(
                    atoms_by_handle,
                    quotient_leaf.leaf,
                    dependent_handles[ordinal],
                )?;
                let Some(key) = key else {
                    unavailable.insert(dependent);
                    continue;
                };
                for supporter_leaf in &constraint.leaves {
                    if supporter_leaf.leaf == quotient_leaf.leaf {
                        continue;
                    }
                    let supporter = group_atoms
                        .get(&(constraint_index, supporter_leaf.leaf, key.clone()))
                        .copied()
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                    requirements.push(kernel_grounded_closure::BipolarSupportRequirement::new(
                        dependent,
                        [supporter],
                    ));
                }
            }
        }
    }
    kernel_grounded_closure::BipolarSupportProgram::new(atom_count, unavailable, requirements)
        .map_err(|_| RelQueryError::InconsistentIncrementalDelta.into())
}

fn semantic_quotient_bfc_rule_directories(
    handles: &[&[PhysicalRowId]],
    constraints: &[&SemanticQuotientConstraint],
    group_atoms: &BTreeMap<SemanticQuotientGroupAtomKey, kernel_grounded_closure::GroundedAtomId>,
    atom_count: usize,
) -> Result<SemanticQuotientRuleDirectories, PhysicalExecutionError> {
    let mut next_rule = 0_usize;
    let mut class_rules_by_atom = PersistentPhysicalVec::from_vec(vec![None; atom_count]);
    for (constraint_index, constraint) in constraints.iter().enumerate() {
        for quotient_leaf in &constraint.leaves {
            for key in quotient_leaf.buckets.keys() {
                let atom = group_atoms
                    .get(&(constraint_index, quotient_leaf.leaf, key.clone()))
                    .copied()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                class_rules_by_atom.set(
                    atom.index(),
                    Some(kernel_grounded_closure::GroundedRuleId::new(next_rule)),
                );
                next_rule = next_rule.saturating_add(1);
            }
        }
    }

    let leaf_count = handles.len();
    let mut row_rules =
        vec![
            vec![vec![StableSemanticQuotientRuleDirectory::default(); leaf_count]; leaf_count];
            constraints.len()
        ];
    for (constraint_index, constraint) in constraints.iter().enumerate() {
        for quotient_leaf in &constraint.leaves {
            let dependent_handles = handles
                .get(quotient_leaf.leaf)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if dependent_handles.len() != quotient_leaf.keys.len() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            for (ordinal, key) in quotient_leaf.keys.iter().enumerate() {
                if key.is_none() {
                    continue;
                }
                let handle = dependent_handles[ordinal];
                for supporter_leaf in &constraint.leaves {
                    if supporter_leaf.leaf == quotient_leaf.leaf {
                        continue;
                    }
                    row_rules[constraint_index][quotient_leaf.leaf][supporter_leaf.leaf].insert(
                        handle,
                        kernel_grounded_closure::GroundedRuleId::new(next_rule),
                    )?;
                    next_rule = next_rule.saturating_add(1);
                }
            }
        }
    }
    Ok((class_rules_by_atom, row_rules))
}

fn semantic_quotient_masks_from_bfc(
    handles: &[&[PhysicalRowId]],
    atoms_by_handle: &[&StableSemanticQuotientAtomDirectory],
    certificate: &kernel_grounded_closure::BipolarSupportCertificate,
) -> Result<Vec<Vec<u64>>, PhysicalExecutionError> {
    handles
        .iter()
        .enumerate()
        .map(|(leaf, rows)| {
            semantic_quotient_mask_from_bfc(rows, atoms_by_handle[leaf], certificate)
        })
        .collect()
}

fn semantic_quotient_mask_from_bfc(
    rows: &[PhysicalRowId],
    atoms_by_handle: &StableSemanticQuotientAtomDirectory,
    certificate: &kernel_grounded_closure::BipolarSupportCertificate,
) -> Result<Vec<u64>, PhysicalExecutionError> {
    let mut mask = vec![0_u64; rows.len().div_ceil(64)];
    for (ordinal, handle) in rows.iter().copied().enumerate() {
        let atom = atoms_by_handle
            .get(handle)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        if certificate.is_supported(atom) {
            mask[ordinal / 64] |= 1_u64 << (ordinal % 64);
        }
    }
    Ok(mask)
}

fn semantic_quotient_mask_from_stable_bfc(
    rows: &StableSemanticQuotientRows,
    atoms_by_handle: &StableSemanticQuotientAtomDirectory,
    certificate: &kernel_grounded_closure::BipolarSupportCertificate,
) -> Result<Vec<u64>, PhysicalExecutionError> {
    let mut mask = vec![0_u64; rows.live_count.div_ceil(64)];
    let mut dense_ordinal = 0_usize;
    for handle in rows.by_ordinal.iter().filter_map(|handle| *handle) {
        let atom = atoms_by_handle
            .get(handle)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        if certificate.is_supported(atom) {
            mask[dense_ordinal / 64] |= 1_u64 << (dense_ordinal % 64);
        }
        dense_ordinal = dense_ordinal.saturating_add(1);
    }
    Ok(mask)
}

#[cfg(test)]
fn quotient_support_masks_legacy(
    handles: &[Vec<PhysicalRowId>],
    constraints: &mut [SemanticQuotientConstraint],
) -> Vec<Vec<u64>> {
    let mut masks = handles
        .iter()
        .map(|rows| full_ordinal_mask(rows.len()))
        .collect::<Vec<_>>();
    let mask_refs = masks.iter().map(Vec::as_slice).collect::<Vec<_>>();
    for constraint in constraints.iter_mut() {
        reset_constraint_live_support(constraint, &mask_refs);
    }
    let changed_leaves = (0..handles.len()).collect::<Vec<_>>();
    propagate_quotient_support_deletions(handles, constraints, &mut masks, &changed_leaves);
    masks
}

