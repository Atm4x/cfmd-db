#[derive(Debug, Clone)]
 struct CyclicPrefixCandidateIndex {
     constraint_indices: Vec<usize>,
    pub(super) buckets: BTreeMap<Vec<kernel_semantics::CanonicalEqKey>, Vec<usize>>,
     max_bucket_len: usize,
}

#[derive(Debug, Clone)]
 struct CyclicPrefixCandidateIndexes {
     by_leaf: Vec<Option<CyclicPrefixCandidateIndex>>,
    build_work_upper_bound: u128,
}

struct OrderPreservingJoinEnumeration<'a> {
    handles: &'a [Vec<PhysicalRowId>],
    constraints: &'a [&'a SemanticQuotientConstraint],
    base_masks: &'a [&'a [u64]],
    search_order: &'a [usize],
    cyclic_prefix_indexes: Option<&'a CyclicPrefixCandidateIndexes>,
}

fn mask_ordinals(mask: &[u64]) -> impl Iterator<Item = usize> + '_ {
    mask.iter().enumerate().flat_map(|(word_index, word)| {
        let mut remaining = *word;
        std::iter::from_fn(move || {
            if remaining == 0 {
                return None;
            }
            let bit = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            Some(word_index * 64 + bit)
        })
    })
}

fn assigned_quotient_key<'a>(
    constraint: &'a SemanticQuotientConstraint,
    current_leaf: usize,
    assignment: &[Option<usize>],
) -> Option<&'a kernel_semantics::CanonicalEqKey> {
    constraint.leaves.iter().find_map(|quotient_leaf| {
        if quotient_leaf.leaf == current_leaf {
            return None;
        }
        let ordinal = assignment[quotient_leaf.leaf]?;
        quotient_leaf.keys[ordinal].as_ref()
    })
}

fn cyclic_prefix_candidate_indexes(
    constraints: &[&SemanticQuotientConstraint],
    base_masks: &[&[u64]],
    search_order: &[usize],
) -> Option<CyclicPrefixCandidateIndexes> {
    if search_order.len() != base_masks.len() {
        return None;
    }
    let mut seen = vec![false; base_masks.len()];
    for leaf in search_order.iter().copied() {
        if leaf >= base_masks.len() || seen[leaf] {
            return None;
        }
        seen[leaf] = true;
    }

    let mut assigned = vec![false; base_masks.len()];
    let mut by_leaf = vec![None; base_masks.len()];
    let mut build_work_upper_bound = 0_u128;
    for leaf in search_order.iter().copied() {
        let constraint_indices = constraints
            .iter()
            .enumerate()
            .filter_map(|(index, constraint)| {
                quotient_leaf(constraint, leaf)?;
                constraint
                    .leaves
                    .iter()
                    .any(|other| other.leaf != leaf && assigned[other.leaf])
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        if !constraint_indices.is_empty() {
            let active_rows = mask_population(base_masks[leaf]);
            let tree_height = u128::from(
                usize::BITS
                    .saturating_sub(active_rows.max(1).leading_zeros())
                    .saturating_add(1),
            );
            build_work_upper_bound = build_work_upper_bound
                .saturating_add(base_masks[leaf].len() as u128)
                .saturating_add(
                    (active_rows as u128)
                        .saturating_mul(constraint_indices.len().saturating_add(1) as u128)
                        .saturating_mul(tree_height),
                );
            if build_work_upper_bound > MAX_BOUNDED_CYCLIC_ENUMERATION_WORK {
                return None;
            }
            let mut buckets = BTreeMap::<Vec<kernel_semantics::CanonicalEqKey>, Vec<usize>>::new();
            for ordinal in mask_ordinals(base_masks[leaf]) {
                let mut signature = Vec::with_capacity(constraint_indices.len());
                for constraint_index in &constraint_indices {
                    let current = quotient_leaf(constraints[*constraint_index], leaf)?;
                    signature.push(current.keys.get(ordinal)?.as_ref()?.clone());
                }
                buckets.entry(signature).or_default().push(ordinal);
            }
            let max_bucket_len = buckets.values().map(Vec::len).max().unwrap_or(0);
            by_leaf[leaf] = Some(CyclicPrefixCandidateIndex {
                constraint_indices,
                buckets,
                max_bucket_len,
            });
        }
        assigned[leaf] = true;
    }
    Some(CyclicPrefixCandidateIndexes {
        by_leaf,
        build_work_upper_bound,
    })
}

fn cyclic_prefix_signature(
    index: &CyclicPrefixCandidateIndex,
    constraints: &[&SemanticQuotientConstraint],
    leaf: usize,
    assignment: &[Option<usize>],
) -> Option<Vec<kernel_semantics::CanonicalEqKey>> {
    index
        .constraint_indices
        .iter()
        .map(|constraint_index| {
            assigned_quotient_key(constraints[*constraint_index], leaf, assignment).cloned()
        })
        .collect()
}

impl OrderPreservingJoinEnumeration<'_> {
    fn enumerate_candidate(
        &self,
        depth: usize,
        leaf_index: usize,
        ordinal: usize,
        assignment: &mut [Option<usize>],
        output: &mut Vec<Vec<usize>>,
        stats: &mut ExecutionStats,
    ) -> Result<(), PhysicalExecutionError> {
        if ordinal >= self.handles[leaf_index].len() {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
        stats.multiway_join_semantic_quotient_candidate_visits = stats
            .multiway_join_semantic_quotient_candidate_visits
            .saturating_add(1);
        let mut viable = true;
        for constraint in self.constraints {
            let Some(current_leaf) = quotient_leaf(constraint, leaf_index) else {
                continue;
            };
            let Some(key) = current_leaf.keys[ordinal].as_ref() else {
                viable = false;
                break;
            };
            if let Some(expected) = assigned_quotient_key(constraint, leaf_index, assignment)
                && expected != key
            {
                viable = false;
                break;
            }
            for later in constraint
                .leaves
                .iter()
                .filter(|later| later.leaf != leaf_index && assignment[later.leaf].is_none())
            {
                let Some(mask) = later.buckets.get(key) else {
                    viable = false;
                    break;
                };
                if !masks_intersect(mask, self.base_masks[later.leaf]) {
                    viable = false;
                    break;
                }
            }
            if !viable {
                break;
            }
        }
        if !viable {
            return Ok(());
        }
        assignment[leaf_index] = Some(ordinal);
        self.enumerate(depth + 1, assignment, output, stats)?;
        assignment[leaf_index] = None;
        Ok(())
    }

    fn enumerate(
        &self,
        depth: usize,
        assignment: &mut [Option<usize>],
        output: &mut Vec<Vec<usize>>,
        stats: &mut ExecutionStats,
    ) -> Result<(), PhysicalExecutionError> {
        if depth == self.search_order.len() {
            output.push(
                assignment
                    .iter()
                    .map(|ordinal| {
                        ordinal.ok_or(RelQueryError::InconsistentIncrementalDelta.into())
                    })
                    .collect::<Result<Vec<_>, PhysicalExecutionError>>()?,
            );
            return Ok(());
        }
        let leaf_index = *self
            .search_order
            .get(depth)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;

        if let Some(index) = self
            .cyclic_prefix_indexes
            .and_then(|indexes| indexes.by_leaf.get(leaf_index))
            .and_then(Option::as_ref)
        {
            let signature =
                cyclic_prefix_signature(index, self.constraints, leaf_index, assignment)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            stats.multiway_join_cyclic_prefix_index_lookups = stats
                .multiway_join_cyclic_prefix_index_lookups
                .saturating_add(1);
            if let Some(ordinals) = index.buckets.get(&signature) {
                for ordinal in ordinals.iter().copied() {
                    self.enumerate_candidate(
                        depth, leaf_index, ordinal, assignment, output, stats,
                    )?;
                }
            }
            return Ok(());
        }

        let mut candidates = self.base_masks[leaf_index].to_vec();
        for constraint in self.constraints {
            let Some(current_leaf) = quotient_leaf(constraint, leaf_index) else {
                continue;
            };
            let Some(expected) = assigned_quotient_key(constraint, leaf_index, assignment) else {
                continue;
            };
            let Some(mask) = current_leaf.buckets.get(expected) else {
                return Ok(());
            };
            mask_intersect_in_place(&mut candidates, mask);
        }

        for ordinal in mask_ordinals(&candidates) {
            self.enumerate_candidate(depth, leaf_index, ordinal, assignment, output, stats)?;
        }
        Ok(())
    }
}

fn materialize_join_assignments(
    assignments: &mut [Vec<usize>],
    leaves: &[MultiwayJoinLeaf],
    predicates: &[MultiwayJoinPredicate],
    handles: &[Vec<PhysicalRowId>],
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    assignments.sort_unstable();
    let mut output = Vec::with_capacity(assignments.len());
    for assignment in assignments {
        let mut rows = Vec::with_capacity(leaves.len());
        for (leaf, ordinal) in assignment.iter().copied().enumerate() {
            rows.push(leaf_row_by_handle(
                store,
                &leaves[leaf],
                handles[leaf][ordinal],
            )?);
        }
        for predicate in predicates {
            let left = rows[predicate.left.leaf]
                .get(predicate.left.column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let right = rows[predicate.right.leaf]
                .get(predicate.right.column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            if !registry.equivalent(context, predicate.equivalence, left, right)? {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
        }
        let width = rows.iter().map(Vec::len).sum();
        let mut materialized = Vec::with_capacity(width);
        for row in rows {
            materialized.extend(row);
        }
        output.push(materialized);
    }
    Ok(output)
}



fn leaf_row_by_handle(
    store: &dyn SemanticQuotientStoreView,
    leaf: &MultiwayJoinLeaf,
    row_id: PhysicalRowId,
) -> Result<kernel_query::Row, PhysicalExecutionError> {
    store.semantic_quotient_row(leaf.relation, leaf.layout, row_id)
}
