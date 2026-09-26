#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SemanticQuotientLeaf {
    pub(super) leaf: usize,
    pub(super) keys: Vec<Option<kernel_semantics::CanonicalEqKey>>,
    pub(super) buckets: BTreeMap<kernel_semantics::CanonicalEqKey, Vec<u64>>,
    pub(super) live_rows_by_key: BTreeMap<kernel_semantics::CanonicalEqKey, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SemanticQuotientConstraint {
    pub(super) leaves: Vec<SemanticQuotientLeaf>,
    pub(super) key_leaf_support: BTreeMap<kernel_semantics::CanonicalEqKey, usize>,
    pub(super) live_key_leaf_support: BTreeMap<kernel_semantics::CanonicalEqKey, usize>,
}

type SemanticQuotientKeySpec = (usize, usize, SemanticId);
type SemanticQuotientKeyCache =
    BTreeMap<SemanticQuotientKeySpec, Vec<kernel_semantics::CanonicalEqKey>>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StableSemanticQuotientRows {
    by_ordinal: PersistentPhysicalVec<Option<PhysicalRowId>>,
    ordinal_by_slot: PersistentPhysicalVec<Option<(u64, usize)>>,
     live_count: usize,
}

impl StableSemanticQuotientRows {
     const COMPACTION_SLACK: usize = 1_024;

     fn from_dense(handles: &[PhysicalRowId]) -> Self {
        let mut by_ordinal = PersistentPhysicalVec::default();
        let mut ordinal_by_slot = PersistentPhysicalVec::default();
        for (ordinal, handle) in handles.iter().copied().enumerate() {
            while ordinal_by_slot.len() <= handle.slot {
                ordinal_by_slot.push(None);
            }
            by_ordinal.push(Some(handle));
            ordinal_by_slot.set(handle.slot, Some((handle.generation, ordinal)));
        }
        Self {
            by_ordinal,
            ordinal_by_slot,
            live_count: handles.len(),
        }
    }

     fn apply_delta(
        &self,
        delta: &PhysicalRelationDelta,
    ) -> Result<(Self, Vec<PhysicalRowId>), PhysicalExecutionError> {
        let mut next = self.clone();
        for (handle, _) in &delta.removed {
            let Some((generation, ordinal)) =
                next.ordinal_by_slot.get(handle.slot).copied().flatten()
            else {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            };
            if generation != handle.generation
                || next.by_ordinal.get(ordinal).copied().flatten() != Some(*handle)
            {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            next.by_ordinal.set(ordinal, None);
            next.ordinal_by_slot.set(handle.slot, None);
            next.live_count = next.live_count.saturating_sub(1);
        }
        let mut inserted = Vec::with_capacity(delta.inserted.len());
        for (handle, _) in &delta.inserted {
            while next.ordinal_by_slot.len() <= handle.slot {
                next.ordinal_by_slot.push(None);
            }
            if next.ordinal_by_slot[handle.slot].is_some() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            let ordinal = next.by_ordinal.len();
            next.by_ordinal.push(Some(*handle));
            next.ordinal_by_slot
                .set(handle.slot, Some((handle.generation, ordinal)));
            next.live_count = next.live_count.saturating_add(1);
            inserted.push(*handle);
        }
        Ok((next, inserted))
    }

    #[cfg(test)]
     fn dense_handles(&self) -> Vec<PhysicalRowId> {
        self.by_ordinal
            .iter()
            .filter_map(|handle| *handle)
            .collect()
    }

     fn matches_dense(&self, handles: &[PhysicalRowId]) -> bool {
        self.live_count == handles.len()
            && self
                .by_ordinal
                .iter()
                .filter_map(|handle| *handle)
                .eq(handles.iter().copied())
    }

    fn logically_equals(&self, other: &Self) -> bool {
        self.live_count == other.live_count
            && self
                .by_ordinal
                .iter()
                .filter_map(|handle| *handle)
                .eq(other.by_ordinal.iter().filter_map(|handle| *handle))
    }

    fn ordinal_for(&self, handle: PhysicalRowId) -> Option<usize> {
        self.ordinal_by_slot
            .get(handle.slot)
            .copied()
            .flatten()
            .and_then(|(generation, ordinal)| (generation == handle.generation).then_some(ordinal))
    }

     fn should_compact(&self) -> bool {
        self.by_ordinal.len()
            > self
                .live_count
                .saturating_mul(2)
                .saturating_add(Self::COMPACTION_SLACK)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
 struct StableSemanticQuotientKeyBucket {
    ordinals: PersistentPhysicalVec<Option<usize>>,
    live_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct StableSemanticQuotientKeyState {
    by_ordinal: PersistentPhysicalVec<Option<kernel_semantics::CanonicalEqKey>>,
    bucket_id_by_key: PersistentOrdMap<kernel_semantics::CanonicalEqKey, usize>,
    buckets: PersistentPhysicalVec<StableSemanticQuotientKeyBucket>,
    bucket_position_by_ordinal: PersistentPhysicalVec<Option<(usize, usize)>>,
}

impl StableSemanticQuotientKeyState {
    fn bucket_storage_estimated_heap_bytes(&self) -> usize {
        self.buckets.estimated_heap_bytes()
    }

     const BUCKET_COMPACTION_SLACK: usize = 1_024;

    fn from_dense(keys: &[Option<kernel_semantics::CanonicalEqKey>]) -> Self {
        let mut state = Self::default();
        for (ordinal, key) in keys.iter().cloned().enumerate() {
            state.push_ordinal(ordinal, key);
        }
        state
    }

    fn key_at(&self, ordinal: usize) -> Option<&kernel_semantics::CanonicalEqKey> {
        self.by_ordinal.get(ordinal).and_then(Option::as_ref)
    }

    fn contains_key(&self, key: &kernel_semantics::CanonicalEqKey) -> bool {
        self.bucket_id_by_key
            .get(key)
            .and_then(|bucket| self.buckets.get(*bucket))
            .is_some_and(|bucket| bucket.live_count != 0)
    }

     fn live_ordinals<'a>(
        &'a self,
        key: &kernel_semantics::CanonicalEqKey,
    ) -> impl Iterator<Item = usize> + 'a {
        self.bucket_id_by_key
            .get(key)
            .and_then(|bucket| self.buckets.get(*bucket))
            .into_iter()
            .flat_map(|bucket| bucket.ordinals.iter().filter_map(|ordinal| *ordinal))
    }

     fn remove_ordinal(&mut self, ordinal: usize) -> Option<kernel_semantics::CanonicalEqKey> {
        let key = self.by_ordinal.get(ordinal).and_then(Clone::clone)?;
        let (bucket_id, position) = self
            .bucket_position_by_ordinal
            .get(ordinal)
            .copied()
            .flatten()
            .expect("keyed stable ordinal must have bucket position");
        let mut bucket = self.buckets[bucket_id].clone();
        bucket.ordinals.set(position, None);
        bucket.live_count = bucket.live_count.saturating_sub(1);
        if bucket.ordinals.len()
            > bucket
                .live_count
                .saturating_mul(2)
                .saturating_add(Self::BUCKET_COMPACTION_SLACK)
        {
            let mut compact = PersistentPhysicalVec::default();
            for live_ordinal in bucket.ordinals.iter().filter_map(|ordinal| *ordinal) {
                let new_position = compact.len();
                compact.push(Some(live_ordinal));
                self.bucket_position_by_ordinal
                    .set(live_ordinal, Some((bucket_id, new_position)));
            }
            bucket.ordinals = compact;
        }
        self.buckets.set(bucket_id, bucket);
        self.by_ordinal.set(ordinal, None);
        self.bucket_position_by_ordinal.set(ordinal, None);
        Some(key)
    }

     fn push_ordinal(&mut self, ordinal: usize, key: Option<kernel_semantics::CanonicalEqKey>) {
        debug_assert_eq!(ordinal, self.by_ordinal.len());
        self.by_ordinal.push(key.clone());
        self.bucket_position_by_ordinal.push(None);
        let Some(key) = key else { return };
        let bucket_id = if let Some(bucket) = self.bucket_id_by_key.get(&key).copied() {
            bucket
        } else {
            let bucket = self.buckets.len();
            self.bucket_id_by_key.insert(key.clone(), bucket);
            self.buckets
                .push(StableSemanticQuotientKeyBucket::default());
            bucket
        };
        let mut bucket = self.buckets[bucket_id].clone();
        let position = bucket.ordinals.len();
        bucket.ordinals.push(Some(ordinal));
        bucket.live_count = bucket.live_count.saturating_add(1);
        self.buckets.set(bucket_id, bucket);
        self.bucket_position_by_ordinal
            .set(ordinal, Some((bucket_id, position)));
    }
}

fn advance_stable_semantic_quotient_keys(
    current: &StableSemanticQuotientKeyState,
    current_rows: &StableSemanticQuotientRows,
    next_rows: &StableSemanticQuotientRows,
    delta: &PhysicalRelationDelta,
    inserted_keys: &[(PhysicalRowId, Option<kernel_semantics::CanonicalEqKey>)],
) -> Option<StableSemanticQuotientKeyState> {
    let mut next = current.clone();
    for (handle, _) in &delta.removed {
        let ordinal = current_rows.ordinal_for(*handle)?;
        next.remove_ordinal(ordinal);
    }
    for (handle, key) in inserted_keys {
        let ordinal = next_rows.ordinal_for(*handle)?;
        if ordinal != next.by_ordinal.len() {
            return None;
        }
        next.push_ordinal(ordinal, key.clone());
    }
    Some(next)
}

fn compact_semantic_quotient_leaf_state(
    rows: &StableSemanticQuotientRows,
    key_states: &mut [(usize, usize, StableSemanticQuotientKeyState)],
) -> Result<StableSemanticQuotientRows, PhysicalExecutionError> {
    let mut compact_rows = StableSemanticQuotientRows {
        by_ordinal: PersistentPhysicalVec::default(),
        ordinal_by_slot: PersistentPhysicalVec::default(),
        live_count: 0,
    };
    let mut compact_keys = key_states
        .iter()
        .map(|_| StableSemanticQuotientKeyState::default())
        .collect::<Vec<_>>();

    for old_ordinal in 0..rows.by_ordinal.len() {
        let Some(handle) = rows.by_ordinal.get(old_ordinal).copied().flatten() else {
            continue;
        };
        while compact_rows.ordinal_by_slot.len() <= handle.slot {
            compact_rows.ordinal_by_slot.push(None);
        }
        let new_ordinal = compact_rows.by_ordinal.len();
        compact_rows.by_ordinal.push(Some(handle));
        compact_rows
            .ordinal_by_slot
            .set(handle.slot, Some((handle.generation, new_ordinal)));
        compact_rows.live_count = compact_rows.live_count.saturating_add(1);

        for ((_, _, state), compact) in key_states.iter().zip(&mut compact_keys) {
            let key = state.by_ordinal.get(old_ordinal).cloned().flatten();
            compact.push_ordinal(new_ordinal, key);
        }
    }

    if compact_rows.live_count != rows.live_count {
        return Err(RelQueryError::InconsistentIncrementalDelta.into());
    }
    for ((_, _, state), compact) in key_states.iter_mut().zip(compact_keys) {
        *state = compact;
    }
    Ok(compact_rows)
}

fn initial_stable_semantic_quotient_keys(
    constraints: &[SemanticQuotientConstraint],
) -> Vec<Vec<Arc<StableSemanticQuotientKeyState>>> {
    constraints
        .iter()
        .map(|constraint| {
            constraint
                .leaves
                .iter()
                .map(|leaf| Arc::new(StableSemanticQuotientKeyState::from_dense(&leaf.keys)))
                .collect()
        })
        .collect()
}

fn semantic_quotient_constraint_leaves(
    constraints: &[SemanticQuotientConstraint],
) -> Vec<Vec<usize>> {
    constraints
        .iter()
        .map(|constraint| constraint.leaves.iter().map(|leaf| leaf.leaf).collect())
        .collect()
}

fn initial_semantic_quotient_dense_projection(
    constraints: &[Arc<SemanticQuotientConstraint>],
) -> OnceLock<Arc<Vec<SemanticQuotientConstraint>>> {
    let projection = OnceLock::new();
    let _ = projection.set(Arc::new(
        constraints
            .iter()
            .map(|constraint| constraint.as_ref().clone())
            .collect(),
    ));
    projection
}

