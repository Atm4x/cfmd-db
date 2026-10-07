#[derive(Debug, Clone, PartialEq, Eq)]
// HOSTILE[P581][ACTIVE][PRIMARY]: catalog-free SAMF over one store/revision atomic semantic-class
// authority. Product identity is the exact tuple of atomic EqClassIds; local atom IDs are only
// reconstructible routing tokens and never semantic/durable authority.
pub struct MaterializedObservableAtomState {
    binding: SemanticIndexBinding,
    key_binding: kernel_semantic_index::SemanticIndexBinding,
    fabric: kernel_semantics::support_atom::SemanticSupportFabric<PhysicalRowId>,
    semantic_class_catalog:
        Arc<kernel_semantics::semantic_class_catalog::RevisionSemanticClassCatalog>,
}

impl MaterializedObservableAtomState {
    fn build_from_semantic_columns(
        binding: SemanticIndexBinding,
        relation: &InstalledRelation,
        semantic_class_catalog: Arc<
            kernel_semantics::semantic_class_catalog::RevisionSemanticClassCatalog,
        >,
        encoded_columns: &[Arc<RevisionSemanticEncodedColumn>],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        if binding.key_parts.is_empty() || encoded_columns.len() != binding.key_parts.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        if semantic_class_catalog.revision() != context.revision() {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        }
        for (part, column) in binding.key_parts.iter().zip(encoded_columns) {
            if column.relation != binding.relation
                || column.layout != binding.layout
                || column.column != part.column
                || column.equivalence != part.equivalence
            {
                return Err(PhysicalExecutionError::PhysicalTypeMismatch);
            }
            let _ = registry.equivalence_domain(context, part.equivalence)?;
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
        let mut fabric = kernel_semantics::support_atom::SemanticSupportFabric::new(
            context.revision(),
            binding.key_parts.len(),
        );
        for position in relation.scan_positions() {
            let row_id = relation.row_id_at(position)?;
            let signature = encoded_columns
                .iter()
                .map(|column| {
                    column
                        .row_classes
                        .get(&row_id)
                        .copied()
                        .ok_or(RelQueryError::InconsistentIncrementalDelta.into())
                })
                .collect::<Result<Vec<_>, PhysicalExecutionError>>()?;
            for class in &signature {
                if semantic_class_catalog.class_key(*class).is_none() {
                    return Err(RelQueryError::InconsistentIncrementalDelta.into());
                }
            }
            fabric
                .insert(row_id, &signature)
                .map_err(semantic_support_fabric_error_to_physical)?;
        }
        Ok(Self {
            binding,
            key_binding,
            fabric,
            semantic_class_catalog,
        })
    }

    fn build_from_durable_core(
        binding: SemanticIndexBinding,
        relation: &InstalledRelation,
        encoded_keys_by_ordinal: &[Vec<u8>],
        semantic_class_catalog: Arc<
            kernel_semantics::semantic_class_catalog::RevisionSemanticClassCatalog,
        >,
        encoded_columns: &[Arc<RevisionSemanticEncodedColumn>],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        let state = Self::build_from_semantic_columns(
            binding,
            relation,
            semantic_class_catalog,
            encoded_columns,
            context,
            registry,
        )?;
        let positions = relation.scan_positions().collect::<Vec<_>>();
        if positions.len() != encoded_keys_by_ordinal.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        for (position, durable) in positions.into_iter().zip(encoded_keys_by_ordinal) {
            let row_id = relation.row_id_at(position)?;
            let keys = state
                .canonical_key_tuple_for_row(row_id)
                .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
            if kernel_semantics::encode_canonical_eq_key_tuple(&keys) != *durable {
                return Err(PhysicalExecutionError::PhysicalTypeMismatch);
            }
        }
        Ok(state)
    }

    fn durable_core(
        &self,
        source_revision: RevisionId,
        relation: &InstalledRelation,
    ) -> Result<DurableArtifactCore, PhysicalExecutionError> {
        let encoded_keys_by_ordinal = relation
            .scan_positions()
            .map(|position| {
                let row_id = relation.row_id_at(position)?;
                let keys = self
                    .canonical_key_tuple_for_row(row_id)
                    .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
                Ok(kernel_semantics::encode_canonical_eq_key_tuple(&keys))
            })
            .collect::<Result<Vec<_>, PhysicalExecutionError>>()?;
        Ok(DurableArtifactCore::ObservableAtom {
            source_revision,
            relation: self.binding.relation,
            key_parts: durable_semantic_key_parts(&self.binding),
            encoded_keys_by_ordinal,
        })
    }

    pub(super) fn compatible_with(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, PhysicalExecutionError> {
        let (_, dependencies) = resolve_semantic_key_binding(&self.binding, context, registry)?;
        if self.fabric.revision() != context.revision()
            || self.semantic_class_catalog.revision() != context.revision()
            || !self.key_binding.is_valid_for(context, &dependencies)
        {
            return Ok(false);
        }
        for part in &self.binding.key_parts {
            let _ = registry.equivalence_domain(context, part.equivalence)?;
        }
        Ok(true)
    }

    fn refresh_semantic_catalog(
        &mut self,
        catalog: Arc<kernel_semantics::semantic_class_catalog::RevisionSemanticClassCatalog>,
    ) {
        self.semantic_class_catalog = catalog;
    }

    #[must_use]
    pub fn semantic_revision(&self) -> kernel_types::SemanticRevision {
        self.semantic_class_catalog.revision()
    }

    #[must_use]
    pub fn arity(&self) -> usize {
        self.fabric.arity()
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.fabric.row_count()
    }

    #[must_use]
    pub fn atom_count(&self) -> usize {
        self.fabric.atom_count()
    }

    #[must_use]
    pub fn distinct_key_count(&self) -> usize {
        self.fabric.atom_count()
    }

    #[must_use]
    pub fn statistics_snapshot(&self) -> SemanticKeyStatistics {
        SemanticKeyStatistics {
            row_count: self.row_count(),
            distinct_key_count: self.distinct_key_count(),
        }
    }

    fn canonical_key_tuple_for_row(
        &self,
        row_id: PhysicalRowId,
    ) -> Option<Vec<kernel_semantics::CanonicalEqKey>> {
        self.fabric
            .row_signature(&row_id)?
            .iter()
            .map(|class| self.semantic_class_catalog.class_key(*class).cloned())
            .collect()
    }

    fn single_key_for(&self, row_id: PhysicalRowId) -> Option<kernel_semantics::CanonicalEqKey> {
        if self.binding.key_parts.len() != 1 {
            return None;
        }
        self.canonical_key_tuple_for_row(row_id)?.into_iter().next()
    }

    /// Exact Set-uniqueness violation measure induced by catalog-free SAMF atom masses.
    /// Product identity is the atomic semantic-class signature itself, not a second `EqClassId`.
    pub fn uniqueness_violation_measure(
        &self,
    ) -> Result<kernel_violation::ViolationMeasure<Vec<kernel_types::EqClassId>>, PhysicalExecutionError>
    {
        let mut measure = kernel_violation::ViolationMeasure::new();
        for (signature, mass) in self.fabric.atom_masses() {
            if mass > 1 {
                let violation_mass = u64::try_from(mass - 1)
                    .map_err(|_| PhysicalExecutionError::StatisticsCountOverflow)?;
                measure
                    .add(signature.to_vec(), violation_mass)
                    .map_err(|_| PhysicalExecutionError::StatisticsCountOverflow)?;
            }
        }
        Ok(measure)
    }

    fn probe_fiber_iter<'s, 'v, I>(
        &'s self,
        values: I,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        classes: &mut Vec<kernel_types::EqClassId>,
    ) -> Result<Option<&'s kernel_persistent::PersistentOrdSet<PhysicalRowId>>, PhysicalExecutionError>
    where
        I: ExactSizeIterator<Item = &'v Value>,
    {
        if values.len() != self.binding.key_parts.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        if self.semantic_class_catalog.revision() != context.revision() {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        }
        classes.clear();
        classes.reserve(self.binding.key_parts.len());
        for (part, value) in self.binding.key_parts.iter().zip(values) {
            let Some(class) = self
                .semantic_class_catalog
                .lookup_value(registry, context, part.equivalence, value)
                .map_err(semantic_class_catalog_error_to_physical)?
            else {
                return Ok(None);
            };
            classes.push(class);
        }
        Ok(self.fabric.joint_fiber(classes))
    }

    fn probe_fiber(
        &self,
        values: &[&Value],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<&kernel_persistent::PersistentOrdSet<PhysicalRowId>>, PhysicalExecutionError> {
        let mut classes = Vec::with_capacity(values.len());
        self.probe_fiber_iter(values.iter().copied(), context, registry, &mut classes)
    }

    fn probe_row_fiber_with_scratch<'a>(
        &'a self,
        values: &[Value],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        classes: &mut Vec<kernel_types::EqClassId>,
    ) -> Result<Option<&'a kernel_persistent::PersistentOrdSet<PhysicalRowId>>, PhysicalExecutionError> {
        self.probe_fiber_iter(values.iter(), context, registry, classes)
    }

    fn probe_row_columns_with_scratch<'a, I>(
        &'a self,
        row: &[Value],
        columns: I,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        classes: &mut Vec<kernel_types::EqClassId>,
    ) -> Result<Option<&'a kernel_persistent::PersistentOrdSet<PhysicalRowId>>, PhysicalExecutionError>
    where
        I: ExactSizeIterator<Item = usize>,
    {
        if columns.len() != self.binding.key_parts.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        if self.semantic_class_catalog.revision() != context.revision() {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        }
        classes.clear();
        classes.reserve(self.binding.key_parts.len());
        for (part, column) in self.binding.key_parts.iter().zip(columns) {
            let value = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?;
            let Some(class) = self
                .semantic_class_catalog
                .lookup_value(registry, context, part.equivalence, value)
                .map_err(semantic_class_catalog_error_to_physical)?
            else {
                return Ok(None);
            };
            classes.push(class);
        }
        Ok(self.fabric.joint_fiber(classes))
    }

    fn probe_values(
        &self,
        values: &[&Value],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Vec<PhysicalRowId>>, PhysicalExecutionError> {
        Ok(self
            .probe_fiber(values, context, registry)?
            .map(|rows| rows.iter().copied().collect()))
    }

    fn probe_slot_value(
        &self,
        slot: usize,
        value: &Value,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Vec<PhysicalRowId>, PhysicalExecutionError> {
        let part = self
            .binding
            .key_parts
            .get(slot)
            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
        let Some(class) = self
            .semantic_class_catalog
            .lookup_value(registry, context, part.equivalence, value)
            .map_err(semantic_class_catalog_error_to_physical)?
        else {
            return Ok(Vec::new());
        };
        self.fabric
            .projected_fiber(slot, class)
            .map(|rows| rows.into_iter().collect())
            .map_err(semantic_support_fabric_error_to_physical)
    }

    fn count_slot_value(
        &self,
        slot: usize,
        value: &Value,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<usize, PhysicalExecutionError> {
        let part = self
            .binding
            .key_parts
            .get(slot)
            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
        let Some(class) = self
            .semantic_class_catalog
            .lookup_value(registry, context, part.equivalence, value)
            .map_err(semantic_class_catalog_error_to_physical)?
        else {
            return Ok(0);
        };
        self.fabric
            .projected_count(slot, class)
            .map_err(semantic_support_fabric_error_to_physical)
    }

    fn validate_physical_delta(
        &self,
        delta: &PhysicalRelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        if self.semantic_class_catalog.revision() != context.revision() {
            return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
        }
        for part in &self.binding.key_parts {
            let _ = registry.equivalence_domain(context, part.equivalence)?;
        }
        for (row_id, _) in &delta.removed {
            if self.fabric.row_signature(row_id).is_none() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
        }
        for (row_id, row) in &delta.inserted {
            if self.fabric.row_signature(row_id).is_some() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            for part in &self.binding.key_parts {
                row.get(part.column).ok_or(RelQueryError::ColumnOutOfBounds)?;
            }
        }
        Ok(())
    }

    fn remove_delta_rows(
        &mut self,
        delta: &PhysicalRelationDelta,
    ) -> Result<(), PhysicalExecutionError> {
        for (row_id, _) in &delta.removed {
            self.fabric
                .remove(row_id)
                .map_err(semantic_support_fabric_error_to_physical)?;
        }
        Ok(())
    }

    fn insert_delta_rows_from_columns(
        &mut self,
        delta: &PhysicalRelationDelta,
        encoded_columns: &[Arc<RevisionSemanticEncodedColumn>],
    ) -> Result<(), PhysicalExecutionError> {
        if encoded_columns.len() != self.binding.key_parts.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        for (row_id, _) in &delta.inserted {
            let signature = encoded_columns
                .iter()
                .map(|column| {
                    column
                        .row_classes
                        .get(row_id)
                        .copied()
                        .ok_or(RelQueryError::InconsistentIncrementalDelta.into())
                })
                .collect::<Result<Vec<_>, PhysicalExecutionError>>()?;
            self.fabric
                .insert(*row_id, &signature)
                .map_err(semantic_support_fabric_error_to_physical)?;
        }
        Ok(())
    }
}

fn semantic_support_fabric_error_to_physical<RowId>(
    _error: kernel_semantics::support_atom::SemanticSupportFabricError<RowId>,
) -> PhysicalExecutionError {
    PhysicalExecutionError::PhysicalTypeMismatch
}

// Catalog-free SAMF remains the single semantic-fiber execution capability. The shared catalog
// supplies canonical payload; the fabric supplies joint/projection retention only.
pub(super) struct SemanticFiberCapability<'a>(&'a MaterializedObservableAtomState);

pub(super) struct SemanticFiberProbe<'a>(
    <&'a kernel_persistent::PersistentOrdSet<PhysicalRowId> as IntoIterator>::IntoIter,
);

#[derive(Default)]
pub(super) struct SemanticFiberProbeScratch {
    observable_classes: Vec<kernel_types::EqClassId>,
}

impl Iterator for SemanticFiberProbe<'_> {
    type Item = PhysicalRowId;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().copied()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl DoubleEndedIterator for SemanticFiberProbe<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().copied()
    }
}

impl ExactSizeIterator for SemanticFiberProbe<'_> {}

impl<'a> SemanticFiberCapability<'a> {
    pub(super) const fn new(state: &'a MaterializedObservableAtomState) -> Self {
        Self(state)
    }

    fn row_count(&self) -> usize {
        self.0.row_count()
    }

    pub(super) fn distinct_key_count(&self) -> usize {
        self.0.distinct_key_count()
    }

    pub(super) fn probe_values_with_scratch<'s>(
        &'s self,
        values: &[&Value],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        scratch: &mut SemanticFiberProbeScratch,
    ) -> Result<Option<SemanticFiberProbe<'s>>, PhysicalExecutionError> {
        Ok(self
            .0
            .probe_fiber_iter(
                values.iter().copied(),
                context,
                registry,
                &mut scratch.observable_classes,
            )?
            .map(|rows| SemanticFiberProbe(rows.into_iter())))
    }

    pub(super) fn probe_row_columns_with_scratch<'s, I>(
        &'s self,
        row: &[Value],
        columns: I,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        scratch: &mut SemanticFiberProbeScratch,
    ) -> Result<Option<SemanticFiberProbe<'s>>, PhysicalExecutionError>
    where
        I: ExactSizeIterator<Item = usize>,
    {
        Ok(self
            .0
            .probe_row_columns_with_scratch(
                row,
                columns,
                context,
                registry,
                &mut scratch.observable_classes,
            )?
            .map(|rows| SemanticFiberProbe(rows.into_iter())))
    }

    fn single_key_for(&self, row_id: PhysicalRowId) -> Option<kernel_semantics::CanonicalEqKey> {
        self.0.single_key_for(row_id)
    }
}
