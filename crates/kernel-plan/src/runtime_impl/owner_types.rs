#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeIndexedHistoryAction {
    effect_id: u128,
    action: RewriteActionLaw,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeIndexedCausalObservation {
    effect_id: u128,
    exact_value: Option<Value>,
    preservation_rule: Option<kernel_schema::SemanticRuleExpr>,
    joint_group_ids: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RuntimeRelationalObservationRef {
    effect_id: u128,
    observation_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeIndexedRelationalCausalObservation {
    capsule_ref: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RuntimeRetainedEpochIndex {
    lineage_floor: Option<RevisionId>,
    relation_supports:
        PersistentOrdMap<SemanticId, PersistentOrdMap<RevisionId, RelationSupportWitness>>,
    relation_deltas: PersistentOrdMap<
        SemanticId,
        PersistentOrdMap<RevisionId, RelationDelta>,
    >,
    writes: PersistentOrdMap<
        RuntimeHistoryCoordinate,
        PersistentOrdMap<RevisionId, RuntimeIndexedHistoryAction>,
    >,
    observations: PersistentOrdMap<
        RuntimeHistoryCoordinate,
        PersistentOrdMap<RevisionId, RuntimeIndexedCausalObservation>,
    >,
    joint_observation_groups: PersistentOrdMap<(u128, u32), RuntimeJointCausalObservationGroup>,
    relational_observations: PersistentOrdMap<
        RuntimeRelationalObservationRef,
        RuntimeIndexedRelationalCausalObservation,
    >,
    relational_capsule_keys: PersistentOrdMap<(RevisionId, Vec<u8>), u32>,
    relational_capsules: PersistentOrdMap<u32, RelCausalCapsule>,
    relational_routes: PersistentOrdMap<
        SemanticId,
        PersistentOrdMap<RevisionId, PersistentOrdSet<RuntimeRelationalObservationRef>>,
    >,
    exact_effects: PersistentOrdMap<RevisionId, u128>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeRetainedSchemaEpoch {
    effect_id: u128,
    source_revision: RevisionId,
    target_revision: RevisionId,
    source_context: kernel_schema::SemanticContext,
    source_fields: kernel_model::CowMap<(SemanticId, kernel_types::EntityId), Value>,
    program: kernel_transport::SchemaMigrationProgram,
    index: RuntimeRetainedEpochIndex,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RuntimeHistoricalDerivedIndex {
    lineage_floor: Option<RevisionId>,
    floor_opaque_effect: Option<u128>,
    relation_supports:
        PersistentOrdMap<SemanticId, PersistentOrdMap<RevisionId, RelationSupportWitness>>,
    relation_deltas: PersistentOrdMap<
        SemanticId,
        PersistentOrdMap<RevisionId, RelationDelta>,
    >,
    writes: PersistentOrdMap<
        RuntimeHistoryCoordinate,
        PersistentOrdMap<RevisionId, RuntimeIndexedHistoryAction>,
    >,
    observations: PersistentOrdMap<
        RuntimeHistoryCoordinate,
        PersistentOrdMap<RevisionId, RuntimeIndexedCausalObservation>,
    >,
    joint_observation_groups: PersistentOrdMap<(u128, u32), RuntimeJointCausalObservationGroup>,
    relational_observations: PersistentOrdMap<
        RuntimeRelationalObservationRef,
        RuntimeIndexedRelationalCausalObservation,
    >,
    relational_capsule_keys: PersistentOrdMap<(RevisionId, Vec<u8>), u32>,
    relational_capsules: PersistentOrdMap<u32, RelCausalCapsule>,
    relational_routes: PersistentOrdMap<
        SemanticId,
        PersistentOrdMap<RevisionId, PersistentOrdSet<RuntimeRelationalObservationRef>>,
    >,
    exact_effects: PersistentOrdMap<RevisionId, u128>,
    retained_schema_epochs: PersistentOrdMap<RevisionId, RuntimeRetainedSchemaEpoch>,
}

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
    historical: RuntimeHistoricalDerivedIndex,
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

/// One runtime lineage owns exactly one durable-store authority. Persistence
/// state lives in the store backend itself; no duplicate runtime state wrapper
/// is permitted here.
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

    /// Latest wake generation relevant to the supplied relation dependency set.
    ///
    /// An empty dependency set is a wildcard and observes every publication.
    /// Providers that do not implement dependency-aware wake filtering may
    /// conservatively return the global generation.
    #[must_use]
    fn generation_for(&self, _dependencies: &[SemanticId]) -> u64 {
        self.generation()
    }

    #[must_use]
    fn wait_after(&self, observed: u64) -> u64;

    /// Blocking wait for a publication relevant to `dependencies`.
    #[must_use]
    fn wait_after_relations(&self, observed: u64, _dependencies: &[SemanticId]) -> u64 {
        self.wait_after(observed)
    }

    /// Registers or refreshes one executor waker for a waiter generation.
    ///
    /// The returned generation is sampled after registration. If it is greater
    /// than `observed`, the caller must treat the source as already ready and
    /// must not rely on a future wake. Implementations must make registration
    /// race-free with `notify_waiters`.
    #[must_use]
    fn register_waker_after(&self, waiter_id: u64, observed: u64, waker: &std::task::Waker) -> u64;

    /// Dependency-aware counterpart of [`Self::register_waker_after`].
    ///
    /// The returned generation is the latest generation relevant to the
    /// dependency set, not necessarily the runtime-global generation.
    #[must_use]
    fn register_waker_after_relations(
        &self,
        waiter_id: u64,
        observed: u64,
        _dependencies: &[SemanticId],
        waker: &std::task::Waker,
    ) -> u64 {
        self.register_waker_after(waiter_id, observed, waker)
    }

    /// Removes a previously registered executor waker.
    fn unregister_waker(&self, waiter_id: u64);

    /// Wakes publication waiters without asserting that database state changed.
    ///
    /// Providers may use the same primitive for real publication, cancellation,
    /// runtime shutdown, duplicate wakes and spurious wakes. Callers must always
    /// re-read authoritative Revision/history state after wake-up.
    fn notify_waiters(&self);

    fn notify_revision_published(&self) {
        self.notify_waiters();
    }

    /// Signals one exact relation-data publication. Implementations may use
    /// the relation frontier to avoid waking subscriptions whose maintained
    /// program cannot observe the transition.
    fn notify_relations_published(&self, _relations: &[SemanticId]) {
        self.notify_revision_published();
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

static NEXT_RUNTIME_PUBLICATION_WAITER_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

#[derive(Debug)]
struct RuntimeRevisionPublicationWaitState {
    waiter_id: u64,
    cancelled: std::sync::atomic::AtomicBool,
    notifier: Arc<dyn RuntimeRevisionPublicationNotifier>,
    dependencies: Box<[SemanticId]>,
}

impl Drop for RuntimeRevisionPublicationWaitState {
    fn drop(&mut self) {
        self.notifier.unregister_waker(self.waiter_id);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeRevisionPublicationWaitOutcome {
    Woken(u64),
    Cancelled,
}

impl RuntimeRevisionPublicationWaitHandle {
    fn new(
        notifier: Arc<dyn RuntimeRevisionPublicationNotifier>,
        dependencies: Box<[SemanticId]>,
    ) -> Self {
        Self {
            inner: Arc::new(RuntimeRevisionPublicationWaitState {
                waiter_id: NEXT_RUNTIME_PUBLICATION_WAITER_ID
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                cancelled: std::sync::atomic::AtomicBool::new(false),
                notifier,
                dependencies,
            }),
        }
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.inner.notifier.generation_for(&self.inner.dependencies)
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
        let generation = self
            .inner
            .notifier
            .wait_after_relations(observed, &self.inner.dependencies);
        if self.is_cancelled() {
            RuntimeRevisionPublicationWaitOutcome::Cancelled
        } else {
            RuntimeRevisionPublicationWaitOutcome::Woken(generation)
        }
    }

    pub fn clear_waker(&self) {
        self.inner.notifier.unregister_waker(self.inner.waiter_id);
    }

    pub fn poll_after(
        &self,
        observed: u64,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<RuntimeRevisionPublicationWaitOutcome> {
        if self.is_cancelled() {
            return std::task::Poll::Ready(RuntimeRevisionPublicationWaitOutcome::Cancelled);
        }
        let generation = self.inner.notifier.register_waker_after_relations(
            self.inner.waiter_id,
            observed,
            &self.inner.dependencies,
            context.waker(),
        );
        if self.is_cancelled() {
            self.inner.notifier.unregister_waker(self.inner.waiter_id);
            std::task::Poll::Ready(RuntimeRevisionPublicationWaitOutcome::Cancelled)
        } else if generation > observed {
            self.inner.notifier.unregister_waker(self.inner.waiter_id);
            std::task::Poll::Ready(RuntimeRevisionPublicationWaitOutcome::Woken(generation))
        } else {
            std::task::Poll::Pending
        }
    }
}

#[derive(Debug)]
struct RegisteredPublicationWaker {
    waker: std::task::Waker,
    dependencies: Box<[SemanticId]>,
}

#[derive(Debug, Default)]
struct InProcessRevisionPublicationState {
    generation: u64,
    opaque_generation: u64,
    relation_generation: BTreeMap<SemanticId, u64>,
    wakers: BTreeMap<u64, RegisteredPublicationWaker>,
    waiters_by_relation: BTreeMap<SemanticId, BTreeSet<u64>>,
    wildcard_waiters: BTreeSet<u64>,
}

impl InProcessRevisionPublicationState {
    fn relevant_generation(&self, dependencies: &[SemanticId]) -> u64 {
        if dependencies.is_empty() {
            return self.generation;
        }
        dependencies
            .iter()
            .fold(self.opaque_generation, |generation, relation| {
                generation.max(self.relation_generation.get(relation).copied().unwrap_or(0))
            })
    }

    fn remove_waiter(&mut self, waiter_id: u64) -> Option<std::task::Waker> {
        let registered = self.wakers.remove(&waiter_id)?;
        if registered.dependencies.is_empty() {
            self.wildcard_waiters.remove(&waiter_id);
        } else {
            for relation in &registered.dependencies {
                let remove_relation =
                    self.waiters_by_relation
                        .get_mut(relation)
                        .is_some_and(|waiters| {
                            waiters.remove(&waiter_id);
                            waiters.is_empty()
                        });
                if remove_relation {
                    self.waiters_by_relation.remove(relation);
                }
            }
        }
        Some(registered.waker)
    }

    fn register_waiter(
        &mut self,
        waiter_id: u64,
        dependencies: &[SemanticId],
        waker: &std::task::Waker,
    ) {
        let unchanged = self.wakers.get(&waiter_id).is_some_and(|registered| {
            registered.dependencies.as_ref() == dependencies && registered.waker.will_wake(waker)
        });
        if unchanged {
            return;
        }
        self.remove_waiter(waiter_id);
        let dependencies: Box<[SemanticId]> = dependencies.into();
        if dependencies.is_empty() {
            self.wildcard_waiters.insert(waiter_id);
        } else {
            for relation in dependencies.iter().copied() {
                self.waiters_by_relation
                    .entry(relation)
                    .or_default()
                    .insert(waiter_id);
            }
        }
        self.wakers.insert(
            waiter_id,
            RegisteredPublicationWaker {
                waker: waker.clone(),
                dependencies,
            },
        );
    }
}

#[derive(Debug, Default)]
pub struct InProcessRevisionPublicationNotifier {
    state: Mutex<InProcessRevisionPublicationState>,
    changed: std::sync::Condvar,
}

impl RuntimeRevisionPublicationNotifier for InProcessRevisionPublicationNotifier {
    fn generation(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .generation
    }

    fn generation_for(&self, dependencies: &[SemanticId]) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .relevant_generation(dependencies)
    }

    fn wait_after(&self, observed: u64) -> u64 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.generation <= observed {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.generation
    }

    fn wait_after_relations(&self, observed: u64, dependencies: &[SemanticId]) -> u64 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.relevant_generation(dependencies) <= observed {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.relevant_generation(dependencies)
    }

    fn register_waker_after(&self, waiter_id: u64, observed: u64, waker: &std::task::Waker) -> u64 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.generation <= observed {
            state.register_waiter(waiter_id, &[], waker);
        }
        state.generation
    }

    fn register_waker_after_relations(
        &self,
        waiter_id: u64,
        observed: u64,
        dependencies: &[SemanticId],
        waker: &std::task::Waker,
    ) -> u64 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = state.relevant_generation(dependencies);
        if generation <= observed {
            state.register_waiter(waiter_id, dependencies, waker);
        }
        generation
    }

    fn unregister_waker(&self, waiter_id: u64) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove_waiter(waiter_id);
    }

    fn notify_waiters(&self) {
        let wakers = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.generation = state.generation.saturating_add(1);
            state.opaque_generation = state.generation;
            self.changed.notify_all();
            state.waiters_by_relation.clear();
            state.wildcard_waiters.clear();
            std::mem::take(&mut state.wakers)
        };
        for registered in wakers.into_values() {
            registered.waker.wake();
        }
    }

    fn notify_relations_published(&self, relations: &[SemanticId]) {
        let wakers = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.generation = state.generation.saturating_add(1);
            let generation = state.generation;
            for relation in relations.iter().copied() {
                state.relation_generation.insert(relation, generation);
            }
            self.changed.notify_all();

            let mut ready = state.wildcard_waiters.clone();
            for relation in relations {
                if let Some(waiters) = state.waiters_by_relation.get(relation) {
                    ready.extend(waiters.iter().copied());
                }
            }
            ready
                .into_iter()
                .filter_map(|waiter_id| state.remove_waiter(waiter_id))
                .collect::<Vec<_>>()
        };
        for waker in wakers {
            waker.wake();
        }
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
    source_context: kernel_schema::SemanticContext,
    source_fields: kernel_model::CowMap<(SemanticId, kernel_types::EntityId), Value>,
    source_historical: RuntimeHistoricalDerivedIndex,
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
