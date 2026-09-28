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
    relation_bases: PersistentOrdMap<SemanticId, RelationBaseWitness>,
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
    revision_publication: Arc<dyn RuntimeRevisionPublicationNotifier>,
}

/// Wake-only backend for reader-visible Revision publication.
///
/// This is deliberately not a state authority. Implementations may emit
/// duplicate or spurious wakes; subscribers recover the real transition from
/// durable causal history. A notifier therefore cannot publish database state
/// or certify concurrent-writer compatibility.
pub trait RuntimeRevisionPublicationNotifier: std::fmt::Debug + Send + Sync {
    #[must_use]
    fn generation(&self) -> u64;

    #[must_use]
    fn wait_after(&self, observed: u64) -> u64;

    /// Wakes publication waiters without asserting that database state changed.
    ///
    /// Providers may use the same primitive for real publication, cancellation,
    /// runtime shutdown, duplicate wakes and spurious wakes. Callers must always
    /// re-read authoritative Revision/history state after wake-up.
    fn notify_waiters(&self);

    fn notify_revision_published(&self) {
        self.notify_waiters();
    }
}

/// Per-subscription cancellation + wait capability for Revision publication.
///
/// This handle owns only the wake backend, never the runtime or database state,
/// so a blocking waiter cannot keep `DurableRuntime` alive. Cancellation is a
/// liveness event only and cannot fabricate a Revision transition.
#[derive(Debug, Clone)]
pub struct RuntimeRevisionPublicationWaitHandle {
    inner: Arc<RuntimeRevisionPublicationWaitState>,
}

#[derive(Debug)]
struct RuntimeRevisionPublicationWaitState {
    cancelled: std::sync::atomic::AtomicBool,
    notifier: Arc<dyn RuntimeRevisionPublicationNotifier>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeRevisionPublicationWaitOutcome {
    Woken(u64),
    Cancelled,
}

impl RuntimeRevisionPublicationWaitHandle {
    fn new(notifier: Arc<dyn RuntimeRevisionPublicationNotifier>) -> Self {
        Self {
            inner: Arc::new(RuntimeRevisionPublicationWaitState {
                cancelled: std::sync::atomic::AtomicBool::new(false),
                notifier,
            }),
        }
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.inner.notifier.generation()
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner
            .cancelled
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn cancel(&self) {
        if !self
            .inner
            .cancelled
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            self.inner.notifier.notify_waiters();
        }
    }

    #[must_use]
    pub fn wait_after(&self, observed: u64) -> RuntimeRevisionPublicationWaitOutcome {
        if self.is_cancelled() {
            return RuntimeRevisionPublicationWaitOutcome::Cancelled;
        }
        let generation = self.inner.notifier.wait_after(observed);
        if self.is_cancelled() {
            RuntimeRevisionPublicationWaitOutcome::Cancelled
        } else {
            RuntimeRevisionPublicationWaitOutcome::Woken(generation)
        }
    }
}

#[derive(Debug, Default)]
pub struct InProcessRevisionPublicationNotifier {
    generation: Mutex<u64>,
    changed: std::sync::Condvar,
}

impl RuntimeRevisionPublicationNotifier for InProcessRevisionPublicationNotifier {
    fn generation(&self) -> u64 {
        *self
            .generation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn wait_after(&self, observed: u64) -> u64 {
        let mut generation = self
            .generation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *generation <= observed {
            generation = self
                .changed
                .wait(generation)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *generation
    }

    fn notify_waiters(&self) {
        let mut generation = self
            .generation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *generation = generation.saturating_add(1);
        self.changed.notify_all();
    }
}

impl Drop for DurableRuntime {
    fn drop(&mut self) {
        let notifier = Arc::clone(&self.revision_publication);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            notifier.notify_waiters();
        }));
    }
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
