#[derive(Debug, Clone, PartialEq, Eq)]
// HOSTILE[P161][COMPAT][P160.L]: legacy/manual/durable semantic-index compatibility surface.
pub struct MaterializedSemanticIndexState {
    binding: SemanticIndexBinding,
    resolved: Vec<ResolvedSemanticIndexKeyPart>,
    key_binding: kernel_semantic_index::SemanticIndexBinding,
    index: kernel_semantic_index::SemanticBucketIndex<
        Vec<kernel_semantics::CanonicalEqKey>,
        PhysicalRowId,
    >,
}

impl MaterializedSemanticIndexState {
    pub(super) fn build(
        binding: SemanticIndexBinding,
        relation: &InstalledRelation,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        let (resolved, dependencies) = resolve_semantic_key_binding(&binding, context, registry)?;
        let structural_definitions = semantic_key_structural_definitions_for_equivalences(
            binding.key_parts.iter().map(|part| part.equivalence),
            context,
            registry,
        )?;
        let key_binding =
            kernel_semantic_index::SemanticIndexBinding::new_with_structural_definitions(
                context,
                dependencies.clone(),
                structural_definitions,
            );
        let mut index = kernel_semantic_index::SemanticBucketIndex::new(key_binding.clone());
        for row_index in relation.scan_positions() {
            let row = materialize_native_row(&relation.data, row_index)?;
            let key = resolved_semantic_index_row_key(&binding, &resolved, &row)?;
            let previous = index.insert(relation.row_id_at(row_index)?, key);
            debug_assert!(previous.is_none());
        }
        Ok(Self {
            binding,
            resolved,
            key_binding,
            index,
        })
    }

    pub(super) fn compatible_with(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, PhysicalExecutionError> {
        let (resolved, dependencies) =
            resolve_semantic_key_binding(&self.binding, context, registry)?;
        Ok(resolved == self.resolved && self.key_binding.is_valid_for(context, &dependencies))
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.index.len()
    }

    #[must_use]
    pub fn distinct_key_count(&self) -> usize {
        self.index.distinct_key_count()
    }

    fn single_key_for(&self, row_id: PhysicalRowId) -> Option<kernel_semantics::CanonicalEqKey> {
        if self.binding.key_parts.len() != 1 {
            return None;
        }
        self.index
            .key_for(&row_id)
            .and_then(|key| key.first())
            .cloned()
    }

    fn probe_values(
        &self,
        values: &[&Value],
        _context: &kernel_schema::SemanticContext,
        _registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<&kernel_semantic_index::SemanticBucket<PhysicalRowId>>, PhysicalExecutionError>
    {
        let mut key = Vec::with_capacity(values.len());
        self.probe_values_with_scratch(values, &mut key)
    }

    fn probe_values_with_scratch(
        &self,
        values: &[&Value],
        key: &mut Vec<kernel_semantics::CanonicalEqKey>,
    ) -> Result<Option<&kernel_semantic_index::SemanticBucket<PhysicalRowId>>, PhysicalExecutionError>
    {
        if values.len() != self.resolved.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        key.clear();
        key.reserve(self.resolved.len());
        for (resolved, value) in self.resolved.iter().zip(values) {
            key.push(match resolved {
                ResolvedSemanticIndexKeyPart::Primitive(module) => {
                    module.canonical_key(value).map_err(PhysicalExecutionError::from)
                }
                ResolvedSemanticIndexKeyPart::Structural(compiled) => {
                    compiled.canonical_key(value).map_err(PhysicalExecutionError::from)
                }
            }?);
        }
        Ok(self.index.bucket(key))
    }

    fn probe_row_columns_with_scratch<I>(
        &self,
        row: &[Value],
        columns: I,
        key: &mut Vec<kernel_semantics::CanonicalEqKey>,
    ) -> Result<Option<&kernel_semantic_index::SemanticBucket<PhysicalRowId>>, PhysicalExecutionError>
    where
        I: ExactSizeIterator<Item = usize>,
    {
        if columns.len() != self.resolved.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        key.clear();
        key.reserve(self.resolved.len());
        for (resolved, column) in self.resolved.iter().zip(columns) {
            let value = row
                .get(column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            key.push(match resolved {
                ResolvedSemanticIndexKeyPart::Primitive(module) => {
                    module.canonical_key(value).map_err(PhysicalExecutionError::from)
                }
                ResolvedSemanticIndexKeyPart::Structural(compiled) => {
                    compiled.canonical_key(value).map_err(PhysicalExecutionError::from)
                }
            }?);
        }
        Ok(self.index.bucket(key))
    }


    fn row_key(
        &self,
        row: &kernel_query::Row,
        _context: &kernel_schema::SemanticContext,
        _registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Vec<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError> {
        resolved_semantic_index_row_key(&self.binding, &self.resolved, row)
    }

    fn validate_physical_delta(
        &self,
        delta: &PhysicalRelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        for (row_id, row) in &delta.removed {
            let key = self.row_key(row, context, registry)?;
            if self.index.key_for(row_id) != Some(&key) {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
        }
        for (_, row) in &delta.inserted {
            let _ = self.row_key(row, context, registry)?;
        }
        Ok(())
    }

    fn apply_physical_delta(
        &mut self,
        delta: &PhysicalRelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        for (row_id, row) in &delta.removed {
            let key = self.row_key(row, context, registry)?;
            if self.index.key_for(row_id) != Some(&key) || self.index.remove(row_id).is_none() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
        }
        for (row_id, row) in &delta.inserted {
            let key = self.row_key(row, context, registry)?;
            if self.index.insert(*row_id, key).is_some() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
        }
        Ok(())
    }
}
