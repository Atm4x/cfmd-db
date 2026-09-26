impl PhysicalStore {
    fn validate_relation_semantic_context(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        for target in self.derived_artifact_targets(relation, layout) {
            let compatible = match target {
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticIndex(binding)) => self
                    .semantic_indexes
                    .get(binding)
                    .expect("derived semantic index dependency points to missing artifact")
                    .compatible_with(context, registry)?,
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticQuotientFactor(
                    binding,
                )) => self
                    .semantic_quotient_factors
                    .get(binding)
                    .expect("derived quotient factor dependency points to missing artifact")
                    .compatible_with(context, registry)?,
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticStatistics(binding)) => {
                    self.semantic_statistics
                        .get(binding)
                        .expect("derived statistics dependency points to missing artifact")
                        .compatible_with(context, registry)?
                }
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::ObservableAtom(binding)) => self
                    .observable_atom_states
                    .get(binding)
                    .expect("derived observable dependency points to missing artifact")
                    .compatible_with(context, registry)?,
                DerivedArtifactTarget::RowOccurrenceAtom(target_relation, target_layout) => self
                    .row_occurrence_atoms
                    .get(&(*target_relation, *target_layout))
                    .expect("derived row-occurrence dependency points to missing artifact")
                    .compatible_with(context, registry)?,
                DerivedArtifactTarget::Artifact(
                    UnifiedArtifactId::I64Index(_) | UnifiedArtifactId::SemanticQuotientSupport(_),
                ) => true,
            };
            if !compatible {
                return Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild);
            }
        }
        Ok(())
    }

    fn validate_relation_derived_delta(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
        delta: &PhysicalRelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        for target in self.derived_artifact_targets(relation, layout) {
            match target {
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::I64Index(binding)) => self
                    .i64_indexes
                    .get(binding)
                    .expect("derived I64 index dependency points to missing artifact")
                    .validate_physical_delta(delta)?,
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticIndex(binding)) => self
                    .semantic_indexes
                    .get(binding)
                    .expect("derived semantic index dependency points to missing artifact")
                    .validate_physical_delta(delta, context, registry)?,
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticQuotientFactor(
                    binding,
                )) => self
                    .semantic_quotient_factors
                    .get(binding)
                    .expect("derived quotient factor dependency points to missing artifact")
                    .validate_physical_delta(delta, context, registry)?,
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticStatistics(binding)) => {
                    self.semantic_statistics
                        .get(binding)
                        .expect("derived statistics dependency points to missing artifact")
                        .validate_physical_delta(delta)?;
                }
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::ObservableAtom(binding)) => self
                    .observable_atom_states
                    .get(binding)
                    .expect("derived observable dependency points to missing artifact")
                    .validate_physical_delta(delta, context, registry)?,
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticQuotientSupport(_)) => {
                    // QCN support validates/prepares its structural patch after authoritative
                    // relation mutation, when stable physical row identities are available.
                }
                DerivedArtifactTarget::RowOccurrenceAtom(target_relation, target_layout) => self
                    .row_occurrence_atoms
                    .get(&(*target_relation, *target_layout))
                    .expect("derived row-occurrence dependency points to missing artifact")
                    .validate_physical_delta(delta, context, registry)?,
            }
        }
        Ok(())
    }

    fn apply_relation_derived_delta(
        &mut self,
        relation: SemanticId,
        layout: LayoutBinding,
        delta: &PhysicalRelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let targets = self.derived_artifact_targets(relation, layout).to_vec();
        for target in targets {
            match target {
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::I64Index(binding)) => {
                    let state = self
                        .i64_indexes
                        .get_mut(&binding)
                        .expect("derived I64 index dependency points to missing artifact");
                    Arc::make_mut(state).apply_physical_delta(delta)?;
                }
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticIndex(binding)) => {
                    let state = self
                        .semantic_indexes
                        .get_mut(&binding)
                        .expect("derived semantic index dependency points to missing artifact");
                    Arc::make_mut(state).apply_physical_delta(delta, context, registry)?;
                }
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticQuotientFactor(
                    binding,
                )) => {
                    let state = self
                        .semantic_quotient_factors
                        .get_mut(&binding)
                        .expect("derived quotient factor dependency points to missing artifact");
                    Arc::make_mut(state).apply_physical_delta(delta, context, registry)?;
                }
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticStatistics(binding)) => {
                    let state = self
                        .semantic_statistics
                        .get_mut(&binding)
                        .expect("derived statistics dependency points to missing artifact");
                    Arc::make_mut(state).apply_physical_delta(delta)?;
                }
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::ObservableAtom(binding)) => {
                    let state = self
                        .observable_atom_states
                        .get_mut(&binding)
                        .expect("derived observable dependency points to missing artifact");
                    Arc::make_mut(state).apply_physical_delta(delta, context, registry)?;
                }
                DerivedArtifactTarget::Artifact(UnifiedArtifactId::SemanticQuotientSupport(_)) => {
                    // QCN support consumes the resolved physical delta in the dedicated
                    // multi-leaf maintenance phase below.
                }
                DerivedArtifactTarget::RowOccurrenceAtom(target_relation, target_layout) => {
                    let state = self
                        .row_occurrence_atoms
                        .get_mut(&(target_relation, target_layout))
                        .expect("derived row-occurrence dependency points to missing artifact");
                    Arc::make_mut(state).apply_physical_delta(delta, context, registry)?;
                }
            }
        }
        Ok(())
    }

    /// Atomically applies a semantic relation delta to the installed physical relation and to
    /// every persisted I64 index bound to that exact relation/layout. The complete mutation is
    /// planned and validated before any state is changed, so user-visible validation failures do
    /// not require cloning the relation or its materialized indexes.
    pub fn apply_relation_delta(
        &mut self,
        relation: SemanticId,
        layout: LayoutBinding,
        delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        self.apply_relation_delta_resolved(relation, layout, delta, context, registry)?;
        Ok(())
    }

    /// Applies a semantic delta to authoritative storage and returns the exact
    /// stable row identities selected/allocated by that transition. Maintained
    /// query plans can consume this receipt without resolving the same semantic
    /// rows a second time.
    pub fn apply_relation_delta_resolved(
        &mut self,
        relation: SemanticId,
        layout: LayoutBinding,
        delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<StorageResolvedRelationDelta, PhysicalExecutionError> {
        if self.revision.is_some() {
            return Err(PhysicalExecutionError::RevisionBoundMutationRequiresPreparedTransition);
        }
        let prepared =
            self.prepare_relation_delta_resolved(relation, layout, delta, context, registry)?;
        let (candidate, resolved) = prepared.into_candidate_if_current(self)?;
        *self = candidate;
        Ok(resolved)
    }

    fn prepare_relation_delta_resolved(
        &self,
        relation: SemanticId,
        layout: LayoutBinding,
        delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedPhysicalStoreTransition, PhysicalExecutionError> {
        if self.revision.is_some() {
            return Err(PhysicalExecutionError::RevisionBoundMutationRequiresPreparedTransition);
        }
        let mut candidate = self.clone();
        let resolved_delta = candidate
            .apply_relation_delta_resolved_in_place(relation, layout, delta, context, registry)?;
        Ok(PreparedPhysicalStoreTransition {
            source_epoch: self.transition_epoch,
            source_revision: None,
            source_identity: Arc::clone(&self.state_identity),
            candidate: Box::new(candidate),
            resolved_delta,
        })
    }

    /// Binds an initialized physical snapshot to the logical revision it represents.
    /// Semantic mutation of a bound store is allowed only through the joint path.
    pub fn bind_revision(&mut self, revision: RevisionId) -> Result<(), PhysicalExecutionError> {
        match self.revision {
            Some(current) if current == revision => return Ok(()),
            Some(_) => return Err(PhysicalExecutionError::RevisionBindingMismatch),
            None => {}
        }
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?;
        self.revision = Some(revision);
        self.transition_epoch = next_epoch;
        self.state_identity = Arc::new(());
        Ok(())
    }

    #[must_use]
    pub const fn revision(&self) -> Option<RevisionId> {
        self.revision
    }

    pub(super) fn rebind_unpublished_candidate_revision(
        &mut self,
        source_revision: RevisionId,
        target_revision: RevisionId,
    ) -> Result<(), PhysicalExecutionError> {
        if self.revision != Some(source_revision) {
            return Err(PhysicalExecutionError::RevisionBindingMismatch);
        }
        self.revision = Some(target_revision);
        Ok(())
    }

    #[must_use]
    pub const fn transition_epoch(&self) -> u64 {
        self.transition_epoch
    }

    /// Number of Γ-QCN support transitions maintained through a local support
    /// derivative rather than the exact full-state rebuild fallback.
    #[must_use]
    pub const fn semantic_quotient_support_local_delta_updates(&self) -> u64 {
        self.semantic_quotient_support_local_delta_updates
    }

    #[must_use]
    fn can_commit_prepared(&self, prepared: &PreparedPhysicalStoreTransition) -> bool {
        Arc::ptr_eq(&self.state_identity, &prepared.source_identity)
            && self.transition_epoch == prepared.source_epoch
            && prepared
                .source_revision
                .is_none_or(|revision| self.revision == Some(revision))
    }

    fn apply_relation_delta_resolved_in_place(
        &mut self,
        relation: SemanticId,
        layout: LayoutBinding,
        delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<StorageResolvedRelationDelta, PhysicalExecutionError> {
        let (resolved, physical_delta) = self
            .apply_relation_delta_resolved_in_place_deferred_support(
                relation, layout, delta, context, registry,
            )?;
        self.maintain_semantic_quotient_supports_for_relation(
            relation,
            layout,
            &physical_delta,
            context,
            registry,
        )?;
        Ok(resolved)
    }

    pub(super) fn apply_relation_delta_resolved_in_place_deferred_support(
        &mut self,
        relation: SemanticId,
        layout: LayoutBinding,
        delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(StorageResolvedRelationDelta, PhysicalRelationDelta), PhysicalExecutionError> {
        let key = (relation, layout.id);
        self.validate_relation_semantic_context(relation, layout, context, registry)?;
        let occurrence_binding = (!delta.removed.is_empty())
            .then(|| full_row_occurrence_binding(relation, layout, context))
            .transpose()?;
        if let Some(binding) = occurrence_binding.as_ref() {
            self.ensure_full_row_occurrence_atom(binding, context, registry)?;
        }
        let installed =
            self.relations
                .get(&key)
                .ok_or(PhysicalExecutionError::MissingPhysicalRelation {
                    relation,
                    layout: layout.id,
                })?;
        let candidate_index = self
            .i64_indexes
            .iter()
            .find(|(binding, _)| binding.relation == relation && binding.layout.id == layout.id)
            .map(|(_, state)| state.as_ref());
        let occurrence_atom = occurrence_binding
            .as_ref()
            .and_then(|binding| {
                self.row_occurrence_atoms
                    .get(&(binding.relation, binding.layout.id))
            })
            .map(Arc::as_ref);
        let physical_delta = plan_installed_relation_delta(
            installed,
            candidate_index,
            occurrence_atom,
            relation,
            delta,
            context,
            registry,
        )?;
        self.validate_relation_derived_delta(relation, layout, &physical_delta, context, registry)?;
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?;

        let installed = self.relations_mut_internal().get_mut(&key).ok_or(
            PhysicalExecutionError::MissingPhysicalRelation {
                relation,
                layout: layout.id,
            },
        )?;
        apply_planned_relation_delta(Arc::make_mut(installed), &physical_delta)?;
        self.apply_relation_derived_delta(relation, layout, &physical_delta, context, registry)?;
        self.transition_epoch = next_epoch;
        self.state_identity = Arc::new(());
        let resolved_delta = RelationDelta {
            inserted: physical_delta
                .inserted
                .iter()
                .map(|(_, row)| row.clone())
                .collect(),
            removed: physical_delta
                .removed
                .iter()
                .map(|(_, row)| row.clone())
                .collect(),
            result_type: delta.result_type.clone(),
        };
        Ok((
            StorageResolvedRelationDelta::from_parts(
                relation,
                resolved_delta,
                physical_delta.removed.iter().map(|(id, _)| *id).collect(),
                physical_delta.inserted.iter().map(|(id, _)| *id).collect(),
            ),
            physical_delta,
        ))
    }

}
