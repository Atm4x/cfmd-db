#[derive(Debug, Clone, PartialEq, Eq)]
struct RevisionSemanticEncodedColumn {
    relation: SemanticId,
    layout: LayoutBinding,
    column: usize,
    equivalence: SemanticId,
    compiled: kernel_semantics::CompiledEquivalence,
    row_classes: PersistentOrdMap<PhysicalRowId, kernel_types::EqClassId>,
    postings: PersistentOrdMap<kernel_types::EqClassId, PersistentOrdSet<PhysicalRowId>>,
    canonicalization_count: usize,
}

fn semantic_class_catalog_error_to_physical(
    error: kernel_semantics::semantic_class_catalog::SemanticClassCatalogError,
) -> PhysicalExecutionError {
    use kernel_semantics::semantic_class_catalog::SemanticClassCatalogError;
    match error {
        SemanticClassCatalogError::Semantic(error) => error.into(),
        SemanticClassCatalogError::RevisionMismatch { .. } => {
            PhysicalExecutionError::SemanticContextTransitionRequiresRebuild
        }
        _ => PhysicalExecutionError::PhysicalTypeMismatch,
    }
}

impl RevisionSemanticEncodedColumn {
    #[allow(clippy::too_many_arguments)]
    fn build(
        relation_id: SemanticId,
        layout: LayoutBinding,
        column: usize,
        equivalence: SemanticId,
        relation: &InstalledRelation,
        catalog: &mut kernel_semantics::semantic_class_catalog::RevisionSemanticClassCatalog,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, PhysicalExecutionError> {
        let compiled = registry.compile_equivalence(context, equivalence)?;
        let mut row_classes = PersistentOrdMap::default();
        let mut postings = PersistentOrdMap::<
            kernel_types::EqClassId,
            PersistentOrdSet<PhysicalRowId>,
        >::default();
        let mut canonicalization_count = 0usize;
        for position in relation.scan_positions() {
            let row = relation.row_id_at(position)?;
            let value = native_value_at(&relation.data, column, position)?;
            let class = catalog
                .retain_value(registry, context, equivalence, &value)
                .map_err(semantic_class_catalog_error_to_physical)?;
            canonicalization_count = canonicalization_count
                .checked_add(1)
                .ok_or(PhysicalExecutionError::StatisticsCountOverflow)?;
            if row_classes.insert(row, class).is_some() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            if let Some(rows) = postings.get_mut(&class) {
                rows.insert(row);
            } else {
                let mut rows = PersistentOrdSet::default();
                rows.insert(row);
                postings.insert(class, rows);
            }
        }
        Ok(Self {
            relation: relation_id,
            layout,
            column,
            equivalence,
            compiled,
            row_classes,
            postings,
            canonicalization_count,
        })
    }

    fn remove_rows_deferred_release(
        &mut self,
        delta: &PhysicalRelationDelta,
        releases: &mut Vec<kernel_types::EqClassId>,
    ) -> Result<(), PhysicalExecutionError> {
        for (row, _) in &delta.removed {
            let class = self
                .row_classes
                .remove(row)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let remove_bucket = {
                let rows = self
                    .postings
                    .get_mut(&class)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                if !rows.remove(row) {
                    return Err(RelQueryError::InconsistentIncrementalDelta.into());
                }
                rows.is_empty()
            };
            if remove_bucket {
                self.postings.remove(&class);
            }
            releases.push(class);
        }
        Ok(())
    }

    fn insert_rows(
        &mut self,
        delta: &PhysicalRelationDelta,
        catalog: &mut kernel_semantics::semantic_class_catalog::RevisionSemanticClassCatalog,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        for (row, values) in &delta.inserted {
            let value = values
                .get(self.column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let class = catalog
                .retain_value(registry, context, self.equivalence, value)
                .map_err(semantic_class_catalog_error_to_physical)?;
            self.canonicalization_count = self
                .canonicalization_count
                .checked_add(1)
                .ok_or(PhysicalExecutionError::StatisticsCountOverflow)?;
            if self.row_classes.insert(*row, class).is_some() {
                return Err(RelQueryError::InconsistentIncrementalDelta.into());
            }
            if let Some(rows) = self.postings.get_mut(&class) {
                rows.insert(*row);
            } else {
                let mut rows = PersistentOrdSet::default();
                rows.insert(*row);
                self.postings.insert(class, rows);
            }
        }
        Ok(())
    }
}

impl PhysicalStore {
    fn semantic_catalog_snapshot(
        &self,
        context: &kernel_schema::SemanticContext,
    ) -> Result<kernel_semantics::semantic_class_catalog::RevisionSemanticClassCatalog, PhysicalExecutionError> {
        match &self.revision_semantic_catalog {
            Some(catalog) if catalog.revision() == context.revision() => Ok((**catalog).clone()),
            Some(_) => Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild),
            None => kernel_semantics::semantic_class_catalog::RevisionSemanticClassCatalog::new(context)
                .map_err(semantic_class_catalog_error_to_physical),
        }
    }

    fn ensure_revision_semantic_columns_for_binding(
        &mut self,
        binding: &SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, PhysicalExecutionError> {
        if binding.key_parts.is_empty() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        let installed = self.installed(binding.relation, binding.layout)?.clone();
        let mut missing = Vec::new();
        for part in &binding.key_parts {
            let key = (binding.relation, binding.layout.id, part.column, part.equivalence);
            if !self.revision_semantic_columns.contains_key(&key) {
                missing.push((key, part.column, part.equivalence));
            }
        }
        if missing.is_empty() {
            return Ok(false);
        }
        let mut catalog = self.semantic_catalog_snapshot(context)?;
        let mut columns = self.revision_semantic_columns.clone();
        for (key, column, equivalence) in missing {
            let state = RevisionSemanticEncodedColumn::build(
                binding.relation,
                binding.layout,
                column,
                equivalence,
                &installed,
                &mut catalog,
                context,
                registry,
            )?;
            columns.insert(key, Arc::new(state));
        }
        let catalog = Arc::new(catalog);
        self.revision_semantic_catalog = Some(Arc::clone(&catalog));
        self.revision_semantic_columns = columns;
        self.refresh_semantic_catalog_handles(&catalog);
        Ok(true)
    }

    fn semantic_columns_for_binding(
        &self,
        binding: &SemanticIndexBinding,
    ) -> Option<Vec<Arc<RevisionSemanticEncodedColumn>>> {
        binding
            .key_parts
            .iter()
            .map(|part| {
                self.revision_semantic_columns
                    .get(&(binding.relation, binding.layout.id, part.column, part.equivalence))
                    .cloned()
            })
            .collect()
    }

    pub(super) fn revision_semantic_column_distinct(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
        column: usize,
        equivalence: SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Option<usize> {
        if self.revision_semantic_catalog.as_ref()?.revision() != context.revision() {
            return None;
        }
        let state = self
            .revision_semantic_columns
            .get(&(relation, layout.id, column, equivalence))?;
        if registry.compile_equivalence(context, equivalence).ok()? != state.compiled {
            return None;
        }
        Some(state.postings.len())
    }

    pub(super) fn revision_semantic_columns_share_authority(
        &self,
        left: (SemanticId, LayoutBinding, usize),
        right: (SemanticId, LayoutBinding, usize),
        equivalence: SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> bool {
        if self
            .revision_semantic_catalog
            .as_ref()
            .is_none_or(|catalog| catalog.revision() != context.revision())
        {
            return false;
        }
        let Some(left) = self
            .revision_semantic_columns
            .get(&(left.0, left.1.id, left.2, equivalence))
        else { return false; };
        let Some(right) = self
            .revision_semantic_columns
            .get(&(right.0, right.1.id, right.2, equivalence))
        else { return false; };
        let Ok(compiled) = registry.compile_equivalence(context, equivalence) else {
            return false;
        };
        left.equivalence == equivalence
            && right.equivalence == equivalence
            && left.compiled == compiled
            && right.compiled == compiled
    }

    pub(super) fn revision_semantic_class_for_row(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
        column: usize,
        equivalence: SemanticId,
        row: PhysicalRowId,
    ) -> Option<kernel_types::EqClassId> {
        self.revision_semantic_columns
            .get(&(relation, layout.id, column, equivalence))?
            .row_classes
            .get(&row)
            .copied()
    }

    pub(super) fn revision_semantic_rows_for_class(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
        column: usize,
        equivalence: SemanticId,
        class: kernel_types::EqClassId,
    ) -> Option<&PersistentOrdSet<PhysicalRowId>> {
        self.revision_semantic_columns
            .get(&(relation, layout.id, column, equivalence))?
            .postings
            .get(&class)
    }

    pub(super) fn revision_semantic_probe_value(
        &self,
        coordinate: (SemanticId, LayoutBinding, usize),
        equivalence: SemanticId,
        value: &Value,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<PersistentOrdSet<PhysicalRowId>>, PhysicalExecutionError> {
        let (relation, layout, column) = coordinate;
        let Some(state) = self
            .revision_semantic_columns
            .get(&(relation, layout.id, column, equivalence))
        else { return Ok(None); };
        let catalog = self
            .revision_semantic_catalog
            .as_ref()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        if catalog.revision() != context.revision()
            || state.compiled != registry.compile_equivalence(context, equivalence)?
        {
            return Ok(None);
        }
        let class = catalog
            .lookup_value(registry, context, equivalence, value)
            .map_err(semantic_class_catalog_error_to_physical)?;
        Ok(Some(class.and_then(|class| state.postings.get(&class).cloned()).unwrap_or_default()))
    }

    fn apply_revision_semantic_columns_delta_deferred_release(
        &mut self,
        relation: SemanticId,
        layout: LayoutBinding,
        delta: &PhysicalRelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Vec<kernel_types::EqClassId>, PhysicalExecutionError> {
        let keys = self.revision_semantic_columns.keys()
            .filter(|(candidate_relation, candidate_layout, _, _)| {
                *candidate_relation == relation && *candidate_layout == layout.id
            })
            .copied().collect::<Vec<_>>();
        if keys.is_empty() { return Ok(Vec::new()); }
        let mut catalog = self.semantic_catalog_snapshot(context)?;
        let mut columns = self.revision_semantic_columns.clone();
        let mut releases = Vec::new();
        for key in keys {
            let state = columns.get_mut(&key).ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let state = Arc::make_mut(state);
            state.remove_rows_deferred_release(delta, &mut releases)?;
            state.insert_rows(delta, &mut catalog, context, registry)?;
        }
        self.revision_semantic_columns = columns;
        self.revision_semantic_catalog = Some(Arc::new(catalog));
        Ok(releases)
    }

    fn finalize_revision_semantic_releases(
        &mut self,
        releases: Vec<kernel_types::EqClassId>,
    ) -> Result<(), PhysicalExecutionError> {
        let Some(current) = self.revision_semantic_catalog.as_ref() else {
            return if releases.is_empty() {
                Ok(())
            } else {
                Err(RelQueryError::InconsistentIncrementalDelta.into())
            };
        };
        let mut catalog = current.as_ref().clone();
        for class in releases {
            catalog
                .release(class)
                .map_err(semantic_class_catalog_error_to_physical)?;
        }
        let catalog = Arc::new(catalog);
        self.revision_semantic_catalog = Some(Arc::clone(&catalog));
        self.refresh_semantic_catalog_handles(&catalog);
        Ok(())
    }
}

impl PhysicalStore {
    fn refresh_semantic_catalog_handles(
        &mut self,
        catalog: &Arc<kernel_semantics::semantic_class_catalog::RevisionSemanticClassCatalog>,
    ) {
        let observable_bindings = self.observable_atom_states.keys().cloned().collect::<Vec<_>>();
        for binding in observable_bindings {
            if let Some(state) = self.observable_atom_states.get_mut(&binding) {
                Arc::make_mut(state).refresh_semantic_catalog(Arc::clone(catalog));
            }
        }
        let occurrence_keys = self.row_occurrence_atoms.keys().copied().collect::<Vec<_>>();
        for key in occurrence_keys {
            if let Some(state) = self.row_occurrence_atoms.get_mut(&key) {
                Arc::make_mut(state).refresh_semantic_catalog(Arc::clone(catalog));
            }
        }
    }

    fn build_catalog_free_observable_atom_state(
        &mut self,
        binding: SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<MaterializedObservableAtomState, PhysicalExecutionError> {
        self.ensure_revision_semantic_columns_for_binding(&binding, context, registry)?;
        let catalog = self
            .revision_semantic_catalog
            .clone()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let columns = self
            .semantic_columns_for_binding(&binding)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let relation = self.installed(binding.relation, binding.layout)?;
        MaterializedObservableAtomState::build_from_semantic_columns(
            binding,
            relation,
            catalog,
            &columns,
            context,
            registry,
        )
    }

    fn build_catalog_free_observable_atom_state_from_durable(
        &mut self,
        binding: SemanticIndexBinding,
        encoded_keys_by_ordinal: &[Vec<u8>],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<MaterializedObservableAtomState, PhysicalExecutionError> {
        self.ensure_revision_semantic_columns_for_binding(&binding, context, registry)?;
        let catalog = self
            .revision_semantic_catalog
            .clone()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let columns = self
            .semantic_columns_for_binding(&binding)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let relation = self.installed(binding.relation, binding.layout)?;
        MaterializedObservableAtomState::build_from_durable_core(
            binding,
            relation,
            encoded_keys_by_ordinal,
            catalog,
            &columns,
            context,
            registry,
        )
    }

    fn remove_semantic_fabric_rows_for_relation(
        &mut self,
        relation: SemanticId,
        layout: LayoutBinding,
        delta: &PhysicalRelationDelta,
    ) -> Result<(), PhysicalExecutionError> {
        let observable_bindings = self
            .observable_atom_states
            .keys()
            .filter(|binding| binding.relation == relation && binding.layout.id == layout.id)
            .cloned()
            .collect::<Vec<_>>();
        for binding in observable_bindings {
            let state = self
                .observable_atom_states
                .get_mut(&binding)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            Arc::make_mut(state).remove_delta_rows(delta)?;
        }
        if let Some(state) = self.row_occurrence_atoms.get_mut(&(relation, layout.id)) {
            Arc::make_mut(state).remove_delta_rows(delta)?;
        }
        Ok(())
    }

    fn insert_semantic_fabric_rows_for_relation(
        &mut self,
        relation: SemanticId,
        layout: LayoutBinding,
        delta: &PhysicalRelationDelta,
    ) -> Result<(), PhysicalExecutionError> {
        let observable_bindings = self
            .observable_atom_states
            .keys()
            .filter(|binding| binding.relation == relation && binding.layout.id == layout.id)
            .cloned()
            .collect::<Vec<_>>();
        for binding in observable_bindings {
            let columns = self
                .semantic_columns_for_binding(&binding)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let state = self
                .observable_atom_states
                .get_mut(&binding)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            Arc::make_mut(state).insert_delta_rows_from_columns(delta, &columns)?;
        }
        if let Some(binding) = self
            .row_occurrence_atoms
            .get(&(relation, layout.id))
            .map(|state| state.binding.clone())
        {
            let columns = self
                .semantic_columns_for_binding(&binding)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let state = self
                .row_occurrence_atoms
                .get_mut(&(relation, layout.id))
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            Arc::make_mut(state).insert_delta_rows_from_columns(delta, &columns)?;
        }
        Ok(())
    }
}
