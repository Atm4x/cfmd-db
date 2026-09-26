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
