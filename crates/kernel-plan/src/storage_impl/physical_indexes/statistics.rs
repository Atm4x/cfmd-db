#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticKeyStatistics {
    pub row_count: usize,
    pub distinct_key_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
 struct MaterializedSemanticStatisticsState {
    binding: SemanticIndexBinding,
    resolved: Vec<kernel_semantics::ResolvedPrimitiveEquivalence>,
    key_binding: kernel_semantic_index::SemanticIndexBinding,
    counts: PersistentOrdMap<Vec<kernel_semantics::CanonicalEqKey>, usize>,
    row_count: usize,
}

impl MaterializedSemanticStatisticsState {
    fn build(
        binding: SemanticIndexBinding,
        relation: &InstalledRelation,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        let (resolved, dependencies) =
            resolve_primitive_semantic_index_binding(&binding, context, registry)?;
        let key_binding = kernel_semantic_index::SemanticIndexBinding::new(context, dependencies);
        let mut counts = PersistentOrdMap::default();
        for row_index in relation.scan_positions() {
            let row = materialize_native_row(&relation.data, row_index)?;
            let key = semantic_index_row_key(&binding, &resolved, &row)?;
            let count: usize = counts.get(&key).copied().unwrap_or(0);
            let count = count
                .checked_add(1)
                .ok_or(PhysicalExecutionError::StatisticsCountOverflow)?;
            counts.insert(key, count);
        }
        Ok(Self {
            binding,
            resolved,
            key_binding,
            counts,
            row_count: native_row_count(&relation.data),
        })
    }

    fn compatible_with(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, PhysicalExecutionError> {
        let (resolved, dependencies) =
            resolve_primitive_semantic_index_binding(&self.binding, context, registry)?;
        Ok(resolved == self.resolved && self.key_binding.is_valid_for(context, &dependencies))
    }

    fn row_key(
        &self,
        row: &kernel_query::Row,
    ) -> Result<Vec<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError> {
        semantic_index_row_key(&self.binding, &self.resolved, row)
    }

    fn validate_physical_delta(
        &self,
        delta: &PhysicalRelationDelta,
    ) -> Result<(), PhysicalExecutionError> {
        let mut removals = BTreeMap::<Vec<kernel_semantics::CanonicalEqKey>, usize>::new();
        for (_, row) in &delta.removed {
            let key = self.row_key(row)?;
            let count = removals.entry(key.clone()).or_insert(0_usize);
            *count = count
                .checked_add(1)
                .ok_or(PhysicalExecutionError::StatisticsCountOverflow)?;
            if *count > self.counts.get(&key).copied().unwrap_or(0) {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
        }
        for (_, row) in &delta.inserted {
            let _ = self.row_key(row)?;
        }
        Ok(())
    }

    fn apply_physical_delta(
        &mut self,
        delta: &PhysicalRelationDelta,
    ) -> Result<(), PhysicalExecutionError> {
        for (_, row) in &delta.removed {
            let key = self.row_key(row)?;
            let count = self
                .counts
                .get(&key)
                .copied()
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let count = count
                .checked_sub(1)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if count == 0 {
                self.counts.remove(&key);
            } else {
                self.counts.insert(key, count);
            }
            self.row_count = self
                .row_count
                .checked_sub(1)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        }
        for (_, row) in &delta.inserted {
            let key = self.row_key(row)?;
            let count = self.counts.get(&key).copied().unwrap_or(0);
            let count = count
                .checked_add(1)
                .ok_or(PhysicalExecutionError::StatisticsCountOverflow)?;
            self.counts.insert(key, count);
            self.row_count = self
                .row_count
                .checked_add(1)
                .ok_or(PhysicalExecutionError::StatisticsCountOverflow)?;
        }
        Ok(())
    }

    fn snapshot(&self) -> SemanticKeyStatistics {
        SemanticKeyStatistics {
            row_count: self.row_count,
            distinct_key_count: self.counts.len(),
        }
    }
}

fn semantic_index_row_key(
    binding: &SemanticIndexBinding,
    resolved: &[kernel_semantics::ResolvedPrimitiveEquivalence],
    row: &kernel_query::Row,
) -> Result<Vec<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError> {
    binding
        .key_parts
        .iter()
        .zip(resolved)
        .map(|(part, module)| {
            let value = row
                .get(part.column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            module.canonical_key(value).map_err(Into::into)
        })
        .collect()
}

