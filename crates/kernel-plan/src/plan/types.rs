use super::{
    AdvisorTelemetry, AggregateSpec, ArtifactTelemetry, BTreeMap, BTreeSet, DurableRuntime, ExecutionStats, I64IndexBinding, LayoutBinding, LayoutId, OrderDirection, PhysicalCapability, PhysicalPressurePolicy, PhysicalPressureSample, PhysicalRecoveryPolicy, PhysicalRecoveryReport, PhysicalStore, PreparedPlan, RelExpr, RelQueryError, RelType, RelationValue, SemanticId, SemanticIndexBinding, TelemetryDecayPolicy, UnifiedAdvisorPolicy, UnifiedArtifactId, Value,
};
use crate::native_relation::{native_column_count};
use crate::semantic_rows::relation_value_from_rows;
use crate::multiway::{
    PreparedAnchorPullbackProgram, PreparedSemanticQuotientProgram,
    prepared_quotient_acceleration_available, try_execute_anchor_pullback_join,
    try_execute_multiway_join, try_execute_nway_order_preserving_join,
};
use crate::execution::{
    execute_anti_join_plan, execute_difference_plan, execute_distinct_plan, execute_filter_columns,
    execute_filter_const, execute_group_plan, execute_join_plan, execute_project_plan,
    execute_top_k_plan, scan_rows, try_execute_native_fast_path,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhysicalExecutionError {
    Query(RelQueryError),
    MissingPhysicalRelation {
        relation: SemanticId,
        layout: LayoutId,
    },
    LayoutFamilyMismatch,
    ColumnShapeMismatch,
    PhysicalTypeMismatch,
    UnsupportedPhysicalPlan,
    MissingPhysicalIndex,
    HandleGenerationExhausted,
    StalePreparedTransition,
    RevisionBindingMismatch,
    InvalidRevisionTransition,
    DuplicateRelationMutation(SemanticId),
    RewriteEffectMismatch(SemanticId),
    DuplicateMaterialization(kernel_types::MaterializationId),
    MaterializationDependencyIndexMismatch,
    MissingRuntimeRelationBinding(SemanticId),
    PhysicalRelationRegistryMismatch,
    LogicalPhysicalStateMismatch(SemanticId),
    LogicalRevisionMutationMismatch,
    SemanticContextTransitionRequiresRebuild,
    RevisionBoundMutationRequiresPreparedTransition,
    TransitionEpochExhausted,
    StatisticsCountOverflow,
    RuntimeRootIdentityExhausted,
    RuntimeRootVersionExhausted,
    RuntimePublicationPoisoned,
    RuntimeRecoveryRequired,
    DurableHeadMismatch,
    AnchorPullbackInvariant,
    CandidateViolationStateNonZero,
    RepairObservationTransportMismatch,
    RepairTransport(kernel_transport::TransportError),
    ResolutionRequiresSingleRelationRewrite,
    ResolutionCoherenceEndpointMismatch,
    Validation(kernel_validation::ValidationError),
}

impl From<RelQueryError> for PhysicalExecutionError {
    fn from(value: RelQueryError) -> Self {
        Self::Query(value)
    }
}

impl From<kernel_semantics::SemanticError> for PhysicalExecutionError {
    fn from(value: kernel_semantics::SemanticError) -> Self {
        Self::Query(RelQueryError::Semantic(value))
    }
}

impl From<kernel_validation::ValidationError> for PhysicalExecutionError {
    fn from(value: kernel_validation::ValidationError) -> Self {
        Self::Validation(value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhysicalCatalog {
    relation_layouts: BTreeMap<SemanticId, LayoutBinding>,
}

impl PhysicalCatalog {
    pub fn bind_relation(&mut self, relation: SemanticId, layout: LayoutBinding) {
        self.relation_layouts.insert(relation, layout);
    }

    #[must_use]
    pub fn relation_layout(&self, relation: SemanticId) -> LayoutBinding {
        self.relation_layouts
            .get(&relation)
            .copied()
            .unwrap_or(LayoutBinding::LOGICAL_MODEL_ROWS)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticAccessPath {
    FullScan,
    PersistedIndex,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
// HOSTILE[P162][ACTIVE][CLEAN:P160.C]: JOIN access work models the actual canonical-bucket
// fallback separately from output cardinality. Access-family admission never charges output rows.
pub struct SemanticAccessCostModel;

impl SemanticAccessCostModel {
    #[must_use]
    pub const fn canonical_bucket_join_access_work(
        left_rows: usize,
        right_rows: usize,
        key_parts: usize,
    ) -> usize {
        let key_parts = if key_parts == 0 { 1 } else { key_parts };
        let key_work = left_rows
            .saturating_add(right_rows)
            .saturating_mul(key_parts);
        key_work.saturating_add(right_rows)
    }

    #[must_use]
    pub const fn persisted_join_access_work(left_rows: usize, key_parts: usize) -> usize {
        let key_parts = if key_parts == 0 { 1 } else { key_parts };
        left_rows.saturating_mul(key_parts)
    }

    #[must_use]
    pub const fn ephemeral_i64_join_access_work(left_rows: usize, right_rows: usize) -> usize {
        left_rows.saturating_add(right_rows)
    }

    #[must_use]
    pub const fn estimated_join_output_rows(
        left_rows: usize,
        right_rows: usize,
        right_distinct_keys: usize,
    ) -> usize {
        if left_rows == 0 || right_rows == 0 || right_distinct_keys == 0 {
            return 0;
        }
        left_rows.saturating_mul(right_rows.div_ceil(right_distinct_keys))
    }

    #[must_use]
    pub const fn choose_filter(row_count: usize, matching_rows: usize) -> SemanticAccessPath {
        let scan_work = row_count;
        let index_work = 1_usize.saturating_add(matching_rows);
        if index_work < scan_work {
            SemanticAccessPath::PersistedIndex
        } else {
            SemanticAccessPath::FullScan
        }
    }
}

/// Minimal physical-plan vocabulary for the correctness-first baseline.
///
/// This is deliberately closed and contains no host callbacks, opaque expression
/// nodes, storage engines, caches, or hidden optimizer state. Stable structural
/// choices live in the plan; runtime-dependent join access is selected later from
/// the installed layout, Γ capabilities, retained artifacts, and the cost model so
/// semantically transparent AST wrappers cannot freeze a stale access strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
// HOSTILE[P163][ACTIVE][CLEAN:P160.A,P160.J]: Plan carries semantics/structure only;
// single-variant algorithm tags are gone and runtime owns capability-dependent access paths.
pub enum Plan {
    Scan {
        relation: SemanticId,
        layout: LayoutBinding,
    },
    FilterEqConst {
        input: Box<Self>,
        column: usize,
        value: Value,
        equivalence: SemanticId,
    },
    FilterEqColumns {
        input: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: SemanticId,
    },
    Project {
        input: Box<Self>,
        columns: Vec<usize>,
    },
    JoinEq {
        left: Box<Self>,
        right: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: SemanticId,
    },
    Difference {
        left: Box<Self>,
        right: Box<Self>,
    },
    AntiJoin {
        left: Box<Self>,
        right: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: SemanticId,
    },
    Distinct {
        input: Box<Self>,
        column_equivalences: Vec<SemanticId>,
    },
    Group {
        input: Box<Self>,
        group_columns: Vec<usize>,
        group_equivalences: Vec<SemanticId>,
        aggregate: AggregateSpec,
    },
    TopKWithTies {
        input: Box<Self>,
        column: usize,
        ordering: SemanticId,
        direction: OrderDirection,
        k: usize,
    },
    PromoteToBag(Box<Self>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticIndexWorkloadSample {
    pub plan: Plan,
    pub expected_executions: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticIndexAdvisorPolicy {
    pub max_managed_key_cells: usize,
    /// Budget for advisor-owned semantic indexes only.
    pub max_managed_estimated_bytes: usize,
    /// Global estimated retained-byte ceiling including fixed relation layouts
    /// and physical families that this advisor is not allowed to evict.
    pub max_total_estimated_bytes: usize,
}

impl Default for SemanticIndexAdvisorPolicy {
    fn default() -> Self {
        Self {
            max_managed_key_cells: usize::MAX,
            max_managed_estimated_bytes: usize::MAX,
            max_total_estimated_bytes: usize::MAX,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticIndexAdvisorReport {
    pub created: Vec<SemanticIndexBinding>,
    pub rebuilt: Vec<SemanticIndexBinding>,
    pub retained: Vec<SemanticIndexBinding>,
    pub evicted: Vec<SemanticIndexBinding>,
    pub reused_existing: Vec<SemanticIndexBinding>,
    pub rejected_unprofitable: Vec<SemanticIndexBinding>,
    pub rejected_budget: Vec<SemanticIndexBinding>,
    pub managed_key_cells: usize,
    pub managed_estimated_bytes: usize,
    pub fixed_estimated_bytes: usize,
    pub total_estimated_bytes_after: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalArtifactAdvisorPolicy {
    pub max_managed_estimated_bytes: usize,
    pub max_total_estimated_bytes: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct I64IndexAdvisorReport {
    pub created: Vec<I64IndexBinding>,
    pub retained: Vec<I64IndexBinding>,
    pub evicted: Vec<I64IndexBinding>,
    pub reused_existing: Vec<I64IndexBinding>,
    pub rejected_unprofitable: Vec<I64IndexBinding>,
    pub rejected_budget: Vec<I64IndexBinding>,
    pub managed_estimated_bytes: usize,
    pub fixed_estimated_bytes: usize,
    pub total_estimated_bytes_after: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticStatisticsAdvisorReport {
    pub created: Vec<SemanticIndexBinding>,
    pub rebuilt: Vec<SemanticIndexBinding>,
    pub retained: Vec<SemanticIndexBinding>,
    pub evicted: Vec<SemanticIndexBinding>,
    pub reused_existing: Vec<SemanticIndexBinding>,
    pub rejected_unprofitable: Vec<SemanticIndexBinding>,
    pub rejected_budget: Vec<SemanticIndexBinding>,
    pub managed_estimated_bytes: usize,
    pub fixed_estimated_bytes: usize,
    pub total_estimated_bytes_after: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservableAtomConvergenceReport {
    pub created: bool,
    pub rebuilt: bool,
    pub retained_manual_observable: bool,
    pub capabilities: BTreeSet<PhysicalCapability>,
    pub retired_legacy_indexes: Vec<SemanticIndexBinding>,
    pub retired_legacy_statistics: Vec<SemanticIndexBinding>,
    pub retired_legacy_quotient_factors: Vec<SemanticIndexBinding>,
}

pub(super) type UnifiedAdvisorTelemetry = AdvisorTelemetry<UnifiedArtifactId>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PhysicalArtifactTelemetryTarget {
    I64Index(I64IndexBinding),
    SemanticIndex(SemanticIndexBinding),
    ObservableAtom(SemanticIndexBinding),
    SemanticQuotientFactor(SemanticIndexBinding),
    SemanticStatistics(SemanticIndexBinding),
}

impl PhysicalArtifactTelemetryTarget {
    fn into_unified(self) -> UnifiedArtifactId {
        match self {
            Self::I64Index(binding) => UnifiedArtifactId::I64Index(binding),
            Self::SemanticIndex(binding) => UnifiedArtifactId::SemanticIndex(binding),
            Self::ObservableAtom(binding) => UnifiedArtifactId::ObservableAtom(binding),
            Self::SemanticQuotientFactor(binding) => {
                UnifiedArtifactId::SemanticQuotientFactor(binding)
            }
            Self::SemanticStatistics(binding) => UnifiedArtifactId::SemanticStatistics(binding),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnifiedObservableAdvisorReport {
    pub created: Vec<SemanticIndexBinding>,
    pub rebuilt: Vec<SemanticIndexBinding>,
    pub retained: Vec<SemanticIndexBinding>,
    pub evicted: Vec<SemanticIndexBinding>,
    pub retired_legacy_indexes: Vec<SemanticIndexBinding>,
    pub retired_legacy_statistics: Vec<SemanticIndexBinding>,
    pub retired_legacy_quotient_factors: Vec<SemanticIndexBinding>,
    pub rejected_unprofitable: Vec<SemanticIndexBinding>,
    pub rejected_budget: Vec<SemanticIndexBinding>,
    pub rejected_pressure: Vec<SemanticIndexBinding>,
    pub rejected_resource_conflict: Vec<SemanticIndexBinding>,
    pub managed_estimated_bytes: usize,
    pub fixed_estimated_bytes: usize,
    pub total_estimated_bytes_after: usize,
}

/// Reconstructible maintenance controller. Its telemetry and scheduling state are
/// explicitly outside `Revision=(S, Γ, M)` and never become durable semantic authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedAdvisorController {
    telemetry: UnifiedAdvisorTelemetry,
    decay: TelemetryDecayPolicy,
    policy: UnifiedAdvisorPolicy,
    pressure_policy: PhysicalPressurePolicy,
}

impl UnifiedAdvisorController {
    #[must_use]
    pub fn new(
        decay: TelemetryDecayPolicy,
        policy: UnifiedAdvisorPolicy,
        pressure_policy: PhysicalPressurePolicy,
    ) -> Self {
        Self {
            telemetry: UnifiedAdvisorTelemetry::default(),
            decay,
            policy,
            pressure_policy,
        }
    }

    pub fn observe(
        &mut self,
        artifact: PhysicalArtifactTelemetryTarget,
        sample: ArtifactTelemetry,
    ) {
        self.telemetry.observe(artifact.into_unified(), sample);
    }

    #[must_use]
    pub fn telemetry_for(&self, artifact: PhysicalArtifactTelemetryTarget) -> ArtifactTelemetry {
        self.telemetry.get(&artifact.into_unified())
    }

    /// Runs one deterministic maintenance epoch. Telemetry decays only after a successful
    /// publication decision, so a failed tick is retryable without observation drift.
    pub fn maintenance_tick(
        &mut self,
        runtime: &DurableRuntime,
        workload: &[SemanticIndexWorkloadSample],
        pressure_sample: PhysicalPressureSample,
    ) -> Result<UnifiedObservableAdvisorReport, PhysicalExecutionError> {
        let report = runtime.advise_unified_observable_atoms(
            workload,
            &self.telemetry,
            self.policy,
            self.pressure_policy,
            pressure_sample,
        )?;
        self.telemetry.advance_epoch(self.decay);
        Ok(report)
    }

    /// Continues an advisor-bounded reopen after live workload observations exist.
    /// Startup itself remains independent of ephemeral telemetry; only this post-start
    /// continuation uses benefit density to choose which reconstructible artifact to rebuild.
    pub fn resume_deferred_recovery(
        &self,
        runtime: &DurableRuntime,
        prior_report: &PhysicalRecoveryReport,
        policy: PhysicalRecoveryPolicy,
    ) -> Result<PhysicalRecoveryReport, PhysicalExecutionError> {
        runtime.resume_deferred_physical_recovery_with_telemetry(
            prior_report,
            policy,
            &self.telemetry,
        )
    }
}

impl Default for PhysicalArtifactAdvisorPolicy {
    fn default() -> Self {
        Self {
            max_managed_estimated_bytes: usize::MAX,
            max_total_estimated_bytes: usize::MAX,
        }
    }
}

#[derive(Clone, Copy)]
pub struct SemanticQuotientFactorWorkloadSample<'a> {
    pub plan: &'a PreparedPlan,
    pub expected_executions: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticQuotientFactorAdvisorReport {
    pub created: Vec<SemanticIndexBinding>,
    pub rebuilt: Vec<SemanticIndexBinding>,
    pub retained: Vec<SemanticIndexBinding>,
    pub evicted: Vec<SemanticIndexBinding>,
    pub reused_existing: Vec<SemanticIndexBinding>,
    pub rejected_unprofitable: Vec<SemanticIndexBinding>,
    pub rejected_budget: Vec<SemanticIndexBinding>,
    pub managed_estimated_bytes: usize,
    pub fixed_estimated_bytes: usize,
    pub total_estimated_bytes_after: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PhysicalArtifactFamily {
    RelationLayout,
    SharedDenseIdentityMap,
    I64Index,
    SemanticIndex,
    ObservableAtom,
    SemanticQuotientFactor,
    SemanticQuotientSupport,
    SemanticStatistics,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhysicalArtifactFamilyMemory {
    pub artifacts: usize,
    pub advisor_managed_artifacts: usize,
    pub estimated_retained_bytes: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhysicalArtifactMemoryReport {
    pub families: BTreeMap<PhysicalArtifactFamily, PhysicalArtifactFamilyMemory>,
    /// Deterministic retained-size estimate. This is an allocator-independent
    /// planning quantity, not an RSS or exact heap-allocation measurement.
    pub total_estimated_retained_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
// HOSTILE[P165][ACTIVE][CLEAN]: structural plan metadata mirrors every stateful producer family;
// it is descriptive only and carries no execution-algorithm authority.
pub struct PlanShape {
    pub nodes: usize,
    pub scans: usize,
    pub filters: usize,
    pub projects: usize,
    pub joins: usize,
    pub differences: usize,
    pub anti_joins: usize,
    pub distincts: usize,
    pub groups: usize,
    pub top_k: usize,
    pub bag_promotions: usize,
}

impl PlanShape {
    #[must_use]
    pub const fn stateful_operator_upper_bound(self) -> usize {
        self.distincts + self.joins + self.differences + self.anti_joins + self.groups + self.top_k
    }
}

