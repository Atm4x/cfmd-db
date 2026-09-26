#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MaterializedSemanticQuotientFactorState {
    binding: SemanticIndexBinding,
    key_binding: kernel_semantic_index::SemanticIndexBinding,
    compiled: Vec<kernel_semantics::CompiledEquivalence>,
    buckets: PersistentOrdMap<Vec<kernel_semantics::CanonicalEqKey>, OrderedPhysicalRowBucket>,
    reverse: PersistentOrdMap<PhysicalRowId, Vec<kernel_semantics::CanonicalEqKey>>,
}

impl MaterializedSemanticQuotientFactorState {
    pub(super) fn build(
        binding: SemanticIndexBinding,
        relation: &InstalledRelation,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        if binding.key_parts.is_empty() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        let (_, dependencies) = resolve_semantic_key_binding(&binding, context, registry)?;
        let structural_definitions = semantic_key_structural_definitions_for_equivalences(
            binding.key_parts.iter().map(|part| part.equivalence),
            context,
            registry,
        )?;
        let key_binding =
            kernel_semantic_index::SemanticIndexBinding::new_with_structural_definitions(
                context,
                dependencies,
                structural_definitions,
            );
        let mut compiled = Vec::with_capacity(binding.key_parts.len());
        for part in &binding.key_parts {
            compiled.push(registry.compile_equivalence(context, part.equivalence)?);
        }
        let mut buckets = PersistentOrdMap::<
            Vec<kernel_semantics::CanonicalEqKey>,
            OrderedPhysicalRowBucket,
        >::default();
        let mut reverse = PersistentOrdMap::default();
        for row_index in relation.scan_positions() {
            let row = materialize_native_row(&relation.data, row_index)?;
            let key = semantic_quotient_factor_row_key(&binding, &compiled, &row)?;
            let row_id = relation.row_id_at(row_index)?;
            let mut bucket = buckets.get(&key).cloned().unwrap_or_default();
            bucket.push(row_id)?;
            buckets.insert(key.clone(), bucket);
            if reverse.insert(row_id, key).is_some() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
        }
        Ok(Self {
            binding,
            key_binding,
            compiled,
            buckets,
            reverse,
        })
    }

    pub(super) fn compatible_with(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, PhysicalExecutionError> {
        let (_, dependencies) = resolve_semantic_key_binding(&self.binding, context, registry)?;
        if !self.key_binding.is_valid_for(context, &dependencies)
            || self.binding.key_parts.len() != self.compiled.len()
        {
            return Ok(false);
        }
        for (part, compiled) in self.binding.key_parts.iter().zip(&self.compiled) {
            let actual = registry.compile_equivalence(context, part.equivalence)?;
            if actual != *compiled {
                return Ok(false);
            }
        }
        Ok(true)
    }

    #[must_use]
    fn row_count(&self) -> usize {
        self.reverse.len()
    }

    fn single_key_for(&self, row_id: PhysicalRowId) -> Option<&kernel_semantics::CanonicalEqKey> {
        if self.binding.key_parts.len() != 1 {
            return None;
        }
        self.reverse.get(&row_id).and_then(|key| key.first())
    }

    fn row_key(
        &self,
        row: &kernel_query::Row,
    ) -> Result<Vec<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError> {
        semantic_quotient_factor_row_key(&self.binding, &self.compiled, row)
    }

    fn validate_physical_delta(
        &self,
        delta: &PhysicalRelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        if !self.compatible_with(context, registry)? {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        }
        for (row_id, row) in &delta.removed {
            let key = self.row_key(row)?;
            if self.reverse.get(row_id) != Some(&key) {
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
        _context: &kernel_schema::SemanticContext,
        _registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        for (row_id, row) in &delta.removed {
            let key = self.row_key(row)?;
            if self.reverse.get(row_id) != Some(&key) {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
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
                self.buckets.insert(key.clone(), bucket);
            }
            self.reverse
                .remove(row_id)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        }
        for (row_id, row) in &delta.inserted {
            let key = self.row_key(row)?;
            if self.reverse.insert(*row_id, key.clone()).is_some() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            let mut bucket = self.buckets.get(&key).cloned().unwrap_or_default();
            bucket.push(*row_id)?;
            self.buckets.insert(key, bucket);
        }
        Ok(())
    }
}

fn semantic_quotient_factor_row_key(
    binding: &SemanticIndexBinding,
    compiled: &[kernel_semantics::CompiledEquivalence],
    row: &kernel_query::Row,
) -> Result<Vec<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError> {
    binding
        .key_parts
        .iter()
        .zip(compiled)
        .map(|(part, compiled)| {
            let value = row
                .get(part.column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            compiled.canonical_key(value).map_err(Into::into)
        })
        .collect()
}

