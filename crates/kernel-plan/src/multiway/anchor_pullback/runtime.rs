#[derive(Debug)]
struct AnchorPullbackLeafRuntime {
    coordinates: Vec<RevisionObservableId>,
    tuples: Vec<Vec<EqClassId>>,
    rows_by_tuple: BTreeMap<Vec<EqClassId>, Vec<(usize, kernel_query::Row)>>,
    coordinate_indexes: Vec<BTreeMap<EqClassId, Vec<usize>>>,
}

impl AnchorPullbackLeafRuntime {
    fn candidate_tuple_indices(
        &self,
        assignments: &BTreeMap<RevisionObservableId, EqClassId>,
    ) -> Vec<usize> {
        let mut seed: Option<&Vec<usize>> = None;
        for (position, coordinate) in self.coordinates.iter().enumerate() {
            let Some(class) = assignments.get(coordinate) else {
                continue;
            };
            let Some(indices) = self.coordinate_indexes[position].get(class) else {
                return Vec::new();
            };
            if seed.is_none_or(|current| indices.len() < current.len()) {
                seed = Some(indices);
            }
        }
        seed.cloned()
            .unwrap_or_else(|| (0..self.tuples.len()).collect())
    }

    fn tuple_compatible(
        &self,
        tuple_index: usize,
        assignments: &BTreeMap<RevisionObservableId, EqClassId>,
    ) -> bool {
        self.coordinates
            .iter()
            .zip(&self.tuples[tuple_index])
            .all(|(coordinate, class)| {
                assignments
                    .get(coordinate)
                    .is_none_or(|assigned| assigned == class)
            })
    }
}

struct AnchorPullbackExecution<'a> {
    factors: &'a [AnchorPullbackLeafRuntime],
    search_order: &'a [usize],
    stats: &'a mut ExecutionStats,
    assignments: BTreeMap<RevisionObservableId, EqClassId>,
    selected: Vec<Option<usize>>,
    matches: Vec<(Vec<usize>, kernel_query::Row)>,
}

impl AnchorPullbackExecution<'_> {
    fn run(mut self) -> Vec<(Vec<usize>, kernel_query::Row)> {
        self.search(0);
        self.matches
    }

    fn search(&mut self, depth: usize) {
        if depth == self.search_order.len() {
            self.expand_selected_rows(0, &mut Vec::new(), &mut Vec::new());
            return;
        }
        let leaf = self.search_order[depth];
        let candidates = self.factors[leaf].candidate_tuple_indices(&self.assignments);
        for tuple_index in candidates {
            self.stats.multiway_join_apnf_candidate_visits = self
                .stats
                .multiway_join_apnf_candidate_visits
                .saturating_add(1);
            if !self.factors[leaf].tuple_compatible(tuple_index, &self.assignments) {
                continue;
            }
            let mut inserted = Vec::new();
            for (&coordinate, &class) in self.factors[leaf]
                .coordinates
                .iter()
                .zip(&self.factors[leaf].tuples[tuple_index])
            {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    self.assignments.entry(coordinate)
                {
                    entry.insert(class);
                    inserted.push(coordinate);
                }
            }
            self.selected[leaf] = Some(tuple_index);
            self.search(depth + 1);
            self.selected[leaf] = None;
            for coordinate in inserted {
                self.assignments.remove(&coordinate);
            }
        }
    }

    fn expand_selected_rows(
        &mut self,
        leaf: usize,
        ordinals: &mut Vec<usize>,
        row: &mut kernel_query::Row,
    ) {
        if leaf == self.factors.len() {
            self.matches.push((ordinals.clone(), row.clone()));
            return;
        }
        let tuple_index = self.selected[leaf].expect("all APNF factors are selected");
        let tuple = &self.factors[leaf].tuples[tuple_index];
        let bucket = self.factors[leaf]
            .rows_by_tuple
            .get(tuple)
            .expect("APNF reconstructed tuple has a physical row bucket");
        for (ordinal, physical_row) in bucket {
            ordinals.push(*ordinal);
            let original_len = row.len();
            row.extend(physical_row.iter().cloned());
            self.expand_selected_rows(leaf + 1, ordinals, row);
            row.truncate(original_len);
            ordinals.pop();
        }
    }
}

type AnchorPullbackRowBuckets = BTreeMap<Vec<EqClassId>, Vec<(usize, kernel_query::Row)>>;

struct AnchorPullbackFactorBuild {
    measure: kernel_semantics::anchor_pullback::RevisionFiniteMeasure,
    rows_by_tuple: AnchorPullbackRowBuckets,
}

struct AnchorPullbackFactorBuilder<'a> {
    predicates: &'a [MultiwayJoinPredicate],
    predicate_observables: &'a [RevisionObservableId],
    catalog: &'a mut kernel_semantics::observable::RevisionObservableCatalog,
    store: &'a PhysicalStore,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
    stats: &'a mut ExecutionStats,
}

impl AnchorPullbackFactorBuilder<'_> {
    fn build(
        &mut self,
        leaf_index: usize,
        leaf: &MultiwayJoinLeaf,
    ) -> Result<AnchorPullbackFactorBuild, PhysicalExecutionError> {
        let incidence = self.incidence(leaf_index);
        let coordinates = incidence
            .keys()
            .map(|index| self.predicate_observables[*index])
            .collect::<Vec<_>>();
        let handles = self.store.logical_row_handles(leaf.relation, leaf.layout)?;
        let installed = self.store.installed(leaf.relation, leaf.layout)?;
        let mut rows_by_tuple = AnchorPullbackRowBuckets::new();
        for (ordinal, handle) in handles.iter().enumerate() {
            let position = installed
                .position(*handle)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let row = materialize_native_row(&installed.data, position)?;
            self.stats.scanned_rows = self.stats.scanned_rows.saturating_add(1);
            if let Some(tuple) = self.observe_tuple(&row, &incidence)? {
                rows_by_tuple.entry(tuple).or_default().push((ordinal, row));
            }
        }
        let measure_rows = rows_by_tuple
            .iter()
            .map(|(tuple, rows)| {
                u64::try_from(rows.len())
                    .map(|weight| (tuple.clone(), weight))
                    .map_err(|_| PhysicalExecutionError::AnchorPullbackInvariant)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let measure = kernel_semantics::anchor_pullback::RevisionFiniteMeasure::new(
            self.catalog,
            coordinates,
            measure_rows,
        )
        .map_err(|_| PhysicalExecutionError::AnchorPullbackInvariant)?;
        Ok(AnchorPullbackFactorBuild {
            measure,
            rows_by_tuple,
        })
    }

    fn incidence(&self, leaf_index: usize) -> BTreeMap<usize, Vec<usize>> {
        let mut incidence = BTreeMap::<usize, Vec<usize>>::new();
        for (predicate_index, predicate) in self.predicates.iter().enumerate() {
            if predicate.left.leaf == leaf_index {
                incidence
                    .entry(predicate_index)
                    .or_default()
                    .push(predicate.left.column);
            }
            if predicate.right.leaf == leaf_index {
                incidence
                    .entry(predicate_index)
                    .or_default()
                    .push(predicate.right.column);
            }
        }
        incidence
    }

    fn observe_tuple(
        &mut self,
        row: &kernel_query::Row,
        incidence: &BTreeMap<usize, Vec<usize>>,
    ) -> Result<Option<Vec<EqClassId>>, PhysicalExecutionError> {
        let mut tuple = Vec::with_capacity(incidence.len());
        for (&predicate_index, columns) in incidence {
            let observable = self.predicate_observables[predicate_index];
            let mut expected = None;
            for column in columns {
                let value = row.get(*column).ok_or(RelQueryError::ColumnOutOfBounds)?;
                self.stats.values_read = self.stats.values_read.saturating_add(1);
                let class = self
                    .catalog
                    .observe_value(self.registry, self.context, observable, value)
                    .map_err(|_| PhysicalExecutionError::AnchorPullbackInvariant)?;
                if expected.is_some_and(|candidate| candidate != class) {
                    return Ok(None);
                }
                expected = Some(class);
            }
            tuple.push(expected.ok_or(PhysicalExecutionError::AnchorPullbackInvariant)?);
        }
        Ok(Some(tuple))
    }
}

fn register_anchor_pullback_observables(
    predicates: &[MultiwayJoinPredicate],
    catalog: &mut kernel_semantics::observable::RevisionObservableCatalog,
    registry: &kernel_semantics::SemanticRegistry,
    context: &kernel_schema::SemanticContext,
) -> Result<Vec<RevisionObservableId>, PhysicalExecutionError> {
    predicates
        .iter()
        .enumerate()
        .map(|(index, predicate)| {
            let coordinate = u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(PhysicalExecutionError::AnchorPullbackInvariant)?;
            catalog
                .register_equivalence_coordinate(
                    registry,
                    context,
                    predicate.equivalence,
                    coordinate,
                )
                .map_err(|_| PhysicalExecutionError::AnchorPullbackInvariant)
        })
        .collect()
}

fn anchor_pullback_runtime_factors(
    normal_form: &kernel_semantics::anchor_pullback::AnchorPullbackNormalForm,
    row_buckets: Vec<AnchorPullbackRowBuckets>,
) -> Result<Vec<AnchorPullbackLeafRuntime>, PhysicalExecutionError> {
    normal_form
        .anchors()
        .iter()
        .zip(row_buckets)
        .map(|(anchor, rows_by_tuple)| {
            let reconstructed = anchor.reconstructed_measure();
            for (tuple, weight) in &reconstructed {
                let actual = rows_by_tuple.get(tuple).map_or(0, Vec::len);
                if u64::try_from(actual).ok() != Some(*weight) {
                    return Err(PhysicalExecutionError::AnchorPullbackInvariant);
                }
            }
            let tuples = reconstructed.keys().cloned().collect::<Vec<_>>();
            let mut coordinate_indexes = anchor
                .coordinates()
                .iter()
                .map(|_| BTreeMap::<EqClassId, Vec<usize>>::new())
                .collect::<Vec<_>>();
            for (tuple_index, tuple) in tuples.iter().enumerate() {
                for (position, class) in tuple.iter().copied().enumerate() {
                    coordinate_indexes[position]
                        .entry(class)
                        .or_default()
                        .push(tuple_index);
                }
            }
            Ok(AnchorPullbackLeafRuntime {
                coordinates: anchor.coordinates().to_vec(),
                tuples,
                rows_by_tuple,
                coordinate_indexes,
            })
        })
        .collect()
}

fn anchor_pullback_search_order(
    normal_form: &kernel_semantics::anchor_pullback::AnchorPullbackNormalForm,
    factors: &[AnchorPullbackLeafRuntime],
) -> Result<(Vec<usize>, bool), PhysicalExecutionError> {
    let mut branch_free = Vec::new();
    for index in 0..factors.len() {
        if normal_form
            .branch_free_from_anchor(index)
            .map_err(|_| PhysicalExecutionError::AnchorPullbackInvariant)?
            .is_some()
        {
            branch_free.push(index);
        }
    }
    let mut search_order = (0..factors.len()).collect::<Vec<_>>();
    search_order.sort_by_key(|index| {
        (
            !branch_free.contains(index),
            factors[*index].tuples.len(),
            *index,
        )
    });
    Ok((search_order, !branch_free.is_empty()))
}

