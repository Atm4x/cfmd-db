#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MaterializedSemanticQuotientFactorState {
    binding: SemanticIndexBinding,
    key_binding: kernel_semantic_index::SemanticIndexBinding,
    compiled: Vec<kernel_semantics::CompiledEquivalence>,
    retention: kernel_semantics::fiber_retention::DirectRowKeyMassRetention<
        PhysicalRowId,
        kernel_semantics::CanonicalEqKey,
    >,
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
        let mut retention = kernel_semantics::fiber_retention::DirectRowKeyMassRetention::default();
        for row_index in relation.scan_positions() {
            let row = materialize_native_row(&relation.data, row_index)?;
            let key = semantic_quotient_factor_row_key(&binding, &compiled, &row)?;
            retention
                .insert(relation.row_id_at(row_index)?, key)
                .map_err(row_key_mass_retention_error_to_physical)?;
        }
        Ok(Self {
            binding,
            key_binding,
            compiled,
            retention,
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
        self.retention.row_count()
    }

    #[must_use]
    fn statistics_snapshot(&self) -> SemanticKeyStatistics {
        SemanticKeyStatistics {
            row_count: self.row_count(),
            distinct_key_count: self.retention.distinct_joint_key_count(),
        }
    }

    fn single_key_for(&self, row_id: PhysicalRowId) -> Option<&kernel_semantics::CanonicalEqKey> {
        if self.binding.key_parts.len() != 1 {
            return None;
        }
        self.retention.row_key(&row_id).and_then(|key| key.first())
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
            if self.retention.row_key(row_id) != Some(key.as_slice()) {
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
            self.retention
                .remove(row_id, &key)
                .map_err(row_key_mass_retention_error_to_physical)?;
        }
        for (row_id, row) in &delta.inserted {
            let key = self.row_key(row)?;
            self.retention
                .insert(*row_id, key)
                .map_err(row_key_mass_retention_error_to_physical)?;
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

fn row_key_mass_retention_error_to_physical(
    error: kernel_semantics::fiber_retention::RowKeyMassRetentionError,
) -> PhysicalExecutionError {
    match error {
        kernel_semantics::fiber_retention::RowKeyMassRetentionError::CountOverflow => {
            PhysicalExecutionError::StatisticsCountOverflow
        }
        kernel_semantics::fiber_retention::RowKeyMassRetentionError::DuplicateRow
        | kernel_semantics::fiber_retention::RowKeyMassRetentionError::MissingRow
        | kernel_semantics::fiber_retention::RowKeyMassRetentionError::InconsistentDelta => {
            RelQueryError::InconsistentIncrementalDelta.into()
        }
    }
}
