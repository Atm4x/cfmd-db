fn quotient_key_cache(
    specs: &[(SemanticId, Vec<SemanticQuotientEndpoint>)],
    leaves: &[SemanticQuotientPhysicalLeaf],
    handles: &[Vec<PhysicalRowId>],
    store: &dyn SemanticQuotientStoreView,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<SemanticQuotientKeyCache>, PhysicalExecutionError> {
    let mut cache = BTreeMap::new();
    for (equivalence, endpoints) in specs {
        if context
            .schema
            .structural_equivalence(*equivalence)
            .is_some()
        {
            registry.equivalence_domain(context, *equivalence)?;
        } else if registry
            .resolve_primitive_equivalence(context, *equivalence)?
            .is_none()
        {
            return Ok(None);
        }
        for endpoint in endpoints {
            let cache_key = (endpoint.leaf, endpoint.column, *equivalence);
            if cache.contains_key(&cache_key) {
                continue;
            }
            let factor_binding = SemanticIndexBinding::single(
                leaves[endpoint.leaf].relation,
                leaves[endpoint.leaf].layout,
                endpoint.column,
                *equivalence,
            );
            let maintained_factor =
                store.has_semantic_quotient_capability(&factor_binding, context, registry)?;
            let mut keys = Vec::with_capacity(handles[endpoint.leaf].len());
            for row_id in &handles[endpoint.leaf] {
                if maintained_factor
                    && let Some(key) = store.semantic_quotient_single_key(
                        &factor_binding,
                        *row_id,
                        context,
                        registry,
                    )?
                {
                    keys.push(key);
                    stats.multiway_join_maintained_quotient_key_hits = stats
                        .multiway_join_maintained_quotient_key_hits
                        .saturating_add(1);
                    continue;
                }
                let leaf = leaves[endpoint.leaf];
                let value = store.semantic_quotient_value(
                    leaf.relation,
                    leaf.layout,
                    *row_id,
                    endpoint.column,
                )?;
                keys.push(registry.canonical_equivalence_key(context, *equivalence, &value)?);
                stats.values_read = stats.values_read.saturating_add(1);
            }
            cache.insert(cache_key, keys);
        }
    }
    Ok(Some(cache))
}

fn quotient_leaf_keys(
    leaf_index: usize,
    columns: &[usize],
    equivalence: SemanticId,
    row_count: usize,
    key_cache: &BTreeMap<(usize, usize, SemanticId), Vec<kernel_semantics::CanonicalEqKey>>,
) -> Result<SemanticQuotientLeaf, PhysicalExecutionError> {
    let mut keys = Vec::with_capacity(row_count);
    for ordinal in 0..row_count {
        let mut row_key = None;
        let mut consistent = true;
        for column in columns {
            let key = key_cache
                .get(&(leaf_index, *column, equivalence))
                .and_then(|keys| keys.get(ordinal))
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if row_key.as_ref().is_some_and(|current| current != key) {
                consistent = false;
                break;
            }
            row_key = Some(key.clone());
        }
        keys.push(consistent.then_some(row_key).flatten());
    }
    let mut buckets = BTreeMap::new();
    let mut live_rows_by_key = BTreeMap::new();
    for (ordinal, key) in keys.iter().enumerate() {
        let Some(key) = key else { continue };
        *live_rows_by_key.entry(key.clone()).or_insert(0) += 1;
        let mask = buckets
            .entry(key.clone())
            .or_insert_with(|| vec![0_u64; keys.len().div_ceil(64)]);
        mask[ordinal / 64] |= 1_u64 << (ordinal % 64);
    }
    Ok(SemanticQuotientLeaf {
        leaf: leaf_index,
        keys,
        buckets,
        live_rows_by_key,
    })
}

fn quotient_key_leaf_support(
    leaves: &[SemanticQuotientLeaf],
) -> BTreeMap<kernel_semantics::CanonicalEqKey, usize> {
    let mut support = BTreeMap::new();
    for leaf in leaves {
        for key in leaf.buckets.keys() {
            *support.entry(key.clone()).or_insert(0) += 1;
        }
    }
    support
}

pub(super) struct PreparedSemanticQuotientBuildContext<'a> {
    pub(super) leaves: &'a [SemanticQuotientPhysicalLeaf],
    pub(super) handles: &'a [Vec<PhysicalRowId>],
    pub(super) store: &'a dyn SemanticQuotientStoreView,
    pub(super) context: &'a kernel_schema::SemanticContext,
    pub(super) registry: &'a kernel_semantics::SemanticRegistry,
}

pub(super) fn build_semantic_quotient_constraints_for_prepared_specs(
    build: &PreparedSemanticQuotientBuildContext<'_>,
    specs: &[(SemanticId, Vec<SemanticQuotientEndpoint>)],
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<SemanticQuotientConstraint>>, PhysicalExecutionError> {
    let Some(key_cache) = quotient_key_cache(
        specs,
        build.leaves,
        build.handles,
        build.store,
        build.context,
        build.registry,
        stats,
    )?
    else {
        return Ok(None);
    };
    let mut constraints = Vec::with_capacity(specs.len());
    for (equivalence, endpoints) in specs {
        let mut columns_by_leaf = BTreeMap::<usize, Vec<usize>>::new();
        for endpoint in endpoints {
            columns_by_leaf
                .entry(endpoint.leaf)
                .or_default()
                .push(endpoint.column);
        }
        let mut quotient_leaves = Vec::with_capacity(columns_by_leaf.len());
        for (leaf, columns) in columns_by_leaf {
            quotient_leaves.push(quotient_leaf_keys(
                leaf,
                &columns,
                *equivalence,
                build.handles[leaf].len(),
                &key_cache,
            )?);
        }
        let key_leaf_support = quotient_key_leaf_support(&quotient_leaves);
        constraints.push(SemanticQuotientConstraint {
            leaves: quotient_leaves,
            live_key_leaf_support: key_leaf_support.clone(),
            key_leaf_support,
        });
    }
    Ok(Some(constraints))
}

pub(super) fn quotient_support_masks(
    handles: &[Vec<PhysicalRowId>],
    constraints: &mut [SemanticQuotientConstraint],
) -> Result<Vec<Vec<u64>>, PhysicalExecutionError> {
    let handle_refs = handles.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let constraint_refs = constraints.iter().collect::<Vec<_>>();
    let (atoms_by_handle, mut atom_count) = semantic_quotient_initial_bfc_atoms(&handle_refs);
    let atom_refs = atoms_by_handle.iter().collect::<Vec<_>>();
    let mut group_atoms = BTreeMap::new();
    ensure_semantic_quotient_group_atoms(&constraint_refs, &mut group_atoms, &mut atom_count);
    let program = compile_semantic_quotient_bfc_program(
        &handle_refs,
        &constraint_refs,
        &atom_refs,
        &group_atoms,
        atom_count,
    )?;
    let (certificate, _) = kernel_grounded_closure::solve_bipolar_support(&program)
        .map_err(|_| RelQueryError::InconsistentIncrementalDelta)?;
    let masks = semantic_quotient_masks_from_bfc(&handle_refs, &atom_refs, &certificate)?;
    let mask_refs = masks.iter().map(Vec::as_slice).collect::<Vec<_>>();
    for constraint in constraints.iter_mut() {
        reset_constraint_live_support(constraint, &mask_refs);
    }
    Ok(masks)
}

fn semantic_quotient_initial_bfc_atoms(
    handles: &[&[PhysicalRowId]],
) -> (Vec<StableSemanticQuotientAtomDirectory>, usize) {
    let mut atom_count = 0_usize;
    let atoms_by_handle = handles
        .iter()
        .map(|rows| {
            let mut directory = StableSemanticQuotientAtomDirectory::default();
            for handle in rows.iter().copied() {
                let atom = kernel_grounded_closure::GroundedAtomId::new(atom_count);
                atom_count = atom_count.saturating_add(1);
                directory
                    .insert(handle, atom)
                    .expect("fresh QCN row handles must be unique");
            }
            directory
        })
        .collect();
    (atoms_by_handle, atom_count)
}

fn semantic_quotient_atom_for_handle(
    atoms_by_handle: &[&StableSemanticQuotientAtomDirectory],
    leaf: usize,
    handle: PhysicalRowId,
) -> Result<kernel_grounded_closure::GroundedAtomId, PhysicalExecutionError> {
    atoms_by_handle
        .get(leaf)
        .and_then(|atoms| atoms.get(handle))
        .ok_or(RelQueryError::InconsistentIncrementalDelta.into())
}

fn append_semantic_quotient_class_requirements(
    handles: &[&[PhysicalRowId]],
    constraints: &[&SemanticQuotientConstraint],
    atoms_by_handle: &[&StableSemanticQuotientAtomDirectory],
    group_atoms: &BTreeMap<SemanticQuotientGroupAtomKey, kernel_grounded_closure::GroundedAtomId>,
    requirements: &mut Vec<kernel_grounded_closure::BipolarSupportRequirement>,
) -> Result<(), PhysicalExecutionError> {
    for (constraint_index, constraint) in constraints.iter().enumerate() {
        for quotient_leaf in &constraint.leaves {
            for (key, bucket) in &quotient_leaf.buckets {
                let group_atom = group_atoms
                    .get(&(constraint_index, quotient_leaf.leaf, key.clone()))
                    .copied()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let supporter_handles = handles
                    .get(quotient_leaf.leaf)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let supporters = mask_ordinals(bucket)
                    .map(|ordinal| {
                        supporter_handles
                            .get(ordinal)
                            .copied()
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)
                    })
                    .map(|handle| {
                        semantic_quotient_atom_for_handle(
                            atoms_by_handle,
                            quotient_leaf.leaf,
                            handle?,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                requirements.push(kernel_grounded_closure::BipolarSupportRequirement::new(
                    group_atom, supporters,
                ));
            }
        }
    }
    Ok(())
}

fn ensure_semantic_quotient_group_atoms(
    constraints: &[&SemanticQuotientConstraint],
    group_atoms: &mut BTreeMap<
        SemanticQuotientGroupAtomKey,
        kernel_grounded_closure::GroundedAtomId,
    >,
    atom_count: &mut usize,
) {
    for (constraint_index, constraint) in constraints.iter().enumerate() {
        let keys = constraint
            .leaves
            .iter()
            .flat_map(|leaf| leaf.buckets.keys().cloned())
            .collect::<BTreeSet<_>>();
        for quotient_leaf in &constraint.leaves {
            for key in &keys {
                let group_key = (constraint_index, quotient_leaf.leaf, key.clone());
                group_atoms.entry(group_key).or_insert_with(|| {
                    let atom = kernel_grounded_closure::GroundedAtomId::new(*atom_count);
                    *atom_count = atom_count.saturating_add(1);
                    atom
                });
            }
        }
    }
}

