use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut, Index},
    sync::Arc,
};

use kernel_persistent::{
    PersistentOrdMapStorageProbe, PersistentOrdSet, PersistentVec, PersistentVecStorageProbe,
};
use kernel_types::SemanticId;

use crate::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CowValue<T>(pub(super) Arc<T>);

impl<T: Default> Default for CowValue<T> {
    fn default() -> Self {
        Self(Arc::new(T::default()))
    }
}

impl<T> From<T> for CowValue<T> {
    fn from(value: T) -> Self {
        Self(Arc::new(value))
    }
}

impl<T> Deref for CowValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Clone> DerefMut for CowValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CowMap<K, V>(pub(super) Arc<BTreeMap<K, V>>);

impl<K, V> Default for CowMap<K, V> {
    fn default() -> Self {
        Self(Arc::new(BTreeMap::new()))
    }
}

impl<K, V> From<BTreeMap<K, V>> for CowMap<K, V> {
    fn from(value: BTreeMap<K, V>) -> Self {
        Self(Arc::new(value))
    }
}

impl<K, V> Deref for CowMap<K, V> {
    type Target = BTreeMap<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<K: Clone + Ord, V: Clone> DerefMut for CowMap<K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

impl<'a, K, V> IntoIterator for &'a CowMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = std::collections::btree_map::Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// One factorized logical relation root over an immutable materialized base.
///
/// `removed_base_positions` is expressed in the base coordinate system, while
/// `inserted_tail` is the ordered survivor tail accumulated after the base.
/// Both are persistent structures, so successor revisions path-copy only the
/// changed metadata/pages instead of retaining a fresh O(N) row buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RelationRowsDeltaRoot {
    base: Arc<Vec<Vec<Value>>>,
    removed_base_positions: PersistentOrdSet<usize>,
    inserted_tail: PersistentVec<Vec<Value>>,
}

impl RelationRowsDeltaRoot {
    fn len(&self) -> usize {
        self.base
            .len()
            .saturating_sub(self.removed_base_positions.len())
            .saturating_add(self.inserted_tail.len())
    }

    fn materialize_owned(&self) -> Vec<Vec<Value>> {
        let mut rows = Vec::with_capacity(self.len());
        rows.extend(
            self.base
                .iter()
                .enumerate()
                .filter(|(index, _)| !self.removed_base_positions.contains(index))
                .map(|(_, row)| row.clone()),
        );
        rows.extend(self.inserted_tail.iter().cloned());
        rows
    }

    fn map_survivor_rank_to_base_position(&self, rank: usize) -> Option<usize> {
        let base_survivors = self.base.len().saturating_sub(self.removed_base_positions.len());
        if rank >= base_survivors {
            return None;
        }
        let mut candidate = rank;
        for removed in &self.removed_base_positions {
            if *removed <= candidate {
                candidate = candidate.checked_add(1)?;
            } else {
                break;
            }
        }
        (candidate < self.base.len()).then_some(candidate)
    }

    fn apply_patch(&self, removed_positions: &[usize], inserted: Vec<Vec<Value>>) -> Option<Self> {
        if removed_positions.windows(2).any(|pair| pair[0] >= pair[1])
            || removed_positions.last().is_some_and(|index| *index >= self.len())
        {
            return None;
        }

        let base_survivors = self.base.len().saturating_sub(self.removed_base_positions.len());
        let mut removed_base_positions = self.removed_base_positions.clone();
        let mut removed_inserted = Vec::new();
        for &position in removed_positions {
            if position < base_survivors {
                let base_position = self.map_survivor_rank_to_base_position(position)?;
                if !removed_base_positions.insert(base_position) {
                    return None;
                }
            } else {
                let tail_index = position - base_survivors;
                if tail_index >= self.inserted_tail.len() {
                    return None;
                }
                removed_inserted.push(tail_index);
            }
        }

        removed_inserted.sort_unstable_by(|left, right| right.cmp(left));
        let mut inserted_tail = self.inserted_tail.clone();
        for index in removed_inserted {
            inserted_tail.remove(index);
        }
        for row in inserted {
            inserted_tail.push(row);
        }

        Some(Self {
            base: Arc::clone(&self.base),
            removed_base_positions,
            inserted_tail,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SharedRelationRowsRepr {
    Materialized(Arc<Vec<Vec<Value>>>),
    DeltaRoot(Arc<RelationRowsDeltaRoot>),
}


#[derive(Debug)]
pub struct SharedRelationRowsStorageProbe {
    removed: Option<PersistentOrdMapStorageProbe<usize, ()>>,
    inserted: Option<PersistentVecStorageProbe<Vec<Value>>>,
}

impl SharedRelationRowsStorageProbe {
    #[must_use]
    pub fn total_nodes(&self) -> usize {
        self.removed.as_ref().map_or(0, PersistentOrdMapStorageProbe::total_nodes)
            + self.inserted.as_ref().map_or(0, PersistentVecStorageProbe::total_nodes)
    }

    #[must_use]
    pub fn is_fully_reclaimed(&self) -> bool {
        self.removed
            .as_ref()
            .is_none_or(PersistentOrdMapStorageProbe::is_fully_reclaimed)
            && self
                .inserted
                .as_ref()
                .is_none_or(PersistentVecStorageProbe::is_fully_reclaimed)
    }
}

#[derive(Debug)]
pub struct SharedRelationRows(pub(super) SharedRelationRowsRepr);

impl Clone for SharedRelationRows {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Default for SharedRelationRows {
    fn default() -> Self {
        Self::from(Vec::new())
    }
}

impl From<Vec<Vec<Value>>> for SharedRelationRows {
    fn from(rows: Vec<Vec<Value>>) -> Self {
        Self(SharedRelationRowsRepr::Materialized(Arc::new(rows)))
    }
}

impl SharedRelationRows {
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.0 {
            SharedRelationRowsRepr::Materialized(rows) => rows.len(),
            SharedRelationRowsRepr::DeltaRoot(root) => root.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn materialize_owned(&self) -> Vec<Vec<Value>> {
        match &self.0 {
            SharedRelationRowsRepr::Materialized(rows) => rows.as_ref().clone(),
            SharedRelationRowsRepr::DeltaRoot(root) => root.materialize_owned(),
        }
    }

    #[must_use]
    pub fn append_persistent(&self, inserted: Vec<Vec<Value>>) -> Self {
        self.patch_persistent(&[], inserted)
            .expect("empty-removal persistent append must be valid")
    }

    pub fn patch_persistent(
        &self,
        removed_positions: &[usize],
        inserted: Vec<Vec<Value>>,
    ) -> Option<Self> {
        if removed_positions.windows(2).any(|pair| pair[0] >= pair[1])
            || removed_positions.last().is_some_and(|index| *index >= self.len())
        {
            return None;
        }
        let root = match &self.0 {
            SharedRelationRowsRepr::Materialized(base) => {
                let mut removed_base_positions = PersistentOrdSet::default();
                for &position in removed_positions {
                    removed_base_positions.insert(position);
                }
                RelationRowsDeltaRoot {
                    base: Arc::clone(base),
                    removed_base_positions,
                    inserted_tail: PersistentVec::from_vec(inserted),
                        }
            }
            SharedRelationRowsRepr::DeltaRoot(root) => {
                return root
                    .apply_patch(removed_positions, inserted)
                    .map(|next| Self(SharedRelationRowsRepr::DeltaRoot(Arc::new(next))));
            }
        };
        Some(Self(SharedRelationRowsRepr::DeltaRoot(Arc::new(root))))
    }

    #[must_use]
    pub fn has_materialized_projection(&self) -> bool {
        match &self.0 {
            SharedRelationRowsRepr::Materialized(_) => true,
            SharedRelationRowsRepr::DeltaRoot(_) => false,
        }
    }

    #[must_use]
    pub fn owned_delta_row_count(&self) -> usize {
        match &self.0 {
            SharedRelationRowsRepr::Materialized(rows) => rows.len(),
            SharedRelationRowsRepr::DeltaRoot(root) => root.inserted_tail.len(),
        }
    }


    #[must_use]
    pub fn persistent_delta_structural_nodes(&self) -> usize {
        match &self.0 {
            SharedRelationRowsRepr::Materialized(_) => 0,
            SharedRelationRowsRepr::DeltaRoot(root) => {
                root.removed_base_positions.structural_node_count()
                    + root.inserted_tail.structural_node_count()
            }
        }
    }

    #[must_use]
    pub fn unique_delta_storage_probe_against(
        &self,
        successor: &Self,
    ) -> SharedRelationRowsStorageProbe {
        match (&self.0, &successor.0) {
            (SharedRelationRowsRepr::DeltaRoot(current), SharedRelationRowsRepr::DeltaRoot(next))
                if Arc::ptr_eq(&current.base, &next.base) =>
            {
                SharedRelationRowsStorageProbe {
                    removed: Some(
                        current
                            .removed_base_positions
                            .unique_storage_probe_against(&next.removed_base_positions),
                    ),
                    inserted: Some(
                        current
                            .inserted_tail
                            .unique_storage_probe_against(&next.inserted_tail),
                    ),
                }
            }
            _ => SharedRelationRowsStorageProbe {
                removed: None,
                inserted: None,
            },
        }
    }

    #[must_use]
    pub fn persistent_delta_nodes_new_since(&self, predecessor: &Self) -> usize {
        match (&self.0, &predecessor.0) {
            (SharedRelationRowsRepr::Materialized(_), _) => 0,
            (SharedRelationRowsRepr::DeltaRoot(current), SharedRelationRowsRepr::DeltaRoot(previous))
                if Arc::ptr_eq(&current.base, &previous.base) =>
            {
                current
                    .removed_base_positions
                    .structural_node_count()
                    .saturating_sub(
                        current
                            .removed_base_positions
                            .shared_structural_node_count_with(&previous.removed_base_positions),
                    )
                    + current
                        .inserted_tail
                        .structural_node_count()
                        .saturating_sub(
                            current
                                .inserted_tail
                                .shared_structural_node_count_with(&previous.inserted_tail),
                        )
            }
            (SharedRelationRowsRepr::DeltaRoot(current), _) => {
                current.removed_base_positions.structural_node_count()
                    + current.inserted_tail.structural_node_count()
            }
        }
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Vec<Value>> {
        match &self.0 {
            SharedRelationRowsRepr::Materialized(rows) => rows.get(index),
            SharedRelationRowsRepr::DeltaRoot(root) => {
                let base_survivors = root
                    .base
                    .len()
                    .saturating_sub(root.removed_base_positions.len());
                if index < base_survivors {
                    let base_position = root.map_survivor_rank_to_base_position(index)?;
                    root.base.get(base_position)
                } else {
                    root.inserted_tail.get(index - base_survivors)
                }
            }
        }
    }

    #[must_use]
    pub fn iter(&self) -> SharedRelationRowsIter<'_> {
        SharedRelationRowsIter {
            rows: self,
            base_index: 0,
            tail_index: 0,
            remaining: self.len(),
        }
    }

    #[must_use]
    pub fn to_vec(&self) -> Vec<Vec<Value>> {
        self.materialize_owned()
    }

    pub fn push(&mut self, row: Vec<Value>) {
        self.materialized_mut().push(row);
    }

    fn materialized_mut(&mut self) -> &mut Vec<Vec<Value>> {
        if let SharedRelationRowsRepr::DeltaRoot(_) = &self.0 {
            let rows = self.materialize_owned();
            self.0 = SharedRelationRowsRepr::Materialized(Arc::new(rows));
        }
        match &mut self.0 {
            SharedRelationRowsRepr::Materialized(rows) => Arc::make_mut(rows),
            SharedRelationRowsRepr::DeltaRoot(_) => unreachable!(),
        }
    }
}

pub struct SharedRelationRowsIter<'a> {
    rows: &'a SharedRelationRows,
    base_index: usize,
    tail_index: usize,
    remaining: usize,
}

impl<'a> Iterator for SharedRelationRowsIter<'a> {
    type Item = &'a Vec<Value>;

    fn next(&mut self) -> Option<Self::Item> {
        match &self.rows.0 {
            SharedRelationRowsRepr::Materialized(rows) => {
                let value = rows.get(self.base_index);
                self.base_index = self.base_index.saturating_add(1);
                if value.is_some() {
                    self.remaining = self.remaining.saturating_sub(1);
                }
                value
            }
            SharedRelationRowsRepr::DeltaRoot(root) => {
                while self.base_index < root.base.len() {
                    let index = self.base_index;
                    self.base_index += 1;
                    if !root.removed_base_positions.contains(&index) {
                        self.remaining = self.remaining.saturating_sub(1);
                        return root.base.get(index);
                    }
                }
                let value = root.inserted_tail.get(self.tail_index);
                self.tail_index = self.tail_index.saturating_add(1);
                if value.is_some() {
                    self.remaining = self.remaining.saturating_sub(1);
                }
                value
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl PartialEq for SharedRelationRows {
    fn eq(&self, other: &Self) -> bool {
        self.materialize_owned() == other.materialize_owned()
    }
}

impl Eq for SharedRelationRows {}

impl Index<usize> for SharedRelationRows {
    type Output = Vec<Value>;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("relation row index out of bounds")
    }
}

impl<'a> IntoIterator for &'a SharedRelationRows {
    type Item = &'a Vec<Value>;
    type IntoIter = SharedRelationRowsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl PartialEq<Vec<Vec<Value>>> for SharedRelationRows {
    fn eq(&self, other: &Vec<Vec<Value>>) -> bool {
        self.materialize_owned() == *other
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RelationStore(pub(super) CowMap<SemanticId, SharedRelationRows>);

impl RelationStore {
    pub fn insert(
        &mut self,
        relation: SemanticId,
        rows: Vec<Vec<Value>>,
    ) -> Option<SharedRelationRows> {
        self.0.insert(relation, rows.into())
    }

    pub fn insert_shared(
        &mut self,
        relation: SemanticId,
        rows: SharedRelationRows,
    ) -> Option<SharedRelationRows> {
        self.0.insert(relation, rows)
    }

    #[must_use]
    pub fn get(&self, relation: &SemanticId) -> Option<&SharedRelationRows> {
        self.0.get(relation)
    }

    #[must_use]
    pub fn get_shared(&self, relation: &SemanticId) -> Option<&SharedRelationRows> {
        self.0.get(relation)
    }

    #[must_use]
    pub fn materialize_owned(&self, relation: &SemanticId) -> Option<Vec<Vec<Value>>> {
        self.0.get(relation).map(SharedRelationRows::materialize_owned)
    }

    pub fn get_mut(&mut self, relation: &SemanticId) -> Option<&mut Vec<Vec<Value>>> {
        self.0.get_mut(relation).map(SharedRelationRows::materialized_mut)
    }

    pub fn append_persistent(&mut self, relation: SemanticId, inserted: Vec<Vec<Value>>) {
        let next = match self.0.get(&relation) {
            Some(rows) => rows.append_persistent(inserted),
            None => SharedRelationRows::from(inserted),
        };
        self.0.insert(relation, next);
    }

    pub fn patch_persistent(
        &mut self,
        relation: SemanticId,
        removed_positions: &[usize],
        inserted: Vec<Vec<Value>>,
    ) -> bool {
        let Some(current) = self.0.get(&relation) else {
            if removed_positions.is_empty() {
                self.0.insert(relation, SharedRelationRows::from(inserted));
                return true;
            }
            return false;
        };
        let Some(next) = current.patch_persistent(removed_positions, inserted) else {
            return false;
        };
        self.0.insert(relation, next);
        true
    }

    pub fn detach_materialized_projections(&mut self) {
        // Persistent relation roots no longer retain lazy contiguous projections.
    }

    pub fn remove(&mut self, relation: &SemanticId) -> Option<SharedRelationRows> {
        self.0.remove(relation)
    }
}

impl Deref for RelationStore {
    type Target = BTreeMap<SemanticId, SharedRelationRows>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for RelationStore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'a> IntoIterator for &'a RelationStore {
    type Item = (&'a SemanticId, &'a SharedRelationRows);
    type IntoIter = std::collections::btree_map::Iter<'a, SemanticId, SharedRelationRows>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
