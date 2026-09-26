fn semantic_quotient_support_key_binding(
    binding: &SemanticQuotientSupportBinding,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<kernel_semantic_index::SemanticIndexBinding, PhysicalExecutionError> {
    let dependencies = semantic_key_dependencies_for_equivalences(
        binding.specs.iter().map(|(equivalence, _)| *equivalence),
        context,
        registry,
    )?;
    let structural_definitions = semantic_key_structural_definitions_for_equivalences(
        binding.specs.iter().map(|(equivalence, _)| *equivalence),
        context,
        registry,
    )?;
    Ok(kernel_semantic_index::SemanticIndexBinding::new_with_structural_definitions(
        context,
        dependencies,
        structural_definitions,
    ))
}

pub(super) fn build_semantic_quotient_support_state(
    binding: SemanticQuotientSupportBinding,
    store: &dyn SemanticQuotientStoreView,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<MaterializedSemanticQuotientSupportState>, PhysicalExecutionError> {
    let key_binding = semantic_quotient_support_key_binding(&binding, context, registry)?;
    let leaves = binding.leaves.clone();
    let handles = leaves
        .iter()
        .map(|leaf| store.semantic_quotient_logical_row_handles(leaf.relation, leaf.layout))
        .collect::<Result<Vec<_>, _>>()?;
    let mut stats = ExecutionStats::default();
    let build = PreparedSemanticQuotientBuildContext {
        leaves: &leaves,
        handles: &handles,
        store,
        context,
        registry,
    };
    let Some(constraints) = build_semantic_quotient_constraints_for_prepared_specs(
        &build,
        binding.specs.as_slice(),
        &mut stats,
    )?
    else {
        return Ok(None);
    };
    let stable_keys = initial_stable_semantic_quotient_keys(&constraints);
    let constraint_leaves = semantic_quotient_constraint_leaves(&constraints);
    let handles = handles.into_iter().map(Arc::new).collect::<Vec<_>>();
    let stable_rows = handles
        .iter()
        .map(|handles| Arc::new(StableSemanticQuotientRows::from_dense(handles)))
        .collect::<Vec<_>>();
    let mut constraints = constraints.into_iter().map(Arc::new).collect::<Vec<_>>();
    let handle_refs = handles
        .iter()
        .map(|handles| handles.as_slice())
        .collect::<Vec<_>>();
    let constraint_refs = constraints.iter().map(Arc::as_ref).collect::<Vec<_>>();
    let (atoms_by_handle, mut atom_count) = semantic_quotient_initial_bfc_atoms(&handle_refs);
    let atom_refs = atoms_by_handle.iter().collect::<Vec<_>>();
    let mut row_leaf_by_atom = PersistentPhysicalVec::from_vec(
        handle_refs
            .iter()
            .enumerate()
            .flat_map(|(leaf, rows)| std::iter::repeat_n(Some(leaf), rows.len()))
            .collect(),
    );
    let mut group_atoms = BTreeMap::new();
    ensure_semantic_quotient_group_atoms(&constraint_refs, &mut group_atoms, &mut atom_count);
    row_leaf_by_atom.resize(atom_count, None);
    let bfc_program = compile_semantic_quotient_bfc_program(
        &handle_refs,
        &constraint_refs,
        &atom_refs,
        &group_atoms,
        atom_count,
    )?;
    let (class_rules_by_atom, row_rules) = semantic_quotient_bfc_rule_directories(
        &handle_refs,
        &constraint_refs,
        &group_atoms,
        atom_count,
    )?;
    let bfc_maintainer = kernel_grounded_closure::BipolarSupportMaintenance::new(&bfc_program)
        .map_err(|_| RelQueryError::InconsistentIncrementalDelta)?;
    let bfc_work = kernel_grounded_closure::GroundedWorkStats::default();
    let base_masks =
        semantic_quotient_masks_from_bfc(&handle_refs, &atom_refs, bfc_maintainer.certificate())?;
    let mask_refs = base_masks.iter().map(Vec::as_slice).collect::<Vec<_>>();
    for constraint in &mut constraints {
        reset_constraint_live_support(Arc::make_mut(constraint), &mask_refs);
    }
    let dense_projection = initial_semantic_quotient_dense_projection(&constraints);
    Ok(Some(MaterializedSemanticQuotientSupportState {
        binding,
        key_binding,
        stable_rows,
        stable_keys,
        constraint_leaves,
        base_masks: base_masks.into_iter().map(Arc::new).collect(),
        dense_projection,
        bfc: SemanticQuotientBfcMaintenance {
            atoms_by_handle: atoms_by_handle.into_iter().map(Arc::new).collect(),
            row_leaf_by_atom,
            group_atoms: group_atoms.into_iter().collect(),
            class_rules_by_atom,
            row_rules,
            maintainer: bfc_maintainer,
            last_work: bfc_work,
        },
    }))
}

pub(super) fn quotient_leaf(
    constraint: &SemanticQuotientConstraint,
    leaf: usize,
) -> Option<&SemanticQuotientLeaf> {
    constraint
        .leaves
        .iter()
        .find(|candidate| candidate.leaf == leaf)
}

