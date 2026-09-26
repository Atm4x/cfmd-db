#[derive(Debug, Clone, PartialEq, Eq)]
// HOSTILE[P161][ACTIVE][PRIMARY]: SAMF/observable-atom semantic fiber authority.
pub struct MaterializedObservableAtomState {
    binding: SemanticIndexBinding,
    key_binding: kernel_semantic_index::SemanticIndexBinding,
    pub(super) catalog: kernel_semantics::observable::RevisionObservableCatalog,
    observables: Vec<kernel_types::RevisionObservableId>,
    product_observable: kernel_types::RevisionObservableId,
    projection: kernel_semantics::observable::CertifiedSemanticMorphism,
    fabric: kernel_semantics::support_atom::SupportAtomFabric<PhysicalRowId>,
}

impl MaterializedObservableAtomState {
    fn build(
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
        let mut catalog = kernel_semantics::observable::RevisionObservableCatalog::new(context)
            .map_err(observable_error_to_physical)?;
        let observables = binding
            .key_parts
            .iter()
            .map(|part| {
                catalog
                    .register_equivalence(registry, context, part.equivalence)
                    .map_err(observable_error_to_physical)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let product_observable = catalog
            .register_product(observables.clone())
            .map_err(observable_error_to_physical)?;
        let mut fabric = kernel_semantics::support_atom::SupportAtomFabric::new(
            &catalog,
            product_observable,
            observables.clone(),
        )
        .map_err(support_atom_error_to_physical)?;

        for row_index in relation.scan_positions() {
            let row = materialize_native_row(&relation.data, row_index)?;
            let classes = observe_atom_row_classes(
                &binding,
                &observables,
                &mut catalog,
                context,
                registry,
                &row,
            )?;
            let atom_class = catalog
                .intern_product_class(product_observable, classes.clone())
                .map_err(observable_error_to_physical)?;
            fabric
                .insert(
                    &catalog,
                    relation.row_id_at(row_index)?,
                    atom_class,
                    &classes,
                )
                .map_err(support_atom_error_to_physical)?;
        }
        let projection =
            kernel_semantics::observable::CertifiedSemanticMorphism::product_projection(
                &catalog,
                product_observable,
                (0..observables.len()).collect(),
            )
            .map_err(observable_error_to_physical)?;
        Ok(Self {
            binding,
            key_binding,
            catalog,
            observables,
            product_observable,
            projection,
            fabric,
        })
    }

    fn build_from_durable_core(
        binding: SemanticIndexBinding,
        relation: &InstalledRelation,
        encoded_keys_by_ordinal: &[Vec<u8>],
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
        let mut catalog = kernel_semantics::observable::RevisionObservableCatalog::new(context)
            .map_err(observable_error_to_physical)?;
        let observables = binding
            .key_parts
            .iter()
            .map(|part| {
                catalog
                    .register_equivalence(registry, context, part.equivalence)
                    .map_err(observable_error_to_physical)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let product_observable = catalog
            .register_product(observables.clone())
            .map_err(observable_error_to_physical)?;
        let mut fabric = kernel_semantics::support_atom::SupportAtomFabric::new(
            &catalog,
            product_observable,
            observables.clone(),
        )
        .map_err(support_atom_error_to_physical)?;
        let positions = relation.scan_positions().collect::<Vec<_>>();
        if positions.len() != encoded_keys_by_ordinal.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        for (row_index, encoded_tuple) in positions.into_iter().zip(encoded_keys_by_ordinal) {
            let keys = kernel_semantics::decode_canonical_eq_key_tuple(encoded_tuple)
                .map_err(|_| PhysicalExecutionError::PhysicalTypeMismatch)?;
            if keys.len() != observables.len() {
                return Err(PhysicalExecutionError::PhysicalTypeMismatch);
            }
            let classes = observables
                .iter()
                .copied()
                .zip(keys)
                .map(|(observable, key)| {
                    catalog
                        .intern_canonical_equivalence_key(observable, key)
                        .map_err(observable_error_to_physical)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let atom_class = catalog
                .intern_product_class(product_observable, classes.clone())
                .map_err(observable_error_to_physical)?;
            fabric
                .insert(
                    &catalog,
                    relation.row_id_at(row_index)?,
                    atom_class,
                    &classes,
                )
                .map_err(support_atom_error_to_physical)?;
        }
        let projection =
            kernel_semantics::observable::CertifiedSemanticMorphism::product_projection(
                &catalog,
                product_observable,
                (0..observables.len()).collect(),
            )
            .map_err(observable_error_to_physical)?;
        Ok(Self {
            binding,
            key_binding,
            catalog,
            observables,
            product_observable,
            projection,
            fabric,
        })
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
        if self.catalog.revision() != context.revision()
            || !self.key_binding.is_valid_for(context, &dependencies)
        {
            return Ok(false);
        }
        for part in &self.binding.key_parts {
            let _ = registry.equivalence_domain(context, part.equivalence)?;
        }
        Ok(true)
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
        let signature = self.fabric.row_signature(&row_id)?;
        signature
            .iter()
            .map(|class| {
                let record = self.catalog.class_record(*class).ok()?;
                match &record.signature {
                    kernel_semantics::observable::ObservableClassSignature::Canonical(key) => {
                        Some(key.clone())
                    }
                    kernel_semantics::observable::ObservableClassSignature::Product(_) => None,
                }
            })
            .collect()
    }

    fn single_key_for(&self, row_id: PhysicalRowId) -> Option<kernel_semantics::CanonicalEqKey> {
        if self.observables.len() != 1 {
            return None;
        }
        self.canonical_key_tuple_for_row(row_id)?.into_iter().next()
    }

    /// Exact Set-uniqueness violation measure induced by SAMF atom masses.
    ///
    /// Each product atom with row mass `m > 1` contributes violation mass
    /// `m - 1`. This is reconstructible validation state, not durable authority.
    pub fn uniqueness_violation_measure(
        &self,
    ) -> Result<kernel_violation::ViolationMeasure<kernel_types::EqClassId>, PhysicalExecutionError>
    {
        let mut measure = kernel_violation::ViolationMeasure::new();
        for (atom, mass) in self.fabric.atom_masses() {
            if mass > 1 {
                let violation_mass = u64::try_from(mass - 1)
                    .map_err(|_| PhysicalExecutionError::StatisticsCountOverflow)?;
                measure
                    .add(atom, violation_mass)
                    .map_err(|_| PhysicalExecutionError::StatisticsCountOverflow)?;
            }
        }
        Ok(measure)
    }

    #[must_use]
    pub const fn product_observable(&self) -> kernel_types::RevisionObservableId {
        self.product_observable
    }

    #[must_use]
    pub const fn product_projection(
        &self,
    ) -> &kernel_semantics::observable::CertifiedSemanticMorphism {
        &self.projection
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
        if values.len() != self.observables.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        classes.clear();
        classes.reserve(self.observables.len());
        for (observable, value) in self.observables.iter().copied().zip(values) {
            let Some(class) = self
                .catalog
                .lookup_value_class(registry, context, observable, value)
                .map_err(observable_error_to_physical)?
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

    // HOSTILE[P189][ACTIVE][CLEAN]: relation-delta full-row occurrence probes reuse the
    // caller's observable-class scratch and consume the row directly; no per-removal
    // Vec<&Value> or EqClassId buffer is allocated on the maintained-delta hot path.
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
        if columns.len() != self.observables.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        classes.clear();
        classes.reserve(self.observables.len());
        for (observable, column) in self.observables.iter().copied().zip(columns) {
            let value = row
                .get(column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let Some(class) = self
                .catalog
                .lookup_value_class(registry, context, observable, value)
                .map_err(observable_error_to_physical)?
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
        let observable = self
            .observables
            .get(slot)
            .copied()
            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
        let Some(class) = self
            .catalog
            .lookup_value_class(registry, context, observable, value)
            .map_err(observable_error_to_physical)?
        else {
            return Ok(Vec::new());
        };
        self.fabric
            .projected_fiber(slot, class)
            .map(|rows| rows.into_iter().collect())
            .map_err(support_atom_error_to_physical)
    }

    fn count_slot_value(
        &self,
        slot: usize,
        value: &Value,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<usize, PhysicalExecutionError> {
        let observable = self
            .observables
            .get(slot)
            .copied()
            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
        let Some(class) = self
            .catalog
            .lookup_value_class(registry, context, observable, value)
            .map_err(observable_error_to_physical)?
        else {
            return Ok(0);
        };
        self.fabric
            .projected_count(slot, class)
            .map_err(support_atom_error_to_physical)
    }

    fn validate_physical_delta(
        &self,
        delta: &PhysicalRelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        for (row_id, row) in &delta.removed {
            let classes = lookup_atom_row_classes(
                &self.binding,
                &self.observables,
                &self.catalog,
                context,
                registry,
                row,
            )?
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if self.fabric.row_signature(row_id) != Some(classes.as_slice()) {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
        }

        let mut catalog = self.catalog.clone();
        for (_, row) in &delta.inserted {
            let classes = observe_atom_row_classes(
                &self.binding,
                &self.observables,
                &mut catalog,
                context,
                registry,
                row,
            )?;
            let _ = catalog
                .intern_product_class(self.product_observable, classes)
                .map_err(observable_error_to_physical)?;
        }
        Ok(())
    }

    fn apply_physical_delta(
        &mut self,
        delta: &PhysicalRelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let mut candidate = self.clone();
        for (row_id, _) in &delta.removed {
            candidate
                .fabric
                .remove(row_id)
                .map_err(support_atom_error_to_physical)?;
        }
        for (row_id, row) in &delta.inserted {
            let classes = observe_atom_row_classes(
                &candidate.binding,
                &candidate.observables,
                &mut candidate.catalog,
                context,
                registry,
                row,
            )?;
            let atom_class = candidate
                .catalog
                .intern_product_class(candidate.product_observable, classes.clone())
                .map_err(observable_error_to_physical)?;
            candidate
                .fabric
                .insert(&candidate.catalog, *row_id, atom_class, &classes)
                .map_err(support_atom_error_to_physical)?;
        }
        candidate.projection =
            kernel_semantics::observable::CertifiedSemanticMorphism::product_projection(
                &candidate.catalog,
                candidate.product_observable,
                (0..candidate.observables.len()).collect(),
            )
            .map_err(observable_error_to_physical)?;
        *self = candidate;
        Ok(())
    }
}

fn observe_atom_row_classes(
    binding: &SemanticIndexBinding,
    observables: &[kernel_types::RevisionObservableId],
    catalog: &mut kernel_semantics::observable::RevisionObservableCatalog,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    row: &kernel_query::Row,
) -> Result<Vec<kernel_types::EqClassId>, PhysicalExecutionError> {
    binding
        .key_parts
        .iter()
        .zip(observables)
        .map(|(part, &observable)| {
            let value = row
                .get(part.column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            catalog
                .observe_value(registry, context, observable, value)
                .map_err(observable_error_to_physical)
        })
        .collect()
}

fn lookup_atom_row_classes(
    binding: &SemanticIndexBinding,
    observables: &[kernel_types::RevisionObservableId],
    catalog: &kernel_semantics::observable::RevisionObservableCatalog,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    row: &kernel_query::Row,
) -> Result<Option<Vec<kernel_types::EqClassId>>, PhysicalExecutionError> {
    let mut classes = Vec::with_capacity(observables.len());
    for (part, &observable) in binding.key_parts.iter().zip(observables) {
        let value = row
            .get(part.column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let Some(class) = catalog
            .lookup_value_class(registry, context, observable, value)
            .map_err(observable_error_to_physical)?
        else {
            return Ok(None);
        };
        classes.push(class);
    }
    Ok(Some(classes))
}

fn observable_error_to_physical(
    error: kernel_semantics::observable::ObservableError,
) -> PhysicalExecutionError {
    match error {
        kernel_semantics::observable::ObservableError::Semantic(error) => error.into(),
        kernel_semantics::observable::ObservableError::RevisionMismatch { .. }
        | kernel_semantics::observable::ObservableError::ObservableCatalogMismatch => {
            PhysicalExecutionError::SemanticContextTransitionRequiresRebuild
        }
        _ => PhysicalExecutionError::PhysicalTypeMismatch,
    }
}

fn support_atom_error_to_physical<RowId>(
    error: kernel_semantics::support_atom::SupportAtomError<RowId>,
) -> PhysicalExecutionError {
    match error {
        kernel_semantics::support_atom::SupportAtomError::Observable(error) => {
            observable_error_to_physical(error)
        }
        kernel_semantics::support_atom::SupportAtomError::CatalogMismatch => {
            PhysicalExecutionError::SemanticContextTransitionRequiresRebuild
        }
        _ => PhysicalExecutionError::PhysicalTypeMismatch,
    }
}

// HOSTILE[P161][ACTIVE][MIXED]: primary observable fibers first; LegacyIndex is compatibility.
pub(super) enum SemanticFiberCapability<'a> {
    ObservableAtom(&'a MaterializedObservableAtomState),
    LegacyIndex(&'a MaterializedSemanticIndexState),
}

// HOSTILE[P185][ACTIVE][CLEAN]: persisted semantic probes borrow maintained fibers directly;
// execution hot paths do not allocate/copy row-id vectors per probe.
pub(super) enum SemanticFiberProbe<'a> {
    Observable(
        <&'a kernel_persistent::PersistentOrdSet<PhysicalRowId> as IntoIterator>::IntoIter,
    ),
    Legacy(
        <&'a kernel_semantic_index::SemanticBucket<PhysicalRowId> as IntoIterator>::IntoIter,
    ),
}

#[derive(Default)]
pub(super) struct SemanticFiberProbeScratch {
    observable_classes: Vec<kernel_types::EqClassId>,
    canonical_keys: Vec<kernel_semantics::CanonicalEqKey>,
}

impl Iterator for SemanticFiberProbe<'_> {
    type Item = PhysicalRowId;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Observable(rows) => rows.next().copied(),
            Self::Legacy(rows) => rows.next().copied(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Observable(rows) => rows.size_hint(),
            Self::Legacy(rows) => rows.size_hint(),
        }
    }
}

impl DoubleEndedIterator for SemanticFiberProbe<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Observable(rows) => rows.next_back().copied(),
            Self::Legacy(rows) => rows.next_back().copied(),
        }
    }
}

impl ExactSizeIterator for SemanticFiberProbe<'_> {}

impl SemanticFiberCapability<'_> {
    fn row_count(&self) -> usize {
        match self {
            Self::ObservableAtom(state) => state.row_count(),
            Self::LegacyIndex(state) => state.row_count(),
        }
    }

    pub(super) fn distinct_key_count(&self) -> usize {
        match self {
            Self::ObservableAtom(state) => state.distinct_key_count(),
            Self::LegacyIndex(state) => state.distinct_key_count(),
        }
    }

    // HOSTILE[P189][ACTIVE][CLEAN]: repeated execution probes may reuse the observable
    // class/canonical-key buffers while preserving borrowed row-id fibers.
    pub(super) fn probe_values_with_scratch<'a>(
        &'a self,
        values: &[&Value],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        scratch: &mut SemanticFiberProbeScratch,
    ) -> Result<Option<SemanticFiberProbe<'a>>, PhysicalExecutionError> {
        match self {
            Self::ObservableAtom(state) => Ok(state
                .probe_fiber_iter(
                    values.iter().copied(),
                    context,
                    registry,
                    &mut scratch.observable_classes,
                )?
                .map(|rows| SemanticFiberProbe::Observable(rows.into_iter()))),
            Self::LegacyIndex(state) => Ok(state
                .probe_values_with_scratch(values, &mut scratch.canonical_keys)?
                .map(|bucket| SemanticFiberProbe::Legacy(bucket.into_iter()))),
        }
    }

    pub(super) fn probe_row_columns_with_scratch<'a, I>(
        &'a self,
        row: &[Value],
        columns: I,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        scratch: &mut SemanticFiberProbeScratch,
    ) -> Result<Option<SemanticFiberProbe<'a>>, PhysicalExecutionError>
    where
        I: ExactSizeIterator<Item = usize>,
    {
        match self {
            Self::ObservableAtom(state) => Ok(state
                .probe_row_columns_with_scratch(
                    row,
                    columns,
                    context,
                    registry,
                    &mut scratch.observable_classes,
                )?
                .map(|rows| SemanticFiberProbe::Observable(rows.into_iter()))),
            Self::LegacyIndex(state) => Ok(state
                .probe_row_columns_with_scratch(row, columns, &mut scratch.canonical_keys)?
                .map(|bucket| SemanticFiberProbe::Legacy(bucket.into_iter()))),
        }
    }

    fn single_key_for(&self, row_id: PhysicalRowId) -> Option<kernel_semantics::CanonicalEqKey> {
        match self {
            Self::ObservableAtom(state) => state.single_key_for(row_id),
            Self::LegacyIndex(state) => state.single_key_for(row_id),
        }
    }
}

