pub(super) struct PhysicalRecoveryInputs<'a> {
    pub(super) specs: &'a [DurablePhysicalArtifactSpec],
    pub(super) artifact_cores: &'a [DurableArtifactCore],
    pub(super) target_revision: RevisionId,
    pub(super) relation_layouts: &'a BTreeMap<SemanticId, LayoutBinding>,
    pub(super) policy: PhysicalRecoveryPolicy,
    pub(super) telemetry: Option<&'a UnifiedAdvisorTelemetry>,
    pub(super) context: &'a kernel_schema::SemanticContext,
    pub(super) registry: &'a kernel_semantics::SemanticRegistry,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PhysicalRebuildWorkEstimate {
    key_evaluations: usize,
    semantic_work_units: usize,
}
