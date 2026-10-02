use super::{RelQueryError, Row};
use kernel_persistent::{PersistentOrdMap, PersistentVec};
use std::sync::Arc;

pub type CanonicalRowKey = Vec<kernel_semantics::CanonicalEqKey>;

pub fn canonical_row_key(
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
    by_key: PersistentOrdMap<Arc<CanonicalRowKey>, PersistentVec<usize>>,
    by_position: PersistentVec<Arc<CanonicalRowKey>>,
    bucket_slot_by_position: PersistentVec<usize>,
}

impl CanonicalRowPositionIndex {
    pub(super) fn remove_position(&mut self, position: usize) {
        let last_position = self
            .by_position
            .len()
            .checked_sub(1)
            .expect("validated canonical Scan removal requires a row");
        let key = Arc::clone(&self.by_position[position]);
        let bucket_slot = self.bucket_slot_by_position[position];
        let mut bucket = self
            .by_key
            .get(key.as_ref())
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
            let moved_key = Arc::clone(&self.by_position[last_position]);
            let moved_bucket_slot = self.bucket_slot_by_position[last_position];
            let mut moved_bucket = self
                .by_key
                .get(moved_key.as_ref())
                .cloned()
                .expect("moved canonical Scan key must exist");
            debug_assert_eq!(moved_bucket[moved_bucket_slot], last_position);
            moved_bucket.set(moved_bucket_slot, position);
            self.by_key.insert(Arc::clone(&moved_key), moved_bucket);
            self.by_position.set(position, moved_key);
            self.bucket_slot_by_position
                .set(position, moved_bucket_slot);
        }
        self.by_position.pop();
        self.bucket_slot_by_position.pop();
    }

    pub(super) fn push_key(&mut self, key: CanonicalRowKey) {
        let position = self.by_position.len();
        let (shared_key, mut bucket) = self
            .by_key
            .get_key_value(&key)
            .map_or_else(
                || (Arc::new(key), PersistentVec::default()),
                |(shared, bucket)| (Arc::clone(shared), bucket.clone()),
            );
        let bucket_slot = bucket.len();
        bucket.push(position);
        self.by_key.insert(Arc::clone(&shared_key), bucket);
        self.by_position.push(shared_key);
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
    let mut by_key = PersistentOrdMap::<Arc<CanonicalRowKey>, PersistentVec<usize>>::default();
    let mut by_position = PersistentVec::default();
    let mut bucket_slot_by_position = PersistentVec::default();
    for (index, row) in rows.iter().enumerate() {
        let key = canonical_row_key(row, column_equivalences, context, registry)?;
        let (shared_key, mut bucket) = by_key.get_key_value(&key).map_or_else(
            || (Arc::new(key), PersistentVec::default()),
            |(shared, bucket)| (Arc::clone(shared), bucket.clone()),
        );
        let bucket_slot = bucket.len();
        bucket.push(index);
        by_key.insert(Arc::clone(&shared_key), bucket);
        by_position.push(shared_key);
        bucket_slot_by_position.push(bucket_slot);
    }
    Ok(CanonicalRowPositionIndex {
        by_key,
        by_position,
        bucket_slot_by_position,
    })
}

pub(super) fn canonical_row_position_index_from_keys<'a>(
    keys: impl IntoIterator<Item = &'a CanonicalRowKey>,
) -> CanonicalRowPositionIndex {
    let mut by_key = PersistentOrdMap::<Arc<CanonicalRowKey>, PersistentVec<usize>>::default();
    let mut by_position = PersistentVec::default();
    let mut bucket_slot_by_position = PersistentVec::default();
    for (index, key) in keys.into_iter().enumerate() {
        let (shared_key, mut bucket) = by_key.get_key_value(key).map_or_else(
            || (Arc::new(key.clone()), PersistentVec::default()),
            |(shared, bucket)| (Arc::clone(shared), bucket.clone()),
        );
        let bucket_slot = bucket.len();
        bucket.push(index);
        by_key.insert(Arc::clone(&shared_key), bucket);
        by_position.push(shared_key);
        bucket_slot_by_position.push(bucket_slot);
    }
    CanonicalRowPositionIndex {
        by_key,
        by_position,
        bucket_slot_by_position,
    }
}

#[cfg(test)]
mod position_index_tests {
    use super::*;
    use kernel_semantics::CanonicalEqKey;

    fn key(value: i64) -> CanonicalRowKey {
        vec![CanonicalEqKey::I64(value)]
    }

    #[test]
    fn position_index_shares_one_canonical_payload_per_gamma_class() {
        let keys = vec![key(1), key(1), key(2), key(1), key(2)];
        let index = canonical_row_position_index_from_keys(keys.iter());

        assert_eq!(index.by_key.len(), 2);
        assert_eq!(index.by_position.len(), 5);
        for shared in &index.by_position {
            let (class_key, _) = index
                .by_key
                .get_key_value(shared.as_ref())
                .expect("position key must have canonical class");
            assert!(Arc::ptr_eq(shared, class_key));
        }
    }

    #[test]
    fn position_index_swap_remove_preserves_shared_class_payloads() {
        let keys = vec![key(1), key(2), key(1), key(3)];
        let mut index = canonical_row_position_index_from_keys(keys.iter());
        index.remove_position(1);
        index.push_key(key(3));

        for shared in &index.by_position {
            let (class_key, _) = index
                .by_key
                .get_key_value(shared.as_ref())
                .expect("position key must have canonical class");
            assert!(Arc::ptr_eq(shared, class_key));
        }
        assert_eq!(index.positions(&key(1)).unwrap().len(), 2);
        assert_eq!(index.positions(&key(3)).unwrap().len(), 2);
    }

    #[test]
    #[ignore = "manual release-mode shared canonical position-index hostile benchmark"]
    fn shared_key_position_index_scale_benchmark() {
        use std::time::Instant;

        for (label, keys) in [
            (
                "unique-100k",
                (0..100_000_i64).map(key).collect::<Vec<_>>(),
            ),
            (
                "repeated-100k-1024-classes",
                (0..100_000_i64).map(|value| key(value % 1024)).collect::<Vec<_>>(),
            ),
        ] {
            let started = Instant::now();
            let mut index = canonical_row_position_index_from_keys(keys.iter());
            let build_ms = started.elapsed().as_secs_f64() * 1_000.0;
            let started = Instant::now();
            for turn in 0..10_000_i64 {
                index.remove_position(index.by_position.len() / 2);
                index.push_key(key(turn % 1024));
            }
            let churn_ms = started.elapsed().as_secs_f64() * 1_000.0;
            eprintln!(
                "shared position index {label}: build_ms={build_ms:.3} churn10k_ms={churn_ms:.3} classes={}",
                index.by_key.len()
            );
        }
    }
}
