impl RuntimeRevisionCell {
    #[must_use]
    pub fn new(root: RuntimeRevisionBundle) -> Self {
        Self {
            root: RwLock::new(RuntimeRevisionCellState::Serving(Arc::new(root))),
        }
    }

    pub fn snapshot(&self) -> Result<RuntimeRevisionSnapshot, PhysicalExecutionError> {
        let state = self
            .root
            .read()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(root) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        Ok(RuntimeRevisionSnapshot { root: root.clone() })
    }

    fn reset_historical_derived_to_current(&self) -> Result<(), PhysicalExecutionError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(live) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired);
        };
        let next_version = live
            .root_identity
            .version
            .0
            .checked_add(1)
            .ok_or(PhysicalExecutionError::RuntimeRootVersionExhausted)?;
        let candidate = RuntimeRevisionBundle {
            root_identity: RuntimeRootIdentity {
                root_id: live.root_identity.root_id,
                version: RuntimeRootVersion(next_version),
            },
            revision: live.revision.clone(),
            violation_state: live.violation_state.clone(),
            physical: live.physical.clone(),
            relation_layouts: live.relation_layouts.clone(),
            relation_bases: live.relation_bases.clone(),
            historical: RuntimeHistoricalDerivedIndex::from_current(
                live.revision.id(),
                &live.relation_bases,
            ),
            materialization_specs: live.materialization_specs.clone(),
            materializations: live.materializations.clone(),
            materialization_dependencies: live.materialization_dependencies.clone(),
            materializations_by_relation: live.materializations_by_relation.clone(),
        };
        *state = RuntimeRevisionCellState::Serving(Arc::new(candidate));
        Ok(())
    }

    pub fn prepare_revision(
        &self,
        request: &RevisionTransitionRequest<'_>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        self.snapshot()?.prepare_revision(request)
    }

    pub fn prepare_rewrites<I>(
        &self,
        request: &RevisionRewriteTransitionRequest<'_, I>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        self.snapshot()?.prepare_rewrites(request)
    }

    fn prepare_rewrites_derived<I>(
        &self,
        endpoint: &DerivedRelationEndpoint,
        rewrites: &[RevisionRelationRewrite<'_, I>],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        self.snapshot()?
            .prepare_rewrites_derived(endpoint, rewrites, registry)
    }

    pub fn prepare_mixed_revision(
        &self,
        request: &MixedRevisionTransitionRequest<'_>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        self.snapshot()?.prepare_mixed_revision(request)
    }

    pub fn prepare_full_revision(
        &self,
        request: &FullRevisionTransitionRequest<'_>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        self.snapshot()?.prepare_full_revision(request)
    }

    pub fn prepare_revision_and_materializations(
        &self,
        request: &RevisionAndMaterializationsTransitionRequest<'_>,
    ) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
        self.snapshot()?
            .prepare_revision_and_materializations(request)
    }

    fn prepare_materialization_configuration(
        &self,
        specs: &[RuntimeMaterializationSpec],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedMaterializationConfiguration, PhysicalExecutionError> {
        self.snapshot()?
            .prepare_materialization_configuration(specs, registry)
    }
}
