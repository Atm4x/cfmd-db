#[cfg(test)]
use super::{AdaptiveDelta, DeltaView, exact_delta_from_legacy, exact_delta_to_legacy_checked};
use super::{
    BTreeMap, BTreeSet, CanonicalRowKey, Change, ExactDelta, ExactDeltaSink, ExactDeltaView,
    MaterializedSetSupportState, OrderDirection, PlannedDeltaEffect, RelExpr, RelQueryError,
    RelType, RelationDelta, RelationValue, Row, apply_exact_to_natural, canonical_row_key,
    exact_natural_difference, materialize_exact_delta_view, rel_delta_optimized,
    relation_column_equivalences,
};
use kernel_persistent::PersistentOrdMap;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CountedTopKClass {
    representative: Row,
    multiplicity: kernel_exact::ExactNatural,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct CountedTopKBucket {
    classes: PersistentOrdMap<CanonicalRowKey, CountedTopKClass>,
    total: kernel_exact::ExactNatural,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TopKBoundary {
    key: Option<kernel_semantics::CanonicalOrderKey>,
    better_mass: kernel_exact::ExactNatural,
}

impl Default for TopKBoundary {
    fn default() -> Self {
        Self {
            key: None,
            better_mass: kernel_exact::ExactNatural::zero(),
        }
    }
}

#[derive(Debug, Clone)]
struct PlannedTopKRows {
    next: CountedOrderedRows,
    order_deltas: BTreeMap<kernel_semantics::CanonicalOrderKey, kernel_exact::ExactInteger>,
    changed_classes: BTreeSet<(kernel_semantics::CanonicalOrderKey, CanonicalRowKey)>,
}

#[derive(Clone, Copy)]
struct TopKMutationSpec<'a> {
    column: usize,
    encoder: kernel_semantics::ResolvedPrimitiveOrdering,
    ty: &'a RelType,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct CountedOrderedRows {
    buckets: PersistentOrdMap<kernel_semantics::CanonicalOrderKey, CountedTopKBucket>,
    total_rows: kernel_exact::ExactNatural,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MaintainedTopKStorage {
    Counted {
        encoder: kernel_semantics::ResolvedPrimitiveOrdering,
        rows: Box<CountedOrderedRows>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TopKDeltaPatch {
    rows: CountedOrderedRows,
    boundary: TopKBoundary,
}

#[cfg(test)]
impl TopKDeltaPatch {
    pub(super) fn test_stats(&self) -> (kernel_exact::ExactNatural, usize, usize) {
        let rows = &self.rows;
        (
            rows.total_rows.clone(),
            rows.buckets.len(),
            rows.buckets
                .values()
                .next()
                .map_or(0, |bucket| bucket.classes.len()),
        )
    }
}

impl CountedOrderedRows {
    #[cfg(test)]
    fn test_assert_consistent(&self) {
        let mut total = kernel_exact::ExactNatural::zero();
        for bucket in self.buckets.values() {
            let mut bucket_total = kernel_exact::ExactNatural::zero();
            for class in bucket.classes.values() {
                assert!(!class.multiplicity.is_zero());
                bucket_total.add_assign(&class.multiplicity);
            }
            assert_eq!(bucket.total, bucket_total);
            assert!(!bucket.total.is_zero());
            total.add_assign(&bucket.total);
        }
        assert_eq!(self.total_rows, total);
    }

    fn build(
        rows: Vec<Row>,
        column: usize,
        encoder: kernel_semantics::ResolvedPrimitiveOrdering,
        ty: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let mut state = Self::default();
        let one = kernel_exact::ExactInteger::from_i64(1);
        let spec = TopKMutationSpec {
            column,
            encoder,
            ty,
            context,
            registry,
        };
        for row in rows {
            state.apply_row_weight(&row, &one, spec)?;
        }
        Ok(state)
    }

    fn apply_row_weight(
        &mut self,
        row: &Row,
        weight: &kernel_exact::ExactInteger,
        spec: TopKMutationSpec<'_>,
    ) -> Result<(), RelQueryError> {
        if weight.is_zero() {
            return Ok(());
        }
        let order_key = spec.encoder.canonical_key(
            row.get(spec.column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?,
        )?;
        let row_key = canonical_row_key(
            row,
            relation_column_equivalences(spec.ty),
            spec.context,
            spec.registry,
        )?;
        let mut bucket = self.buckets.get(&order_key).cloned().unwrap_or_default();
        let mut class = bucket
            .classes
            .get(&row_key)
            .cloned()
            .unwrap_or_else(|| CountedTopKClass {
                representative: row.clone(),
                multiplicity: kernel_exact::ExactNatural::zero(),
            });
        apply_exact_to_natural(&mut class.multiplicity, weight)?;
        apply_exact_to_natural(&mut bucket.total, weight)?;
        apply_exact_to_natural(&mut self.total_rows, weight)?;
        if matches!(
            spec.ty.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        ) && class.multiplicity > kernel_exact::ExactNatural::one()
        {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        if class.multiplicity.is_zero() {
            if bucket.classes.remove(&row_key).is_none() {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
        } else {
            bucket.classes.insert(row_key.clone(), class);
        }
        if bucket.classes.is_empty() {
            debug_assert!(bucket.total.is_zero());
            self.buckets.remove(&order_key);
        } else {
            debug_assert!(!bucket.total.is_zero());
            self.buckets.insert(order_key, bucket);
        }
        Ok(())
    }

    fn plan_exact_mutation<D: ExactDeltaView<Row>>(
        &self,
        input_delta: &D,
        spec: TopKMutationSpec<'_>,
    ) -> Result<PlannedTopKRows, RelQueryError> {
        let mut classes = BTreeMap::<
            (kernel_semantics::CanonicalOrderKey, CanonicalRowKey),
            (Row, kernel_exact::ExactInteger),
        >::new();
        let mut error = None;
        input_delta.visit_exact(|weight, row| {
            if weight.is_zero() || error.is_some() {
                return;
            }
            let Some(value) = row.get(spec.column) else {
                error = Some(RelQueryError::ColumnOutOfBounds);
                return;
            };
            let order_key = match spec.encoder.canonical_key(value) {
                Ok(key) => key,
                Err(cause) => {
                    error = Some(cause.into());
                    return;
                }
            };
            let row_key = match canonical_row_key(
                row,
                relation_column_equivalences(spec.ty),
                spec.context,
                spec.registry,
            ) {
                Ok(key) => key,
                Err(cause) => {
                    error = Some(cause);
                    return;
                }
            };
            let entry = classes
                .entry((order_key, row_key))
                .or_insert_with(|| (row.clone(), kernel_exact::ExactInteger::default()));
            entry.1.add_assign(weight);
        });
        if let Some(error) = error {
            return Err(error);
        }
        let mut next = self.clone();
        let mut order_deltas =
            BTreeMap::<kernel_semantics::CanonicalOrderKey, kernel_exact::ExactInteger>::new();
        let mut changed_classes = BTreeSet::new();
        for ((order_key, row_key), (row, weight)) in classes {
            if !weight.is_zero() {
                order_deltas
                    .entry(order_key.clone())
                    .or_default()
                    .add_assign(&weight);
                changed_classes.insert((order_key, row_key));
                next.apply_row_weight(&row, &weight, spec)?;
            }
        }
        Ok(PlannedTopKRows {
            next,
            order_deltas,
            changed_classes,
        })
    }

    fn boundary(&self, direction: OrderDirection, k: usize) -> TopKBoundary {
        if k == 0 || self.buckets.is_empty() {
            return TopKBoundary::default();
        }
        let target = kernel_exact::ExactNatural::from_u128(k as u128);
        let mut better_mass = kernel_exact::ExactNatural::zero();
        let mut last = None;
        let mut visit = |key: &kernel_semantics::CanonicalOrderKey, bucket: &CountedTopKBucket| {
            let before = better_mass.clone();
            better_mass.add_assign(&bucket.total);
            last = Some(TopKBoundary {
                key: Some(key.clone()),
                better_mass: before,
            });
            better_mass >= target
        };
        match direction {
            OrderDirection::Ascending => {
                for (key, bucket) in &self.buckets {
                    if visit(key, bucket) {
                        break;
                    }
                }
            }
            OrderDirection::Descending => {
                for (key, bucket) in self.buckets.iter().rev() {
                    if visit(key, bucket) {
                        break;
                    }
                }
            }
        }
        last.unwrap_or_default()
    }

    fn repair_boundary(
        next: &Self,
        old: &TopKBoundary,
        order_deltas: &BTreeMap<kernel_semantics::CanonicalOrderKey, kernel_exact::ExactInteger>,
        direction: OrderDirection,
        k: usize,
    ) -> Result<TopKBoundary, RelQueryError> {
        if k == 0 || next.buckets.is_empty() {
            return Ok(TopKBoundary::default());
        }
        let target = kernel_exact::ExactNatural::from_u128(k as u128);
        if next.total_rows <= target {
            let (key, bucket) = match direction {
                OrderDirection::Ascending => next.buckets.iter().next_back(),
                OrderDirection::Descending => next.buckets.iter().next(),
            }
            .expect("non-empty TopK rows must have a worst bucket");
            let mut better_mass = next.total_rows.clone();
            let subtracted = better_mass.checked_sub_assign(&bucket.total);
            debug_assert!(subtracted);
            return Ok(TopKBoundary {
                key: Some(key.clone()),
                better_mass,
            });
        }
        let Some(old_key) = old.key.as_ref() else {
            return Ok(next.boundary(direction, k));
        };

        let mut better_mass = old.better_mass.clone();
        for (key, delta) in order_deltas {
            if Self::key_is_better(key, old_key, direction) {
                apply_exact_to_natural(&mut better_mass, delta)?;
            }
        }

        if better_mass >= target {
            let mut pivot = old_key;
            let mut through_candidate = better_mass;
            loop {
                let Some((candidate, bucket)) = Self::next_better_bucket(next, pivot, direction)
                else {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                };
                let mut before_candidate = through_candidate.clone();
                if !before_candidate.checked_sub_assign(&bucket.total) {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
                if before_candidate < target {
                    return Ok(TopKBoundary {
                        key: Some(candidate.clone()),
                        better_mass: before_candidate,
                    });
                }
                through_candidate = before_candidate;
                pivot = candidate;
            }
        }

        let mut through = better_mass.clone();
        if let Some(bucket) = next.buckets.get(old_key) {
            through.add_assign(&bucket.total);
            if through >= target {
                return Ok(TopKBoundary {
                    key: Some(old_key.clone()),
                    better_mass,
                });
            }
        }

        let mut pivot = old_key;
        loop {
            let Some((candidate, bucket)) = Self::next_worse_bucket(next, pivot, direction) else {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            };
            let before_candidate = through.clone();
            through.add_assign(&bucket.total);
            if through >= target {
                return Ok(TopKBoundary {
                    key: Some(candidate.clone()),
                    better_mass: before_candidate,
                });
            }
            pivot = candidate;
        }
    }

    fn key_is_better(
        left: &kernel_semantics::CanonicalOrderKey,
        right: &kernel_semantics::CanonicalOrderKey,
        direction: OrderDirection,
    ) -> bool {
        match direction {
            OrderDirection::Ascending => left < right,
            OrderDirection::Descending => left > right,
        }
    }

    fn next_better_bucket<'a>(
        rows: &'a Self,
        pivot: &kernel_semantics::CanonicalOrderKey,
        direction: OrderDirection,
    ) -> Option<(
        &'a kernel_semantics::CanonicalOrderKey,
        &'a CountedTopKBucket,
    )> {
        match direction {
            OrderDirection::Ascending => rows.buckets.predecessor(pivot),
            OrderDirection::Descending => rows.buckets.successor(pivot),
        }
    }

    fn next_worse_bucket<'a>(
        rows: &'a Self,
        pivot: &kernel_semantics::CanonicalOrderKey,
        direction: OrderDirection,
    ) -> Option<(
        &'a kernel_semantics::CanonicalOrderKey,
        &'a CountedTopKBucket,
    )> {
        match direction {
            OrderDirection::Ascending => rows.buckets.successor(pivot),
            OrderDirection::Descending => rows.buckets.predecessor(pivot),
        }
    }

    fn selected_measure(
        &self,
        direction: OrderDirection,
        k: usize,
    ) -> BTreeMap<CanonicalRowKey, (Row, kernel_exact::ExactNatural)> {
        let mut selected = BTreeMap::new();
        if k == 0 {
            return selected;
        }
        let target = kernel_exact::ExactNatural::from_u128(k as u128);
        let mut cumulative = kernel_exact::ExactNatural::zero();
        let mut visit_bucket = |bucket: &CountedTopKBucket| {
            for (row_key, class) in &bucket.classes {
                selected.insert(
                    row_key.clone(),
                    (class.representative.clone(), class.multiplicity.clone()),
                );
            }
            cumulative.add_assign(&bucket.total);
            cumulative >= target
        };
        match direction {
            OrderDirection::Ascending => {
                for bucket in self.buckets.values() {
                    if visit_bucket(bucket) {
                        break;
                    }
                }
            }
            OrderDirection::Descending => {
                for bucket in self.buckets.values().rev() {
                    if visit_bucket(bucket) {
                        break;
                    }
                }
            }
        }
        selected
    }

    fn selected_effect(
        &self,
        next: &Self,
        before_boundary: &TopKBoundary,
        after_boundary: &TopKBoundary,
        direction: OrderDirection,
        k: usize,
        changed_classes: &BTreeSet<(kernel_semantics::CanonicalOrderKey, CanonicalRowKey)>,
    ) -> ExactDelta<Row> {
        let mut keys = changed_classes.clone();
        match (&before_boundary.key, &after_boundary.key) {
            (Some(before), Some(after)) if before != after => {
                if Self::key_is_better(after, before, direction) {
                    let mut pivot = after;
                    while let Some((key, bucket)) = Self::next_worse_bucket(self, pivot, direction)
                    {
                        Self::insert_bucket_class_keys(key, bucket, &mut keys);
                        if key == before {
                            break;
                        }
                        pivot = key;
                    }
                } else {
                    let mut pivot = before;
                    while let Some((key, bucket)) = Self::next_worse_bucket(next, pivot, direction)
                    {
                        Self::insert_bucket_class_keys(key, bucket, &mut keys);
                        if key == after {
                            break;
                        }
                        pivot = key;
                    }
                }
            }
            (None, Some(after)) => {
                next.insert_selected_prefix_class_keys(after, direction, &mut keys);
            }
            (Some(before), None) => {
                self.insert_selected_prefix_class_keys(before, direction, &mut keys);
            }
            _ => {}
        }

        let mut effect = ExactDelta::with_capacity(keys.len());
        for (order_key, row_key) in keys {
            let before = self.selected_class(&order_key, &row_key, before_boundary, direction, k);
            let after = next.selected_class(&order_key, &row_key, after_boundary, direction, k);
            let before_count = before.map_or_else(kernel_exact::ExactNatural::zero, |class| {
                class.multiplicity.clone()
            });
            let after_count = after.map_or_else(kernel_exact::ExactNatural::zero, |class| {
                class.multiplicity.clone()
            });
            let weight = exact_natural_difference(&after_count, &before_count);
            if weight.is_zero() {
                continue;
            }
            let row = after
                .or(before)
                .expect("selected TopK class must exist on one side")
                .representative
                .clone();
            effect.push_exact(weight, row);
        }
        effect
    }

    fn selected_class<'a>(
        &'a self,
        order_key: &kernel_semantics::CanonicalOrderKey,
        row_key: &CanonicalRowKey,
        boundary: &TopKBoundary,
        direction: OrderDirection,
        k: usize,
    ) -> Option<&'a CountedTopKClass> {
        if k == 0 {
            return None;
        }
        let boundary_key = boundary.key.as_ref()?;
        let selected = match direction {
            OrderDirection::Ascending => order_key <= boundary_key,
            OrderDirection::Descending => order_key >= boundary_key,
        };
        selected
            .then(|| self.buckets.get(order_key)?.classes.get(row_key))
            .flatten()
    }

    fn insert_bucket_class_keys(
        order_key: &kernel_semantics::CanonicalOrderKey,
        bucket: &CountedTopKBucket,
        keys: &mut BTreeSet<(kernel_semantics::CanonicalOrderKey, CanonicalRowKey)>,
    ) {
        keys.extend(
            bucket
                .classes
                .keys()
                .cloned()
                .map(|row_key| (order_key.clone(), row_key)),
        );
    }

    fn insert_selected_prefix_class_keys(
        &self,
        boundary: &kernel_semantics::CanonicalOrderKey,
        direction: OrderDirection,
        keys: &mut BTreeSet<(kernel_semantics::CanonicalOrderKey, CanonicalRowKey)>,
    ) {
        match direction {
            OrderDirection::Ascending => {
                for (key, bucket) in &self.buckets {
                    Self::insert_bucket_class_keys(key, bucket, keys);
                    if key == boundary {
                        break;
                    }
                }
            }
            OrderDirection::Descending => {
                for (key, bucket) in self.buckets.iter().rev() {
                    Self::insert_bucket_class_keys(key, bucket, keys);
                    if key == boundary {
                        break;
                    }
                }
            }
        }
    }

    fn output_rows(&self, direction: OrderDirection, k: usize) -> Result<Vec<Row>, RelQueryError> {
        let selected = self.selected_measure(direction, k);
        let mut rows = Vec::new();
        for (_, (row, count)) in selected {
            let count = count
                .to_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(RelQueryError::DerivedIdentityExhausted)?;
            rows.try_reserve(count)
                .map_err(|_| RelQueryError::DerivedIdentityExhausted)?;
            rows.extend(std::iter::repeat_n(row, count));
        }
        Ok(rows)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedTopKDeltaState {
    query: RelExpr,
    semantic_context: kernel_schema::SemanticContext,
    input: RelExpr,
    input_type: RelType,
    result_type: RelType,
    column: usize,
    ordering: kernel_types::SemanticId,
    direction: OrderDirection,
    k: usize,
    boundary: TopKBoundary,
    storage: MaintainedTopKStorage,
}

impl MaterializedTopKDeltaState {
    pub fn build(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        let RelExpr::TopKWithTies { input, .. } = query else {
            return Ok(None);
        };
        query.prepare(context, registry)?;
        let input_value = input.evaluate(old, context, registry)?;
        Self::build_from_input_value(query, input_value, context, registry)
    }

    pub(super) fn build_from_input_value(
        query: &RelExpr,
        input_value: RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        query.prepare(context, registry)?;
        let RelExpr::TopKWithTies {
            input,
            column,
            ordering,
            direction,
            k,
        } = query
        else {
            return Ok(None);
        };
        let input_type = input.typecheck(context, registry)?;
        let result_type = query.typecheck(context, registry)?;
        MaterializedSetSupportState::validate_rows(
            input_value.rows(),
            &input_type,
            context,
            registry,
        )?;
        let encoder = registry.resolve_primitive_ordering(context, *ordering)?;
        let rows = CountedOrderedRows::build(
            input_value.into_rows(),
            *column,
            encoder,
            &input_type,
            context,
            registry,
        )?;
        let boundary = rows.boundary(*direction, *k);
        Ok(Some(Self {
            query: query.clone(),
            semantic_context: context.clone(),
            input: input.as_ref().clone(),
            input_type,
            result_type,
            column: *column,
            ordering: *ordering,
            direction: *direction,
            k: *k,
            boundary,
            storage: MaintainedTopKStorage::Counted {
                encoder,
                rows: Box::new(rows),
            },
        }))
    }

    #[must_use]
    pub fn query(&self) -> &RelExpr {
        &self.query
    }
    pub(super) fn result_type(&self) -> &RelType {
        &self.result_type
    }

    #[cfg(test)]
    pub(super) fn test_storage_is_counted(&self) -> bool {
        matches!(self.storage, MaintainedTopKStorage::Counted { .. })
    }

    #[cfg(test)]
    pub(super) fn test_stats(&self) -> (kernel_exact::ExactNatural, usize, usize) {
        let MaintainedTopKStorage::Counted { rows, .. } = &self.storage;
        (
            rows.total_rows.clone(),
            rows.buckets.len(),
            rows.buckets
                .values()
                .next()
                .map_or(0, |bucket| bucket.classes.len()),
        )
    }

    #[cfg(test)]
    pub(super) fn test_shares_storage_with(&self, other: &Self) -> bool {
        let MaintainedTopKStorage::Counted { rows: left, .. } = &self.storage;
        let MaintainedTopKStorage::Counted { rows: right, .. } = &other.storage;
        left.buckets.shares_root_with(&right.buckets)
    }

    #[cfg(test)]
    pub(super) fn test_assert_internal_consistency(&self) {
        let MaintainedTopKStorage::Counted { rows, .. } = &self.storage;
        rows.test_assert_consistent();
        assert_eq!(self.boundary, rows.boundary(self.direction, self.k));
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        let MaintainedTopKStorage::Counted { rows, .. } = &self.storage;
        rows.total_rows
            .to_u64()
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(usize::MAX)
    }

    pub fn apply_model_change(
        &mut self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let input_delta = rel_delta_optimized(&self.input, old, change, context, registry)?
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        self.apply_input_delta(&input_delta, context, registry)
    }

    pub fn apply_input_delta(
        &mut self,
        input_delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        if input_delta.result_type != self.input_type {
            return Err(RelQueryError::TypeMismatch);
        }
        let planned =
            self.plan_exact_delta_view(&input_delta.as_delta_view(), context, registry)?;
        let effect = materialize_exact_delta_view(&planned.effect, self.result_type.clone())?;
        self.commit_topk_patch(planned.patch);
        Ok(effect)
    }

    pub(super) fn plan_exact_delta_view<D: ExactDeltaView<Row>>(
        &self,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<TopKDeltaPatch, ExactDelta<Row>>, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let mut validation_error = None;
        input_delta.visit_exact(|weight, row| {
            if weight.is_zero() || validation_error.is_some() {
                return;
            }
            if let Err(error) = MaterializedSetSupportState::validate_row_shapes(
                std::slice::from_ref(row),
                &self.input_type,
            ) {
                validation_error = Some(error);
            }
        });
        if let Some(error) = validation_error {
            return Err(error);
        }
        let MaintainedTopKStorage::Counted { encoder, rows } = &self.storage;
        let planned_rows = rows.plan_exact_mutation(
            input_delta,
            TopKMutationSpec {
                column: self.column,
                encoder: *encoder,
                ty: &self.input_type,
                context,
                registry,
            },
        )?;
        let next_boundary = CountedOrderedRows::repair_boundary(
            &planned_rows.next,
            &self.boundary,
            &planned_rows.order_deltas,
            self.direction,
            self.k,
        )?;
        let effect = rows.selected_effect(
            &planned_rows.next,
            &self.boundary,
            &next_boundary,
            self.direction,
            self.k,
            &planned_rows.changed_classes,
        );
        Ok(PlannedDeltaEffect {
            patch: TopKDeltaPatch {
                rows: planned_rows.next,
                boundary: next_boundary,
            },
            effect,
        })
    }

    #[cfg(test)]
    pub(super) fn plan_delta_view<D: DeltaView<Row>>(
        &self,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<TopKDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        let exact = exact_delta_from_legacy(input_delta);
        let planned = self.plan_exact_delta_view(&exact, context, registry)?;
        Ok(PlannedDeltaEffect {
            patch: planned.patch,
            effect: exact_delta_to_legacy_checked(&planned.effect)?,
        })
    }

    pub(super) fn commit_topk_patch(&mut self, patch: TopKDeltaPatch) {
        let TopKDeltaPatch {
            rows: next,
            boundary,
        } = patch;
        let MaintainedTopKStorage::Counted { rows, .. } = &mut self.storage;
        **rows = next;
        self.boundary = boundary;
    }

    pub(super) fn output_value(&self) -> Result<RelationValue, RelQueryError> {
        let MaintainedTopKStorage::Counted { rows, .. } = &self.storage;
        let rows = rows.output_rows(self.direction, self.k)?;
        Ok(match &self.result_type.semantics {
            kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.clone(),
            },
        })
    }
}
