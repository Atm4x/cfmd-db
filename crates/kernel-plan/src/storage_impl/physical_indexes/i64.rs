#[derive(Debug, Clone, PartialEq, Eq, Default)]
 struct OrderedPhysicalRowBucket {
    rows: PersistentOrdMap<u64, PhysicalRowId>,
    ordinals: PersistentOrdMap<PhysicalRowId, u64>,
    next_ordinal: u64,
}

impl OrderedPhysicalRowBucket {
     fn push(&mut self, row_id: PhysicalRowId) -> Result<(), PhysicalExecutionError> {
        if self.ordinals.contains_key(&row_id) {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
        let ordinal = self.next_ordinal;
        self.next_ordinal = self
            .next_ordinal
            .checked_add(1)
            .ok_or(PhysicalExecutionError::StatisticsCountOverflow)?;
        if self.rows.insert(ordinal, row_id).is_some()
            || self.ordinals.insert(row_id, ordinal).is_some()
        {
            return Err(RelQueryError::InconsistentIncrementalDelta.into());
        }
        Ok(())
    }

     fn remove(&mut self, row_id: PhysicalRowId) -> bool {
        let Some(ordinal) = self.ordinals.remove(&row_id) else {
            return false;
        };
        self.rows.remove(&ordinal) == Some(row_id)
    }

     fn contains(&self, row_id: &PhysicalRowId) -> bool {
        self.ordinals.contains_key(row_id)
    }

     fn len(&self) -> usize {
        self.rows.len()
    }

    fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    fn estimated_retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.rows.estimated_heap_bytes())
            .saturating_add(self.ordinals.estimated_heap_bytes())
    }
}

pub(super) struct OrderedPhysicalRowBucketIter<'a> {
    inner: kernel_persistent::PersistentOrdMapIter<'a, u64, PhysicalRowId>,
}

impl<'a> Iterator for OrderedPhysicalRowBucketIter<'a> {
    type Item = &'a PhysicalRowId;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(_, row_id)| row_id)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl DoubleEndedIterator for OrderedPhysicalRowBucketIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|(_, row_id)| row_id)
    }
}

impl ExactSizeIterator for OrderedPhysicalRowBucketIter<'_> {}

pub(super) struct I64IndexProbe<'a> {
    inner: OrderedPhysicalRowBucketIter<'a>,
}

impl Iterator for I64IndexProbe<'_> {
    type Item = PhysicalRowId;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().copied()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl DoubleEndedIterator for I64IndexProbe<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().copied()
    }
}

impl ExactSizeIterator for I64IndexProbe<'_> {}

// HOSTILE[P185][ACTIVE][CLEAN]: execution sees only ordered row-handle probes and exact key cardinality;
// the materialized I64 index representation remains storage-owned.
#[derive(Debug, Clone, Copy)]
pub(super) struct I64IndexCapability<'a> {
    state: &'a MaterializedI64IndexState,
}

impl<'a> I64IndexCapability<'a> {
    fn new(state: &'a MaterializedI64IndexState) -> Self {
        Self { state }
    }

    pub(super) fn distinct_key_count(self) -> usize {
        self.state.distinct_key_count()
    }

     pub(super) fn probe(&self, key: i64) -> Option<I64IndexProbe<'_>> {
        self.state.probe(key).map(|bucket| I64IndexProbe {
            inner: bucket.into_iter(),
        })
    }
}

impl<'a> IntoIterator for &'a OrderedPhysicalRowBucket {
    type Item = &'a PhysicalRowId;
    type IntoIter = OrderedPhysicalRowBucketIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        OrderedPhysicalRowBucketIter {
            inner: self.rows.iter(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedI64IndexState {
    binding: I64IndexBinding,
    result_type: RelType,
    buckets: PersistentOrdMap<i64, OrderedPhysicalRowBucket>,
}

impl MaterializedI64IndexState {
     fn build(
        binding: I64IndexBinding,
        relation: &InstalledRelation,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        let definition = context
            .schema
            .relation(binding.relation)
            .ok_or(RelQueryError::UnknownRelation(binding.relation))?;
        let result_type = RelType {
            columns: definition.columns.clone(),
            semantics: definition.semantics.clone(),
        };
        let Some(resolved) =
            registry.resolve_primitive_equivalence(context, binding.equivalence)?
        else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        if !matches!(
            resolved.bind_right(&Value::I64(0)),
            Ok(kernel_semantics::BoundPrimitivePredicate::I64(_))
        ) {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        validate_indexable_i64_relation(context, binding.relation, &relation.data)?;
        let keys = native_i64_column(&relation.data, binding.key_column)
            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
        let mut buckets = PersistentOrdMap::<i64, OrderedPhysicalRowBucket>::default();
        for row_index in relation.scan_positions() {
            let key = keys.value(row_index);
            let mut bucket = buckets.get(&key).cloned().unwrap_or_default();
            bucket.push(relation.row_id_at(row_index)?)?;
            buckets.insert(key, bucket);
        }
        Ok(Self {
            binding,
            result_type,
            buckets,
        })
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.buckets
            .values()
            .map(OrderedPhysicalRowBucket::len)
            .sum()
    }

    #[must_use]
    pub fn distinct_key_count(&self) -> usize {
        self.buckets.len()
    }

    fn probe(&self, key: i64) -> Option<&OrderedPhysicalRowBucket> {
        self.buckets.get(&key)
    }

    fn apply_physical_delta(
        &mut self,
        delta: &PhysicalRelationDelta,
    ) -> Result<(), PhysicalExecutionError> {
        for (row_id, row) in &delta.removed {
            let key = match row.get(self.binding.key_column) {
                Some(Value::I64(value)) => *value,
                _ => return Err(PhysicalExecutionError::PhysicalTypeMismatch),
            };
            let mut bucket = self
                .buckets
                .get(&key)
                .cloned()
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if !bucket.remove(*row_id) {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            if bucket.is_empty() {
                self.buckets.remove(&key);
            } else {
                self.buckets.insert(key, bucket);
            }
        }
        for (row_id, row) in &delta.inserted {
            let key = match row.get(self.binding.key_column) {
                Some(Value::I64(value)) => *value,
                _ => return Err(PhysicalExecutionError::PhysicalTypeMismatch),
            };
            let mut bucket = self.buckets.get(&key).cloned().unwrap_or_default();
            bucket.push(*row_id)?;
            self.buckets.insert(key, bucket);
        }
        Ok(())
    }

    fn validate_physical_delta(
        &self,
        delta: &PhysicalRelationDelta,
    ) -> Result<(), PhysicalExecutionError> {
        for (row_id, row) in &delta.removed {
            let key = match row.get(self.binding.key_column) {
                Some(Value::I64(value)) => *value,
                _ => return Err(PhysicalExecutionError::PhysicalTypeMismatch),
            };
            if !self
                .buckets
                .get(&key)
                .is_some_and(|bucket| bucket.contains(row_id))
            {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
        }
        for (_, row) in &delta.inserted {
            if !matches!(row.get(self.binding.key_column), Some(Value::I64(_))) {
                return Err(PhysicalExecutionError::PhysicalTypeMismatch);
            }
        }
        Ok(())
    }
}

