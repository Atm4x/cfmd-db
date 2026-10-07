use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
};

use crate::{Error, ErrorKind, Query, RelationResult, Result, RevisionId, Row};

static NEXT_WATCH_SUBSCRIPTION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WatchSubscriptionId(u64);

impl WatchSubscriptionId {
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WatchReadinessSourceId(u64);

impl WatchReadinessSourceId {
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchWake {
    Woken { generation: u64 },
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct WatchReadiness {
    source_id: WatchReadinessSourceId,
    inner: kernel_plan::RuntimeRevisionPublicationWaitHandle,
}

impl WatchReadiness {
    #[must_use]
    pub const fn source_id(&self) -> WatchReadinessSourceId {
        self.source_id
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.inner.generation()
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }

    #[must_use]
    pub fn wait_after(&self, observed: u64) -> WatchWake {
        match self.inner.wait_after(observed) {
            kernel_plan::RuntimeRevisionPublicationWaitOutcome::Woken(generation) => {
                WatchWake::Woken { generation }
            }
            kernel_plan::RuntimeRevisionPublicationWaitOutcome::Cancelled => WatchWake::Cancelled,
        }
    }

    /// Removes any executor waker registered by [`Self::poll_after`].
    ///
    /// Async adapters should call this when a pending future is dropped so a
    /// quiet database cannot retain an abandoned executor task indefinitely.
    pub fn clear_waker(&self) {
        self.inner.clear_waker();
    }

    pub fn poll_after(
        &self,
        observed: u64,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<WatchWake> {
        match self.inner.poll_after(observed, context) {
            std::task::Poll::Ready(kernel_plan::RuntimeRevisionPublicationWaitOutcome::Woken(
                generation,
            )) => std::task::Poll::Ready(WatchWake::Woken { generation }),
            std::task::Poll::Ready(
                kernel_plan::RuntimeRevisionPublicationWaitOutcome::Cancelled,
            ) => std::task::Poll::Ready(WatchWake::Cancelled),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchDrain<T> {
    events: Vec<T>,
    status: WatchStatus,
}

impl<T> WatchDrain<T> {
    #[must_use]
    pub fn events(&self) -> &[T] {
        &self.events
    }

    #[must_use]
    pub const fn status(&self) -> WatchStatus {
        self.status
    }

    #[must_use]
    pub fn into_events(self) -> Vec<T> {
        self.events
    }

    #[must_use]
    pub fn has_more(&self) -> bool {
        matches!(
            self.status,
            WatchStatus::Lagging {
                pending_transitions: 1..,
                ..
            }
        )
    }

    fn try_map<U>(self, mut map: impl FnMut(T) -> Result<U>) -> Result<WatchDrain<U>> {
        Ok(WatchDrain {
            events: self
                .events
                .into_iter()
                .map(&mut map)
                .collect::<Result<Vec<_>>>()?,
            status: self.status,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEvent {
    source_revision: RevisionId,
    target_revision: RevisionId,
    inserted: Vec<Row>,
    removed: Vec<Row>,
}

impl WatchEvent {
    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.source_revision
    }

    #[must_use]
    pub const fn target_revision(&self) -> RevisionId {
        self.target_revision
    }

    #[must_use]
    pub fn inserted(&self) -> &[Row] {
        &self.inserted
    }

    #[must_use]
    pub fn removed(&self) -> &[Row] {
        &self.removed
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty() && self.removed.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct WatchCancellation {
    inner: kernel_plan::RuntimeRevisionPublicationWaitHandle,
}

impl WatchCancellation {
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStatus {
    Current {
        revision: RevisionId,
    },
    Lagging {
        anchor_revision: RevisionId,
        head_revision: RevisionId,
        pending_transitions: usize,
    },
    Cancelled {
        revision: RevisionId,
    },
    RuntimeClosed {
        revision: RevisionId,
    },
    Unavailable {
        anchor_revision: RevisionId,
        head_revision: RevisionId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchMigrationMode {
    SchemaBound,
    DefinitionallyEquivalent,
}

#[derive(Debug)]
pub struct QueryWatch {
    runtime: Weak<kernel_plan::DurableRuntime>,
    state: kernel_query::MaterializedRelPlanState,
    subscription_id: WatchSubscriptionId,
    cursor_revision: RevisionId,
    observable_revision: RevisionId,
    publication_generation: u64,
    cancellation: WatchCancellation,
    readiness: WatchReadiness,
    initial: RelationResult,
    authorization: WatchAuthorization,
    migration_mode: WatchMigrationMode,
    schema_revision: u64,
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct WatchAuthorization {
    authority: crate::security::RuntimeAuthority,
    footprint: kernel_query::RelReadFootprint,
}

impl WatchAuthorization {
    pub fn reauthorize(&self) -> Result<()> {
        self.authority.require(crate::Permission::Watch)?;
        self.authority.require_read_footprint(&self.footprint)
    }

    fn replace_footprint(&mut self, footprint: kernel_query::RelReadFootprint) -> Result<()> {
        self.authority.require(crate::Permission::Watch)?;
        self.authority.require_read_footprint(&footprint)?;
        self.footprint = footprint;
        Ok(())
    }
}

impl QueryWatch {
    pub(crate) fn new(context: &crate::ReadContext, query: &Query) -> Result<Self> {
        Self::new_with_mode(context, query, WatchMigrationMode::SchemaBound)
    }

    pub(crate) fn new_migratable(context: &crate::ReadContext, query: &Query) -> Result<Self> {
        Self::new_with_mode(context, query, WatchMigrationMode::DefinitionallyEquivalent)
    }

    fn new_with_mode(
        context: &crate::ReadContext,
        query: &Query,
        migration_mode: WatchMigrationMode,
    ) -> Result<Self> {
        context.authority.require(crate::Permission::Watch)?;
        if !context.is_live() {
            return Err(Error::new(
                ErrorKind::WatchUnavailable,
                "watch requires a live snapshot; historical db.at(...) views are immutable",
            ));
        }
        let prepared = context.prepare(query)?;
        let footprint = prepared.inner.read_footprint().map_err(|error| {
            Error::new(
                ErrorKind::Query,
                format!("watch authorization footprint failed: {error:?}"),
            )
        })?;
        context.authority.require_read_footprint(&footprint)?;
        let authorization = WatchAuthorization {
            authority: context.authority.clone(),
            footprint,
        };
        let runtime = context.runtime_arc();
        let revision = context.kernel_revision();
        let seeded = if let Some(snapshot) = context.live_snapshot_ref() {
            let mut seeds = std::collections::BTreeMap::new();
            for relation in prepared.inner.expression().scan_relations() {
                let seed = snapshot
                    .relation_scan_occurrence_seed(relation)
                    .map_err(|error| {
                        Error::new(
                            ErrorKind::Internal,
                            format!("watch scan-evidence derivation failed: {error:?}"),
                        )
                    })?
                    .ok_or_else(|| {
                        Error::new(
                            ErrorKind::Internal,
                            format!("runtime has no relation witness for {relation:?}"),
                        )
                    })?;
                seeds.insert(relation, seed);
            }
            Some(seeds)
        } else {
            None
        };
        let state = match seeded.as_ref() {
            Some(seeds) => kernel_query::MaterializedRelPlanState::build_with_scan_seeds(
                prepared.inner.expression(),
                &revision.state().model,
                revision.semantic_context(),
                runtime.semantic_registry(),
                seeds,
            ),
            None => kernel_query::MaterializedRelPlanState::build(
                prepared.inner.expression(),
                &revision.state().model,
                revision.semantic_context(),
                runtime.semantic_registry(),
            ),
        }
        .map_err(|error| {
            Error::new(
                ErrorKind::WatchUnavailable,
                format!("query has no exact maintained watch program: {error:?}"),
            )
        })?;
        let initial = prepared.execute(context)?;
        let dependencies = state.scan_relations();
        let wait_handle =
            runtime.revision_publication_wait_handle_for_relations(dependencies.iter().copied());
        let publication_generation = wait_handle.generation();
        let readiness = WatchReadiness {
            source_id: WatchReadinessSourceId(context.database_identity()),
            inner: runtime
                .revision_publication_wait_handle_for_relations(dependencies.iter().copied()),
        };
        let revision = context.revision();
        Ok(Self {
            runtime: Arc::downgrade(runtime),
            state,
            subscription_id: WatchSubscriptionId(
                NEXT_WATCH_SUBSCRIPTION_ID.fetch_add(1, Ordering::Relaxed),
            ),
            cursor_revision: revision,
            observable_revision: revision,
            publication_generation,
            cancellation: WatchCancellation { inner: wait_handle },
            readiness,
            initial,
            authorization,
            migration_mode,
            schema_revision: context.schema_revision(),
        })
    }

    #[doc(hidden)]
    #[must_use]
    pub fn authorization(&self) -> WatchAuthorization {
        self.authorization.clone()
    }

    #[must_use]
    pub const fn subscription_id(&self) -> WatchSubscriptionId {
        self.subscription_id
    }

    /// Returns an executor-neutral wake source shared by every watch created
    /// from the same live database runtime.
    ///
    /// The returned handle has independent cancellation state. Async adapters
    /// can therefore deduplicate watches by `source_id()` and keep one blocking
    /// waiter or OS registration per runtime instead of one waiter per watch.
    #[must_use]
    pub fn readiness(&self) -> WatchReadiness {
        self.readiness.clone()
    }

    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.cursor_revision
    }

    #[must_use]
    pub const fn schema_revision(&self) -> u64 {
        self.schema_revision
    }

    #[must_use]
    pub const fn initial(&self) -> &RelationResult {
        &self.initial
    }

    #[must_use]
    pub fn cancellation(&self) -> WatchCancellation {
        self.cancellation.clone()
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.cancellation.is_cancelled() || self.runtime.strong_count() == 0
    }

    pub fn close(&self) {
        self.cancellation.cancel();
    }

    pub fn status(&self) -> Result<WatchStatus> {
        self.authorization.reauthorize()?;
        if self.cancellation.is_cancelled() {
            return Ok(WatchStatus::Cancelled {
                revision: self.cursor_revision,
            });
        }
        let Some(runtime) = self.runtime.upgrade() else {
            return Ok(WatchStatus::RuntimeClosed {
                revision: self.cursor_revision,
            });
        };
        let head = runtime
            .snapshot()
            .map_err(|error| {
                Error::new(
                    ErrorKind::Recovery,
                    format!("watch snapshot failed: {error:?}"),
                )
            })?
            .revision()
            .id();
        let head_revision = RevisionId::from(head);
        if head_revision == self.cursor_revision {
            return Ok(WatchStatus::Current {
                revision: self.cursor_revision,
            });
        }
        let Some(effects) = runtime.revision_history(head).map_err(|error| {
            Error::new(
                ErrorKind::Recovery,
                format!("watch history lookup failed: {error:?}"),
            )
        })?
        else {
            return Ok(WatchStatus::Unavailable {
                anchor_revision: self.cursor_revision,
                head_revision,
            });
        };
        let source = kernel_types::RevisionId::new(self.cursor_revision.raw());
        let Some(path) = effect_path(source, head, &effects) else {
            return Ok(WatchStatus::Unavailable {
                anchor_revision: self.cursor_revision,
                head_revision,
            });
        };
        Ok(WatchStatus::Lagging {
            anchor_revision: self.cursor_revision,
            head_revision,
            pending_transitions: path.len(),
        })
    }

    pub fn try_recv(&mut self) -> Result<Option<WatchEvent>> {
        self.authorization.reauthorize()?;
        if self.cancellation.is_cancelled() {
            return Err(watch_closed("watch was cancelled"));
        }
        let runtime = self
            .runtime
            .upgrade()
            .ok_or_else(|| watch_closed("watch database runtime is no longer open"))?;
        let head = runtime
            .snapshot()
            .map_err(|error| {
                Error::new(
                    ErrorKind::Recovery,
                    format!("watch snapshot failed: {error:?}"),
                )
            })?
            .revision()
            .id();
        while head != kernel_types::RevisionId::new(self.cursor_revision.raw()) {
            if let Some(event) = self.advance_one(&runtime, head)? {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }

    /// Drains at most `max_events` already-certified causal transitions without
    /// blocking for future publication. Durable history remains the event
    /// authority; this does not introduce a second in-memory event queue.
    pub fn drain_ready(&mut self, max_events: usize) -> Result<WatchDrain<WatchEvent>> {
        let mut events = Vec::with_capacity(max_events.min(64));
        while events.len() < max_events {
            let Some(event) = self.try_recv()? else {
                break;
            };
            events.push(event);
        }
        Ok(WatchDrain {
            events,
            status: self.status()?,
        })
    }

    /// Waits asynchronously for the next observable exact delta.
    ///
    /// This is an ordinary standard-library [`Future`]; no executor-specific
    /// adapter or conversion step is required.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> WatchNext<'_, Self> {
        WatchNext::new(self)
    }

    pub fn recv(&mut self) -> Result<WatchEvent> {
        loop {
            if let Some(event) = self.try_recv()? {
                return Ok(event);
            }
            match self
                .cancellation
                .inner
                .wait_after(self.publication_generation)
            {
                kernel_plan::RuntimeRevisionPublicationWaitOutcome::Woken(generation) => {
                    self.publication_generation = generation;
                }
                kernel_plan::RuntimeRevisionPublicationWaitOutcome::Cancelled => {
                    self.authorization.reauthorize()?;
                    return Err(watch_closed("watch was cancelled"));
                }
            }
        }
    }

    fn advance_one(
        &mut self,
        runtime: &kernel_plan::DurableRuntime,
        head: kernel_types::RevisionId,
    ) -> Result<Option<WatchEvent>> {
        let effect = next_effect(runtime, self.cursor_revision, head)?;
        let source = historical_revision(runtime, effect.source_revision, "source")?;
        let target = historical_revision(runtime, effect.target_revision, "target")?;
        if source.semantic_context() != target.semantic_context() {
            self.advance_schema_migration(runtime, &effect, &source, &target)?;
            return Ok(None);
        }
        let deltas = watch_input_deltas(&self.state, runtime, &source, &effect)?;
        let output = if deltas.is_empty() {
            None
        } else {
            Some(
                self.state
                    .apply_relation_deltas(
                        &deltas,
                        source.semantic_context(),
                        runtime.semantic_registry(),
                    )
                    .map_err(|error| {
                        Error::new(
                            ErrorKind::WatchUnavailable,
                            format!("exact watch delta rejected by kernel: {error:?}"),
                        )
                    })?,
            )
        };
        self.cursor_revision = effect.target_revision.into();
        let Some(output) = output else {
            return Ok(None);
        };
        if output.inserted.is_empty() && output.removed.is_empty() {
            return Ok(None);
        }
        let target_revision = effect.target_revision.into();
        let event = WatchEvent {
            source_revision: self.observable_revision,
            target_revision,
            inserted: convert_rows(&output.inserted),
            removed: convert_rows(&output.removed),
        };
        self.observable_revision = target_revision;
        Ok(Some(event))
    }

    fn advance_schema_migration(
        &mut self,
        runtime: &kernel_plan::DurableRuntime,
        effect: &kernel_plan::RuntimeHistoryEffect,
        source: &kernel_revision::Revision,
        target: &kernel_revision::Revision,
    ) -> Result<()> {
        if effect.kind != kernel_plan::RuntimeHistoryEffectKind::SchemaMigration
            || self.migration_mode != WatchMigrationMode::DefinitionallyEquivalent
        {
            return Err(Error::new(
                ErrorKind::WatchUnavailable,
                "watch cannot cross a semantic schema revision",
            ));
        }
        let definitional = self.state.rebind_definitionally_equivalent_context(
            target.semantic_context(),
            runtime.semantic_registry(),
        );
        if let Err(definitional_error) = definitional {
            let program = effect.schema_migration_program.as_ref().ok_or_else(|| {
                Error::new(
                    ErrorKind::WatchUnavailable,
                    "watch contract is not migratable: migration program is unavailable",
                )
            })?;
            let transport = program
                .verify(source.semantic_context(), runtime.semantic_registry())
                .map_err(|error| {
                    Error::new(
                        ErrorKind::WatchUnavailable,
                        format!(
                            "watch contract is not migratable across this schema boundary: {error:?}"
                        ),
                    )
                })?;
            let mut relation_map = BTreeMap::new();
            let mut target_relations = BTreeSet::new();
            for relation in self.state.scan_relations() {
                let target_relation = match transport
                    .classify_observation_relation_transport_exact(relation)
                    .map_err(|error| {
                        Error::new(
                            ErrorKind::WatchUnavailable,
                            format!(
                                "watch contract is not migratable across this schema boundary: {error:?}"
                            ),
                        )
                    })? {
                    kernel_transport::ObservationRelationTransport::RowIdentity {
                        target_relation,
                    } => target_relation,
                    kernel_transport::ObservationRelationTransport::RowLocalStateTransform {
                        target_relation,
                    } => {
                        return Err(Error::new(
                            ErrorKind::WatchUnavailable,
                            format!(
                                "watch contract is not migratable across this schema boundary: row-local rewrite {relation:?} -> {target_relation:?} changes maintained row representation; exact future delta transport exists, but the current maintained state has no zero-row-touch value-domain transport theorem"
                            ),
                        ));
                    }
                };
                if !target_relations.insert(target_relation) {
                    return Err(Error::new(
                        ErrorKind::WatchUnavailable,
                        "watch contract structural transport aliases distinct source relations onto one target relation",
                    ));
                }
                relation_map.insert(relation, target_relation);
            }
            self.state
                .retarget_row_identity_migration_context(
                    &relation_map,
                    target.semantic_context(),
                    runtime.semantic_registry(),
                )
                .map_err(|error| {
                    Error::new(
                        ErrorKind::WatchUnavailable,
                        format!(
                            "watch contract is not migratable across this schema boundary: {definitional_error:?}; structural identity rebind failed: {error:?}"
                        ),
                    )
                })?;
        }
        let footprint = self
            .state
            .query()
            .prepare(target.semantic_context(), runtime.semantic_registry())
            .and_then(|prepared| prepared.read_footprint())
            .map_err(|error| {
                Error::new(
                    ErrorKind::WatchUnavailable,
                    format!("transported watch authorization footprint failed: {error:?}"),
                )
            })?;
        self.authorization.replace_footprint(footprint)?;
        self.cursor_revision = effect.target_revision.into();
        self.schema_revision = target.semantic_context().schema.revision.raw();
        Ok(())
    }
}

/// Schema-neutral exact watch whose maintained observation may cross only a
/// definitionally equivalent schema boundary. Typed/object materialization is
/// intentionally delayed until the caller inspects `schema_revision()`.
#[derive(Debug)]
pub struct MigratableQueryWatch {
    inner: QueryWatch,
    initial_schema_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigratableWatchEvent {
    source_revision: RevisionId,
    target_revision: RevisionId,
    schema_revision: u64,
    inserted: Vec<Row>,
    removed: Vec<Row>,
}

impl MigratableWatchEvent {
    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.source_revision
    }

    #[must_use]
    pub const fn target_revision(&self) -> RevisionId {
        self.target_revision
    }

    #[must_use]
    pub const fn schema_revision(&self) -> u64 {
        self.schema_revision
    }

    #[must_use]
    pub fn inserted(&self) -> &[Row] {
        &self.inserted
    }

    #[must_use]
    pub fn removed(&self) -> &[Row] {
        &self.removed
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty() && self.removed.is_empty()
    }

    /// Materializes this schema-neutral delta only after the caller has
    /// explicitly selected the event's schema branch.
    ///
    /// The watch never owns or mutates a "current type". Typical callers
    /// match `schema_revision()` first, select the matching typed Context, and
    /// pass that Context's decoder here. A branch mismatch fails closed before
    /// user materialization code runs.
    pub fn materialize_schema<T>(
        &self,
        expected_schema_revision: u64,
        materialize: impl FnOnce(&[Row], &[Row]) -> Result<T>,
    ) -> Result<T> {
        if self.schema_revision != expected_schema_revision {
            return Err(Error::new(
                ErrorKind::InvalidSchema,
                format!(
                    "migratable watch event belongs to schema {}, not requested schema {}",
                    self.schema_revision, expected_schema_revision
                ),
            ));
        }
        materialize(&self.inserted, &self.removed)
    }
}

impl MigratableQueryWatch {
    pub(crate) fn new(context: &crate::ReadContext, query: &Query) -> Result<Self> {
        let inner = QueryWatch::new_migratable(context, query)?;
        Ok(Self {
            initial_schema_revision: inner.schema_revision(),
            inner,
        })
    }

    #[must_use]
    pub const fn initial_schema_revision(&self) -> u64 {
        self.initial_schema_revision
    }

    #[must_use]
    pub const fn initial(&self) -> &RelationResult {
        self.inner.initial()
    }

    #[must_use]
    pub const fn schema_revision(&self) -> u64 {
        self.inner.schema_revision()
    }

    #[must_use]
    pub const fn subscription_id(&self) -> WatchSubscriptionId {
        self.inner.subscription_id()
    }

    #[must_use]
    pub fn readiness(&self) -> WatchReadiness {
        self.inner.readiness()
    }

    pub fn try_recv(&mut self) -> Result<Option<MigratableWatchEvent>> {
        let event = self.inner.try_recv()?;
        Ok(event.map(|event| MigratableWatchEvent {
            source_revision: event.source_revision,
            target_revision: event.target_revision,
            schema_revision: self.inner.schema_revision(),
            inserted: event.inserted,
            removed: event.removed,
        }))
    }

    pub fn recv(&mut self) -> Result<MigratableWatchEvent> {
        loop {
            if let Some(event) = self.try_recv()? {
                return Ok(event);
            }
            match self
                .inner
                .cancellation
                .inner
                .wait_after(self.inner.publication_generation)
            {
                kernel_plan::RuntimeRevisionPublicationWaitOutcome::Woken(generation) => {
                    self.inner.publication_generation = generation;
                }
                kernel_plan::RuntimeRevisionPublicationWaitOutcome::Cancelled => {
                    self.inner.authorization.reauthorize()?;
                    return Err(watch_closed("watch was cancelled"));
                }
            }
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> WatchNext<'_, Self> {
        WatchNext::new(self)
    }

    #[must_use]
    pub fn cancellation(&self) -> WatchCancellation {
        self.inner.cancellation()
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    pub fn close(&self) {
        self.inner.close();
    }

    pub fn status(&self) -> Result<WatchStatus> {
        self.inner.status()
    }
}

fn next_effect(
    runtime: &kernel_plan::DurableRuntime,
    anchor: RevisionId,
    head: kernel_types::RevisionId,
) -> Result<kernel_plan::RuntimeHistoryEffect> {
    let effects = runtime
        .revision_history(head)
        .map_err(|error| {
            Error::new(
                ErrorKind::Recovery,
                format!("watch history lookup failed: {error:?}"),
            )
        })?
        .ok_or_else(|| {
            Error::new(
                ErrorKind::WatchUnavailable,
                "watch anchor is outside retained exact causal history",
            )
        })?;
    let source = kernel_types::RevisionId::new(anchor.raw());
    let effect = first_effect_on_path(source, head, &effects).ok_or_else(|| {
        Error::new(
            ErrorKind::WatchUnavailable,
            "current head is not exactly reachable from the watch anchor",
        )
    })?;
    if effect.kind == kernel_plan::RuntimeHistoryEffectKind::FullRevision {
        return Err(Error::new(
            ErrorKind::WatchUnavailable,
            "watch cannot cross an opaque/full historical transition",
        ));
    }
    Ok(effect.clone())
}

fn historical_revision(
    runtime: &kernel_plan::DurableRuntime,
    revision: kernel_types::RevisionId,
    role: &str,
) -> Result<kernel_revision::Revision> {
    runtime.revision_at(revision).map_err(|error| {
        Error::new(
            ErrorKind::WatchUnavailable,
            format!("watch {role} revision is unavailable: {error:?}"),
        )
    })
}

fn watch_input_deltas(
    state: &kernel_query::MaterializedRelPlanState,
    runtime: &kernel_plan::DurableRuntime,
    source: &kernel_revision::Revision,
    effect: &kernel_plan::RuntimeHistoryEffect,
) -> Result<BTreeMap<kernel_types::SemanticId, kernel_query::RelationDelta>> {
    let dependencies = state.scan_relations();
    effect
        .relation_mutations
        .iter()
        .filter(|mutation| dependencies.contains(&mutation.relation))
        .map(|mutation| {
            let result_type = kernel_query::RelExpr::Scan(mutation.relation)
                .typecheck(source.semantic_context(), runtime.semantic_registry())
                .map_err(|error| {
                    Error::new(
                        ErrorKind::WatchUnavailable,
                        format!("watch relation semantics unavailable: {error:?}"),
                    )
                })?;
            Ok((
                mutation.relation,
                kernel_query::RelationDelta {
                    inserted: mutation.inserted.clone(),
                    removed: mutation.removed.clone(),
                    result_type,
                },
            ))
        })
        .collect()
}

fn effect_path(
    source: kernel_types::RevisionId,
    target: kernel_types::RevisionId,
    effects: &[kernel_plan::RuntimeHistoryEffect],
) -> Option<Vec<&kernel_plan::RuntimeHistoryEffect>> {
    if source == target {
        return Some(Vec::new());
    }
    let mut outgoing = BTreeMap::<kernel_types::RevisionId, Vec<usize>>::new();
    for (index, effect) in effects.iter().enumerate() {
        outgoing
            .entry(effect.source_revision)
            .or_default()
            .push(index);
    }
    let mut queue = VecDeque::from([source]);
    let mut seen = BTreeSet::from([source]);
    let mut predecessor =
        BTreeMap::<kernel_types::RevisionId, (kernel_types::RevisionId, usize)>::new();
    while let Some(current) = queue.pop_front() {
        for &index in outgoing.get(&current).map_or(&[][..], Vec::as_slice) {
            let next = effects[index].target_revision;
            if seen.insert(next) {
                predecessor.insert(next, (current, index));
                if next == target {
                    let mut cursor = target;
                    let mut path = Vec::new();
                    while cursor != source {
                        let &(previous, effect_index) = predecessor.get(&cursor)?;
                        path.push(effect_index);
                        cursor = previous;
                    }
                    path.reverse();
                    return Some(path.into_iter().map(|index| &effects[index]).collect());
                }
                queue.push_back(next);
            }
        }
    }
    None
}

fn first_effect_on_path(
    source: kernel_types::RevisionId,
    target: kernel_types::RevisionId,
    effects: &[kernel_plan::RuntimeHistoryEffect],
) -> Option<&kernel_plan::RuntimeHistoryEffect> {
    effect_path(source, target, effects)?.into_iter().next()
}

fn watch_closed(message: &str) -> Error {
    Error::new(ErrorKind::WatchClosed, message)
}

fn convert_rows(rows: &[kernel_query::Row]) -> Vec<Row> {
    rows.iter()
        .map(|row| row.iter().cloned().map(Into::into).collect())
        .collect()
}

#[derive(Debug)]
pub struct ObjectWatch<E: crate::Object> {
    inner: QueryWatch,
    initial: Vec<E>,
    marker: PhantomData<fn() -> E>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectWatchEvent<E> {
    source_revision: RevisionId,
    target_revision: RevisionId,
    inserted: Vec<E>,
    removed: Vec<E>,
}

impl<E> ObjectWatchEvent<E> {
    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.source_revision
    }
    #[must_use]
    pub const fn target_revision(&self) -> RevisionId {
        self.target_revision
    }
    #[must_use]
    pub fn inserted(&self) -> &[E] {
        &self.inserted
    }
    #[must_use]
    pub fn removed(&self) -> &[E] {
        &self.removed
    }
}

impl<E: crate::Object> ObjectWatch<E> {
    pub(crate) fn new(query: &crate::ObjectQuery<E>) -> Result<Self> {
        let inner = QueryWatch::new(&query.context, &query.inner.clone().raw())?;
        let initial = inner
            .initial()
            .rows()
            .iter()
            .map(E::from_row)
            .collect::<Result<_>>()?;
        Ok(Self {
            inner,
            initial,
            marker: PhantomData,
        })
    }

    #[must_use]
    pub fn initial(&self) -> &[E] {
        &self.initial
    }

    pub fn try_recv(&mut self) -> Result<Option<ObjectWatchEvent<E>>> {
        self.inner
            .try_recv()?
            .map(|event| decode_object_event::<E>(&event))
            .transpose()
    }

    #[must_use]
    pub const fn subscription_id(&self) -> WatchSubscriptionId {
        self.inner.subscription_id()
    }

    #[must_use]
    pub fn readiness(&self) -> WatchReadiness {
        self.inner.readiness()
    }

    pub fn drain_ready(&mut self, max_events: usize) -> Result<WatchDrain<ObjectWatchEvent<E>>> {
        self.inner
            .drain_ready(max_events)?
            .try_map(|event| decode_object_event::<E>(&event))
    }

    /// Waits asynchronously for the next observable exact object delta.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> WatchNext<'_, Self> {
        WatchNext::new(self)
    }

    pub fn recv(&mut self) -> Result<ObjectWatchEvent<E>> {
        decode_object_event(&self.inner.recv()?)
    }

    #[must_use]
    pub fn cancellation(&self) -> WatchCancellation {
        self.inner.cancellation()
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    pub fn close(&self) {
        self.inner.close();
    }

    pub fn status(&self) -> Result<WatchStatus> {
        self.inner.status()
    }
}

pub struct ProjectionWatch<R, P: crate::Projection<R>> {
    inner: QueryWatch,
    projection: P,
    initial: Vec<P::Output>,
    marker: PhantomData<fn() -> R>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionWatchEvent<T> {
    source_revision: RevisionId,
    target_revision: RevisionId,
    inserted: Vec<T>,
    removed: Vec<T>,
}

impl<T> ProjectionWatchEvent<T> {
    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.source_revision
    }
    #[must_use]
    pub const fn target_revision(&self) -> RevisionId {
        self.target_revision
    }
    #[must_use]
    pub fn inserted(&self) -> &[T] {
        &self.inserted
    }
    #[must_use]
    pub fn removed(&self) -> &[T] {
        &self.removed
    }
}

impl<R, P: crate::Projection<R>> ProjectionWatch<R, P> {
    pub(crate) fn new(
        context: &crate::ReadContext,
        query: &crate::TypedQuery<R, P>,
    ) -> Result<Self> {
        let inner = QueryWatch::new(context, query.raw())?;
        let initial = query.decode_result(inner.initial())?;
        Ok(Self {
            inner,
            projection: query.projection().clone(),
            initial,
            marker: PhantomData,
        })
    }

    #[must_use]
    pub fn initial(&self) -> &[P::Output] {
        &self.initial
    }

    pub fn try_recv(&mut self) -> Result<Option<ProjectionWatchEvent<P::Output>>> {
        self.inner
            .try_recv()?
            .map(|event| decode_projection_event::<R, P>(&self.projection, &event))
            .transpose()
    }

    #[must_use]
    pub const fn subscription_id(&self) -> WatchSubscriptionId {
        self.inner.subscription_id()
    }

    #[must_use]
    pub fn readiness(&self) -> WatchReadiness {
        self.inner.readiness()
    }

    pub fn drain_ready(
        &mut self,
        max_events: usize,
    ) -> Result<WatchDrain<ProjectionWatchEvent<P::Output>>> {
        self.inner
            .drain_ready(max_events)?
            .try_map(|event| decode_projection_event::<R, P>(&self.projection, &event))
    }

    /// Waits asynchronously for the next observable exact projection delta.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> WatchNext<'_, Self> {
        WatchNext::new(self)
    }

    pub fn recv(&mut self) -> Result<ProjectionWatchEvent<P::Output>> {
        decode_projection_event::<R, P>(&self.projection, &self.inner.recv()?)
    }

    #[must_use]
    pub fn cancellation(&self) -> WatchCancellation {
        self.inner.cancellation()
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    pub fn close(&self) {
        self.inner.close();
    }

    pub fn status(&self) -> Result<WatchStatus> {
        self.inner.status()
    }
}

pub struct GroupedAggregateWatch<K, A> {
    inner: QueryWatch,
    decode: fn(&Row) -> Result<(K, A)>,
    initial: Vec<(K, A)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupedAggregateWatchEvent<K, A> {
    source_revision: RevisionId,
    target_revision: RevisionId,
    inserted: Vec<(K, A)>,
    removed: Vec<(K, A)>,
}

impl<K, A> GroupedAggregateWatchEvent<K, A> {
    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.source_revision
    }

    #[must_use]
    pub const fn target_revision(&self) -> RevisionId {
        self.target_revision
    }

    #[must_use]
    pub fn inserted(&self) -> &[(K, A)] {
        &self.inserted
    }

    #[must_use]
    pub fn removed(&self) -> &[(K, A)] {
        &self.removed
    }
}

impl<K, A> GroupedAggregateWatch<K, A> {
    pub(crate) fn new(
        context: &crate::ReadContext,
        query: &Query,
        decode: fn(&Row) -> Result<(K, A)>,
    ) -> Result<Self> {
        let inner = QueryWatch::new(context, query)?;
        let initial = inner
            .initial()
            .rows()
            .iter()
            .map(decode)
            .collect::<Result<_>>()?;
        Ok(Self {
            inner,
            decode,
            initial,
        })
    }

    #[must_use]
    pub fn initial(&self) -> &[(K, A)] {
        &self.initial
    }

    pub fn try_recv(&mut self) -> Result<Option<GroupedAggregateWatchEvent<K, A>>> {
        self.inner
            .try_recv()?
            .map(|event| decode_grouped_aggregate_event(self.decode, &event))
            .transpose()
    }

    #[must_use]
    pub const fn subscription_id(&self) -> WatchSubscriptionId {
        self.inner.subscription_id()
    }

    #[must_use]
    pub fn readiness(&self) -> WatchReadiness {
        self.inner.readiness()
    }

    pub fn drain_ready(
        &mut self,
        max_events: usize,
    ) -> Result<WatchDrain<GroupedAggregateWatchEvent<K, A>>> {
        let decode = self.decode;
        self.inner
            .drain_ready(max_events)?
            .try_map(|event| decode_grouped_aggregate_event(decode, &event))
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> WatchNext<'_, Self> {
        WatchNext::new(self)
    }

    pub fn recv(&mut self) -> Result<GroupedAggregateWatchEvent<K, A>> {
        decode_grouped_aggregate_event(self.decode, &self.inner.recv()?)
    }

    #[must_use]
    pub fn cancellation(&self) -> WatchCancellation {
        self.inner.cancellation()
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    pub fn close(&self) {
        self.inner.close();
    }

    pub fn status(&self) -> Result<WatchStatus> {
        self.inner.status()
    }
}

fn decode_grouped_aggregate_event<K, A>(
    decode: fn(&Row) -> Result<(K, A)>,
    event: &WatchEvent,
) -> Result<GroupedAggregateWatchEvent<K, A>> {
    Ok(GroupedAggregateWatchEvent {
        source_revision: event.source_revision(),
        target_revision: event.target_revision(),
        inserted: event.inserted().iter().map(decode).collect::<Result<_>>()?,
        removed: event.removed().iter().map(decode).collect::<Result<_>>()?,
    })
}

mod watch_receiver_sealed {
    pub trait Sealed {}
}

/// Internal shape contract shared by the three public watch types.
///
/// It is public only because [`WatchNext`] is a public concrete Future type;
/// implementations are sealed to CFMD.
#[doc(hidden)]
pub trait WatchReceiver: watch_receiver_sealed::Sealed {
    type Event;

    fn readiness(&self) -> WatchReadiness;
    fn try_recv(&mut self) -> Result<Option<Self::Event>>;
}

impl watch_receiver_sealed::Sealed for QueryWatch {}
impl WatchReceiver for QueryWatch {
    type Event = WatchEvent;

    fn readiness(&self) -> WatchReadiness {
        QueryWatch::readiness(self)
    }

    fn try_recv(&mut self) -> Result<Option<Self::Event>> {
        QueryWatch::try_recv(self)
    }
}

impl watch_receiver_sealed::Sealed for MigratableQueryWatch {}
impl WatchReceiver for MigratableQueryWatch {
    type Event = MigratableWatchEvent;

    fn readiness(&self) -> WatchReadiness {
        MigratableQueryWatch::readiness(self)
    }

    fn try_recv(&mut self) -> Result<Option<Self::Event>> {
        MigratableQueryWatch::try_recv(self)
    }
}

impl<E: crate::Object> watch_receiver_sealed::Sealed for ObjectWatch<E> {}
impl<E: crate::Object> WatchReceiver for ObjectWatch<E> {
    type Event = ObjectWatchEvent<E>;

    fn readiness(&self) -> WatchReadiness {
        ObjectWatch::readiness(self)
    }

    fn try_recv(&mut self) -> Result<Option<Self::Event>> {
        ObjectWatch::try_recv(self)
    }
}

impl<R, P: crate::Projection<R>> watch_receiver_sealed::Sealed for ProjectionWatch<R, P> {}
impl<R, P: crate::Projection<R>> WatchReceiver for ProjectionWatch<R, P> {
    type Event = ProjectionWatchEvent<P::Output>;

    fn readiness(&self) -> WatchReadiness {
        ProjectionWatch::readiness(self)
    }

    fn try_recv(&mut self) -> Result<Option<Self::Event>> {
        ProjectionWatch::try_recv(self)
    }
}

impl<K, A> watch_receiver_sealed::Sealed for GroupedAggregateWatch<K, A> {}
impl<K, A> WatchReceiver for GroupedAggregateWatch<K, A> {
    type Event = GroupedAggregateWatchEvent<K, A>;

    fn readiness(&self) -> WatchReadiness {
        GroupedAggregateWatch::readiness(self)
    }

    fn try_recv(&mut self) -> Result<Option<Self::Event>> {
        GroupedAggregateWatch::try_recv(self)
    }
}

/// Executor-neutral Future returned directly by `watch.next()`.
pub struct WatchNext<'a, W: WatchReceiver> {
    watch: &'a mut W,
    readiness: WatchReadiness,
}

impl<'a, W: WatchReceiver> WatchNext<'a, W> {
    fn new(watch: &'a mut W) -> Self {
        let readiness = watch.readiness();
        Self { watch, readiness }
    }
}

impl<W: WatchReceiver> Drop for WatchNext<'_, W> {
    fn drop(&mut self) {
        self.readiness.clear_waker();
    }
}

impl<W: WatchReceiver> Future for WatchNext<'_, W> {
    type Output = Result<W::Event>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            let observed = this.readiness.generation();
            match this.watch.try_recv() {
                Ok(Some(event)) => return Poll::Ready(Ok(event)),
                Ok(None) => {}
                Err(error) => return Poll::Ready(Err(error)),
            }
            match this.readiness.poll_after(observed, context) {
                Poll::Ready(WatchWake::Woken { .. }) => {}
                Poll::Ready(WatchWake::Cancelled) => {
                    return Poll::Ready(Err(watch_closed("watch was cancelled")));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

fn decode_projection_event<R, P: crate::Projection<R>>(
    projection: &P,
    event: &WatchEvent,
) -> Result<ProjectionWatchEvent<P::Output>> {
    let inserted = event
        .inserted
        .iter()
        .map(|row| projection.decode(row))
        .collect::<Result<_>>()?;
    let removed = event
        .removed
        .iter()
        .map(|row| projection.decode(row))
        .collect::<Result<_>>()?;
    Ok(ProjectionWatchEvent {
        source_revision: event.source_revision,
        target_revision: event.target_revision,
        inserted,
        removed,
    })
}

fn decode_object_event<E: crate::Object>(event: &WatchEvent) -> Result<ObjectWatchEvent<E>> {
    Ok(ObjectWatchEvent {
        source_revision: event.source_revision,
        target_revision: event.target_revision,
        inserted: event
            .inserted
            .iter()
            .map(E::from_row)
            .collect::<Result<_>>()?,
        removed: event
            .removed
            .iter()
            .map(E::from_row)
            .collect::<Result<_>>()?,
    })
}
