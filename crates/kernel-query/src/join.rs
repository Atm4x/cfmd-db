#[cfg(test)]
use super::{AdaptiveDelta, DeltaSink, DeltaView, exact_integer_to_i64};
use super::{
    CanonicalRowKey, ExactDelta, ExactDeltaSink, ExactDeltaView, MaterializedSetSupportState,
    PlannedDeltaEffect, RelExpr, RelQueryError, RelType, RelationDelta, RelationValue, Row,
    canonical_row_key, relation_column_equivalences, relation_value_from_rows,
    validate_exact_delta_view_rows,
};
use kernel_persistent::PersistentOrdMap;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CountedJoinClass {
    representative: Row,
    multiplicity: kernel_exact::ExactNatural,
}

type CountedJoinBucket = PersistentOrdMap<CanonicalRowKey, CountedJoinClass>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct CountedJoinSide {
    buckets: PersistentOrdMap<kernel_semantics::CanonicalEqKey, CountedJoinBucket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CountedJoinStorage {
    left: CountedJoinSide,
    right: CountedJoinSide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CountedJoinDeltaAtom {
    join_key: kernel_semantics::CanonicalEqKey,
    row_key: CanonicalRowKey,
    representative: Row,
    weight: kernel_exact::ExactInteger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CountedJoinMutationPlan {
    next: CountedJoinSide,
    delta: Vec<CountedJoinDeltaAtom>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MaintainedJoinStorage {
    Counted(Box<CountedJoinStorage>),
}

#[derive(Debug)]
pub(super) struct JoinDeltaPatch {
    left: CountedJoinSide,
    right: CountedJoinSide,
}

#[derive(Clone, Copy)]
struct GenericJoinMaintenanceSpec<'a> {
    left_column: usize,
    right_column: usize,
    left_type: &'a RelType,
    right_type: &'a RelType,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

impl CountedJoinSide {
    fn build(
        rows: &[Row],
        join_column: usize,
        equivalence: kernel_types::SemanticId,
        ty: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let row_equivalences = relation_column_equivalences(ty);
        let mut side = Self::default();
        for row in rows {
            let join_key = Self::join_key(row, join_column, equivalence, context, registry)?;
            let row_key = canonical_row_key(row, row_equivalences, context, registry)?;
            let mut bucket = side.buckets.get(&join_key).cloned().unwrap_or_default();
            if let Some(mut class) = bucket.get(&row_key).cloned() {
                class.multiplicity.add_u128(1);
                bucket.insert(row_key, class);
            } else {
                bucket.insert(
                    row_key,
                    CountedJoinClass {
                        representative: row.clone(),
                        multiplicity: kernel_exact::ExactNatural::one(),
                    },
                );
            }
            side.buckets.insert(join_key, bucket);
        }
        Ok(side)
    }

    fn join_key(
        row: &Row,
        join_column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<kernel_semantics::CanonicalEqKey, RelQueryError> {
        registry
            .canonical_equivalence_key(
                context,
                equivalence,
                row.get(join_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?,
            )
            .map_err(Into::into)
    }

    fn bucket(&self, key: &kernel_semantics::CanonicalEqKey) -> Option<&CountedJoinBucket> {
        self.buckets.get(key)
    }

    fn plan_mutation_view<D: ExactDeltaView<Row>>(
        &self,
        delta: &D,
        join_column: usize,
        equivalence: kernel_types::SemanticId,
        ty: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<CountedJoinMutationPlan, RelQueryError> {
        let row_equivalences = relation_column_equivalences(ty);
        let mut classes = BTreeMap::<CanonicalRowKey, CountedJoinDeltaAtom>::new();
        let mut error = None;
        delta.visit_exact(|weight, row| {
            if weight.is_zero() || error.is_some() {
                return;
            }
            let join_key = match Self::join_key(row, join_column, equivalence, context, registry) {
                Ok(key) => key,
                Err(next) => {
                    error = Some(next);
                    return;
                }
            };
            let row_key = match canonical_row_key(row, row_equivalences, context, registry) {
                Ok(key) => key,
                Err(next) => {
                    error = Some(next);
                    return;
                }
            };
            let coefficient = weight.clone();
            match classes.entry(row_key.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(CountedJoinDeltaAtom {
                        join_key,
                        row_key,
                        representative: row.clone(),
                        weight: coefficient,
                    });
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if entry.get().join_key != join_key {
                        error = Some(RelQueryError::InconsistentIncrementalDelta);
                        return;
                    }
                    entry.get_mut().weight.add_assign(&coefficient);
                }
            }
        });
        if let Some(error) = error {
            return Err(error);
        }

        let is_set = matches!(ty.semantics, kernel_schema::RelationSemantics::Set { .. });
        let mut next = self.clone();
        let mut normalized = Vec::with_capacity(classes.len());
        for (_, atom) in classes {
            if atom.weight.is_zero() {
                continue;
            }
            let mut bucket = next
                .buckets
                .get(&atom.join_key)
                .cloned()
                .unwrap_or_default();
            let existing = bucket.get(&atom.row_key).cloned();
            let mut multiplicity = existing
                .as_ref()
                .map_or_else(kernel_exact::ExactNatural::zero, |class| {
                    class.multiplicity.clone()
                });
            if atom.weight.is_negative() {
                if !multiplicity.checked_sub_assign(atom.weight.magnitude()) {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
            } else {
                multiplicity.add_assign(atom.weight.magnitude());
            }
            if is_set && multiplicity > kernel_exact::ExactNatural::one() {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            if multiplicity.is_zero() {
                bucket.remove(&atom.row_key);
            } else {
                bucket.insert(
                    atom.row_key.clone(),
                    CountedJoinClass {
                        representative: existing.map_or_else(
                            || atom.representative.clone(),
                            |class| class.representative,
                        ),
                        multiplicity,
                    },
                );
            }
            if bucket.is_empty() {
                next.buckets.remove(&atom.join_key);
            } else {
                next.buckets.insert(atom.join_key.clone(), bucket);
            }
            normalized.push(atom);
        }
        Ok(CountedJoinMutationPlan {
            next,
            delta: normalized,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedJoinDeltaState {
    query: RelExpr,
    semantic_context: kernel_schema::SemanticContext,
    left_type: RelType,
    right_type: RelType,
    result_type: RelType,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    storage: MaintainedJoinStorage,
}

impl MaterializedJoinDeltaState {
    pub fn build(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        query.prepare(context, registry)?;
        let RelExpr::JoinEq { left, right, .. } = query else {
            return Ok(None);
        };
        let left_value = left.evaluate(old, context, registry)?;
        let right_value = right.evaluate(old, context, registry)?;
        Self::build_from_input_values(query, &left_value, &right_value, context, registry)
    }

    fn build_counted_storage(
        left_value: &RelationValue,
        right_value: &RelationValue,
        equivalence: kernel_types::SemanticId,
        spec: GenericJoinMaintenanceSpec<'_>,
    ) -> Result<CountedJoinStorage, RelQueryError> {
        Ok(CountedJoinStorage {
            left: CountedJoinSide::build(
                left_value.rows(),
                spec.left_column,
                equivalence,
                spec.left_type,
                spec.context,
                spec.registry,
            )?,
            right: CountedJoinSide::build(
                right_value.rows(),
                spec.right_column,
                equivalence,
                spec.right_type,
                spec.context,
                spec.registry,
            )?,
        })
    }

    pub(super) fn build_from_input_values(
        query: &RelExpr,
        left_value: &RelationValue,
        right_value: &RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        query.prepare(context, registry)?;
        let RelExpr::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } = query
        else {
            return Ok(None);
        };
        let left_type = left.typecheck(context, registry)?;
        let right_type = right.typecheck(context, registry)?;
        let result_type = query.typecheck(context, registry)?;
        MaterializedSetSupportState::validate_rows(
            left_value.rows(),
            &left_type,
            context,
            registry,
        )?;
        MaterializedSetSupportState::validate_rows(
            right_value.rows(),
            &right_type,
            context,
            registry,
        )?;
        let spec = GenericJoinMaintenanceSpec {
            left_column: *left_column,
            right_column: *right_column,
            left_type: &left_type,
            right_type: &right_type,
            context,
            registry,
        };
        let storage = MaintainedJoinStorage::Counted(Box::new(Self::build_counted_storage(
            left_value,
            right_value,
            *equivalence,
            spec,
        )?));
        Ok(Some(Self {
            query: query.clone(),
            semantic_context: context.clone(),
            left_type,
            right_type,
            result_type,
            left_column: *left_column,
            right_column: *right_column,
            equivalence: *equivalence,
            storage,
        }))
    }

    pub(super) fn output_value(
        &self,
        _context: &kernel_schema::SemanticContext,
        _registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        let MaintainedJoinStorage::Counted(storage) = &self.storage;
        let mut rows = Vec::new();
        for (join_key, left_bucket) in &storage.left.buckets {
            let Some(right_bucket) = storage.right.bucket(join_key) else {
                continue;
            };
            for left_class in left_bucket.values() {
                for right_class in right_bucket.values() {
                    let multiplicity = left_class
                        .multiplicity
                        .multiplied(&right_class.multiplicity)
                        .to_u64()
                        .and_then(|value| usize::try_from(value).ok())
                        .ok_or(RelQueryError::DerivedIdentityExhausted)?;
                    let joined =
                        Self::join_pair(&left_class.representative, &right_class.representative);
                    rows.extend(std::iter::repeat_n(joined, multiplicity));
                }
            }
        }
        Ok(relation_value_from_rows(rows, &self.result_type))
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
        matches!(self.storage, MaintainedJoinStorage::Counted(_))
    }

    #[cfg(test)]
    pub(super) fn test_first_right_multiplicity_u64(&self) -> Option<u64> {
        let MaintainedJoinStorage::Counted(storage) = &self.storage;
        storage
            .right
            .buckets
            .values()
            .next()
            .and_then(|bucket| bucket.values().next())
            .and_then(|class| class.multiplicity.to_u64())
    }

    #[cfg(test)]
    pub(super) fn test_left_bucket_stats(&self) -> (usize, usize) {
        let MaintainedJoinStorage::Counted(storage) = &self.storage;
        (
            storage.left.buckets.len(),
            storage
                .left
                .buckets
                .values()
                .next()
                .map_or(0, PersistentOrdMap::len),
        )
    }

    #[cfg(test)]
    pub(super) fn test_shares_storage_with(&self, other: &Self) -> (bool, bool) {
        let MaintainedJoinStorage::Counted(left) = &self.storage;
        let MaintainedJoinStorage::Counted(right) = &other.storage;
        (
            left.left.buckets.shares_root_with(&right.left.buckets),
            left.right.buckets.shares_root_with(&right.right.buckets),
        )
    }

    #[cfg(test)]
    pub(super) fn test_plan_left_mutation_stats(
        &self,
        delta: &RelationDelta,
        column: usize,
        equivalence: kernel_types::SemanticId,
        ty: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(usize, bool, usize), RelQueryError> {
        let MaintainedJoinStorage::Counted(storage) = &self.storage;
        let plan = storage.left.plan_mutation_view(
            &delta.as_delta_view(),
            column,
            equivalence,
            ty,
            context,
            registry,
        )?;
        Ok((
            plan.delta.len(),
            plan.delta
                .first()
                .is_some_and(|atom| atom.weight.is_negative()),
            plan.next
                .buckets
                .values()
                .next()
                .map_or(0, PersistentOrdMap::len),
        ))
    }

    pub fn apply_input_deltas(
        &mut self,
        left_delta: &RelationDelta,
        right_delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if left_delta.result_type != self.left_type || right_delta.result_type != self.right_type {
            return Err(RelQueryError::TypeMismatch);
        }
        let planned = self.plan_exact_delta_views(
            &left_delta.as_delta_view(),
            &right_delta.as_delta_view(),
            context,
            registry,
        )?;
        let effect = self.materialize_exact_effect(&planned.effect)?;
        self.commit_join_patch(planned.patch);
        Ok(effect)
    }

    #[cfg(test)]
    pub(super) fn plan_delta_views<
        LD: DeltaView<Row> + ExactDeltaView<Row>,
        RD: DeltaView<Row> + ExactDeltaView<Row>,
    >(
        &self,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<JoinDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        let exact = self.plan_exact_delta_views(left_delta, right_delta, context, registry)?;
        Ok(PlannedDeltaEffect {
            patch: exact.patch,
            effect: Self::exact_effect_to_legacy(&exact.effect)?,
        })
    }

    pub(super) fn plan_exact_delta_views<LD: ExactDeltaView<Row>, RD: ExactDeltaView<Row>>(
        &self,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<JoinDeltaPatch, ExactDelta<Row>>, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        validate_exact_delta_view_rows(left_delta, &self.left_type, context, registry)?;
        validate_exact_delta_view_rows(right_delta, &self.right_type, context, registry)?;
        let MaintainedJoinStorage::Counted(storage) = &self.storage;
        let left_plan = storage.left.plan_mutation_view(
            left_delta,
            self.left_column,
            self.equivalence,
            &self.left_type,
            context,
            registry,
        )?;
        let right_plan = storage.right.plan_mutation_view(
            right_delta,
            self.right_column,
            self.equivalence,
            &self.right_type,
            context,
            registry,
        )?;
        let effect = self.plan_exact_join_effect(
            &storage.right,
            &left_plan,
            &right_plan,
            context,
            registry,
        )?;
        Ok(PlannedDeltaEffect {
            patch: JoinDeltaPatch {
                left: left_plan.next,
                right: right_plan.next,
            },
            effect,
        })
    }

    pub(super) fn commit_join_patch(&mut self, patch: JoinDeltaPatch) {
        let MaintainedJoinStorage::Counted(storage) = &mut self.storage;
        storage.left = patch.left;
        storage.right = patch.right;
    }

    fn join_pair(left: &Row, right: &Row) -> Row {
        let mut joined = Vec::with_capacity(left.len() + right.len());
        joined.extend(left.iter().cloned());
        joined.extend(right.iter().cloned());
        joined
    }

    fn plan_exact_join_effect(
        &self,
        old_right: &CountedJoinSide,
        left_plan: &CountedJoinMutationPlan,
        right_plan: &CountedJoinMutationPlan,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<ExactDelta<Row>, RelQueryError> {
        let result_equivalences = relation_column_equivalences(&self.result_type);
        let mut quotient = BTreeMap::<CanonicalRowKey, (Row, kernel_exact::ExactInteger)>::new();
        let mut accumulate =
            |row: Row, weight: kernel_exact::ExactInteger| -> Result<(), RelQueryError> {
                if weight.is_zero() {
                    return Ok(());
                }
                let key = canonical_row_key(&row, result_equivalences, context, registry)?;
                match quotient.entry(key) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert((row, weight));
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        entry.get_mut().1.add_assign(&weight);
                    }
                }
                Ok(())
            };

        // Bilinear differential law: dL ⋈ R + (L + dL) ⋈ dR.
        for atom in &left_plan.delta {
            let Some(right_bucket) = old_right.bucket(&atom.join_key) else {
                continue;
            };
            for right_class in right_bucket.values() {
                accumulate(
                    Self::join_pair(&atom.representative, &right_class.representative),
                    atom.weight.scale_by_natural(&right_class.multiplicity),
                )?;
            }
        }
        for atom in &right_plan.delta {
            let Some(left_bucket) = left_plan.next.bucket(&atom.join_key) else {
                continue;
            };
            for left_class in left_bucket.values() {
                accumulate(
                    Self::join_pair(&left_class.representative, &atom.representative),
                    atom.weight.scale_by_natural(&left_class.multiplicity),
                )?;
            }
        }

        let mut effect = ExactDelta::with_capacity(quotient.len());
        for (_, (row, weight)) in quotient {
            effect.push_exact(weight, row);
        }
        Ok(effect)
    }

    #[cfg(test)]
    pub(super) fn exact_integer_to_i64(weight: &kernel_exact::ExactInteger) -> Option<i64> {
        exact_integer_to_i64(weight)
    }

    #[cfg(test)]
    fn exact_effect_to_legacy(
        effect: &ExactDelta<Row>,
    ) -> Result<AdaptiveDelta<Row, 4>, RelQueryError> {
        let mut legacy = AdaptiveDelta::default();
        let mut error = None;
        effect.visit_exact(|weight, row| {
            if error.is_some() {
                return;
            }
            let Some(weight) = Self::exact_integer_to_i64(weight) else {
                error = Some(RelQueryError::DerivedIdentityExhausted);
                return;
            };
            legacy.push_weighted(weight, row.clone());
        });
        error.map_or(Ok(legacy), Err)
    }

    fn materialize_exact_effect(
        &self,
        effect: &ExactDelta<Row>,
    ) -> Result<RelationDelta, RelQueryError> {
        let mut inserted = Vec::new();
        let mut removed = Vec::new();
        let mut error = None;
        effect.visit_exact(|weight, row| {
            if error.is_some() || weight.is_zero() {
                return;
            }
            let Some(magnitude) = weight
                .magnitude()
                .to_u64()
                .and_then(|value| usize::try_from(value).ok())
            else {
                error = Some(RelQueryError::DerivedIdentityExhausted);
                return;
            };
            if weight.is_negative() {
                removed.extend(std::iter::repeat_n(row.clone(), magnitude));
            } else {
                inserted.extend(std::iter::repeat_n(row.clone(), magnitude));
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        Ok(RelationDelta {
            result_type: self.result_type.clone(),
            inserted,
            removed,
        })
    }
}
