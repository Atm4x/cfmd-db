impl PreparedOrderedView<'_> {
    fn run_key(
        &self,
        row: &kernel_query::Row,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<
        (
            kernel_semantics::CanonicalOrderClassKey,
            Vec<kernel_semantics::CanonicalEqKey>,
        ),
        OrderedViewError,
    > {
        let order_value = row
            .get(self.spec.column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let order_class = registry.canonical_order_key(
            &self.plan.semantic_context,
            self.spec.ordering,
            order_value,
        )?;
        let row_key = row
            .iter()
            .zip(&self.equivalences)
            .map(|(value, equivalence)| {
                registry.canonical_equivalence_key(&self.plan.semantic_context, *equivalence, value)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((order_class, row_key))
    }

    // HOSTILE[P166][ACTIVE][CLEAN:P164.O]: the initial snapshot canonicalizes the current
    // maintained output once into an ordered map. Later revisions advance this exact artifact
    // from PreparedRuntimeRevisionTransition::output_deltas instead of rebuilding/sorting it.
    fn snapshot_from_value(
        &self,
        revision: RevisionId,
        value: RelationValue,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<OrderedViewSnapshot, OrderedViewError> {
        let mut runs = BTreeMap::<
            kernel_semantics::CanonicalOrderClassKey,
            BTreeMap<Vec<kernel_semantics::CanonicalEqKey>, OrderedViewRun>,
        >::new();
        for row in value.into_rows() {
            let (order_class, row_key) = self.run_key(&row, registry)?;
            let entry = runs
                .entry(order_class)
                .or_default()
                .entry(row_key)
                .or_insert_with(|| OrderedViewRun {
                    row: row.clone(),
                    count: 0,
                });
            entry.count = entry
                .count
                .checked_add(1)
                .ok_or(RelQueryError::DerivedIdentityExhausted)?;
        }
        Ok(OrderedViewSnapshot {
            revision,
            semantic_revision: self.plan.semantic_context.revision(),
            semantic_context: self.plan.semantic_context.clone(),
            logical: self.plan.logical().clone(),
            result_type: self.plan.result_type.clone(),
            equivalences: self.equivalences.clone(),
            spec: self.spec.clone(),
            runs,
        })
    }

    pub fn snapshot_native_pinned(
        &self,
        store: &PhysicalStore,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<OrderedViewSnapshot, OrderedViewError> {
        let revision = store
            .revision()
            .ok_or(OrderedViewError::UnboundPhysicalRevision)?;
        let (value, _) = self.plan.execute_native_pinned(store, registry)?;
        self.snapshot_from_value(revision, value, registry)
    }

    // HOSTILE[P164][ACTIVE][CLEAN:P163.M]: runtime-owned maintained output is the authority
    // for the initial ordered snapshot of a registered materialization.
    pub fn snapshot_materialized_pinned(
        &self,
        runtime: &RuntimeRevisionBundle,
        materialization: kernel_types::MaterializationId,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<OrderedViewSnapshot, OrderedViewError> {
        let maintained = runtime
            .materialization(materialization)
            .ok_or(OrderedViewError::UnknownMaterialization(materialization))?;
        if maintained.query() != self.plan.logical() {
            return Err(OrderedViewError::MaterializationQueryMismatch);
        }
        let value = maintained.output_value(&self.plan.semantic_context, registry)?;
        self.snapshot_from_value(runtime.revision_id(), value, registry)
    }
}

impl OrderedViewSnapshot {
    fn run_key_for_row(
        &self,
        row: &kernel_query::Row,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<
        (
            kernel_semantics::CanonicalOrderClassKey,
            Vec<kernel_semantics::CanonicalEqKey>,
        ),
        OrderedViewError,
    > {
        let order_value = row
            .get(self.spec.column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let order_class = registry.canonical_order_key(
            &self.semantic_context,
            self.spec.ordering,
            order_value,
        )?;
        let row_key = row
            .iter()
            .zip(&self.equivalences)
            .map(|(value, equivalence)| {
                registry.canonical_equivalence_key(&self.semantic_context, *equivalence, value)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((order_class, row_key))
    }

    pub(super) fn advance_with_delta(
        &self,
        target_revision: RevisionId,
        delta: Option<&RelationDelta>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, OrderedViewError> {
        let Some(delta) = delta else {
            let mut next = self.clone();
            next.revision = target_revision;
            return Ok(next);
        };
        if delta.result_type != self.result_type {
            return Err(OrderedViewError::SnapshotDeltaTypeMismatch);
        }
        let mut runs = self.runs.clone();
        for row in &delta.removed {
            let (order_class, row_key) = self.run_key_for_row(row, registry)?;
            let remove_group = {
                let group = runs
                    .get_mut(&order_class)
                    .ok_or(OrderedViewError::SnapshotMultiplicityUnderflow)?;
                let remove_run = {
                    let run = group
                        .get_mut(&row_key)
                        .ok_or(OrderedViewError::SnapshotMultiplicityUnderflow)?;
                    if run.count == 0 {
                        return Err(OrderedViewError::SnapshotMultiplicityUnderflow);
                    }
                    run.count -= 1;
                    run.count == 0
                };
                if remove_run {
                    group.remove(&row_key);
                }
                group.is_empty()
            };
            if remove_group {
                runs.remove(&order_class);
            }
        }
        for row in &delta.inserted {
            let (order_class, row_key) = self.run_key_for_row(row, registry)?;
            let run = runs
                .entry(order_class)
                .or_default()
                .entry(row_key)
                .or_insert_with(|| OrderedViewRun {
                    row: row.clone(),
                    count: 0,
                });
            run.count = run
                .count
                .checked_add(1)
                .ok_or(RelQueryError::DerivedIdentityExhausted)?;
        }
        let mut next = self.clone();
        next.revision = target_revision;
        next.runs = runs;
        Ok(next)
    }

    fn validate_cursor(&self, cursor: &OrderedViewCursor) -> Result<(), OrderedViewError> {
        if cursor.revision != self.revision
            || cursor.semantic_revision != self.semantic_revision
            || cursor.semantic_context != self.semantic_context
            || cursor.logical != self.logical
            || cursor.column != self.spec.column
            || cursor.ordering != self.spec.ordering
            || cursor.direction != self.spec.direction
        {
            return Err(OrderedViewError::CursorBindingMismatch);
        }
        Ok(())
    }

    fn cursor(
        &self,
        order_class: &kernel_semantics::CanonicalOrderClassKey,
        row_key: &[kernel_semantics::CanonicalEqKey],
        occurrence: u64,
    ) -> OrderedViewCursor {
        OrderedViewCursor {
            revision: self.revision,
            semantic_revision: self.semantic_revision,
            semantic_context: self.semantic_context.clone(),
            logical: self.logical.clone(),
            column: self.spec.column,
            ordering: self.spec.ordering,
            direction: self.spec.direction,
            order_class: order_class.clone(),
            row_key: row_key.to_vec(),
            occurrence,
        }
    }

    fn append_run(
        selected: &mut Vec<(
            kernel_query::Row,
            kernel_semantics::CanonicalOrderClassKey,
            Vec<kernel_semantics::CanonicalEqKey>,
            u64,
        )>,
        order_class: &kernel_semantics::CanonicalOrderClassKey,
        row_key: &[kernel_semantics::CanonicalEqKey],
        run: &OrderedViewRun,
        start: u64,
        limit: usize,
    ) {
        let mut occurrence = start;
        while occurrence < run.count && selected.len() <= limit {
            selected.push((
                run.row.clone(),
                order_class.clone(),
                row_key.to_vec(),
                occurrence,
            ));
            occurrence = occurrence.saturating_add(1);
        }
    }

    fn append_group(
        selected: &mut Vec<(
            kernel_query::Row,
            kernel_semantics::CanonicalOrderClassKey,
            Vec<kernel_semantics::CanonicalEqKey>,
            u64,
        )>,
        order_class: &kernel_semantics::CanonicalOrderClassKey,
        group: &BTreeMap<Vec<kernel_semantics::CanonicalEqKey>, OrderedViewRun>,
        limit: usize,
    ) {
        for (row_key, run) in group {
            if selected.len() > limit {
                break;
            }
            Self::append_run(selected, order_class, row_key, run, 0, limit);
        }
    }

    pub fn page(
        &self,
        after: Option<&OrderedViewCursor>,
        limit: usize,
    ) -> Result<OrderedViewPage, OrderedViewError> {
        use std::ops::Bound::{Excluded, Unbounded};

        if limit == 0 {
            return Err(OrderedViewError::ZeroPageSize);
        }
        let mut selected = Vec::with_capacity(limit.saturating_add(1));
        let mut after_order_class = None;
        if let Some(cursor) = after {
            self.validate_cursor(cursor)?;
            let group = self
                .runs
                .get(&cursor.order_class)
                .ok_or(OrderedViewError::CursorPositionMismatch)?;
            let run = group
                .get(&cursor.row_key)
                .ok_or(OrderedViewError::CursorPositionMismatch)?;
            if cursor.occurrence >= run.count {
                return Err(OrderedViewError::CursorPositionMismatch);
            }
            let next = cursor.occurrence.saturating_add(1);
            if next < run.count {
                Self::append_run(
                    &mut selected,
                    &cursor.order_class,
                    &cursor.row_key,
                    run,
                    next,
                    limit,
                );
            }
            for (row_key, later_run) in group.range::<Vec<kernel_semantics::CanonicalEqKey>, _>((
                Excluded(&cursor.row_key),
                Unbounded,
            )) {
                if selected.len() > limit {
                    break;
                }
                Self::append_run(
                    &mut selected,
                    &cursor.order_class,
                    row_key,
                    later_run,
                    0,
                    limit,
                );
            }
            after_order_class = Some(cursor.order_class.clone());
        }

        match self.spec.direction {
            OrderDirection::Ascending => {
                let lower = after_order_class.as_ref().map_or(Unbounded, Excluded);
                for (order_class, group) in self.runs.range((lower, Unbounded)) {
                    if selected.len() > limit {
                        break;
                    }
                    Self::append_group(&mut selected, order_class, group, limit);
                }
            }
            OrderDirection::Descending => {
                let upper = after_order_class.as_ref().map_or(Unbounded, Excluded);
                for (order_class, group) in self.runs.range((Unbounded, upper)).rev() {
                    if selected.len() > limit {
                        break;
                    }
                    Self::append_group(&mut selected, order_class, group, limit);
                }
            }
        }

        let has_more = selected.len() > limit;
        selected.truncate(limit);
        let next_cursor = if has_more {
            selected
                .last()
                .map(|(_, order_class, row_key, occurrence)| {
                    self.cursor(order_class, row_key, *occurrence)
                })
        } else {
            None
        };
        Ok(OrderedViewPage {
            rows: selected.into_iter().map(|(row, _, _, _)| row).collect(),
            next_cursor,
        })
    }
}


#[cfg(test)]
impl OrderedViewSnapshot {
    pub(crate) fn run_count_for_test(&self) -> usize {
        self.runs.values().map(BTreeMap::len).sum()
    }

    pub(crate) fn first_run_count_for_test(&self) -> Option<u64> {
        self.runs
            .values()
            .next()
            .and_then(|group| group.values().next())
            .map(|run| run.count)
    }
}
