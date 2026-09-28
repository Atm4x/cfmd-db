use super::{RelQueryError, Row};
use kernel_persistent::{PersistentOrdMap, PersistentVec};

pub(super) type CanonicalRowKey = Vec<kernel_semantics::CanonicalEqKey>;

pub(super) fn canonical_row_key(
    row: &Row,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<CanonicalRowKey, RelQueryError> {
    if row.len() != column_equivalences.len() {
        return Err(RelQueryError::EquivalenceArityMismatch);
    }
    row.iter()
        .zip(column_equivalences)
        .map(|(value, equivalence)| {
            registry
                .canonical_equivalence_key(context, *equivalence, value)
                .map_err(RelQueryError::from)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CanonicalRowPositionIndex {
    by_key: PersistentOrdMap<CanonicalRowKey, PersistentVec<usize>>,
    by_position: PersistentVec<CanonicalRowKey>,
    bucket_slot_by_position: PersistentVec<usize>,
}

impl CanonicalRowPositionIndex {
    pub(super) fn remove_position(&mut self, position: usize) {
        let last_position = self
            .by_position
            .len()
            .checked_sub(1)
            .expect("validated canonical Scan removal requires a row");
        let key = self.by_position[position].clone();
        let bucket_slot = self.bucket_slot_by_position[position];
        let mut bucket = self
            .by_key
            .get(&key)
            .cloned()
            .expect("validated canonical Scan removal key must exist");
        let removed_position = bucket.swap_remove(bucket_slot);
        debug_assert_eq!(removed_position, position);
        if bucket_slot < bucket.len() {
            let bucket_moved_position = bucket[bucket_slot];
            self.bucket_slot_by_position[bucket_moved_position] = bucket_slot;
        }
        if bucket.is_empty() {
            self.by_key.remove(&key);
        } else {
            self.by_key.insert(key, bucket);
        }
        if position != last_position {
            let moved_key = self.by_position[last_position].clone();
            let moved_bucket_slot = self.bucket_slot_by_position[last_position];
            let mut moved_bucket = self
                .by_key
                .get(&moved_key)
                .cloned()
                .expect("moved canonical Scan key must exist");
            debug_assert_eq!(moved_bucket[moved_bucket_slot], last_position);
            moved_bucket.set(moved_bucket_slot, position);
            self.by_key.insert(moved_key.clone(), moved_bucket);
            self.by_position.set(position, moved_key);
            self.bucket_slot_by_position
                .set(position, moved_bucket_slot);
        }
        self.by_position.pop();
        self.bucket_slot_by_position.pop();
    }

    pub(super) fn push_key(&mut self, key: CanonicalRowKey) {
        let position = self.by_position.len();
        let mut bucket = self.by_key.get(&key).cloned().unwrap_or_default();
        let bucket_slot = bucket.len();
        bucket.push(position);
        self.by_key.insert(key.clone(), bucket);
        self.by_position.push(key);
        self.bucket_slot_by_position.push(bucket_slot);
    }

    pub(super) fn positions(&self, key: &CanonicalRowKey) -> Option<&PersistentVec<usize>> {
        self.by_key.get(key)
    }
}

pub(super) fn canonical_row_position_index(
    rows: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<CanonicalRowPositionIndex, RelQueryError> {
    let mut by_key = PersistentOrdMap::<CanonicalRowKey, PersistentVec<usize>>::default();
    let mut by_position = PersistentVec::default();
    let mut bucket_slot_by_position = PersistentVec::default();
    for (index, row) in rows.iter().enumerate() {
        let key = canonical_row_key(row, column_equivalences, context, registry)?;
        let mut bucket = by_key.get(&key).cloned().unwrap_or_default();
        let bucket_slot = bucket.len();
        bucket.push(index);
        by_key.insert(key.clone(), bucket);
        by_position.push(key);
        bucket_slot_by_position.push(bucket_slot);
    }
    Ok(CanonicalRowPositionIndex {
        by_key,
        by_position,
        bucket_slot_by_position,
    })
}
