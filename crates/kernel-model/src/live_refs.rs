use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use kernel_identity::{DenseEntityIds, LocalEntityId};
use kernel_persistent::{PersistentOrdMap, PersistentOrdSet};
use kernel_types::{EntityId, SemanticId};

use crate::{FiniteModel, Value};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LiveRefConsumers {
    fields: BTreeSet<(SemanticId, EntityId)>,
    relation_rows: BTreeMap<SemanticId, BTreeSet<usize>>,
}

impl LiveRefConsumers {
    #[must_use]
    pub fn fields(&self) -> &BTreeSet<(SemanticId, EntityId)> {
        &self.fields
    }
    #[must_use]
    pub fn relation_rows(&self) -> &BTreeMap<SemanticId, BTreeSet<usize>> {
        &self.relation_rows
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LiveRefSensitivityIndex {
    pub(super) field_by_target: Arc<BTreeMap<LocalEntityId, BTreeSet<(SemanticId, EntityId)>>>,
    pub(super) field_unresolved: Arc<BTreeMap<EntityId, BTreeSet<(SemanticId, EntityId)>>>,
    pub(super) relations: BTreeMap<SemanticId, Arc<RelationLiveRefSensitivity>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RelationRefTarget {
    Dense(LocalEntityId),
    Unresolved(EntityId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RelationRowToken {
    Base(usize),
    Tail(u64),
}

/// Compact reconstructible reverse-reference authority for one relation.
///
/// Initial rows keep immutable `Base(position)` identity. Exact relation
/// transitions only remove base positions and append tail tokens, matching the
/// persistent relation-root law. Metadata therefore scales with live-reference
/// rows plus accumulated delta, never with unrelated non-reference rows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct RelationLiveRefSensitivity {
    pub(super) by_target: PersistentOrdMap<LocalEntityId, PersistentOrdSet<RelationRowToken>>,
    pub(super) unresolved: PersistentOrdMap<EntityId, PersistentOrdSet<RelationRowToken>>,
    row_targets: PersistentOrdMap<RelationRowToken, Arc<Vec<RelationRefTarget>>>,
    base_len: usize,
    removed_base_positions: PersistentOrdSet<usize>,
    active_tail: PersistentOrdSet<u64>,
    next_tail: u64,
}

impl RelationLiveRefSensitivity {
    fn targets_for_row(row: &[Value], ids: &DenseEntityIds) -> Vec<RelationRefTarget> {
        let mut external = Vec::new();
        for value in row {
            value.collect_live_refs(&mut external);
        }
        external.sort_unstable();
        external.dedup();
        external
            .into_iter()
            .map(|target| {
                ids.local(target).map_or(
                    RelationRefTarget::Unresolved(target),
                    RelationRefTarget::Dense,
                )
            })
            .collect()
    }

    fn insert_reverse(&mut self, token: RelationRowToken, targets: &[RelationRefTarget]) {
        for target in targets {
            match *target {
                RelationRefTarget::Dense(target) => {
                    if let Some(tokens) = self.by_target.get_mut(&target) {
                        tokens.insert(token);
                    } else {
                        let mut tokens = PersistentOrdSet::default();
                        tokens.insert(token);
                        self.by_target.insert(target, tokens);
                    }
                }
                RelationRefTarget::Unresolved(target) => {
                    if let Some(tokens) = self.unresolved.get_mut(&target) {
                        tokens.insert(token);
                    } else {
                        let mut tokens = PersistentOrdSet::default();
                        tokens.insert(token);
                        self.unresolved.insert(target, tokens);
                    }
                }
            }
        }
    }

    fn remove_reverse(&mut self, token: RelationRowToken, targets: &[RelationRefTarget]) {
        for target in targets {
            match *target {
                RelationRefTarget::Dense(target) => {
                    let remove_key = self.by_target.get_mut(&target).is_some_and(|tokens| {
                        tokens.remove(&token);
                        tokens.is_empty()
                    });
                    if remove_key {
                        self.by_target.remove(&target);
                    }
                }
                RelationRefTarget::Unresolved(target) => {
                    let remove_key = self.unresolved.get_mut(&target).is_some_and(|tokens| {
                        tokens.remove(&token);
                        tokens.is_empty()
                    });
                    if remove_key {
                        self.unresolved.remove(&target);
                    }
                }
            }
        }
    }

    fn install_targets(&mut self, token: RelationRowToken, row: &[Value], ids: &DenseEntityIds) {
        let targets = Self::targets_for_row(row, ids);
        if targets.is_empty() {
            return;
        }
        self.insert_reverse(token, &targets);
        self.row_targets.insert(token, Arc::new(targets));
    }

    fn len(&self) -> usize {
        self.base_len
            .saturating_sub(self.removed_base_positions.len())
            .saturating_add(self.active_tail.len())
    }

    fn base_position_for_survivor_rank(&self, rank: usize) -> Option<usize> {
        let survivors = self
            .base_len
            .saturating_sub(self.removed_base_positions.len());
        if rank >= survivors {
            return None;
        }
        let mut low = rank;
        let mut high = self.base_len;
        while low < high {
            let mid = low + (high - low) / 2;
            let removed_through = self.removed_base_positions.rank_before(&mid)
                + usize::from(self.removed_base_positions.contains(&mid));
            let survivors_through = mid.saturating_add(1).saturating_sub(removed_through);
            if survivors_through > rank {
                high = mid;
            } else {
                low = mid.saturating_add(1);
            }
        }
        (low < self.base_len && !self.removed_base_positions.contains(&low)).then_some(low)
    }

    fn token_at_rank(&self, rank: usize) -> Option<RelationRowToken> {
        let base_survivors = self
            .base_len
            .saturating_sub(self.removed_base_positions.len());
        if rank < base_survivors {
            self.base_position_for_survivor_rank(rank)
                .map(RelationRowToken::Base)
        } else {
            self.active_tail
                .value_at_rank(rank - base_survivors)
                .copied()
                .map(RelationRowToken::Tail)
        }
    }

    fn current_rank(&self, token: RelationRowToken) -> Option<usize> {
        match token {
            RelationRowToken::Base(position) => (position < self.base_len
                && !self.removed_base_positions.contains(&position))
            .then(|| position.saturating_sub(self.removed_base_positions.rank_before(&position))),
            RelationRowToken::Tail(token) => self.active_tail.rank_of(&token).map(|rank| {
                self.base_len
                    .saturating_sub(self.removed_base_positions.len())
                    .saturating_add(rank)
            }),
        }
    }

    fn remove_row_at(&mut self, position: usize) -> Option<()> {
        let token = self.token_at_rank(position)?;
        match token {
            RelationRowToken::Base(position) => {
                self.removed_base_positions.insert(position);
            }
            RelationRowToken::Tail(token) => {
                self.active_tail.remove(&token);
            }
        }
        if let Some(targets) = self.row_targets.remove(&token) {
            self.remove_reverse(token, targets.as_ref());
        }
        Some(())
    }

    fn append_row(&mut self, row: &[Value], ids: &DenseEntityIds) -> Option<()> {
        let token = self.next_tail;
        self.next_tail = self.next_tail.checked_add(1)?;
        self.active_tail.insert(token);
        self.install_targets(RelationRowToken::Tail(token), row, ids);
        Some(())
    }

    fn apply_delta(
        &self,
        removed_positions: &[usize],
        inserted: &[Vec<Value>],
        ids: &DenseEntityIds,
    ) -> Option<Self> {
        if removed_positions.windows(2).any(|pair| pair[0] >= pair[1])
            || removed_positions
                .last()
                .is_some_and(|position| *position >= self.len())
        {
            return None;
        }
        let mut next = self.clone();
        for &position in removed_positions.iter().rev() {
            next.remove_row_at(position)?;
        }
        for row in inserted {
            next.append_row(row, ids)?;
        }
        Some(next)
    }

    pub(super) fn row_positions(
        &self,
        tokens: &PersistentOrdSet<RelationRowToken>,
    ) -> BTreeSet<usize> {
        tokens
            .iter()
            .filter_map(|token| self.current_rank(*token))
            .collect()
    }
}

impl LiveRefSensitivityIndex {
    #[must_use]
    pub fn relation_has_live_refs(&self, relation: SemanticId) -> bool {
        self.relations
            .get(&relation)
            .is_some_and(|s| !s.by_target.is_empty() || !s.unresolved.is_empty())
    }

    #[must_use]
    pub fn compile(model: &FiniteModel, ids: &DenseEntityIds) -> Self {
        let mut field_by_target = BTreeMap::<LocalEntityId, BTreeSet<_>>::new();
        let mut field_unresolved = BTreeMap::<EntityId, BTreeSet<_>>::new();
        for (&field, value) in &model.fields {
            let mut targets = Vec::new();
            value.collect_live_refs(&mut targets);
            for target in targets {
                if let Some(local) = ids.local(target) {
                    field_by_target.entry(local).or_default().insert(field);
                } else {
                    field_unresolved.entry(target).or_default().insert(field);
                }
            }
        }
        let relations = model
            .relations
            .iter()
            .map(|(&relation, rows)| {
                let materialized = rows.materialize_owned();
                (
                    relation,
                    Arc::new(Self::compile_relation(&materialized, ids)),
                )
            })
            .collect();
        Self {
            field_by_target: Arc::new(field_by_target),
            field_unresolved: Arc::new(field_unresolved),
            relations,
        }
    }

    fn compile_relation(rows: &[Vec<Value>], ids: &DenseEntityIds) -> RelationLiveRefSensitivity {
        let mut sensitivity = RelationLiveRefSensitivity {
            base_len: rows.len(),
            ..Default::default()
        };
        for (position, row) in rows.iter().enumerate() {
            sensitivity.install_targets(RelationRowToken::Base(position), row, ids);
        }
        sensitivity
    }

    #[must_use]
    pub fn consumers(&self, target: LocalEntityId) -> Option<LiveRefConsumers> {
        let mut consumers = LiveRefConsumers::default();
        if let Some(fields) = self.field_by_target.get(&target) {
            consumers.fields.clone_from(fields);
        }
        for (&relation, sensitivity) in &self.relations {
            if let Some(tokens) = sensitivity.by_target.get(&target) {
                consumers
                    .relation_rows
                    .insert(relation, sensitivity.row_positions(tokens));
            }
        }
        (!consumers.fields.is_empty() || !consumers.relation_rows.is_empty()).then_some(consumers)
    }

    #[must_use]
    pub fn unresolved(&self, target: EntityId) -> Option<LiveRefConsumers> {
        let mut consumers = LiveRefConsumers::default();
        if let Some(fields) = self.field_unresolved.get(&target) {
            consumers.fields.clone_from(fields);
        }
        for (&relation, sensitivity) in &self.relations {
            if let Some(tokens) = sensitivity.unresolved.get(&target) {
                consumers
                    .relation_rows
                    .insert(relation, sensitivity.row_positions(tokens));
            }
        }
        (!consumers.fields.is_empty() || !consumers.relation_rows.is_empty()).then_some(consumers)
    }

    #[must_use]
    pub fn with_relation_delta(
        &self,
        relation: SemanticId,
        removed_positions: &[usize],
        inserted: &[Vec<Value>],
        ids: &DenseEntityIds,
    ) -> Option<Self> {
        let mut next_relations = self.relations.clone();
        let next = match self.relations.get(&relation) {
            Some(current) => current.apply_delta(removed_positions, inserted, ids)?,
            None if removed_positions.is_empty() => {
                let mut next = RelationLiveRefSensitivity::default();
                for row in inserted {
                    next.append_row(row, ids)?;
                }
                next
            }
            None => return None,
        };
        next_relations.insert(relation, Arc::new(next));
        Some(Self {
            field_by_target: Arc::clone(&self.field_by_target),
            field_unresolved: Arc::clone(&self.field_unresolved),
            relations: next_relations,
        })
    }

    #[must_use]
    pub fn with_relations_recompiled(
        &self,
        model: &FiniteModel,
        ids: &DenseEntityIds,
        relations: &BTreeSet<SemanticId>,
    ) -> Self {
        let mut next_relations = self.relations.clone();
        for &relation in relations {
            if let Some(rows) = model.relations.materialize_owned(&relation) {
                next_relations.insert(relation, Arc::new(Self::compile_relation(&rows, ids)));
            } else {
                next_relations.remove(&relation);
            }
        }
        Self {
            field_by_target: Arc::clone(&self.field_by_target),
            field_unresolved: Arc::clone(&self.field_unresolved),
            relations: next_relations,
        }
    }
}
