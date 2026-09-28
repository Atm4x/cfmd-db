use std::collections::BTreeMap;

use kernel_change::Change;
use kernel_model::Value;
use kernel_persistent::{PersistentOrdMap, PersistentVec};

#[cfg(test)]
use super::{AdaptiveDelta, DeltaView};
use super::{
    AggregateSpec, ExactDelta, ExactDeltaSink, ExactDeltaView, MaterializedSetSupportState,
    PlannedDeltaEffect, RelExpr, RelQueryError, RelType, RelationDelta, RelationValue, Row,
    materialize_exact_delta_view, rel_delta_optimized, relation_column_equivalences,
    relation_value_from_rows, rows_semantically_equal,
};

mod sealed;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaintainedGroupBucket {
    key: Row,
    count: kernel_aggregate::ExactCount,
    sum: kernel_aggregate::ExactF64Sum,
}

const DENSE_GROUP_MARGIN: usize = 64;
const DENSE_GROUP_MAX_SLOTS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DenseWindowGroupCount {
    base: i64,
    counts: Vec<kernel_aggregate::ExactCount>,
}

impl DenseWindowGroupCount {
    fn try_build(groups: &[MaintainedGroupBucket]) -> Option<Self> {
        let mut keys = groups.iter().filter_map(|group| {
            let [Value::I64(key)] = group.key.as_slice() else {
                return None;
            };
            Some(*key)
        });
        let Some(first) = keys.next() else {
            return Some(Self {
                base: 0,
                counts: Vec::new(),
            });
        };
        let (mut min, mut max) = (first, first);
        for key in keys {
            min = min.min(key);
            max = max.max(key);
        }
        let margin = i64::try_from(DENSE_GROUP_MARGIN).ok()?;
        let lower = min.saturating_sub(margin);
        let upper = max.saturating_add(margin);
        let span = i128::from(upper) - i128::from(lower) + 1;
        let slots = usize::try_from(span).ok()?;
        if slots == 0 || slots > DENSE_GROUP_MAX_SLOTS {
            return None;
        }
        let mut dense = Self {
            base: lower,
            counts: vec![kernel_aggregate::ExactCount::default(); slots],
        };
        for group in groups {
            let [Value::I64(key)] = group.key.as_slice() else {
                return None;
            };
            let index = dense.index(*key)?;
            dense.counts[index] = group.count.clone();
        }
        Some(dense)
    }

    fn index(&self, key: i64) -> Option<usize> {
        let offset = i128::from(key) - i128::from(self.base);
        let index = usize::try_from(offset).ok()?;
        (index < self.counts.len()).then_some(index)
    }

    fn count(&self, key: i64) -> Option<&kernel_aggregate::ExactCount> {
        self.index(key).map(|index| &self.counts[index])
    }

    #[cfg(test)]
    fn set(&mut self, key: i64, count: kernel_aggregate::ExactCount) -> bool {
        let Some(index) = self.index(key) else {
            return false;
        };
        self.counts[index] = count;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct I64CountGroupPatch {
    changes: Vec<(i64, kernel_aggregate::ExactCount)>,
    retain_dense: bool,
    dense_move: Option<(i64, i64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GenericGroupPatch {
    planned: Vec<(Row, Option<MaintainedGroupBucket>)>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum IndexedGroupKey {
    I64(i64),
    Semantic(Vec<kernel_semantics::CanonicalEqKey>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GroupDeltaPatch {
    Generic(GenericGroupPatch),
    I64Count(I64CountGroupPatch),
}

#[derive(Debug)]
pub(super) struct GroupCommitPatch(sealed::SealedGroupPatch);

#[cfg(test)]
pub(super) struct GroupI64PlanObservation {
    pub(super) retain_dense: bool,
    pub(super) dense_move: Option<(i64, i64)>,
    pub(super) effect: AdaptiveDelta<Row, 4>,
    pub(super) unchanged_before_commit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedGroupDeltaState {
    query: RelExpr,
    semantic_context: kernel_schema::SemanticContext,
    input: RelExpr,
    input_type: RelType,
    group_columns: Vec<usize>,
    group_equivalences: Vec<kernel_types::SemanticId>,
    aggregate: AggregateSpec,
    result_type: RelType,
    groups: PersistentVec<MaintainedGroupBucket>,
    i64_lookup: Option<PersistentOrdMap<i64, usize>>,
    semantic_lookup: Option<PersistentOrdMap<Vec<kernel_semantics::CanonicalEqKey>, usize>>,
    group_encoders: Option<Vec<kernel_semantics::ResolvedPrimitiveEquivalence>>,
    fast_i64_count: bool,
    dense_i64_count: Option<DenseWindowGroupCount>,
}

impl MaterializedGroupDeltaState {
    pub(super) fn result_type(&self) -> &RelType {
        &self.result_type
    }

    pub fn build(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        let RelExpr::Group { input, .. } = query else {
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
        let RelExpr::Group {
            input,
            group_columns,
            group_equivalences,
            aggregate,
        } = query
        else {
            return Ok(None);
        };
        let result_type = query.typecheck(context, registry)?;
        let input_type = input.typecheck(context, registry)?;
        MaterializedSetSupportState::validate_rows(
            input_value.rows(),
            &input_type,
            context,
            registry,
        )?;
        let i64_lookup = if group_columns.len() == 1
            && matches!(
                input_type.columns.get(group_columns[0]),
                Some(kernel_schema::TypeExpr::Scalar(
                    kernel_schema::ScalarType::I64
                ))
            )
            && matches!(
                registry
                    .resolve_primitive_equivalence(context, group_equivalences[0])?
                    .map(|resolved| resolved.bind_right(&Value::I64(0))),
                Some(Ok(kernel_semantics::BoundPrimitivePredicate::I64(0)))
            ) {
            Some(PersistentOrdMap::default())
        } else {
            None
        };
        let group_encoders = if i64_lookup.is_none() && !group_columns.is_empty() {
            let mut encoders = Vec::with_capacity(group_equivalences.len());
            let mut supported = true;
            for equivalence in group_equivalences {
                let Some(encoder) =
                    registry.resolve_primitive_equivalence(context, *equivalence)?
                else {
                    supported = false;
                    break;
                };
                encoders.push(encoder);
            }
            supported.then_some(encoders)
        } else {
            None
        };
        let semantic_lookup = i64_lookup.is_none().then(PersistentOrdMap::default);
        let fast_i64_count = i64_lookup.is_some()
            && matches!(aggregate, AggregateSpec::Count { .. })
            && matches!(
                aggregate,
                AggregateSpec::Count { result_equivalence }
                    if matches!(
                        registry
                            .resolve_primitive_equivalence(context, *result_equivalence)?
                            .map(|resolved| resolved.bind_right(&Value::I64(0))),
                        Some(Ok(kernel_semantics::BoundPrimitivePredicate::I64(0)))
                    )
            );
        let mut state = Self {
            query: query.clone(),
            semantic_context: context.clone(),
            input: input.as_ref().clone(),
            input_type,
            group_columns: group_columns.clone(),
            group_equivalences: group_equivalences.clone(),
            aggregate: aggregate.clone(),
            result_type,
            groups: PersistentVec::default(),
            i64_lookup,
            semantic_lookup,
            group_encoders,
            fast_i64_count,
            dense_i64_count: None,
        };
        for row in input_value.into_rows() {
            state.insert_row(&row, context, registry)?;
        }
        if state.group_columns.is_empty() && state.groups.is_empty() {
            state.push_group_bucket(Self::empty_bucket(Vec::new()), registry)?;
        }
        if state.fast_i64_count {
            state.dense_i64_count = DenseWindowGroupCount::try_build(state.groups.as_slice());
        }
        Ok(Some(state))
    }

    pub(super) fn output_value(&self) -> Result<RelationValue, RelQueryError> {
        let rows = self
            .groups
            .iter()
            .map(|group| self.output_for_bucket(Some(group)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(relation_value_from_rows(rows, &self.result_type))
    }

    #[must_use]
    pub fn query(&self) -> &RelExpr {
        &self.query
    }

    #[must_use]
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    pub(super) fn supports_join_group_topk_composition(&self) -> bool {
        self.fast_i64_count
    }

    pub(super) fn plan_exact_effect<D: ExactDeltaView<Row>>(
        &self,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<ExactDelta<Row>, RelQueryError> {
        Ok(self
            .plan_exact_delta_view(input_delta, context, registry)?
            .effect)
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
        if input_delta.result_type != self.input_type {
            return Err(RelQueryError::TypeMismatch);
        }
        let planned =
            self.plan_exact_delta_view(&input_delta.as_delta_view(), context, registry)?;
        let effect = materialize_exact_delta_view(&planned.effect, self.result_type.clone())?;
        let sealed = sealed::seal_group_patch(self, planned.patch, context, registry)?;
        sealed::commit_sealed_group_patch(self, sealed);
        Ok(effect)
    }

    fn plan_exact_delta_view<D: ExactDeltaView<Row>>(
        &self,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<GroupDeltaPatch, ExactDelta<Row>>, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let mut validation_error = None;
        input_delta.visit_exact(|weight, row| {
            if weight.is_zero() || validation_error.is_some() {
                return;
            }
            if let Err(error) = MaterializedSetSupportState::validate_rows(
                std::slice::from_ref(row),
                &self.input_type,
                context,
                registry,
            ) {
                validation_error = Some(error);
            }
        });
        if let Some(error) = validation_error {
            return Err(error);
        }
        if self.fast_i64_count {
            let planned = self.plan_exact_i64_count_delta(input_delta)?;
            return Ok(PlannedDeltaEffect {
                patch: GroupDeltaPatch::I64Count(planned.patch),
                effect: planned.effect,
            });
        }
        self.plan_exact_generic_delta_view(input_delta, context, registry)
    }

    pub(super) fn plan_sealed_exact_delta_view<D: ExactDeltaView<Row>>(
        &self,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<GroupCommitPatch, ExactDelta<Row>>, RelQueryError> {
        let planned = self.plan_exact_delta_view(input_delta, context, registry)?;
        let patch = sealed::seal_group_patch(self, planned.patch, context, registry)?;
        Ok(PlannedDeltaEffect {
            patch: GroupCommitPatch(patch),
            effect: planned.effect,
        })
    }

    pub(super) fn commit_sealed_patch(&mut self, patch: GroupCommitPatch) {
        sealed::commit_sealed_group_patch(self, patch.0);
    }

    fn plan_exact_generic_delta_view<D: ExactDeltaView<Row>>(
        &self,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<GroupDeltaPatch, ExactDelta<Row>>, RelQueryError> {
        let mut grouped =
            BTreeMap::<IndexedGroupKey, (Row, Vec<(kernel_exact::ExactInteger, Row)>)>::new();
        let mut error = None;
        input_delta.visit_exact(|weight, row| {
            if weight.is_zero() || error.is_some() {
                return;
            }
            let key = match self.key_for_row(row) {
                Ok(key) => key,
                Err(next) => {
                    error = Some(next);
                    return;
                }
            };
            let indexed = match self.indexed_group_key(&key, registry) {
                Ok(indexed) => indexed,
                Err(next) => {
                    error = Some(next);
                    return;
                }
            };
            grouped
                .entry(indexed)
                .or_insert_with(|| (key, Vec::new()))
                .1
                .push((weight.clone(), row.clone()));
        });
        if let Some(error) = error {
            return Err(error);
        }

        let equivalences = relation_column_equivalences(&self.result_type);
        let mut planned = Vec::with_capacity(grouped.len());
        let mut effect = ExactDelta::<Row>::default();
        for (indexed, (key, bucket_entries)) in grouped {
            let current = self.group_index_by_indexed_key(&indexed);
            let before = self.output_for_bucket(current.map(|index| &self.groups[index]))?;
            let mut next = current.map_or_else(
                || Self::empty_bucket(key.clone()),
                |index| self.groups[index].clone(),
            );
            for (weight, row) in bucket_entries {
                self.apply_exact_weight_to_bucket(&mut next, &row, &weight)?;
            }
            let next = if next.count.is_zero() && !self.group_columns.is_empty() {
                None
            } else {
                Some(next)
            };
            let after = self.output_for_bucket(next.as_ref())?;
            match (before, after) {
                (Some(old_row), Some(new_row)) => {
                    if !rows_semantically_equal(
                        &old_row,
                        &new_row,
                        equivalences,
                        context,
                        registry,
                    )? {
                        effect.push_exact(kernel_exact::ExactInteger::from_i64(-1), old_row);
                        effect.push_exact(kernel_exact::ExactInteger::from_i64(1), new_row);
                    }
                }
                (Some(old_row), None) => {
                    effect.push_exact(kernel_exact::ExactInteger::from_i64(-1), old_row);
                }
                (None, Some(new_row)) => {
                    effect.push_exact(kernel_exact::ExactInteger::from_i64(1), new_row);
                }
                (None, None) => {}
            }
            planned.push((key, next));
        }
        Ok(PlannedDeltaEffect {
            patch: GroupDeltaPatch::Generic(GenericGroupPatch { planned }),
            effect,
        })
    }

    fn plan_exact_i64_count_delta<D: ExactDeltaView<Row>>(
        &self,
        input_delta: &D,
    ) -> Result<PlannedDeltaEffect<I64CountGroupPatch, ExactDelta<Row>>, RelQueryError> {
        let group_column = self.group_columns[0];
        let mut signed = BTreeMap::<i64, kernel_exact::ExactInteger>::new();
        let mut error = None;
        input_delta.visit_exact(|weight, row| {
            if error.is_some() || weight.is_zero() {
                return;
            }
            let Some(Value::I64(key)) = row.get(group_column) else {
                error = Some(RelQueryError::TypeMismatch);
                return;
            };
            signed.entry(*key).or_default().add_assign(weight);
        });
        if let Some(error) = error {
            return Err(error);
        }
        signed.retain(|_, weight| !weight.is_zero());

        let retain_dense = self
            .dense_i64_count
            .as_ref()
            .is_some_and(|dense| signed.keys().all(|key| dense.index(*key).is_some()));
        let lookup = self
            .i64_lookup
            .as_ref()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let mut changes = Vec::with_capacity(signed.len());
        let mut effect = ExactDelta::<Row>::default();
        let dense_move = self.plan_exact_dense_singleton_move(&signed, retain_dense)?;
        for (key, weight) in signed {
            let current = if retain_dense {
                self.dense_i64_count
                    .as_ref()
                    .and_then(|dense| dense.count(key))
                    .cloned()
                    .unwrap_or_default()
            } else {
                lookup
                    .get(&key)
                    .map(|index| self.groups[*index].count.clone())
                    .unwrap_or_default()
            };
            let before = (!current.is_zero())
                .then(|| current.finish_i64())
                .transpose()?;
            let mut next = current;
            if weight.is_negative() {
                next.remove_exact(weight.magnitude())?;
            } else {
                next.add_exact(weight.magnitude());
            }
            let after = (!next.is_zero()).then(|| next.finish_i64()).transpose()?;
            if before != after {
                if let Some(value) = before {
                    effect.push_exact(
                        kernel_exact::ExactInteger::from_i64(-1),
                        vec![Value::I64(key), Value::I64(value)],
                    );
                }
                if let Some(value) = after {
                    effect.push_exact(
                        kernel_exact::ExactInteger::from_i64(1),
                        vec![Value::I64(key), Value::I64(value)],
                    );
                }
            }
            changes.push((key, next));
        }
        Ok(PlannedDeltaEffect {
            patch: I64CountGroupPatch {
                changes,
                retain_dense,
                dense_move,
            },
            effect,
        })
    }

    fn plan_exact_dense_singleton_move(
        &self,
        signed: &BTreeMap<i64, kernel_exact::ExactInteger>,
        retain_dense: bool,
    ) -> Result<Option<(i64, i64)>, RelQueryError> {
        if !retain_dense || signed.len() != 2 {
            return Ok(None);
        }
        let removed = signed.iter().find_map(|(key, weight)| {
            (weight.is_negative() && weight.magnitude().is_one()).then_some(*key)
        });
        let inserted = signed.iter().find_map(|(key, weight)| {
            (!weight.is_negative() && weight.magnitude().is_one()).then_some(*key)
        });
        let (Some(removed), Some(inserted)) = (removed, inserted) else {
            return Ok(None);
        };
        if removed == inserted {
            return Ok(None);
        }
        let dense = self
            .dense_i64_count
            .as_ref()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let source = dense
            .count(removed)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let target = dense
            .count(inserted)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        Ok((source.is_one() && target.is_zero()).then_some((removed, inserted)))
    }

    #[cfg(test)]
    fn plan_delta_view<D: DeltaView<Row>>(
        &self,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<GroupDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        let exact = super::exact_delta_from_legacy(input_delta);
        let planned = self.plan_exact_delta_view(&exact, context, registry)?;
        Ok(PlannedDeltaEffect {
            patch: planned.patch,
            effect: super::exact_delta_to_legacy_checked(&planned.effect)?,
        })
    }

    #[cfg(test)]
    fn commit_group_patch(
        &mut self,
        patch: GroupDeltaPatch,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        match patch {
            GroupDeltaPatch::I64Count(patch) => self.commit_i64_count_patch(patch),
            GroupDeltaPatch::Generic(patch) => {
                self.commit_generic_group_patch(patch, context, registry)
            }
        }
    }

    #[cfg(test)]
    fn commit_i64_count_patch(&mut self, patch: I64CountGroupPatch) -> Result<(), RelQueryError> {
        if !patch.retain_dense {
            self.dense_i64_count = None;
        }
        if let Some((removed, inserted)) = patch.dense_move {
            let dense = self
                .dense_i64_count
                .as_mut()
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let removed_index = dense
                .index(removed)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let inserted_index = dense
                .index(inserted)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let moved = std::mem::take(&mut dense.counts[removed_index]);
            if !moved.is_one() || !dense.counts[inserted_index].is_zero() {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            dense.counts[inserted_index] = moved;
        }
        for (key, count) in patch.changes {
            if let Some(dense) = &mut self.dense_i64_count
                && !patch
                    .dense_move
                    .is_some_and(|(removed, inserted)| key == removed || key == inserted)
                && !dense.set(key, count.clone())
            {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            let current = self
                .i64_lookup
                .as_ref()
                .and_then(|lookup| lookup.get(&key).copied());
            match (current, count.is_zero()) {
                (Some(index), false) => self.groups[index].count = count,
                (Some(index), true) => self.remove_i64_group_at(index)?,
                (None, false) => {
                    let index = self.groups.len();
                    let mut bucket = Self::empty_bucket(vec![Value::I64(key)]);
                    bucket.count = count;
                    self.groups.push(bucket);
                    self.i64_lookup
                        .as_mut()
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                        .insert(key, index);
                }
                (None, true) => {}
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn remove_i64_group_at(&mut self, index: usize) -> Result<(), RelQueryError> {
        let [Value::I64(removed_key)] = self.groups[index].key.as_slice() else {
            return Err(RelQueryError::TypeMismatch);
        };
        self.i64_lookup
            .as_mut()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?
            .remove(removed_key);
        self.groups.swap_remove(index);
        if index < self.groups.len() {
            let [Value::I64(moved_key)] = self.groups[index].key.as_slice() else {
                return Err(RelQueryError::TypeMismatch);
            };
            self.i64_lookup
                .as_mut()
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                .insert(*moved_key, index);
        }
        Ok(())
    }

    #[cfg(test)]
    fn commit_generic_group_patch(
        &mut self,
        patch: GenericGroupPatch,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        for (key, next_bucket) in patch.planned {
            let current = self.find_group(&key, context, registry)?;
            match (current, next_bucket) {
                (Some(index), Some(bucket)) => self.groups[index] = bucket,
                (Some(index), None) => self.remove_group_at(index, registry)?,
                (None, Some(bucket)) => self.push_group_bucket(bucket, registry)?,
                (None, None) => {}
            }
        }
        if self.group_columns.is_empty() && self.groups.is_empty() {
            self.push_group_bucket(Self::empty_bucket(Vec::new()), registry)?;
        }
        Ok(())
    }

    fn empty_bucket(key: Row) -> MaintainedGroupBucket {
        MaintainedGroupBucket {
            key,
            count: kernel_aggregate::ExactCount::default(),
            sum: kernel_aggregate::ExactF64Sum::default(),
        }
    }

    fn key_for_row(&self, row: &Row) -> Result<Row, RelQueryError> {
        self.group_columns
            .iter()
            .map(|column| {
                row.get(*column)
                    .cloned()
                    .ok_or(RelQueryError::ColumnOutOfBounds)
            })
            .collect()
    }

    fn indexed_group_key(
        &self,
        key: &Row,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<IndexedGroupKey, RelQueryError> {
        if self.i64_lookup.is_some() {
            let [Value::I64(key)] = key.as_slice() else {
                return Err(RelQueryError::TypeMismatch);
            };
            return Ok(IndexedGroupKey::I64(*key));
        }
        Ok(IndexedGroupKey::Semantic(
            self.canonical_group_key(key, registry)?,
        ))
    }

    fn group_index_by_indexed_key(&self, key: &IndexedGroupKey) -> Option<usize> {
        match key {
            IndexedGroupKey::I64(key) => self.i64_lookup.as_ref()?.get(key).copied(),
            IndexedGroupKey::Semantic(key) => self.semantic_lookup.as_ref()?.get(key).copied(),
        }
    }

    fn canonical_group_key(
        &self,
        key: &Row,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Vec<kernel_semantics::CanonicalEqKey>, RelQueryError> {
        if let Some(encoders) = &self.group_encoders {
            if encoders.len() != key.len() {
                return Err(RelQueryError::EquivalenceArityMismatch);
            }
            return encoders
                .iter()
                .zip(key)
                .map(|(encoder, value)| encoder.canonical_key(value).map_err(RelQueryError::from))
                .collect();
        }
        if self.group_equivalences.len() != key.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        self.group_equivalences
            .iter()
            .zip(key)
            .map(|(equivalence, value)| {
                registry
                    .canonical_equivalence_key(&self.semantic_context, *equivalence, value)
                    .map_err(RelQueryError::from)
            })
            .collect()
    }

    fn find_group(
        &self,
        key: &Row,
        _context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<usize>, RelQueryError> {
        if let Some(lookup) = &self.i64_lookup {
            let [Value::I64(key)] = key.as_slice() else {
                return Err(RelQueryError::TypeMismatch);
            };
            return Ok(lookup.get(key).copied());
        }
        let lookup = self
            .semantic_lookup
            .as_ref()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let canonical = self.canonical_group_key(key, registry)?;
        Ok(lookup.get(&canonical).copied())
    }

    fn insert_row(
        &mut self,
        row: &Row,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        let key = self.key_for_row(row)?;
        let index = if let Some(index) = self.find_group(&key, context, registry)? {
            index
        } else {
            self.push_group_bucket(Self::empty_bucket(key), registry)?;
            self.groups.len() - 1
        };
        let aggregate = self.aggregate.clone();
        Self::add_to_bucket_with(&aggregate, &mut self.groups[index], row)?;
        Ok(())
    }

    fn apply_exact_weight_to_bucket(
        &self,
        bucket: &mut MaintainedGroupBucket,
        row: &Row,
        weight: &kernel_exact::ExactInteger,
    ) -> Result<(), RelQueryError> {
        Self::apply_exact_weight_to_bucket_with(&self.aggregate, bucket, row, weight)
    }

    fn apply_exact_weight_to_bucket_with(
        aggregate: &AggregateSpec,
        bucket: &mut MaintainedGroupBucket,
        row: &Row,
        weight: &kernel_exact::ExactInteger,
    ) -> Result<(), RelQueryError> {
        if weight.is_zero() {
            return Ok(());
        }
        let mut count = bucket.count.clone();
        if weight.is_negative() {
            count.remove_exact(weight.magnitude())?;
        } else {
            count.add_exact(weight.magnitude());
        }
        let mut sum = bucket.sum.clone();
        if let AggregateSpec::ExactF64Sum { value_column, .. } = aggregate {
            let Value::F64Bits(bits) = row
                .get(*value_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?
            else {
                return Err(RelQueryError::TypeMismatch);
            };
            if weight.is_negative() {
                sum.remove_exact(f64::from_bits(*bits), weight.magnitude())?;
            } else {
                sum.add_exact(f64::from_bits(*bits), weight.magnitude())?;
            }
        }
        bucket.count = count;
        bucket.sum = sum;
        Ok(())
    }

    fn apply_weight_to_bucket_with(
        aggregate: &AggregateSpec,
        bucket: &mut MaintainedGroupBucket,
        row: &Row,
        weight: i64,
    ) -> Result<(), RelQueryError> {
        if weight == 0 {
            return Ok(());
        }
        let magnitude = u128::from(weight.unsigned_abs());
        let mut count = bucket.count.clone();
        if weight < 0 {
            count.remove_many(magnitude)?;
        } else {
            count.add_many(magnitude);
        }
        let mut sum = bucket.sum.clone();
        if let AggregateSpec::ExactF64Sum { value_column, .. } = aggregate {
            let Value::F64Bits(bits) = row
                .get(*value_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?
            else {
                return Err(RelQueryError::TypeMismatch);
            };
            if weight < 0 {
                sum.remove_many(f64::from_bits(*bits), weight.unsigned_abs())?;
            } else {
                sum.add_many(f64::from_bits(*bits), weight.unsigned_abs())?;
            }
        }
        bucket.count = count;
        bucket.sum = sum;
        Ok(())
    }

    fn add_to_bucket_with(
        aggregate: &AggregateSpec,
        bucket: &mut MaintainedGroupBucket,
        row: &Row,
    ) -> Result<(), RelQueryError> {
        Self::apply_weight_to_bucket_with(aggregate, bucket, row, 1)
    }

    fn push_group_bucket(
        &mut self,
        bucket: MaintainedGroupBucket,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        let index = self.groups.len();
        if let Some(lookup) = &mut self.i64_lookup {
            let [Value::I64(key)] = bucket.key.as_slice() else {
                return Err(RelQueryError::TypeMismatch);
            };
            lookup.insert(*key, index);
        } else if self.semantic_lookup.is_some() {
            let canonical = self.canonical_group_key(&bucket.key, registry)?;
            if let Some(lookup) = &mut self.semantic_lookup {
                lookup.insert(canonical, index);
            }
        }
        self.groups.push(bucket);
        Ok(())
    }

    #[cfg(test)]
    fn remove_group_at(
        &mut self,
        index: usize,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        if let Some(lookup) = &mut self.i64_lookup {
            let [Value::I64(removed_key)] = self.groups[index].key.as_slice() else {
                return Err(RelQueryError::TypeMismatch);
            };
            lookup.remove(removed_key);
        } else if self.semantic_lookup.is_some() {
            let canonical = self.canonical_group_key(&self.groups[index].key, registry)?;
            if let Some(lookup) = &mut self.semantic_lookup {
                lookup.remove(&canonical);
            }
        }
        self.groups.swap_remove(index);
        if index < self.groups.len() {
            if let Some(lookup) = &mut self.i64_lookup {
                let [Value::I64(moved_key)] = self.groups[index].key.as_slice() else {
                    return Err(RelQueryError::TypeMismatch);
                };
                lookup.insert(*moved_key, index);
            } else if self.semantic_lookup.is_some() {
                let canonical = self.canonical_group_key(&self.groups[index].key, registry)?;
                if let Some(lookup) = &mut self.semantic_lookup {
                    lookup.insert(canonical, index);
                }
            }
        }
        Ok(())
    }

    fn output_for_bucket(
        &self,
        group: Option<&MaintainedGroupBucket>,
    ) -> Result<Option<Row>, RelQueryError> {
        let Some(group) = group else {
            return Ok(None);
        };
        let mut row = group.key.clone();
        let aggregate = match self.aggregate {
            AggregateSpec::Count { .. } => Value::I64(group.count.finish_i64()?),
            AggregateSpec::ExactF64Sum { .. } => Value::F64Bits(group.sum.finish().to_bits()),
        };
        row.push(aggregate);
        Ok(Some(row))
    }

    #[cfg(test)]
    pub(super) fn test_storage_sharing_with(&self, other: &Self) -> (bool, bool) {
        let groups = self.groups.shares_storage_with(&other.groups);
        let lookup = self
            .semantic_lookup
            .as_ref()
            .zip(other.semantic_lookup.as_ref())
            .is_some_and(|(left, right)| left.shares_root_with(right));
        (groups, lookup)
    }

    #[cfg(test)]
    pub(super) fn test_has_semantic_lookup(&self) -> bool {
        self.semantic_lookup.is_some()
    }

    #[cfg(test)]
    pub(super) fn test_group_encoder_count(&self) -> Option<usize> {
        self.group_encoders.as_ref().map(Vec::len)
    }

    #[cfg(test)]
    pub(super) fn test_dense_i64_count_enabled(&self) -> bool {
        self.dense_i64_count.is_some()
    }

    #[cfg(test)]
    pub(super) fn test_plan_legacy_effect<D: DeltaView<Row>>(
        &self,
        input_delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<AdaptiveDelta<Row, 4>, RelQueryError> {
        let exact = super::exact_delta_from_legacy(input_delta);
        let effect = self.plan_exact_effect(&exact, context, registry)?;
        super::exact_delta_to_legacy_checked(&effect)
    }

    #[cfg(test)]
    pub(super) fn test_plan_commit_i64_count<D: DeltaView<Row>>(
        &mut self,
        input_delta: &D,
    ) -> Result<GroupI64PlanObservation, RelQueryError> {
        let before = self.clone();
        let exact = super::exact_delta_from_legacy(input_delta);
        let planned = self.plan_exact_i64_count_delta(&exact)?;
        let observation = GroupI64PlanObservation {
            retain_dense: planned.patch.retain_dense,
            dense_move: planned.patch.dense_move,
            effect: super::exact_delta_to_legacy_checked(&planned.effect)?,
            unchanged_before_commit: *self == before,
        };
        self.commit_i64_count_patch(planned.patch)?;
        Ok(observation)
    }
}
