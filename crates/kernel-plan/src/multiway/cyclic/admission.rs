const MAX_BOUNDED_CYCLIC_ENUMERATION_WORK: u128 = 1_000_000;

fn mask_population(mask: &[u64]) -> usize {
    mask.iter().map(|word| word.count_ones() as usize).sum()
}

fn masked_bucket_population(mask: &[u64], bucket: &[u64]) -> usize {
    mask.iter()
        .zip(bucket)
        .map(|(left, right)| (left & right).count_ones() as usize)
        .sum()
}

fn quotient_enumeration_inputs_valid(
    constraints: &[&SemanticQuotientConstraint],
    base_masks: &[&[u64]],
    row_counts: &[usize],
    search_order: &[usize],
) -> bool {
    if search_order.len() != base_masks.len() || row_counts.len() != base_masks.len() {
        return false;
    }
    for (leaf, (mask, row_count)) in base_masks.iter().zip(row_counts).enumerate() {
        if mask.len() != row_count.div_ceil(64) {
            return false;
        }
        if let Some(last) = mask.last() {
            let used = row_count % 64;
            if used != 0 && *last >> used != 0 {
                return false;
            }
        }
        for constraint in constraints {
            let Some(current) = quotient_leaf(constraint, leaf) else {
                continue;
            };
            if current.keys.len() != *row_count
                || current
                    .buckets
                    .values()
                    .any(|bucket| bucket.len() != mask.len())
            {
                return false;
            }
        }
    }
    let mut seen = vec![false; base_masks.len()];
    for leaf in search_order.iter().copied() {
        if leaf >= base_masks.len() || seen[leaf] {
            return false;
        }
        seen[leaf] = true;
    }
    true
}

fn quotient_depth_work_upper_bound(
    constraints: &[&SemanticQuotientConstraint],
    base_masks: &[&[u64]],
    assigned: &[bool],
    leaf: usize,
    prefix_index: Option<&CyclicPrefixCandidateIndex>,
) -> (usize, usize) {
    let mut domain = prefix_index.map_or_else(
        || mask_population(base_masks[leaf]),
        |index| index.max_bucket_len,
    );
    let mut assigned_bucket_intersections = 0_usize;
    let mut candidate_check_units = 1_usize;
    for constraint in constraints {
        let Some(current) = quotient_leaf(constraint, leaf) else {
            continue;
        };
        candidate_check_units = candidate_check_units.saturating_add(1);
        if constraint
            .leaves
            .iter()
            .any(|other| other.leaf != leaf && assigned[other.leaf])
        {
            assigned_bucket_intersections = assigned_bucket_intersections.saturating_add(1);
            let max_bucket = current
                .buckets
                .values()
                .map(|bucket| masked_bucket_population(base_masks[leaf], bucket))
                .max()
                .unwrap_or(0);
            domain = domain.min(max_bucket);
        }
        for later in constraint
            .leaves
            .iter()
            .filter(|later| later.leaf != leaf && !assigned[later.leaf])
        {
            candidate_check_units = candidate_check_units
                .saturating_add(1)
                .saturating_add(base_masks[later.leaf].len());
        }
    }
    let per_parent_work = if let Some(index) = prefix_index {
        let lookup_height = usize::BITS
            .saturating_sub(index.buckets.len().max(1).leading_zeros())
            .saturating_add(1) as usize;
        index
            .constraint_indices
            .len()
            .saturating_add(1)
            .saturating_mul(lookup_height)
            .saturating_add(domain.saturating_mul(candidate_check_units))
    } else {
        let mask_word_units = base_masks[leaf]
            .len()
            .saturating_mul(1_usize.saturating_add(assigned_bucket_intersections));
        mask_word_units.saturating_add(domain.saturating_mul(candidate_check_units))
    };
    (per_parent_work, domain)
}
fn quotient_enumeration_work_upper_bound(
    constraints: &[&SemanticQuotientConstraint],
    base_masks: &[&[u64]],
    row_counts: &[usize],
    search_order: &[usize],
    cyclic_prefix_indexes: Option<&CyclicPrefixCandidateIndexes>,
) -> Option<u128> {
    if !quotient_enumeration_inputs_valid(constraints, base_masks, row_counts, search_order) {
        return None;
    }
    let mut assigned = vec![false; base_masks.len()];
    let mut parent_prefix = 1_u128;
    let mut work = cyclic_prefix_indexes.map_or(0, |indexes| indexes.build_work_upper_bound);
    if work > MAX_BOUNDED_CYCLIC_ENUMERATION_WORK {
        return Some(work);
    }
    for leaf in search_order.iter().copied() {
        let prefix_index = cyclic_prefix_indexes
            .and_then(|indexes| indexes.by_leaf.get(leaf))
            .and_then(Option::as_ref);
        let (per_parent_work, domain) =
            quotient_depth_work_upper_bound(constraints, base_masks, &assigned, leaf, prefix_index);
        work = work.saturating_add(parent_prefix.saturating_mul(per_parent_work as u128));
        if work > MAX_BOUNDED_CYCLIC_ENUMERATION_WORK {
            return Some(work);
        }
        assigned[leaf] = true;
        parent_prefix = parent_prefix.saturating_mul(domain as u128);
    }
    work = work.saturating_add(parent_prefix.saturating_mul(search_order.len() as u128));
    Some(work)
}

enum CyclicExecutionAdmission {
    NotCyclic,
    Bounded(CyclicPrefixCandidateIndexes),
}

fn bounded_cyclic_execution_certificate(
    prepared_program: Option<&PreparedSemanticQuotientProgram>,
    constraints: &[&SemanticQuotientConstraint],
    base_masks: &[&[u64]],
    row_counts: &[usize],
    search_order: &[usize],
) -> Option<CyclicExecutionAdmission> {
    if !prepared_program.is_some_and(|program| {
        program.search_certificate == Some(SemanticQuotientSearchCertificate::BoundedCyclic)
    }) {
        return Some(CyclicExecutionAdmission::NotCyclic);
    }
    let indexes = cyclic_prefix_candidate_indexes(constraints, base_masks, search_order)?;
    let bound = quotient_enumeration_work_upper_bound(
        constraints,
        base_masks,
        row_counts,
        search_order,
        Some(&indexes),
    )?;
    (bound <= MAX_BOUNDED_CYCLIC_ENUMERATION_WORK)
        .then_some(CyclicExecutionAdmission::Bounded(indexes))
}

fn record_quotient_support_stats(
    stats: &mut ExecutionStats,
    handles: &[Vec<PhysicalRowId>],
    constraints: &[&SemanticQuotientConstraint],
    base_masks: &[&[u64]],
) {
    stats.multiway_join_semantic_quotient_constraints = stats
        .multiway_join_semantic_quotient_constraints
        .saturating_add(constraints.len());
    let supported_rows = base_masks
        .iter()
        .map(|mask| mask_population(mask))
        .sum::<usize>();
    stats.multiway_join_semantic_quotient_pruned_rows = stats
        .multiway_join_semantic_quotient_pruned_rows
        .saturating_add(
            handles
                .iter()
                .map(Vec::len)
                .sum::<usize>()
                .saturating_sub(supported_rows),
        );
}

#[derive(Clone, Copy)]
struct QuotientJoinExecutionRequest<'a> {
    leaves: &'a [MultiwayJoinLeaf],
    predicates: &'a [MultiwayJoinPredicate],
    search_order: &'a [usize],
    prepared_program: Option<&'a PreparedSemanticQuotientProgram>,
    store: &'a PhysicalStore,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

fn execute_order_preserving_quotient_join(
    request: QuotientJoinExecutionRequest<'_>,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let QuotientJoinExecutionRequest {
        leaves,
        predicates,
        search_order: _,
        prepared_program,
        store,
        context,
        registry,
    } = request;
    let handles = leaves
        .iter()
        .map(|leaf| store.logical_row_handles(leaf.relation, leaf.layout))
        .collect::<Result<Vec<_>, _>>()?;
    stats.scanned_rows = stats
        .scanned_rows
        .saturating_add(handles.iter().map(Vec::len).sum::<usize>());

    let support_binding =
        prepared_program.map(|program| semantic_quotient_support_binding(leaves, program));
    let maintained_support = support_binding
        .as_ref()
        .and_then(|binding| store.semantic_quotient_support(binding))
        .filter(|state| {
            state.matches_dense_handles(&handles)
                && matches!(state.compatible_with(context, registry), Ok(true))
        });

    let owned_constraints;
    let owned_masks;
    let constraint_refs;
    let mask_refs;
    if let Some(state) = maintained_support {
        stats.multiway_join_maintained_quotient_support_hits = stats
            .multiway_join_maintained_quotient_support_hits
            .saturating_add(1);
        owned_constraints = state.materialize_dense_constraints()?;
        constraint_refs = owned_constraints.iter().collect::<Vec<_>>();
        mask_refs = state
            .base_masks
            .iter()
            .map(|mask| mask.as_slice())
            .collect::<Vec<_>>();
    } else {
        let build = SemanticQuotientBuildContext {
            leaves,
            predicates,
            handles: &handles,
            store,
            context,
            registry,
        };
        let Some(mut constraints) = build_semantic_quotient_constraints(
            &build,
            prepared_program.map(|program| program.specs.as_slice()),
            stats,
        )?
        else {
            return Ok(None);
        };
        owned_masks = quotient_support_masks(&handles, &mut constraints)?;
        owned_constraints = Arc::new(constraints);
        constraint_refs = owned_constraints.iter().collect::<Vec<_>>();
        mask_refs = owned_masks.iter().map(Vec::as_slice).collect::<Vec<_>>();
    }
    let constraints = constraint_refs.as_slice();
    let base_masks = mask_refs.as_slice();

    execute_order_preserving_quotient_join_with_support(
        request,
        &handles,
        constraints,
        base_masks,
        stats,
    )
}

fn execute_order_preserving_quotient_join_with_support(
    request: QuotientJoinExecutionRequest<'_>,
    handles: &[Vec<PhysicalRowId>],
    constraints: &[&SemanticQuotientConstraint],
    base_masks: &[&[u64]],
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let QuotientJoinExecutionRequest {
        leaves,
        predicates,
        search_order,
        prepared_program,
        store,
        context,
        registry,
    } = request;

    record_quotient_support_stats(stats, handles, constraints, base_masks);
    if base_masks
        .iter()
        .any(|mask| mask.iter().all(|word| *word == 0))
    {
        return Ok(Some(Vec::new()));
    }
    let row_counts = handles.iter().map(Vec::len).collect::<Vec<_>>();
    let Some(cyclic_admission) = bounded_cyclic_execution_certificate(
        prepared_program,
        constraints,
        base_masks,
        &row_counts,
        search_order,
    ) else {
        stats.multiway_join_cyclic_budget_rejections = stats
            .multiway_join_cyclic_budget_rejections
            .saturating_add(1);
        return Ok(None);
    };
    let execution = OrderPreservingJoinEnumeration {
        handles,
        constraints,
        base_masks,
        search_order,
        cyclic_prefix_indexes: match &cyclic_admission {
            CyclicExecutionAdmission::NotCyclic => None,
            CyclicExecutionAdmission::Bounded(indexes) => Some(indexes),
        },
    };
    let mut assignment = vec![None; leaves.len()];
    let mut assignments = Vec::new();
    execution.enumerate(0, &mut assignment, &mut assignments, stats)?;
    materialize_join_assignments(
        &mut assignments,
        leaves,
        predicates,
        handles,
        store,
        context,
        registry,
    )
    .map(Some)
}

fn leaf_pair_distinct_estimate(
    store: &PhysicalStore,
    leaves: &[MultiwayJoinLeaf],
    predicate: MultiwayJoinPredicate,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<(usize, usize)>, PhysicalExecutionError> {
    let left_leaf = &leaves[predicate.left.leaf];
    let right_leaf = &leaves[predicate.right.leaf];
    let left_binding = SemanticIndexBinding::single(
        left_leaf.relation,
        left_leaf.layout,
        predicate.left.column,
        predicate.equivalence,
    );
    let right_binding = SemanticIndexBinding::single(
        right_leaf.relation,
        right_leaf.layout,
        predicate.right.column,
        predicate.equivalence,
    );
    let Some(left) = store.semantic_statistics(&left_binding, context, registry)? else {
        return Ok(None);
    };
    let Some(right) = store.semantic_statistics(&right_binding, context, registry)? else {
        return Ok(None);
    };
    Ok(Some((
        left.distinct_key_count.max(1),
        right.distinct_key_count.max(1),
    )))
}

fn estimated_quotient_build_work(
    store: &PhysicalStore,
    leaves: &[MultiwayJoinLeaf],
    predicates: &[MultiwayJoinPredicate],
    prepared_program: Option<&PreparedSemanticQuotientProgram>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<u128, PhysicalExecutionError> {
    let owned_specs;
    let quotient_specs = if let Some(program) = prepared_program {
        program.specs.as_slice()
    } else {
        owned_specs = semantic_quotient_specs(predicates, context, registry)?;
        &owned_specs
    };
    let mut specs = BTreeSet::new();
    for (equivalence, endpoints) in quotient_specs {
        for endpoint in endpoints {
            specs.insert((endpoint.leaf, endpoint.column, *equivalence));
        }
    }
    let mut work = 0_u128;
    for (leaf, column, equivalence) in specs {
        let leaf = &leaves[leaf];
        let binding = SemanticIndexBinding::single(leaf.relation, leaf.layout, column, equivalence);
        let compatible = store.has_semantic_quotient_capability(&binding, context, registry)?;
        if !compatible {
            work = work.saturating_add(leaf.rows as u128);
        }
    }
    Ok(work)
}

