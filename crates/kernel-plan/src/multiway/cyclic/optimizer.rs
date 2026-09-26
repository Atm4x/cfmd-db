fn interval_width(leaves: &[MultiwayJoinLeaf], start: usize, end: usize) -> usize {
    leaves[start..=end]
        .iter()
        .fold(0_usize, |width, leaf| width.saturating_add(leaf.width))
}

fn interval_column(
    leaves: &[MultiwayJoinLeaf],
    start: usize,
    column: MultiwayJoinColumnRef,
) -> usize {
    leaves[start..column.leaf]
        .iter()
        .fold(0_usize, |offset, leaf| offset.saturating_add(leaf.width))
        .saturating_add(column.column)
}

fn crossing_join_predicates(
    predicates: &[MultiwayJoinPredicate],
    start: usize,
    split: usize,
    end: usize,
) -> Vec<MultiwayJoinPredicate> {
    predicates
        .iter()
        .filter_map(|predicate| {
            let left_in_left = (start..=split).contains(&predicate.left.leaf);
            let right_in_right = ((split + 1)..=end).contains(&predicate.right.leaf);
            if left_in_left && right_in_right {
                Some(*predicate)
            } else {
                let right_in_left = (start..=split).contains(&predicate.right.leaf);
                let left_in_right = ((split + 1)..=end).contains(&predicate.left.leaf);
                (right_in_left && left_in_right).then_some(MultiwayJoinPredicate {
                    left: predicate.right,
                    right: predicate.left,
                    equivalence: predicate.equivalence,
                })
            }
        })
        .collect()
}

fn multiway_right_access_estimate(
    store: &PhysicalStore,
    leaf: &MultiwayJoinLeaf,
    predicate: MultiwayJoinPredicate,
    left_rows: usize,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<JoinAccessDecision, PhysicalExecutionError> {
    let binding = SemanticIndexBinding::single(
        leaf.relation,
        leaf.layout,
        predicate.right.column,
        predicate.equivalence,
    );
    let allow_ephemeral = store
        .semantic_statistics(&binding, context, registry)?
        .is_some();
    right_scan_join_access_decision(
        RightJoinAccessRequest {
            left_rows,
            right_relation: leaf.relation,
            right_layout: leaf.layout,
            right_column: predicate.right.column,
            equivalence: predicate.equivalence,
            allow_ephemeral,
        },
        store,
        context,
        registry,
    )
}

fn multiway_join_merge_estimate(
    planner: &MultiwayPlannerContext<'_>,
    crossing: &[MultiwayJoinPredicate],
    interval: MultiwayJoinInterval,
    left_rows: usize,
    right_rows: usize,
) -> Result<(u128, usize, usize), PhysicalExecutionError> {
    let fallback_rows = left_rows.saturating_mul(right_rows);
    let fallback_work =
        (SemanticAccessCostModel::canonical_bucket_join_access_work(left_rows, right_rows, 1)
            as u128)
            .saturating_add(fallback_rows as u128);
    let mut best = (fallback_work, fallback_rows, 0_usize);
    if interval.split + 1 != interval.end {
        return Ok(best);
    }
    let right_leaf = &planner.leaves[interval.end];
    for (index, predicate) in crossing.iter().copied().enumerate() {
        let decision = multiway_right_access_estimate(
            planner.store,
            right_leaf,
            predicate,
            left_rows,
            planner.context,
            planner.registry,
        )?;
        let total_work =
            (decision.work as u128).saturating_add(decision.estimated_output_rows as u128);
        if decision.family != JoinAccessFamily::FullScan && total_work < best.0 {
            best = (total_work, decision.estimated_output_rows, index);
        }
    }
    Ok(best)
}

fn build_multiway_join_merge(
    planner: &MultiwayPlannerContext<'_>,
    interval: MultiwayJoinInterval,
    left: Plan,
    right: Plan,
    crossing: &[MultiwayJoinPredicate],
    primary_index: usize,
) -> Plan {
    let leaves = planner.leaves;
    let primary = crossing[primary_index];
    let left_column = interval_column(leaves, interval.start, primary.left);
    let right_column = interval_column(leaves, interval.split + 1, primary.right);
    let mut plan = Plan::JoinEq {
        left: Box::new(left),
        right: Box::new(right),
        left_column,
        right_column,
        equivalence: primary.equivalence,
    };
    let left_width = interval_width(leaves, interval.start, interval.split);
    for (index, predicate) in crossing.iter().copied().enumerate() {
        if index == primary_index {
            continue;
        }
        plan = Plan::FilterEqColumns {
            input: Box::new(plan),
            left_column: interval_column(leaves, interval.start, predicate.left),
            right_column: left_width.saturating_add(interval_column(
                leaves,
                interval.split + 1,
                predicate.right,
            )),
            equivalence: predicate.equivalence,
        };
    }
    debug_assert_eq!(
        interval_width(leaves, interval.start, interval.end),
        left_width + interval_width(leaves, interval.split + 1, interval.end)
    );
    plan
}

fn best_contiguous_multiway_join_candidate(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<MultiwayJoinCandidate>, PhysicalExecutionError> {
    let mut leaves = Vec::new();
    let mut predicates = Vec::new();
    let Some(_) = flatten_multiway_join_tree(plan, store, &mut leaves, &mut predicates)? else {
        return Ok(None);
    };
    if leaves.len() < 3 || leaves.len() > 32 {
        return Ok(None);
    }

    let planner = MultiwayPlannerContext {
        store,
        leaves: &leaves,
        context,
        registry,
    };
    let count = leaves.len();
    let mut best = BTreeMap::<(usize, usize), MultiwayJoinCandidate>::new();
    for (index, leaf) in leaves.iter().enumerate() {
        best.insert(
            (index, index),
            MultiwayJoinCandidate {
                plan: leaf.plan.clone(),
                cost: 0,
                estimated_rows: leaf.rows,
            },
        );
    }

    for len in 2..=count {
        for start in 0..=(count - len) {
            let end = start + len - 1;
            let mut interval_best: Option<MultiwayJoinCandidate> = None;
            for split in start..end {
                let crossing = crossing_join_predicates(&predicates, start, split, end);
                if crossing.is_empty() {
                    continue;
                }
                let Some(left) = best.get(&(start, split)) else {
                    continue;
                };
                let Some(right) = best.get(&(split + 1, end)) else {
                    continue;
                };
                let interval = MultiwayJoinInterval { start, split, end };
                let (join_work, estimated_rows, primary_index) = multiway_join_merge_estimate(
                    &planner,
                    &crossing,
                    interval,
                    left.estimated_rows,
                    right.estimated_rows,
                )?;
                let candidate = MultiwayJoinCandidate {
                    plan: build_multiway_join_merge(
                        &planner,
                        interval,
                        left.plan.clone(),
                        right.plan.clone(),
                        &crossing,
                        primary_index,
                    ),
                    cost: left
                        .cost
                        .saturating_add(right.cost)
                        .saturating_add(join_work),
                    estimated_rows,
                };
                let replace = interval_best.as_ref().is_none_or(|current| {
                    candidate.cost < current.cost
                        || (candidate.cost == current.cost
                            && candidate.estimated_rows < current.estimated_rows)
                });
                if replace {
                    interval_best = Some(candidate);
                }
            }
            if let Some(candidate) = interval_best {
                best.insert((start, end), candidate);
            }
        }
    }

    Ok(best.remove(&(0, count - 1)))
}

fn optimize_contiguous_multiway_join(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<Plan>, PhysicalExecutionError> {
    let Some(candidate) = best_contiguous_multiway_join_candidate(plan, store, context, registry)?
    else {
        return Ok(None);
    };
    if candidate.plan == *plan {
        Ok(None)
    } else {
        Ok(Some(candidate.plan))
    }
}


