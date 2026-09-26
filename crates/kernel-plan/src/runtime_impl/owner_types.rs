/// Reader-visible owner for one coherent authoritative revision.
///
/// The logical revision, authoritative physical relation layouts, and every
/// registered maintained materialization are published together. Bootstrap
/// constructs maintained states from the revision itself rather than trusting
/// arbitrary externally supplied snapshots.
#[derive(Debug, PartialEq, Eq)]
// HOSTILE[P161][ACTIVE][CLEAN]: authoritative in-memory revision + physical/materialized state.
// HOSTILE[P173][ACTIVE][CLEAN]: owner state is runtime_impl-private; root keeps re-exports only.
pub struct RuntimeRevisionBundle {
    root_identity: RuntimeRootIdentity,
    revision: kernel_revision::Revision,
    violation_state: RuntimeViolationState,
    physical: PhysicalStore,
    relation_layouts: PersistentOrdMap<SemanticId, LayoutBinding>,
    materialization_specs: PersistentOrdMap<kernel_types::MaterializationId, RelExpr>,
    materializations: PersistentOrdMap<kernel_types::MaterializationId, MaterializedRelPlanState>,
    materialization_dependencies:
        PersistentOrdMap<kernel_types::MaterializationId, PersistentOrdSet<SemanticId>>,
    materializations_by_relation:
        PersistentOrdMap<SemanticId, PersistentOrdSet<kernel_types::MaterializationId>>,
}

/// Immutable reader snapshot of one coherent runtime root.
///
/// Readers retain the old `Arc` across publication, so a writer can swap the
/// live root without exposing a mixed logical/physical/materialized revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRevisionSnapshot {
    root: Arc<RuntimeRevisionBundle>,
}

/// Synchronization/publication owner for one runtime root lineage.
///
/// Writers publish by replacing one `Arc<RuntimeRevisionBundle>` under the
/// write lock. Readers only clone the current `Arc` and never observe partial
/// field mutation.
#[derive(Debug)]
enum RuntimeRevisionCellState {
    Serving(Arc<RuntimeRevisionBundle>),
    RecoveryRequired,
}

#[derive(Debug)]
pub struct RuntimeRevisionCell {
    root: RwLock<RuntimeRevisionCellState>,
}

/// Unified production owner for one reader-visible runtime lineage and its
/// authoritative durable generation/WAL. Keeping the store paired with the
/// runtime prevents callers from accidentally committing one runtime through
/// an unrelated durable head.
#[derive(Debug)]
pub struct DurableRuntime {
    cell: RuntimeRevisionCell,
    durability: Mutex<DurableRevisionStore>,
    registry: kernel_semantics::SemanticRegistry,
}

/// Process-level owner that can discard a fail-stopped runtime, reopen the
/// authoritative durable generation, and resolve an uncertain client commit
/// before retrying it under the same transaction identity.
#[derive(Debug)]
pub struct DurableRuntimeSupervisor {
    directory: std::path::PathBuf,
    runtime: Mutex<Option<DurableRuntime>>,
    physical_recovery_policy: PhysicalRecoveryPolicy,
}

/// Fully prepared but not yet freshness-sealed revision transition.
#[derive(Debug, PartialEq, Eq)]
pub struct PreparedRuntimeRevisionTransition {
    descriptor: RevisionCommitDescriptor,
    source_identity: RuntimeRootIdentity,
    candidate: Box<RuntimeRevisionBundle>,
    output_deltas: BTreeMap<kernel_types::MaterializationId, RelationDelta>,
}

/// One prepared single-relation resolution candidate whose concrete endpoint
/// is bound to a complete residual cube and to exact candidate Γ-VMF closure.
///
/// This wrapper does not create a second state authority: the candidate still
/// lives exclusively inside `PreparedRuntimeRevisionTransition`. The cube and
/// VMF certificate only justify publishing that exact candidate.
#[derive(Debug, PartialEq, Eq)]
pub struct PreparedCoherentResolutionTransition<I = Value> {
    prepared: PreparedRuntimeRevisionTransition,
    relation: SemanticId,
    cube: RewriteResidualCubeCertificate<RelationValue, I>,
    invariant_closure: RuntimeInvariantClosureCertificate,
}

/// Exclusive pre-publication capability.
///
/// The write guard is held from successful freshness validation until
/// `publish` or guard drop. A later WAL layer can append/fsync its COMMIT marker
/// while this guard exists without allowing the live root to advance first.
pub struct SealedRuntimeRevisionTransition<'a> {
    live: RwLockWriteGuard<'a, RuntimeRevisionCellState>,
    candidate: RuntimeRevisionBundle,
    descriptor: RevisionCommitDescriptor,
    output_deltas: BTreeMap<kernel_types::MaterializationId, RelationDelta>,
}

#[derive(Debug, PartialEq, Eq)]
struct PreparedMaterializationConfiguration {
    source_identity: RuntimeRootIdentity,
    candidate: Box<RuntimeRevisionBundle>,
}

struct SealedMaterializationConfiguration<'a> {
    live: RwLockWriteGuard<'a, RuntimeRevisionCellState>,
    candidate: RuntimeRevisionBundle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableMaterializationConfigOutcome {
    Applied(DurableGenerationReceipt),
    AlreadyApplied,
}

