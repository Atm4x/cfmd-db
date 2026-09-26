/// Resource policy for rebuilding optional physical derivatives during reopen.
///
/// Relation layouts are reconstructed before this policy is applied because
/// they are the active physical carrier selected by the durable layout recipe.
/// Indexes, quotient factors and statistics remain optional reconstructible
/// state. Their rebuild can therefore be bounded without weakening logical
/// recovery from the authoritative Revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRecoveryPolicy {
    /// Maximum row/key-part evaluations attempted for advisor-owned artifacts.
    /// Manual durable pins are fixed intent and are not evicted by this policy.
    pub max_advisor_rebuild_key_evaluations: usize,
    /// Maximum deterministic structural semantic-work units attempted for
    /// advisor-owned rebuilds. One unit is one logical value node or one byte
    /// of stable text payload. This is a size-aware planning metric, not CPU
    /// time; manual durable pins remain fixed intent.
    pub max_advisor_rebuild_semantic_work_units: usize,
    /// Admission ceiling for advisor-owned rebuilds after relation layouts and
    /// manual durable pins have been reconstructed. Fixed state may itself
    /// exceed this ceiling; in that case no advisor-owned artifact is admitted.
    pub max_total_estimated_bytes: usize,
    /// Maximum estimated retained bytes contributed by advisor-owned artifacts.
    pub max_advisor_estimated_bytes: usize,
}

impl Default for PhysicalRecoveryPolicy {
    fn default() -> Self {
        Self {
            max_advisor_rebuild_key_evaluations: usize::MAX,
            max_advisor_rebuild_semantic_work_units: usize::MAX,
            max_total_estimated_bytes: usize::MAX,
            max_advisor_estimated_bytes: usize::MAX,
        }
    }
}

/// Evidence emitted by bounded physical reconstruction during reopen.
///
/// Dropping or budget-skipping advisor state never changes logical recovery.
/// Manual durable pins are rebuilt unconditionally; advisor-owned recipes may
/// be recreated later by lifecycle advice if useful again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhysicalRecoveryReport {
    pub rehydrated: Vec<DurablePhysicalArtifactSpec>,
    pub rebuilt: Vec<DurablePhysicalArtifactSpec>,
    pub skipped_key_evaluation_budget: Vec<DurablePhysicalArtifactSpec>,
    pub skipped_semantic_work_budget: Vec<DurablePhysicalArtifactSpec>,
    pub skipped_estimated_byte_budget: Vec<DurablePhysicalArtifactSpec>,
    pub dropped_incompatible: Vec<DurablePhysicalArtifactSpec>,
    pub attempted_rebuild_key_evaluations: usize,
    pub advisor_rebuild_key_evaluations: usize,
    pub attempted_rebuild_semantic_work_units: usize,
    pub advisor_rebuild_semantic_work_units: usize,
    pub advisor_estimated_bytes: usize,
    pub total_estimated_bytes_after: usize,
}

impl PhysicalRecoveryReport {
    /// Advisor-owned recipes that were compatible but not admitted by the
    /// selected recovery resource policy. These remain reconstructible hints,
    /// not logical authority, and may be retried after the runtime starts
    /// serving without reopening the durable generation.
    #[must_use]
    pub fn deferred_advisor_artifacts(&self) -> Vec<DurablePhysicalArtifactSpec> {
        let mut deferred = self
            .skipped_key_evaluation_budget
            .iter()
            .chain(self.skipped_semantic_work_budget.iter())
            .chain(self.skipped_estimated_byte_budget.iter())
            .filter(|spec| recovery::durable_physical_artifact_is_advisor_managed(spec))
            .cloned()
            .collect::<Vec<_>>();
        deferred.sort();
        deferred.dedup();
        deferred
    }
}
