fn semantic_quotient_union_find_root(parent: &mut [usize], mut vertex: usize) -> usize {
    let mut root = vertex;
    while parent[root] != root {
        root = parent[root];
    }
    while parent[vertex] != vertex {
        let next = parent[vertex];
        parent[vertex] = root;
        vertex = next;
    }
    root
}

fn quotient_component_endpoints(
    predicates: &[MultiwayJoinPredicate],
    target: SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<Vec<MultiwayJoinColumnRef>>, PhysicalExecutionError> {
    let mut vertex_index = BTreeMap::<MultiwayJoinColumnRef, usize>::new();
    let mut vertices = Vec::<MultiwayJoinColumnRef>::new();
    let mut edges = Vec::<(usize, usize)>::new();
    for predicate in predicates {
        if !registry.equivalence_refines(context, predicate.equivalence, target)? {
            continue;
        }
        let left = *vertex_index.entry(predicate.left).or_insert_with(|| {
            let index = vertices.len();
            vertices.push(predicate.left);
            index
        });
        let right = *vertex_index.entry(predicate.right).or_insert_with(|| {
            let index = vertices.len();
            vertices.push(predicate.right);
            index
        });
        edges.push((left, right));
    }

    let mut parent = (0..vertices.len()).collect::<Vec<_>>();
    let mut rank = vec![0_u8; vertices.len()];
    for (left, right) in edges {
        let left_root = semantic_quotient_union_find_root(&mut parent, left);
        let right_root = semantic_quotient_union_find_root(&mut parent, right);
        if left_root == right_root {
            continue;
        }
        match rank[left_root].cmp(&rank[right_root]) {
            std::cmp::Ordering::Less => parent[left_root] = right_root,
            std::cmp::Ordering::Greater => parent[right_root] = left_root,
            std::cmp::Ordering::Equal => {
                parent[right_root] = left_root;
                rank[left_root] = rank[left_root].saturating_add(1);
            }
        }
    }

    let mut components = BTreeMap::<usize, BTreeSet<MultiwayJoinColumnRef>>::new();
    for (index, vertex) in vertices.into_iter().enumerate() {
        let root = semantic_quotient_union_find_root(&mut parent, index);
        components.entry(root).or_default().insert(vertex);
    }
    Ok(components
        .into_values()
        .filter(|component| component.len() > 1)
        .map(|component| component.into_iter().collect())
        .collect())
}

fn semantic_quotient_specs(
    predicates: &[MultiwayJoinPredicate],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<(SemanticId, Vec<MultiwayJoinColumnRef>)>, PhysicalExecutionError> {
    let equivalences = predicates
        .iter()
        .map(|predicate| predicate.equivalence)
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();
    for equivalence in equivalences {
        for endpoints in quotient_component_endpoints(predicates, equivalence, context, registry)? {
            candidates.push((equivalence, endpoints));
        }
    }

    let mut keep = vec![true; candidates.len()];
    let mut by_component = BTreeMap::<Vec<MultiwayJoinColumnRef>, Vec<usize>>::new();
    for (index, (_, endpoints)) in candidates.iter().enumerate() {
        by_component
            .entry(endpoints.clone())
            .or_default()
            .push(index);
    }
    for indices in by_component.values() {
        for &i in indices {
            for &j in indices {
                if i == j {
                    continue;
                }
                let j_refines_i =
                    registry.equivalence_refines(context, candidates[j].0, candidates[i].0)?;
                let i_refines_j =
                    registry.equivalence_refines(context, candidates[i].0, candidates[j].0)?;
                if j_refines_i && (!i_refines_j || j < i) {
                    keep[i] = false;
                    break;
                }
            }
        }
    }
    Ok(candidates
        .into_iter()
        .zip(keep)
        .filter_map(|(candidate, keep)| keep.then_some(candidate))
        .collect())
}

fn quotient_maximal_hyperedges(
    edges: Vec<BTreeSet<usize>>,
    leaf_count: usize,
) -> Vec<BTreeSet<usize>> {
    let edges = edges
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if edges.len() < 2 {
        return edges;
    }
    let words = edges.len().div_ceil(64);
    let mut incidence = vec![vec![0_u64; words]; leaf_count];
    for (edge_index, edge) in edges.iter().enumerate() {
        for &leaf in edge {
            if leaf < leaf_count {
                incidence[leaf][edge_index / 64] |= 1_u64 << (edge_index % 64);
            }
        }
    }
    let last_mask = if edges.len() % 64 == 0 {
        u64::MAX
    } else {
        (1_u64 << (edges.len() % 64)) - 1
    };
    let mut keep = vec![true; edges.len()];
    for (edge_index, edge) in edges.iter().enumerate() {
        let mut supersets = vec![u64::MAX; words];
        if let Some(last) = supersets.last_mut() {
            *last &= last_mask;
        }
        for &leaf in edge {
            let Some(leaf_incidence) = incidence.get(leaf) else {
                supersets.fill(0);
                break;
            };
            for (candidate, contains_leaf) in supersets.iter_mut().zip(leaf_incidence) {
                *candidate &= *contains_leaf;
            }
        }
        supersets[edge_index / 64] &= !(1_u64 << (edge_index % 64));
        if supersets.iter().any(|word| *word != 0) {
            keep[edge_index] = false;
        }
    }
    edges
        .into_iter()
        .zip(keep)
        .filter_map(|(edge, keep)| keep.then_some(edge))
        .collect()
}

fn quotient_hypergraph_search_order(
    leaf_count: usize,
    specs: &[(SemanticId, Vec<MultiwayJoinColumnRef>)],
) -> Option<Vec<usize>> {
    if leaf_count < 3 {
        return None;
    }
    let mut edges = specs
        .iter()
        .map(|(_, endpoints)| {
            endpoints
                .iter()
                .map(|endpoint| endpoint.leaf)
                .collect::<BTreeSet<_>>()
        })
        .filter(|edge| edge.len() > 1)
        .collect::<Vec<_>>();
    let covered = edges
        .iter()
        .flat_map(|edge| edge.iter().copied())
        .collect::<BTreeSet<_>>();
    if covered.len() != leaf_count || covered.iter().copied().ne(0..leaf_count) {
        return None;
    }

    let mut remaining = (0..leaf_count).collect::<BTreeSet<_>>();
    let mut eliminated = Vec::with_capacity(leaf_count);
    while !remaining.is_empty() {
        edges = quotient_maximal_hyperedges(edges, leaf_count);

        let mut degree = vec![0_usize; leaf_count];
        for edge in &edges {
            for leaf in edge {
                degree[*leaf] = degree[*leaf].saturating_add(1);
            }
        }
        let leaf = remaining
            .iter()
            .copied()
            .filter(|leaf| degree[*leaf] <= 1)
            .min()?;
        eliminated.push(leaf);
        remaining.remove(&leaf);
        for edge in &mut edges {
            edge.remove(&leaf);
        }
        edges.retain(|edge| !edge.is_empty());
    }
    eliminated.reverse();
    Some(eliminated)
}

fn quotient_bitset_members(words: &[u64]) -> impl Iterator<Item = usize> + '_ {
    words.iter().enumerate().flat_map(|(word_index, word)| {
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

fn quotient_hypergraph_cyclic_search_order(
    leaf_count: usize,
    specs: &[(SemanticId, Vec<MultiwayJoinColumnRef>)],
) -> Option<Vec<usize>> {
    if leaf_count < 3 {
        return None;
    }
    let edges = specs
        .iter()
        .map(|(_, endpoints)| {
            endpoints
                .iter()
                .map(|endpoint| endpoint.leaf)
                .collect::<BTreeSet<_>>()
        })
        .filter(|edge| edge.len() > 1)
        .collect::<Vec<_>>();
    let covered = edges
        .iter()
        .flat_map(|edge| edge.iter().copied())
        .collect::<BTreeSet<_>>();
    if covered.len() != leaf_count || covered.iter().copied().ne(0..leaf_count) {
        return None;
    }

    let words = leaf_count.div_ceil(64);
    let mut adjacency = vec![vec![0_u64; words]; leaf_count];
    for edge in edges {
        let vertices = edge.into_iter().collect::<Vec<_>>();
        for &left in &vertices {
            for &right in &vertices {
                if left != right {
                    adjacency[left][right / 64] |= 1_u64 << (right % 64);
                }
            }
        }
    }

    let mut remaining = vec![u64::MAX; words];
    if let Some(last) = remaining.last_mut()
        && !leaf_count.is_multiple_of(64)
    {
        *last = (1_u64 << (leaf_count % 64)) - 1;
    }
    let mut eliminated = Vec::with_capacity(leaf_count);
    while eliminated.len() < leaf_count {
        let mut best = None::<(usize, usize, usize, Vec<u64>)>;
        for leaf in quotient_bitset_members(&remaining) {
            if leaf >= leaf_count {
                continue;
            }
            let mut neighbors = adjacency[leaf]
                .iter()
                .zip(&remaining)
                .map(|(adjacent, active)| adjacent & active)
                .collect::<Vec<_>>();
            neighbors[leaf / 64] &= !(1_u64 << (leaf % 64));
            let degree = neighbors
                .iter()
                .map(|word| word.count_ones() as usize)
                .sum::<usize>();
            let mut twice_existing = 0_usize;
            for neighbor in
                quotient_bitset_members(&neighbors).filter(|neighbor| *neighbor < leaf_count)
            {
                twice_existing = twice_existing.saturating_add(
                    adjacency[neighbor]
                        .iter()
                        .zip(&neighbors)
                        .map(|(adjacent, candidates)| (adjacent & candidates).count_ones() as usize)
                        .sum::<usize>(),
                );
            }
            let possible = degree.saturating_mul(degree.saturating_sub(1)) / 2;
            let existing = twice_existing / 2;
            let fill = possible.saturating_sub(existing);
            let candidate = (fill, degree, leaf, neighbors);
            if best
                .as_ref()
                .is_none_or(|current| (fill, degree, leaf) < (current.0, current.1, current.2))
            {
                best = Some(candidate);
            }
        }
        let (_, _, leaf, neighbors) = best?;
        for neighbor in quotient_bitset_members(&neighbors)
            .filter(|neighbor| *neighbor < leaf_count)
            .collect::<Vec<_>>()
        {
            for (word, clique) in adjacency[neighbor].iter_mut().zip(&neighbors) {
                *word |= *clique;
            }
            adjacency[neighbor][neighbor / 64] &= !(1_u64 << (neighbor % 64));
        }
        remaining[leaf / 64] &= !(1_u64 << (leaf % 64));
        eliminated.push(leaf);
    }
    eliminated.reverse();
    Some(eliminated)
}

fn flatten_multiway_join_shape(
    plan: &Plan,
    context: &kernel_schema::SemanticContext,
    leaf_count: &mut usize,
    predicates: &mut Vec<MultiwayJoinPredicate>,
) -> Result<Option<Vec<MultiwayJoinColumnRef>>, RelQueryError> {
    match plan {
        Plan::Scan { relation, .. } => {
            let definition = context
                .schema
                .relation(*relation)
                .ok_or(RelQueryError::UnknownRelation(*relation))?;
            let leaf = *leaf_count;
            *leaf_count = leaf.saturating_add(1);
            Ok(Some(
                (0..definition.columns.len())
                    .map(|column| MultiwayJoinColumnRef { leaf, column })
                    .collect(),
            ))
        }
        Plan::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
            ..
        } => {
            let Some(mut left_columns) =
                flatten_multiway_join_shape(left, context, leaf_count, predicates)?
            else {
                return Ok(None);
            };
            let Some(right_columns) =
                flatten_multiway_join_shape(right, context, leaf_count, predicates)?
            else {
                return Ok(None);
            };
            let left_ref = left_columns
                .get(*left_column)
                .copied()
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let right_ref = right_columns
                .get(*right_column)
                .copied()
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            predicates.push(MultiwayJoinPredicate {
                left: left_ref,
                right: right_ref,
                equivalence: *equivalence,
            });
            left_columns.extend(right_columns);
            Ok(Some(left_columns))
        }
        Plan::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => {
            let Some(columns) =
                flatten_multiway_join_shape(input, context, leaf_count, predicates)?
            else {
                return Ok(None);
            };
            let left_ref = columns
                .get(*left_column)
                .copied()
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let right_ref = columns
                .get(*right_column)
                .copied()
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            if left_ref.leaf == right_ref.leaf {
                return Ok(None);
            }
            predicates.push(MultiwayJoinPredicate {
                left: left_ref,
                right: right_ref,
                equivalence: *equivalence,
            });
            Ok(Some(columns))
        }
        Plan::Project { input, columns } => {
            let Some(input_columns) =
                flatten_multiway_join_shape(input, context, leaf_count, predicates)?
            else {
                return Ok(None);
            };
            if columns.len() != input_columns.len()
                || columns.iter().copied().ne(0..input_columns.len())
            {
                return Ok(None);
            }
            Ok(Some(input_columns))
        }
        Plan::PromoteToBag(input) => {
            flatten_multiway_join_shape(input, context, leaf_count, predicates)
        }
        _ => Ok(None),
    }
}

