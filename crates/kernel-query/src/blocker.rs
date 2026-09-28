#[cfg(test)]
use super::{AdaptiveDelta, DeltaView, exact_delta_from_legacy, exact_delta_to_legacy_checked};
use super::{
    CanonicalRowKey, ExactDelta, ExactDeltaSink, ExactDeltaView, MaterializedSetSupportState,
    PlannedDeltaEffect, RelQueryError, RelType, RelationValue, Row, apply_exact_to_natural,
    canonical_row_key, exact_natural_difference, relation_column_equivalences,
    relation_value_from_rows, validate_exact_delta_view_rows,
};
use kernel_persistent::PersistentOrdMap;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MaintainedBlockerKind {
    Difference,
    AntiJoin {
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct DifferenceBlockerClass {
    representative: Option<Row>,
    left_count: kernel_exact::ExactNatural,
    right_count: kernel_exact::ExactNatural,
}

impl DifferenceBlockerClass {
    fn output_count(&self) -> kernel_exact::ExactNatural {
        let mut count = self.left_count.clone();
        if !count.checked_sub_assign(&self.right_count) {
            return kernel_exact::ExactNatural::zero();
        }
        count
    }

    fn representative(&self) -> Option<&Row> {
        self.representative.as_ref()
    }

    fn is_empty(&self) -> bool {
        self.left_count.is_zero() && self.right_count.is_zero()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CountedBlockerRowClass {
    representative: Row,
    multiplicity: kernel_exact::ExactNatural,
}

type CountedBlockerRows = PersistentOrdMap<CanonicalRowKey, CountedBlockerRowClass>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct AntiJoinBlockerClass {
    left: CountedBlockerRows,
    right_count: kernel_exact::ExactNatural,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MaintainedBlockerStorage {
    Difference {
        equivalences: Vec<kernel_types::SemanticId>,
        classes: PersistentOrdMap<CanonicalRowKey, Arc<DifferenceBlockerClass>>,
    },
    AntiJoin {
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        left_equivalences: Vec<kernel_types::SemanticId>,
        classes: PersistentOrdMap<kernel_semantics::CanonicalEqKey, Arc<AntiJoinBlockerClass>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MaterializedBlockerDeltaState {
    left_type: RelType,
    right_type: RelType,
    result_type: RelType,
    semantic_context: kernel_schema::SemanticContext,
    storage: MaintainedBlockerStorage,
}

#[derive(Debug)]
pub(super) struct BlockerDeltaPatch(BlockerDeltaPatchInner);

#[derive(Debug)]
enum BlockerDeltaPatchInner {
    Difference(Vec<(CanonicalRowKey, DifferenceBlockerClass)>),
    AntiJoin(Vec<(kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass)>),
}

#[cfg(test)]
impl BlockerDeltaPatch {
    pub(super) fn test_first_difference_counts(
        &self,
    ) -> Option<(kernel_exact::ExactNatural, kernel_exact::ExactNatural)> {
        match &self.0 {
            BlockerDeltaPatchInner::Difference(writes) => writes
                .first()
                .map(|(_, class)| (class.left_count.clone(), class.right_count.clone())),
            BlockerDeltaPatchInner::AntiJoin(_) => None,
        }
    }
}

pub(super) struct BlockerBuildSpec<'a> {
    pub(super) kind: &'a MaintainedBlockerKind,
    pub(super) left_type: RelType,
    pub(super) right_type: RelType,
    pub(super) result_type: RelType,
    pub(super) context: &'a kernel_schema::SemanticContext,
    pub(super) registry: &'a kernel_semantics::SemanticRegistry,
}

#[derive(Clone, Copy)]
enum DifferenceBlockerSide {
    Left,
    Right,
}

type DifferenceBlockerWrites = Vec<(CanonicalRowKey, DifferenceBlockerClass)>;
type DifferenceBlockerEffect = (ExactDelta<Row>, DifferenceBlockerWrites);
type AntiJoinBlockerWrites = Vec<(kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass)>;
type AntiJoinBlockerEffect = (ExactDelta<Row>, AntiJoinBlockerWrites);
type AntiJoinChangeMap = BTreeMap<
    kernel_semantics::CanonicalEqKey,
    BTreeMap<CanonicalRowKey, (Row, kernel_exact::ExactInteger)>,
>;

struct AntiJoinLeftChanges {
    inserted: AntiJoinChangeMap,
}

impl MaterializedBlockerDeltaState {
    pub(super) fn result_type(&self) -> &RelType {
        &self.result_type
    }

    pub(super) fn build(
        left: &RelationValue,
        right: &RelationValue,
        spec: BlockerBuildSpec<'_>,
    ) -> Result<Self, RelQueryError> {
        let BlockerBuildSpec {
            kind,
            left_type,
            right_type,
            result_type,
            context,
            registry,
        } = spec;
        MaterializedSetSupportState::validate_rows(left.rows(), &left_type, context, registry)?;
        MaterializedSetSupportState::validate_rows(right.rows(), &right_type, context, registry)?;
        let storage = match kind {
            MaintainedBlockerKind::Difference => {
                let equivalences = relation_column_equivalences(&result_type).to_vec();
                let mut classes = BTreeMap::<CanonicalRowKey, DifferenceBlockerClass>::new();
                for row in left.rows() {
                    let key = canonical_row_key(row, &equivalences, context, registry)?;
                    let class = classes.entry(key).or_default();
                    class
                        .left_count
                        .add_assign(&kernel_exact::ExactNatural::one());
                    class.representative.get_or_insert_with(|| row.clone());
                }
                for row in right.rows() {
                    let key = canonical_row_key(row, &equivalences, context, registry)?;
                    let class = classes.entry(key).or_default();
                    class
                        .right_count
                        .add_assign(&kernel_exact::ExactNatural::one());
                }
                MaintainedBlockerStorage::Difference {
                    equivalences,
                    classes: classes
                        .into_iter()
                        .map(|(key, class)| (key, Arc::new(class)))
                        .collect(),
                }
            }
            MaintainedBlockerKind::AntiJoin {
                left_column,
                right_column,
                equivalence,
            } => {
                let left_equivalences = relation_column_equivalences(&left_type).to_vec();
                let mut classes =
                    BTreeMap::<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>::new();
                for row in left.rows() {
                    let key =
                        Self::anti_join_key(row, *left_column, *equivalence, context, registry)?;
                    let row_key = canonical_row_key(row, &left_equivalences, context, registry)?;
                    let class = classes.entry(key).or_insert_with(|| AntiJoinBlockerClass {
                        left: PersistentOrdMap::default(),
                        right_count: kernel_exact::ExactNatural::zero(),
                    });
                    let mut row_class = class.left.get(&row_key).cloned().unwrap_or_else(|| {
                        CountedBlockerRowClass {
                            representative: row.clone(),
                            multiplicity: kernel_exact::ExactNatural::zero(),
                        }
                    });
                    row_class
                        .multiplicity
                        .add_assign(&kernel_exact::ExactNatural::one());
                    class.left.insert(row_key, row_class);
                }
                for row in right.rows() {
                    let key =
                        Self::anti_join_key(row, *right_column, *equivalence, context, registry)?;
                    classes
                        .entry(key)
                        .or_insert_with(|| AntiJoinBlockerClass {
                            left: PersistentOrdMap::default(),
                            right_count: kernel_exact::ExactNatural::zero(),
                        })
                        .right_count
                        .add_assign(&kernel_exact::ExactNatural::one());
                }
                MaintainedBlockerStorage::AntiJoin {
                    left_column: *left_column,
                    right_column: *right_column,
                    equivalence: *equivalence,
                    left_equivalences,
                    classes: classes
                        .into_iter()
                        .map(|(key, class)| (key, Arc::new(class)))
                        .collect(),
                }
            }
        };
        Ok(Self {
            left_type,
            right_type,
            result_type,
            semantic_context: context.clone(),
            storage,
        })
    }

    fn anti_join_key(
        row: &Row,
        column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<kernel_semantics::CanonicalEqKey, RelQueryError> {
        let value = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?;
        registry
            .canonical_equivalence_key(context, equivalence, value)
            .map_err(RelQueryError::from)
    }

    pub(super) fn output_value(&self) -> Result<RelationValue, RelQueryError> {
        let mut rows = Vec::new();
        match &self.storage {
            MaintainedBlockerStorage::Difference { classes, .. } => {
                for class in classes.values() {
                    let count = class.output_count();
                    if count.is_zero() {
                        continue;
                    }
                    if let Some(row) = class.representative() {
                        let count = count
                            .to_u64()
                            .and_then(|count| usize::try_from(count).ok())
                            .ok_or(RelQueryError::DerivedIdentityExhausted)?;
                        rows.extend(std::iter::repeat_n(row.clone(), count));
                    }
                }
            }
            MaintainedBlockerStorage::AntiJoin { classes, .. } => {
                for class in classes.values() {
                    if class.right_count.is_zero() {
                        for row_class in class.left.values() {
                            let count = row_class
                                .multiplicity
                                .to_u64()
                                .and_then(|count| usize::try_from(count).ok())
                                .ok_or(RelQueryError::DerivedIdentityExhausted)?;
                            rows.extend(std::iter::repeat_n(
                                row_class.representative.clone(),
                                count,
                            ));
                        }
                    }
                }
            }
        }
        Ok(relation_value_from_rows(rows, &self.result_type))
    }

    #[cfg(test)]
    pub(super) fn plan_delta_views<LD: DeltaView<Row>, RD: DeltaView<Row>>(
        &self,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<BlockerDeltaPatch, AdaptiveDelta<Row, 4>>, RelQueryError> {
        let left = exact_delta_from_legacy(left_delta);
        let right = exact_delta_from_legacy(right_delta);
        let planned = self.plan_exact_delta_views(&left, &right, context, registry)?;
        Ok(PlannedDeltaEffect {
            patch: planned.patch,
            effect: exact_delta_to_legacy_checked(&planned.effect)?,
        })
    }

    pub(super) fn plan_exact_delta_views<LD: ExactDeltaView<Row>, RD: ExactDeltaView<Row>>(
        &self,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<BlockerDeltaPatch, ExactDelta<Row>>, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        validate_exact_delta_view_rows(left_delta, &self.left_type, context, registry)?;
        validate_exact_delta_view_rows(right_delta, &self.right_type, context, registry)?;
        match &self.storage {
            MaintainedBlockerStorage::Difference {
                equivalences,
                classes,
            } => Self::plan_exact_difference(
                equivalences,
                classes,
                left_delta,
                right_delta,
                context,
                registry,
            ),
            MaintainedBlockerStorage::AntiJoin {
                left_column,
                right_column,
                equivalence,
                left_equivalences,
                classes,
            } => Self::plan_exact_anti_join(
                *left_column,
                *right_column,
                *equivalence,
                left_equivalences,
                classes,
                left_delta,
                right_delta,
                context,
                registry,
            ),
        }
    }

    fn plan_exact_difference<LD: ExactDeltaView<Row>, RD: ExactDeltaView<Row>>(
        equivalences: &[kernel_types::SemanticId],
        classes: &PersistentOrdMap<CanonicalRowKey, Arc<DifferenceBlockerClass>>,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<BlockerDeltaPatch, ExactDelta<Row>>, RelQueryError> {
        let mut local = BTreeMap::<CanonicalRowKey, DifferenceBlockerClass>::new();
        Self::apply_exact_difference_delta(
            &mut local,
            classes,
            equivalences,
            left_delta,
            DifferenceBlockerSide::Left,
            context,
            registry,
        )?;
        Self::apply_exact_difference_delta(
            &mut local,
            classes,
            equivalences,
            right_delta,
            DifferenceBlockerSide::Right,
            context,
            registry,
        )?;
        let (effect, writes) = Self::difference_effect(classes, local)?;
        Ok(PlannedDeltaEffect {
            patch: BlockerDeltaPatch(BlockerDeltaPatchInner::Difference(writes)),
            effect,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_exact_difference_delta<D: ExactDeltaView<Row>>(
        local: &mut BTreeMap<CanonicalRowKey, DifferenceBlockerClass>,
        classes: &PersistentOrdMap<CanonicalRowKey, Arc<DifferenceBlockerClass>>,
        equivalences: &[kernel_types::SemanticId],
        delta: &D,
        side: DifferenceBlockerSide,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        let mut error = None;
        delta.visit_exact(|weight, row| {
            if error.is_some() || weight.is_zero() {
                return;
            }
            let result = (|| {
                let key = canonical_row_key(row, equivalences, context, registry)?;
                let class = local.entry(key.clone()).or_insert_with(|| {
                    classes
                        .get(&key)
                        .map_or_else(DifferenceBlockerClass::default, |class| {
                            class.as_ref().clone()
                        })
                });
                let count = match side {
                    DifferenceBlockerSide::Left => &mut class.left_count,
                    DifferenceBlockerSide::Right => &mut class.right_count,
                };
                apply_exact_to_natural(count, weight)?;
                if matches!(side, DifferenceBlockerSide::Left) {
                    if class.left_count.is_zero() {
                        class.representative = None;
                    } else {
                        class.representative.get_or_insert_with(|| row.clone());
                    }
                }
                Ok::<(), RelQueryError>(())
            })();
            if let Err(err) = result {
                error = Some(err);
            }
        });
        error.map_or(Ok(()), Err)
    }

    fn difference_effect(
        classes: &PersistentOrdMap<CanonicalRowKey, Arc<DifferenceBlockerClass>>,
        local: BTreeMap<CanonicalRowKey, DifferenceBlockerClass>,
    ) -> Result<DifferenceBlockerEffect, RelQueryError> {
        let mut effect = ExactDelta::<Row>::default();
        let mut writes = Vec::with_capacity(local.len());
        for (key, after) in local {
            let before = classes
                .get(&key)
                .map_or(DifferenceBlockerClass::default(), |class| {
                    class.as_ref().clone()
                });
            let before_count = before.output_count();
            let after_count = after.output_count();
            let weight = exact_natural_difference(&after_count, &before_count);
            if !weight.is_zero() {
                let row = if weight.is_negative() {
                    before.representative()
                } else {
                    after.representative()
                }
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                effect.push_exact(weight, row.clone());
            }
            writes.push((key, after));
        }
        Ok((effect, writes))
    }

    fn empty_anti_join_class() -> AntiJoinBlockerClass {
        AntiJoinBlockerClass {
            left: PersistentOrdMap::default(),
            right_count: kernel_exact::ExactNatural::zero(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_exact_anti_join<LD: ExactDeltaView<Row>, RD: ExactDeltaView<Row>>(
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        left_equivalences: &[kernel_types::SemanticId],
        classes: &PersistentOrdMap<kernel_semantics::CanonicalEqKey, Arc<AntiJoinBlockerClass>>,
        left_delta: &LD,
        right_delta: &RD,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<BlockerDeltaPatch, ExactDelta<Row>>, RelQueryError> {
        let mut local = BTreeMap::<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>::new();
        let changes = Self::apply_exact_anti_join_left_delta(
            &mut local,
            classes,
            left_delta,
            left_column,
            equivalence,
            left_equivalences,
            context,
            registry,
        )?;
        Self::apply_exact_anti_join_right_delta(
            &mut local,
            classes,
            right_delta,
            right_column,
            equivalence,
            context,
            registry,
        )?;
        let (effect, writes) = Self::anti_join_effect(classes, local, &changes);
        Ok(PlannedDeltaEffect {
            patch: BlockerDeltaPatch(BlockerDeltaPatchInner::AntiJoin(writes)),
            effect,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_exact_anti_join_left_delta<D: ExactDeltaView<Row>>(
        local: &mut BTreeMap<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>,
        classes: &PersistentOrdMap<kernel_semantics::CanonicalEqKey, Arc<AntiJoinBlockerClass>>,
        delta: &D,
        left_column: usize,
        equivalence: kernel_types::SemanticId,
        left_equivalences: &[kernel_types::SemanticId],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<AntiJoinLeftChanges, RelQueryError> {
        let mut changes = AntiJoinChangeMap::new();
        let mut error = None;
        delta.visit_exact(|weight, row| {
            if error.is_some() || weight.is_zero() {
                return;
            }
            let result = (|| {
                let join_key =
                    Self::anti_join_key(row, left_column, equivalence, context, registry)?;
                let row_key = canonical_row_key(row, left_equivalences, context, registry)?;
                let class = local.entry(join_key.clone()).or_insert_with(|| {
                    classes
                        .get(&join_key)
                        .map_or_else(Self::empty_anti_join_class, |class| class.as_ref().clone())
                });
                let existing = class.left.get(&row_key).cloned();
                let mut multiplicity = existing
                    .as_ref()
                    .map_or_else(kernel_exact::ExactNatural::zero, |class| {
                        class.multiplicity.clone()
                    });
                apply_exact_to_natural(&mut multiplicity, weight)?;
                if multiplicity.is_zero() {
                    class.left.remove(&row_key);
                } else {
                    class.left.insert(
                        row_key.clone(),
                        CountedBlockerRowClass {
                            representative: existing
                                .map_or_else(|| row.clone(), |class| class.representative),
                            multiplicity,
                        },
                    );
                }
                let row_changes = changes.entry(join_key).or_default();
                let entry = row_changes
                    .entry(row_key)
                    .or_insert_with(|| (row.clone(), kernel_exact::ExactInteger::default()));
                entry.1.add_assign(weight);
                Ok::<(), RelQueryError>(())
            })();
            if let Err(err) = result {
                error = Some(err);
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        for changes in changes.values_mut() {
            changes.retain(|_, (_, weight)| !weight.is_zero());
        }
        Ok(AntiJoinLeftChanges { inserted: changes })
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_exact_anti_join_right_delta<D: ExactDeltaView<Row>>(
        local: &mut BTreeMap<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>,
        classes: &PersistentOrdMap<kernel_semantics::CanonicalEqKey, Arc<AntiJoinBlockerClass>>,
        delta: &D,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        let mut error = None;
        delta.visit_exact(|weight, row| {
            if error.is_some() || weight.is_zero() {
                return;
            }
            let result = (|| {
                let key = Self::anti_join_key(row, right_column, equivalence, context, registry)?;
                let class = local.entry(key.clone()).or_insert_with(|| {
                    classes
                        .get(&key)
                        .map_or_else(Self::empty_anti_join_class, |class| class.as_ref().clone())
                });
                apply_exact_to_natural(&mut class.right_count, weight)
            })();
            if let Err(err) = result {
                error = Some(err);
            }
        });
        error.map_or(Ok(()), Err)
    }

    fn anti_join_effect(
        classes: &PersistentOrdMap<kernel_semantics::CanonicalEqKey, Arc<AntiJoinBlockerClass>>,
        local: BTreeMap<kernel_semantics::CanonicalEqKey, AntiJoinBlockerClass>,
        changes: &AntiJoinLeftChanges,
    ) -> AntiJoinBlockerEffect {
        let mut effect = ExactDelta::<Row>::default();
        let mut writes = Vec::with_capacity(local.len());
        for (key, after) in local {
            let before = classes
                .get(&key)
                .map_or_else(Self::empty_anti_join_class, |class| class.as_ref().clone());
            match (before.right_count.is_zero(), after.right_count.is_zero()) {
                (true, true) => {
                    Self::append_anti_join_changes(&mut effect, changes.inserted.get(&key));
                }
                (true, false) => {
                    for class in before.left.values() {
                        effect.push_exact(
                            kernel_exact::ExactInteger::from_parts(
                                true,
                                class.multiplicity.clone(),
                            ),
                            class.representative.clone(),
                        );
                    }
                }
                (false, true) => {
                    for class in after.left.values() {
                        effect.push_exact(
                            kernel_exact::ExactInteger::from_parts(
                                false,
                                class.multiplicity.clone(),
                            ),
                            class.representative.clone(),
                        );
                    }
                }
                (false, false) => {}
            }
            writes.push((key, after));
        }
        (effect, writes)
    }

    fn append_anti_join_changes(
        effect: &mut ExactDelta<Row>,
        changes: Option<&BTreeMap<CanonicalRowKey, (Row, kernel_exact::ExactInteger)>>,
    ) {
        let Some(changes) = changes else {
            return;
        };
        for (row, weight) in changes.values() {
            effect.push_exact(weight.clone(), row.clone());
        }
    }

    #[cfg(test)]
    pub(super) fn test_shares_difference_storage_with(&self, other: &Self) -> Option<bool> {
        match (&self.storage, &other.storage) {
            (
                MaintainedBlockerStorage::Difference { classes: left, .. },
                MaintainedBlockerStorage::Difference { classes: right, .. },
            ) => Some(left.shares_root_with(right)),
            _ => None,
        }
    }

    pub(super) fn commit_patch(&mut self, patch: BlockerDeltaPatch) {
        match (&mut self.storage, patch.0) {
            (
                MaintainedBlockerStorage::Difference { classes, .. },
                BlockerDeltaPatchInner::Difference(writes),
            ) => {
                for (key, class) in writes {
                    if class.is_empty() {
                        classes.remove(&key);
                    } else {
                        classes.insert(key, Arc::new(class));
                    }
                }
            }
            (
                MaintainedBlockerStorage::AntiJoin { classes, .. },
                BlockerDeltaPatchInner::AntiJoin(writes),
            ) => {
                for (key, class) in writes {
                    if class.left.is_empty() && class.right_count.is_zero() {
                        classes.remove(&key);
                    } else {
                        classes.insert(key, Arc::new(class));
                    }
                }
            }
            _ => unreachable!("blocker patch/storage mismatch"),
        }
    }
}
