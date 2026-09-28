use std::{
    collections::BTreeMap,
    ops::{Deref, DerefMut},
    sync::{Arc, OnceLock},
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

#[derive(Debug)]
pub(super) struct RelationRowsAppendPatch {
    base: SharedRelationRows,
    inserted: Arc<Vec<Vec<Value>>>,
    pub(super) materialized: OnceLock<Vec<Vec<Value>>>,
}

#[derive(Debug, Clone)]
pub(super) enum SharedRelationRowsRepr {
    Materialized(Arc<Vec<Vec<Value>>>),
    AppendPatch(Arc<RelationRowsAppendPatch>),
}

#[derive(Debug, Clone)]
pub struct SharedRelationRows(pub(super) SharedRelationRowsRepr);

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
    pub fn append_persistent(&self, inserted: Vec<Vec<Value>>) -> Self {
        if inserted.is_empty() {
            return self.clone();
        }
        Self(SharedRelationRowsRepr::AppendPatch(Arc::new(
            RelationRowsAppendPatch {
                base: self.clone(),
                inserted: Arc::new(inserted),
                materialized: OnceLock::new(),
            },
        )))
    }

    fn materialized(&self) -> &Vec<Vec<Value>> {
        match &self.0 {
            SharedRelationRowsRepr::Materialized(rows) => rows,
            SharedRelationRowsRepr::AppendPatch(patch) => patch.materialized.get_or_init(|| {
                let mut segments = Vec::new();
                patch.base.collect_materialized_segments(&mut segments);
                segments.push(patch.inserted.as_slice());
                let capacity = segments
                    .iter()
                    .fold(0_usize, |len, segment| len.saturating_add(segment.len()));
                let mut rows = Vec::with_capacity(capacity);
                for segment in segments {
                    rows.extend(segment.iter().cloned());
                }
                rows
            }),
        }
    }

    fn collect_materialized_segments<'a>(&'a self, output: &mut Vec<&'a [Vec<Value>]>) {
        let mut tail = Vec::new();
        let mut cursor = self;
        loop {
            match &cursor.0 {
                SharedRelationRowsRepr::Materialized(rows) => {
                    output.push(rows.as_slice());
                    break;
                }
                SharedRelationRowsRepr::AppendPatch(patch) => {
                    if let Some(rows) = patch.materialized.get() {
                        output.push(rows.as_slice());
                        break;
                    }
                    tail.push(patch.inserted.as_slice());
                    cursor = &patch.base;
                }
            }
        }
        output.extend(tail.into_iter().rev());
    }
}

impl PartialEq for SharedRelationRows {
    fn eq(&self, other: &Self) -> bool {
        self.materialized() == other.materialized()
    }
}

impl Eq for SharedRelationRows {}

impl Deref for SharedRelationRows {
    type Target = Vec<Vec<Value>>;

    fn deref(&self) -> &Self::Target {
        self.materialized()
    }
}

impl DerefMut for SharedRelationRows {
    fn deref_mut(&mut self) -> &mut Self::Target {
        if let SharedRelationRowsRepr::AppendPatch(_) = &self.0 {
            let rows = self.materialized().clone();
            self.0 = SharedRelationRowsRepr::Materialized(Arc::new(rows));
        }
        match &mut self.0 {
            SharedRelationRowsRepr::Materialized(rows) => Arc::make_mut(rows),
            SharedRelationRowsRepr::AppendPatch(_) => unreachable!(),
        }
    }
}

impl<'a> IntoIterator for &'a SharedRelationRows {
    type Item = &'a Vec<Value>;
    type IntoIter = std::slice::Iter<'a, Vec<Value>>;

    fn into_iter(self) -> Self::IntoIter {
        self.materialized().iter()
    }
}

impl PartialEq<Vec<Vec<Value>>> for SharedRelationRows {
    fn eq(&self, other: &Vec<Vec<Value>>) -> bool {
        self.as_slice() == other.as_slice()
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

    #[must_use]
    pub fn get(&self, relation: &SemanticId) -> Option<&Vec<Vec<Value>>> {
        self.0.get(relation).map(|rows| &**rows)
    }

    pub fn get_mut(&mut self, relation: &SemanticId) -> Option<&mut Vec<Vec<Value>>> {
        self.0.get_mut(relation).map(SharedRelationRows::deref_mut)
    }

    pub fn append_persistent(&mut self, relation: SemanticId, inserted: Vec<Vec<Value>>) {
        let next = match self.0.get(&relation) {
            Some(rows) => rows.append_persistent(inserted),
            None => SharedRelationRows::from(inserted),
        };
        self.0.insert(relation, next);
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
