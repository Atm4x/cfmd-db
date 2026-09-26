impl PhysicalStore {
    /// Returns exact `(handle, row)` bindings in authoritative logical scan order.
    /// This is the bootstrap contract consumed by maintained `Scan` leaves.
    pub fn logical_rows_with_handles(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
    ) -> Result<Vec<(PhysicalRowId, kernel_query::Row)>, PhysicalExecutionError> {
        let installed = self.installed(relation, layout)?;
        installed
            .scan_positions()
            .map(|position| {
                Ok((
                    installed.row_id_at(position)?,
                    materialize_native_row(&installed.data, position)?,
                ))
            })
            .collect()
    }

    /// Returns live row handles in the relation's logical scan order.
    pub fn logical_row_handles(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
    ) -> Result<Vec<PhysicalRowId>, PhysicalExecutionError> {
        let installed = self.installed(relation, layout)?;
        installed
            .scan_positions()
            .map(|position| installed.row_id_at(position))
            .collect()
    }

    fn i64_index(&self, index: I64IndexBinding) -> Option<&MaterializedI64IndexState> {
        self.i64_indexes.get(&index).map(Arc::as_ref)
    }

    pub(super) fn i64_index_capability(
        &self,
        index: I64IndexBinding,
    ) -> Option<I64IndexCapability<'_>> {
        self.i64_index(index).map(I64IndexCapability::new)
    }


    pub(super) fn semantic_fiber_capability(
        &self,
        binding: &SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<SemanticFiberCapability<'_>>, PhysicalExecutionError> {
        let relation = self.installed(binding.relation, binding.layout)?;
        let expected_rows = native_row_count(&relation.data);
        if let Some(state) = self.observable_atom_states.get(binding)
            && state.compatible_with(context, registry)?
            && state.row_count() == expected_rows
        {
            return Ok(Some(SemanticFiberCapability::ObservableAtom(state)));
        }
        if let Some(state) = self.semantic_indexes.get(binding)
            && state.compatible_with(context, registry)?
            && state.row_count() == expected_rows
        {
            return Ok(Some(SemanticFiberCapability::LegacyIndex(state)));
        }
        Ok(None)
    }

    pub(super) fn semantic_quotient_single_key(
        &self,
        binding: &SemanticIndexBinding,
        row_id: PhysicalRowId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError> {
        if binding.key_parts.len() != 1 {
            return Ok(None);
        }
        if let Some(state) = self.semantic_fiber_capability(binding, context, registry)?
            && let Some(key) = state.single_key_for(row_id)
        {
            return Ok(Some(key));
        }
        let Some(state) = self.semantic_quotient_factors.get(binding) else {
            return Ok(None);
        };
        let expected_rows =
            native_row_count(&self.installed(binding.relation, binding.layout)?.data);
        if !state.compatible_with(context, registry)? || state.row_count() != expected_rows {
            return Ok(None);
        }
        Ok(state.single_key_for(row_id).cloned())
    }

    pub(super) fn has_semantic_quotient_capability(
        &self,
        binding: &SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, PhysicalExecutionError> {
        if self
            .semantic_fiber_capability(binding, context, registry)?
            .is_some()
        {
            return Ok(true);
        }
        let expected_rows =
            native_row_count(&self.installed(binding.relation, binding.layout)?.data);
        Ok(self
            .semantic_quotient_factors
            .get(binding)
            .is_some_and(|state| {
                state.row_count() == expected_rows
                    && matches!(state.compatible_with(context, registry), Ok(true))
            }))
    }


    pub(super) fn semantic_quotient_support(
        &self,
        binding: &SemanticQuotientSupportBinding,
    ) -> Option<&MaterializedSemanticQuotientSupportState> {
        self.semantic_quotient_supports
            .get(binding)
            .map(Arc::as_ref)
    }

    fn maintain_semantic_quotient_supports_for_relation(
        &mut self,
        relation: SemanticId,
        layout: LayoutBinding,
        delta: &PhysicalRelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        self.maintain_semantic_quotient_supports_for_changes(
            &[(relation, layout, delta)],
            context,
            registry,
        )
    }

    pub(super) fn maintain_semantic_quotient_supports_for_changes(
        &mut self,
        changes: &[(SemanticId, LayoutBinding, &PhysicalRelationDelta)],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let mut bindings = BTreeSet::new();
        let mut changes_by_relation_layout = BTreeMap::new();
        for (relation, layout, delta) in changes {
            changes_by_relation_layout
                .entry((*relation, layout.id))
                .or_insert(*delta);
            for target in self.derived_artifact_targets(*relation, *layout) {
                if let DerivedArtifactTarget::Artifact(
                    UnifiedArtifactId::SemanticQuotientSupport(binding),
                ) = target
                {
                    bindings.insert(binding.clone());
                }
            }
        }
        for binding in bindings {
            let leaf_updates = binding
                .leaves
                .iter()
                .enumerate()
                .filter_map(|(leaf, binding_leaf)| {
                    changes_by_relation_layout
                        .get(&(binding_leaf.relation, binding_leaf.layout.id))
                        .map(|delta| (leaf, *delta))
                })
                .collect::<Vec<_>>();
            let structural_refresh =
                if let Some(state) = self.semantic_quotient_supports.get(&binding) {
                    prepare_semantic_quotient_component_refresh(
                        state,
                        &leaf_updates,
                        self,
                        context,
                        registry,
                    )?
                } else {
                    None
                };
            let locally_maintained = structural_refresh.is_some_and(|refresh| {
                self.semantic_quotient_supports
                    .get_mut(&binding)
                    .is_some_and(|state| refresh(Arc::make_mut(state)))
            });
            if locally_maintained {
                self.semantic_quotient_support_local_delta_updates = self
                    .semantic_quotient_support_local_delta_updates
                    .saturating_add(1);
                continue;
            }
            let state =
                build_semantic_quotient_support_state(binding.clone(), self, context, registry)?
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            self.semantic_quotient_supports
                .insert(binding, Arc::new(state));
        }
        Ok(())
    }

    // HOSTILE[P186][ACTIVE][CLEAN]: preparation supplies only semantic quotient bindings.
    // Storage owns staging, representation compatibility, advisor-ownership release, and the
    // single reconstructible-artifact commit boundary.
    pub(super) fn materialize_semantic_quotient_factors(
        &mut self,
        bindings: &BTreeSet<SemanticIndexBinding>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<usize, PhysicalExecutionError> {
        let (created, _) = self.materialize_semantic_quotient_artifacts(
            bindings,
            None,
            context,
            registry,
        )?;
        Ok(created)
    }

    pub(super) fn materialize_semantic_quotient_support(
        &mut self,
        factor_bindings: &BTreeSet<SemanticIndexBinding>,
        support_binding: SemanticQuotientSupportBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, PhysicalExecutionError> {
        let (_, support_changed) = self.materialize_semantic_quotient_artifacts(
            factor_bindings,
            Some(support_binding),
            context,
            registry,
        )?;
        Ok(support_changed)
    }

    fn materialize_semantic_quotient_artifacts(
        &mut self,
        factor_bindings: &BTreeSet<SemanticIndexBinding>,
        support_binding: Option<SemanticQuotientSupportBinding>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(usize, bool), PhysicalExecutionError> {
        let mut prepared_factors = Vec::new();
        for binding in factor_bindings {
            let compatible = match self.semantic_quotient_factors.get(binding) {
                Some(state) => state.compatible_with(context, registry)?,
                None => false,
            };
            if compatible {
                continue;
            }
            let relation = self.installed(binding.relation, binding.layout)?;
            prepared_factors.push((
                binding.clone(),
                Arc::new(MaterializedSemanticQuotientFactorState::build(
                    binding.clone(),
                    relation,
                    context,
                    registry,
                )?),
            ));
        }

        let releases_advisor_ownership = factor_bindings.iter().any(|binding| {
            self.advisor_managed_artifacts
                .contains(&UnifiedArtifactId::SemanticQuotientFactor(binding.clone()))
        });

        let prepared_support = if let Some(binding) = support_binding {
            // Support construction may consume freshly built quotient factors. Stage those factors
            // on a persistent clone so a failure cannot partially mutate the authoritative store.
            let mut staged = self.clone();
            if !prepared_factors.is_empty() {
                staged.invalidate_derived_artifact_dependencies();
            }
            for (factor_binding, state) in &prepared_factors {
                staged
                    .semantic_quotient_factors
                    .insert(factor_binding.clone(), Arc::clone(state));
            }
            let state = build_semantic_quotient_support_state(
                binding.clone(),
                &staged,
                context,
                registry,
            )?
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let changed = self
                .semantic_quotient_supports
                .get(&binding)
                .is_none_or(|existing| existing.as_ref() != &state);
            Some((binding, Arc::new(state), changed))
        } else {
            None
        };

        let support_changed = prepared_support
            .as_ref()
            .is_some_and(|(_, _, changed)| *changed);
        if prepared_factors.is_empty() && !releases_advisor_ownership && !support_changed {
            return Ok((0, false));
        }
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?;

        let created = prepared_factors.len();
        if created != 0 || support_changed {
            self.invalidate_derived_artifact_dependencies();
        }
        for (binding, state) in prepared_factors {
            self.semantic_quotient_factors.insert(binding, state);
        }
        for binding in factor_bindings {
            self.advisor_managed_artifacts
                .remove(&UnifiedArtifactId::SemanticQuotientFactor(binding.clone()));
        }
        if let Some((binding, state, true)) = prepared_support {
            self.semantic_quotient_supports.insert(binding, state);
        }
        self.transition_epoch = next_epoch;
        self.state_identity = Arc::new(());
        Ok((created, support_changed))
    }

    pub(super) fn installed(
        &self,
        relation: SemanticId,
        binding: LayoutBinding,
    ) -> Result<&InstalledRelation, PhysicalExecutionError> {
        self.relations
            .get(&(relation, binding.id))
            .map(Arc::as_ref)
            .ok_or(PhysicalExecutionError::MissingPhysicalRelation {
                relation,
                layout: binding.id,
            })
    }
}

impl SemanticQuotientStoreView for PhysicalStore {
    fn semantic_quotient_logical_row_handles(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
    ) -> Result<Vec<PhysicalRowId>, PhysicalExecutionError> {
        self.logical_row_handles(relation, layout)
    }

    fn semantic_quotient_row(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
        row_id: PhysicalRowId,
    ) -> Result<kernel_query::Row, PhysicalExecutionError> {
        let installed = self.installed(relation, layout)?;
        let position = installed
            .position(row_id)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        materialize_native_row(&installed.data, position)
    }

    fn semantic_quotient_value(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
        row_id: PhysicalRowId,
        column: usize,
    ) -> Result<Value, PhysicalExecutionError> {
        let installed = self.installed(relation, layout)?;
        let position = installed
            .position(row_id)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        match &installed.data {
            NativeRelation::RowStore(rows) => rows
                .get(position)
                .and_then(|row| row.get(column))
                .cloned()
                .ok_or(RelQueryError::ColumnOutOfBounds.into()),
            NativeRelation::Columnar { columns, row_count } => {
                if position >= *row_count {
                    return Err(PhysicalExecutionError::ColumnShapeMismatch);
                }
                columns
                    .get(column)
                    .and_then(|values| values.get(position))
                    .cloned()
                    .ok_or(RelQueryError::ColumnOutOfBounds.into())
            }
            NativeRelation::I64Columnar { columns, row_count } => {
                if position >= *row_count {
                    return Err(PhysicalExecutionError::ColumnShapeMismatch);
                }
                columns
                    .get(column)
                    .and_then(|values| values.get(position))
                    .copied()
                    .map(Value::I64)
                    .ok_or(RelQueryError::ColumnOutOfBounds.into())
            }
            NativeRelation::TypedColumnar { columns, row_count } => {
                if position >= *row_count {
                    return Err(PhysicalExecutionError::ColumnShapeMismatch);
                }
                columns
                    .get(column)
                    .map(|column| column.value_at(position))
                    .ok_or(RelQueryError::ColumnOutOfBounds.into())
            }
        }
    }

    fn semantic_quotient_single_key(
        &self,
        binding: &SemanticIndexBinding,
        row_id: PhysicalRowId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError> {
        PhysicalStore::semantic_quotient_single_key(self, binding, row_id, context, registry)
    }

    fn has_semantic_quotient_capability(
        &self,
        binding: &SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<bool, PhysicalExecutionError> {
        PhysicalStore::has_semantic_quotient_capability(self, binding, context, registry)
    }
}
