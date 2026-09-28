use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    marker::PhantomData,
    sync::{Arc, Weak},
};

use crate::{Error, ErrorKind, Query, RelationResult, Result, RevisionId, Row};

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

#[derive(Debug)]
pub struct QueryWatch {
    runtime: Weak<kernel_plan::DurableRuntime>,
    state: kernel_query::MaterializedRelPlanState,
    anchor_revision: RevisionId,
    publication_generation: u64,
    cancellation: WatchCancellation,
    initial: RelationResult,
}

impl QueryWatch {
    pub(crate) fn new(context: &crate::ReadContext, query: &Query) -> Result<Self> {
        context.authority.require(crate::Permission::Watch)?;
        if !context.is_live() {
            return Err(Error::new(
                ErrorKind::WatchUnavailable,
                "watch requires a live snapshot; historical db.at(...) views are immutable",
            ));
        }
        let runtime = context.runtime_arc();
        let revision = context.kernel_revision();
        let state = kernel_query::MaterializedRelPlanState::build(
            &query.inner,
            &revision.state().model,
            revision.semantic_context(),
            runtime.semantic_registry(),
        )
        .map_err(|error| {
            Error::new(
                ErrorKind::WatchUnavailable,
                format!("query has no exact maintained watch program: {error:?}"),
            )
        })?;
        let initial = context.execute(query)?;
        let wait_handle = runtime.revision_publication_wait_handle();
        let publication_generation = wait_handle.generation();
        Ok(Self {
            runtime: Arc::downgrade(runtime),
            state,
            anchor_revision: context.revision(),
            publication_generation,
            cancellation: WatchCancellation { inner: wait_handle },
            initial,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.anchor_revision
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
        if self.cancellation.is_cancelled() {
            return Ok(WatchStatus::Cancelled {
                revision: self.anchor_revision,
            });
        }
        let Some(runtime) = self.runtime.upgrade() else {
            return Ok(WatchStatus::RuntimeClosed {
                revision: self.anchor_revision,
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
        if head_revision == self.anchor_revision {
            return Ok(WatchStatus::Current {
                revision: self.anchor_revision,
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
                anchor_revision: self.anchor_revision,
                head_revision,
            });
        };
        let source = kernel_types::RevisionId::new(self.anchor_revision.raw());
        let Some(path) = effect_path(source, head, &effects) else {
            return Ok(WatchStatus::Unavailable {
                anchor_revision: self.anchor_revision,
                head_revision,
            });
        };
        Ok(WatchStatus::Lagging {
            anchor_revision: self.anchor_revision,
            head_revision,
            pending_transitions: path.len(),
        })
    }

    pub fn try_recv(&mut self) -> Result<Option<WatchEvent>> {
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
        if head == kernel_types::RevisionId::new(self.anchor_revision.raw()) {
            return Ok(None);
        }
        self.advance_one(&runtime, head).map(Some)
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
                    return Err(watch_closed("watch was cancelled"));
                }
            }
        }
    }

    fn advance_one(
        &mut self,
        runtime: &kernel_plan::DurableRuntime,
        head: kernel_types::RevisionId,
    ) -> Result<WatchEvent> {
        let effect = next_effect(runtime, self.anchor_revision, head)?;
        let source = historical_revision(runtime, effect.source_revision, "source")?;
        let target = historical_revision(runtime, effect.target_revision, "target")?;
        if source.semantic_context() != target.semantic_context() {
            return Err(Error::new(
                ErrorKind::WatchUnavailable,
                "watch cannot cross a semantic schema revision",
            ));
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
        let event = WatchEvent {
            source_revision: effect.source_revision.into(),
            target_revision: effect.target_revision.into(),
            inserted: output
                .as_ref()
                .map_or_else(Vec::new, |delta| convert_rows(&delta.inserted)),
            removed: output
                .as_ref()
                .map_or_else(Vec::new, |delta| convert_rows(&delta.removed)),
        };
        self.anchor_revision = effect.target_revision.into();
        Ok(event)
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
    if matches!(
        effect.kind,
        kernel_plan::RuntimeHistoryEffectKind::FullRevision
            | kernel_plan::RuntimeHistoryEffectKind::SchemaMigration
            | kernel_plan::RuntimeHistoryEffectKind::LegacyTargetOnly
    ) {
        return Err(Error::new(
            ErrorKind::WatchUnavailable,
            "watch cannot cross an opaque/full/schema historical transition",
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
