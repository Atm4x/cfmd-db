#[derive(Debug, Clone)]
struct SubsetJoinCandidate {
    cost: u128,
    estimated_rows: usize,
}

fn predicate_crosses_subsets(predicate: MultiwayJoinPredicate, left: u64, right: u64) -> bool {
    let left_endpoint = 1_u64 << predicate.left.leaf;
    let right_endpoint = 1_u64 << predicate.right.leaf;
    (left & left_endpoint != 0 && right & right_endpoint != 0)
        || (left & right_endpoint != 0 && right & left_endpoint != 0)
}

fn subset_singleton_prefers_indexed_access(
    planner: &MultiwayPlannerContext<'_>,
    predicates: &[MultiwayJoinPredicate],
    singleton_subset: u64,
    other_subset: u64,
    other_rows: usize,
) -> Result<bool, PhysicalExecutionError> {
    if singleton_subset.count_ones() != 1 {
        return Ok(false);
    }
    let singleton = singleton_subset.trailing_zeros() as usize;
    for predicate in predicates
        .iter()
        .copied()
        .filter(|predicate| predicate_crosses_subsets(*predicate, singleton_subset, other_subset))
    {
        let oriented = if predicate.right.leaf == singleton {
            predicate
        } else if predicate.left.leaf == singleton {
            MultiwayJoinPredicate {
                left: predicate.right,
                right: predicate.left,
                equivalence: predicate.equivalence,
            }
        } else {
            continue;
        };
        let decision = multiway_right_access_estimate(
            planner.store,
            &planner.leaves[singleton],
            oriented,
            other_rows,
            planner.context,
            planner.registry,
        )?;
        if matches!(
            decision.family,
            JoinAccessFamily::PersistedI64 | JoinAccessFamily::PersistedSemantic
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn subset_merge_estimate(
    planner: &MultiwayPlannerContext<'_>,
    predicates: &[MultiwayJoinPredicate],
    left_subset: u64,
    right_subset: u64,
    left_rows: usize,
    right_rows: usize,
) -> Result<Option<(u128, usize)>, PhysicalExecutionError> {
    let product_rows = left_rows.saturating_mul(right_rows);
    if subset_singleton_prefers_indexed_access(
        planner,
        predicates,
        right_subset,
        left_subset,
        left_rows,
    )? || subset_singleton_prefers_indexed_access(
        planner,
        predicates,
        left_subset,
        right_subset,
        right_rows,
    )? {
        return Ok(None);
    }
    let mut estimated_rows = None;
    for predicate in predicates
        .iter()
        .copied()
        .filter(|predicate| predicate_crosses_subsets(*predicate, left_subset, right_subset))
    {
        let Some((left_distinct, right_distinct)) = leaf_pair_distinct_estimate(
            planner.store,
            planner.leaves,
            predicate,
            planner.context,
            planner.registry,
        )?
        else {
            continue;
        };
        let rows = product_rows.div_ceil(left_distinct.max(right_distinct).max(1));
        estimated_rows = Some(estimated_rows.map_or(rows, |current: usize| current.min(rows)));
    }
    Ok(estimated_rows.map(|rows| {
        let access_work =
            SemanticAccessCostModel::canonical_bucket_join_access_work(left_rows, right_rows, 1)
                as u128;
        (access_work.saturating_add(rows as u128), rows)
    }))
}

fn best_subset_join_candidate(
    planner: &MultiwayPlannerContext<'_>,
    predicates: &[MultiwayJoinPredicate],
) -> Result<Option<SubsetJoinCandidate>, PhysicalExecutionError> {
    let count = planner.leaves.len();
    if !(3..=8).contains(&count) {
        return Ok(None);
    }
    let full = (1_u64 << count) - 1;
    let mut best = BTreeMap::<u64, SubsetJoinCandidate>::new();
    for (leaf, relation) in planner.leaves.iter().enumerate() {
        best.insert(
            1_u64 << leaf,
            SubsetJoinCandidate {
                cost: 0,
                estimated_rows: relation.rows,
            },
        );
    }

    for cardinality in 2..=count {
        for subset in 1_u64..=full {
            if subset.count_ones() as usize != cardinality {
                continue;
            }
            let mut candidate_best: Option<SubsetJoinCandidate> = None;
            let mut left_subset = (subset - 1) & subset;
            while left_subset != 0 {
                let right_subset = subset ^ left_subset;
                if right_subset != 0 && left_subset < right_subset {
                    let (Some(left), Some(right)) =
                        (best.get(&left_subset), best.get(&right_subset))
                    else {
                        left_subset = (left_subset - 1) & subset;
                        continue;
                    };
                    if let Some((join_work, estimated_rows)) = subset_merge_estimate(
                        planner,
                        predicates,
                        left_subset,
                        right_subset,
                        left.estimated_rows,
                        right.estimated_rows,
                    )? {
                        let candidate = SubsetJoinCandidate {
                            cost: left
                                .cost
                                .saturating_add(right.cost)
                                .saturating_add(join_work),
                            estimated_rows,
                        };
                        let replace = candidate_best.as_ref().is_none_or(|current| {
                            candidate.cost < current.cost
                                || (candidate.cost == current.cost
                                    && candidate.estimated_rows < current.estimated_rows)
                        });
                        if replace {
                            candidate_best = Some(candidate);
                        }
                    }
                }
                left_subset = (left_subset - 1) & subset;
            }
            if let Some(candidate) = candidate_best {
                best.insert(subset, candidate);
            }
        }
    }
    Ok(best.remove(&full))
}

fn bounded_cyclic_advisor_preflight(
    leaves: &[MultiwayJoinLeaf],
    predicates: &[MultiwayJoinPredicate],
    program: &PreparedSemanticQuotientProgram,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, PhysicalExecutionError> {
    if program.search_certificate != Some(SemanticQuotientSearchCertificate::BoundedCyclic) {
        return Ok(true);
    }
    let handles = leaves
        .iter()
        .map(|leaf| store.logical_row_handles(leaf.relation, leaf.layout))
        .collect::<Result<Vec<_>, _>>()?;
    let build = SemanticQuotientBuildContext {
        leaves,
        predicates,
        handles: &handles,
        store,
        context,
        registry,
    };
    let mut stats = ExecutionStats::default();
    let Some(mut constraints) =
        build_semantic_quotient_constraints(&build, Some(program.specs.as_slice()), &mut stats)?
    else {
        return Ok(false);
    };
    let base_masks = quotient_support_masks(&handles, &mut constraints)?;
    if base_masks
        .iter()
        .any(|mask| mask.iter().all(|word| *word == 0))
    {
        return Ok(true);
    }
    let row_counts = handles.iter().map(Vec::len).collect::<Vec<_>>();
    let constraint_refs = constraints.iter().collect::<Vec<_>>();
    let mask_refs = base_masks.iter().map(Vec::as_slice).collect::<Vec<_>>();
    Ok(bounded_cyclic_execution_certificate(
        Some(program),
        &constraint_refs,
        &mask_refs,
        &row_counts,
        program.hypergraph_order.as_deref().unwrap_or(&[]),
    )
    .is_some())
}

pub(super) fn preferred_nway_order_preserving_join_available(
    plan: &Plan,
    prepared_program: Option<&PreparedSemanticQuotientProgram>,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    advisor_preflight: bool,
) -> Result<bool, PhysicalExecutionError> {
    Ok(preferred_nway_order_preserving_join_inputs(
        plan,
        prepared_program,
        store,
        context,
        registry,
        advisor_preflight,
    )?
    .is_some())
}

fn preferred_nway_order_preserving_join_inputs(
    plan: &Plan,
    prepared_program: Option<&PreparedSemanticQuotientProgram>,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    advisor_preflight: bool,
) -> Result<Option<PreferredNwayOrderPreservingJoin>, PhysicalExecutionError> {
    let mut leaves = Vec::new();
    let mut predicates = Vec::new();
    let Some(_) = flatten_multiway_join_tree(plan, store, &mut leaves, &mut predicates)? else {
        return Ok(None);
    };
    if leaves.len() < 3 {
        return Ok(None);
    }
    let hypergraph_order = prepared_program
        .and_then(|program| program.hypergraph_order.as_ref())
        .filter(|order| order.len() == leaves.len());
    if leaves.len() > 8 {
        let Some(search_order) = hypergraph_order else {
            return Ok(None);
        };
        if advisor_preflight
            && let Some(program) = prepared_program
            && !bounded_cyclic_advisor_preflight(
                &leaves,
                &predicates,
                program,
                store,
                context,
                registry,
            )?
        {
            return Ok(None);
        }
        return Ok(Some(PreferredNwayOrderPreservingJoin {
            leaves,
            predicates,
            search_order: search_order.clone(),
        }));
    }
    let planner = MultiwayPlannerContext {
        store,
        leaves: &leaves,
        context,
        registry,
    };
    let Some(candidate) = best_subset_join_candidate(&planner, &predicates)? else {
        return Ok(None);
    };
    let Some(contiguous) = best_contiguous_multiway_join_candidate(plan, store, context, registry)?
    else {
        return Ok(None);
    };
    let estimated_work = candidate
        .cost
        .saturating_add(estimated_quotient_build_work(
            store,
            &leaves,
            &predicates,
            prepared_program,
            context,
            registry,
        )?)
        .saturating_add(candidate.estimated_rows as u128);
    if estimated_work >= contiguous.cost {
        return Ok(None);
    }
    Ok(Some(PreferredNwayOrderPreservingJoin {
        search_order: hypergraph_order
            .cloned()
            .unwrap_or_else(|| (0..leaves.len()).collect()),
        leaves,
        predicates,
    }))
}

pub(super) fn try_execute_nway_order_preserving_join(
    plan: &Plan,
    prepared_program: Option<&PreparedSemanticQuotientProgram>,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let Some(preferred) = preferred_nway_order_preserving_join_inputs(
        plan,
        prepared_program,
        store,
        context,
        registry,
        false,
    )?
    else {
        return Ok(None);
    };
    execute_order_preserving_quotient_join(
        QuotientJoinExecutionRequest {
            leaves: &preferred.leaves,
            predicates: &preferred.predicates,
            search_order: &preferred.search_order,
            prepared_program,
            store,
            context,
            registry,
        },
        stats,
    )
}

