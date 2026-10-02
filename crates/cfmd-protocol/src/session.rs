use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, TryLockError},
};

use cfmd_runtime::{RevisionId, SessionDatabase, TransactionId};

use crate::{
    HistoryEntryDto, OpenWatchRequest, OpenWatchResponse, ProtocolError, ProtocolErrorCode,
    QueryRequest, QueryResponse, Result, Row, SnapshotTarget, SubscriptionId, WatchEventDto,
    WatchStatusDto,
};

pub const PROTOCOL_VERSION: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolLimits {
    pub max_query_nodes: usize,
    pub max_query_depth: usize,
    pub max_mutations: usize,
    pub max_rows_per_commit: usize,
    pub max_row_width: usize,
    pub max_value_nodes: usize,
    pub max_watch_subscriptions: usize,
}

impl Default for ProtocolLimits {
    fn default() -> Self {
        Self {
            max_query_nodes: 4_096,
            max_query_depth: 128,
            max_mutations: 1_024,
            max_rows_per_commit: 100_000,
            max_row_width: 1_024,
            max_value_nodes: 1_000_000,
            max_watch_subscriptions: 1_024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationMutation {
    pub relation: u128,
    pub inserted: Vec<Row>,
    pub removed: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    pub base_revision: u64,
    pub transaction: u128,
    pub mutations: Vec<RelationMutation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitResponse {
    Committed { revision: u64 },
    AlreadyCommitted { revision: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostedRequest {
    CurrentRevision,
    Query(QueryRequest),
    History { target: SnapshotTarget },
    Commit(CommitRequest),
    OpenWatch(OpenWatchRequest),
    NextWatch { subscription: SubscriptionId },
    WatchStatus { subscription: SubscriptionId },
    CancelWatch { subscription: SubscriptionId },
    CloseWatch { subscription: SubscriptionId },
    CloseSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostedResponse {
    CurrentRevision {
        revision: u64,
    },
    Query(QueryResponse),
    History {
        anchor_revision: u64,
        entries: Vec<HistoryEntryDto>,
    },
    Commit(CommitResponse),
    WatchOpened(OpenWatchResponse),
    WatchEvent(WatchEventDto),
    WatchStatus {
        subscription: SubscriptionId,
        status: WatchStatusDto,
    },
    WatchCancelled {
        subscription: SubscriptionId,
    },
    WatchClosed {
        subscription: SubscriptionId,
    },
    SessionClosed,
}

#[derive(Debug)]
struct HostedSubscription {
    watch: Mutex<cfmd_runtime::QueryWatch>,
    cancellation: cfmd_runtime::WatchCancellation,
    authorization: cfmd_runtime::WatchAuthorization,
}

#[derive(Debug)]
struct SubscriptionRegistryState {
    next_id: u64,
    closed: bool,
    subscriptions: BTreeMap<SubscriptionId, Arc<HostedSubscription>>,
}

#[derive(Debug)]
struct SubscriptionRegistry {
    state: Mutex<SubscriptionRegistryState>,
}

impl SubscriptionRegistry {
    fn new() -> Self {
        Self {
            state: Mutex::new(SubscriptionRegistryState {
                next_id: 1,
                closed: false,
                subscriptions: BTreeMap::new(),
            }),
        }
    }

    fn insert(
        &self,
        watch: cfmd_runtime::QueryWatch,
        max_subscriptions: usize,
    ) -> Result<SubscriptionId> {
        let cancellation = watch.cancellation();
        let authorization = watch.authorization();
        let mut state = self.state.lock().map_err(|_| protocol_internal())?;
        if state.closed {
            return Err(session_closed());
        }
        if state.subscriptions.len() >= max_subscriptions {
            return Err(ProtocolError::new(
                ProtocolErrorCode::ResourceLimit,
                "hosted session reached its watch subscription limit",
            ));
        }
        let id = SubscriptionId::new(state.next_id);
        state.next_id = state.next_id.checked_add(1).ok_or_else(protocol_internal)?;
        state.subscriptions.insert(
            id,
            Arc::new(HostedSubscription {
                watch: Mutex::new(watch),
                cancellation,
                authorization,
            }),
        );
        Ok(id)
    }

    fn get(&self, id: SubscriptionId) -> Result<Arc<HostedSubscription>> {
        let state = self.state.lock().map_err(|_| protocol_internal())?;
        if state.closed {
            return Err(session_closed());
        }
        state
            .subscriptions
            .get(&id)
            .cloned()
            .ok_or_else(|| unknown_subscription(id))
    }

    fn remove(&self, id: SubscriptionId) -> Result<Arc<HostedSubscription>> {
        let mut state = self.state.lock().map_err(|_| protocol_internal())?;
        if state.closed {
            return Err(session_closed());
        }
        state
            .subscriptions
            .remove(&id)
            .ok_or_else(|| unknown_subscription(id))
    }

    fn cancel_all(&self) -> Result<()> {
        let subscriptions = {
            let state = self.state.lock().map_err(|_| protocol_internal())?;
            if state.closed {
                return Err(session_closed());
            }
            state.subscriptions.values().cloned().collect::<Vec<_>>()
        };
        for subscription in subscriptions {
            subscription.cancellation.cancel();
        }
        Ok(())
    }

    fn reauthorize_all(&self) -> Result<()> {
        let subscriptions = {
            let state = self.state.lock().map_err(|_| protocol_internal())?;
            if state.closed {
                return Err(session_closed());
            }
            state.subscriptions.values().cloned().collect::<Vec<_>>()
        };
        for subscription in subscriptions {
            if subscription.authorization.reauthorize().is_err() {
                subscription.cancellation.cancel();
            }
        }
        Ok(())
    }

    fn close(&self) -> Result<()> {
        let subscriptions = {
            let mut state = self.state.lock().map_err(|_| protocol_internal())?;
            if state.closed {
                return Ok(());
            }
            state.closed = true;
            std::mem::take(&mut state.subscriptions)
        };
        for subscription in subscriptions.into_values() {
            subscription.cancellation.cancel();
        }
        Ok(())
    }

    fn is_closed(&self) -> Result<bool> {
        Ok(self.state.lock().map_err(|_| protocol_internal())?.closed)
    }
}

#[derive(Debug, Clone)]
pub struct HostedSession {
    database: SessionDatabase,
    limits: ProtocolLimits,
    subscriptions: Arc<SubscriptionRegistry>,
}

impl HostedSession {
    #[must_use]
    pub const fn protocol_version() -> u16 {
        PROTOCOL_VERSION
    }

    #[must_use]
    pub fn principal(&self) -> cfmd_runtime::PrincipalId {
        self.database.session().principal()
    }

    pub fn permissions(&self) -> cfmd_runtime::Result<cfmd_runtime::PermissionSet> {
        self.database
            .session()
            .snapshot()
            .map(|snapshot| snapshot.permissions().clone())
    }

    #[must_use]
    pub fn new(database: SessionDatabase) -> Self {
        Self {
            database,
            limits: ProtocolLimits::default(),
            subscriptions: Arc::new(SubscriptionRegistry::new()),
        }
    }

    #[must_use]
    pub fn with_limits(database: SessionDatabase, limits: ProtocolLimits) -> Self {
        Self {
            database,
            limits,
            subscriptions: Arc::new(SubscriptionRegistry::new()),
        }
    }

    #[must_use]
    pub const fn limits(&self) -> ProtocolLimits {
        self.limits
    }

    pub fn cancel_all_watches(&self) -> Result<()> {
        self.subscriptions.cancel_all()
    }

    pub fn reauthorize_watches(&self) -> Result<()> {
        self.subscriptions.reauthorize_all()
    }

    pub fn close(&self) -> Result<()> {
        self.subscriptions.close()
    }

    pub fn is_closed(&self) -> Result<bool> {
        self.subscriptions.is_closed()
    }

    pub fn execute(&self, request: HostedRequest) -> Result<HostedResponse> {
        if matches!(request, HostedRequest::CloseSession) {
            self.close()?;
            return Ok(HostedResponse::SessionClosed);
        }
        if self.is_closed()? {
            return Err(session_closed());
        }
        match request {
            HostedRequest::CurrentRevision => self.current_revision(),
            HostedRequest::Query(request) => self.query(request).map(HostedResponse::Query),
            HostedRequest::History { target } => self.history(target),
            HostedRequest::Commit(request) => self.commit(request).map(HostedResponse::Commit),
            HostedRequest::OpenWatch(request) => {
                self.open_watch(request).map(HostedResponse::WatchOpened)
            }
            HostedRequest::NextWatch { subscription } => self
                .next_watch(subscription)
                .map(HostedResponse::WatchEvent),
            HostedRequest::WatchStatus { subscription } => {
                self.watch_status(subscription)
                    .map(|status| HostedResponse::WatchStatus {
                        subscription,
                        status,
                    })
            }
            HostedRequest::CancelWatch { subscription } => {
                self.cancel_watch(subscription)?;
                Ok(HostedResponse::WatchCancelled { subscription })
            }
            HostedRequest::CloseWatch { subscription } => {
                self.close_watch(subscription)?;
                Ok(HostedResponse::WatchClosed { subscription })
            }
            HostedRequest::CloseSession => unreachable!("handled before session-open check"),
        }
    }

    fn current_revision(&self) -> Result<HostedResponse> {
        Ok(HostedResponse::CurrentRevision {
            revision: self.database.current_revision()?.raw(),
        })
    }

    fn context(&self, target: SnapshotTarget) -> Result<cfmd_runtime::ReadContext> {
        Ok(match target {
            SnapshotTarget::Head => self.database.snapshot()?,
            SnapshotTarget::Revision(revision) => self.database.at(RevisionId::new(revision))?,
        })
    }

    fn query(&self, request: QueryRequest) -> Result<QueryResponse> {
        validate_query(&request.query, self.limits)?;
        let context = self.context(request.target)?;
        let revision = context.revision();
        let result = context.execute(&request.query.into_runtime())?;
        Ok(QueryResponse::from_runtime(revision, result))
    }

    fn history(&self, target: SnapshotTarget) -> Result<HostedResponse> {
        let context = self.context(target)?;
        let anchor_revision = context.revision().raw();
        let history = context.history()?;
        Ok(HostedResponse::History {
            anchor_revision,
            entries: history.entries().iter().map(Into::into).collect(),
        })
    }

    fn open_watch(&self, request: OpenWatchRequest) -> Result<OpenWatchResponse> {
        validate_query(&request.query, self.limits)?;
        let context = self.database.snapshot()?;
        let watch = context.watch(&request.query.into_runtime())?;
        let initial = QueryResponse::from_runtime(watch.revision(), watch.initial().clone());
        let subscription = self
            .subscriptions
            .insert(watch, self.limits.max_watch_subscriptions)?;
        Ok(OpenWatchResponse {
            subscription,
            initial,
        })
    }

    fn next_watch(&self, subscription: SubscriptionId) -> Result<WatchEventDto> {
        let subscription_state = self.subscriptions.get(subscription)?;
        let mut watch = try_lock_watch(&subscription_state)?;
        let event = watch.recv()?;
        Ok(WatchEventDto::from_runtime(subscription, &event))
    }

    fn watch_status(&self, subscription: SubscriptionId) -> Result<WatchStatusDto> {
        let subscription_state = self.subscriptions.get(subscription)?;
        let watch = try_lock_watch(&subscription_state)?;
        Ok(watch.status()?.into())
    }

    fn cancel_watch(&self, subscription: SubscriptionId) -> Result<()> {
        self.subscriptions.get(subscription)?.cancellation.cancel();
        Ok(())
    }

    fn close_watch(&self, subscription: SubscriptionId) -> Result<()> {
        self.subscriptions
            .remove(subscription)?
            .cancellation
            .cancel();
        Ok(())
    }

    fn commit(&self, request: CommitRequest) -> Result<CommitResponse> {
        validate_commit(&request, self.limits)?;
        let mut plan = self.database.plan()?;
        if plan.base_revision().raw() != request.base_revision {
            return Err(ProtocolError::new(
                ProtocolErrorCode::StaleRevision,
                format!(
                    "commit base revision {} is stale; current revision is {}",
                    request.base_revision,
                    plan.base_revision().raw()
                ),
            ));
        }
        if request.mutations.is_empty() {
            return Err(ProtocolError::new(
                ProtocolErrorCode::InvalidRequest,
                "commit requires at least one relation mutation",
            ));
        }
        for mutation in request.mutations {
            let relation = cfmd_runtime::RelationId::new(mutation.relation);
            for row in mutation.removed {
                plan.remove(relation, row.into_iter().map(Into::into).collect());
            }
            for row in mutation.inserted {
                plan.insert(relation, row.into_iter().map(Into::into).collect());
            }
        }
        let outcome = self
            .database
            .commit_plan(&plan, TransactionId::new(request.transaction))?;
        Ok(match outcome {
            cfmd_runtime::CommitOutcome::Committed { revision } => CommitResponse::Committed {
                revision: revision.raw(),
            },
            cfmd_runtime::CommitOutcome::AlreadyCommitted { revision } => {
                CommitResponse::AlreadyCommitted {
                    revision: revision.raw(),
                }
            }
        })
    }
}

fn try_lock_watch(
    subscription: &HostedSubscription,
) -> Result<std::sync::MutexGuard<'_, cfmd_runtime::QueryWatch>> {
    match subscription.watch.try_lock() {
        Ok(watch) => Ok(watch),
        Err(TryLockError::WouldBlock) => Err(ProtocolError::new(
            ProtocolErrorCode::InvalidRequest,
            "watch subscription already has an in-flight consumer",
        )),
        Err(TryLockError::Poisoned(_)) => Err(protocol_internal()),
    }
}

fn unknown_subscription(id: SubscriptionId) -> ProtocolError {
    ProtocolError::new(
        ProtocolErrorCode::NotFound,
        format!("watch subscription {} does not exist", id.raw()),
    )
}

fn session_closed() -> ProtocolError {
    ProtocolError::new(ProtocolErrorCode::SessionClosed, "hosted session is closed")
}

fn protocol_internal() -> ProtocolError {
    ProtocolError::new(ProtocolErrorCode::Internal, "internal protocol state error")
}

fn validate_query(query: &crate::ProtocolQuery, limits: ProtocolLimits) -> Result<()> {
    let mut stack = vec![(query, 1_usize)];
    let mut nodes = 0_usize;
    let mut value_nodes = 0_usize;
    while let Some((node, depth)) = stack.pop() {
        nodes = nodes.saturating_add(1);
        if nodes > limits.max_query_nodes || depth > limits.max_query_depth {
            return Err(ProtocolError::new(
                ProtocolErrorCode::ResourceLimit,
                "query exceeds protocol complexity limit",
            ));
        }
        match node {
            crate::ProtocolQuery::Scan { .. } => {}
            crate::ProtocolQuery::FilterEq { input, value, .. } => {
                value_nodes = value_nodes.saturating_add(count_value_nodes(
                    value,
                    limits.max_value_nodes.saturating_sub(value_nodes),
                )?);
                stack.push((input, depth + 1));
            }
            crate::ProtocolQuery::Project { input, .. }
            | crate::ProtocolQuery::GroupCount { input, .. }
            | crate::ProtocolQuery::Distinct { input, .. }
            | crate::ProtocolQuery::TopKWithTies { input, .. } => stack.push((input, depth + 1)),
            crate::ProtocolQuery::JoinEq { left, right, .. }
            | crate::ProtocolQuery::Difference { left, right }
            | crate::ProtocolQuery::AntiJoin { left, right, .. } => {
                stack.push((left, depth + 1));
                stack.push((right, depth + 1));
            }
        }
    }
    Ok(())
}

fn validate_commit(request: &CommitRequest, limits: ProtocolLimits) -> Result<()> {
    if request.mutations.is_empty() {
        return Err(ProtocolError::new(
            ProtocolErrorCode::InvalidRequest,
            "commit requires at least one relation mutation",
        ));
    }
    if request.mutations.len() > limits.max_mutations {
        return Err(ProtocolError::new(
            ProtocolErrorCode::ResourceLimit,
            "commit exceeds mutation limit",
        ));
    }
    let mut rows = 0_usize;
    let mut value_nodes = 0_usize;
    for mutation in &request.mutations {
        rows = rows
            .saturating_add(mutation.inserted.len())
            .saturating_add(mutation.removed.len());
        if rows > limits.max_rows_per_commit {
            return Err(ProtocolError::new(
                ProtocolErrorCode::ResourceLimit,
                "commit exceeds row limit",
            ));
        }
        for row in mutation.inserted.iter().chain(&mutation.removed) {
            if row.len() > limits.max_row_width {
                return Err(ProtocolError::new(
                    ProtocolErrorCode::ResourceLimit,
                    "row exceeds protocol width limit",
                ));
            }
            for value in row {
                value_nodes = value_nodes.saturating_add(count_value_nodes(
                    value,
                    limits.max_value_nodes.saturating_sub(value_nodes),
                )?);
                if value_nodes > limits.max_value_nodes {
                    return Err(ProtocolError::new(
                        ProtocolErrorCode::ResourceLimit,
                        "commit exceeds value complexity limit",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn count_value_nodes(value: &crate::ProtocolValue, remaining: usize) -> Result<usize> {
    let mut stack = vec![value];
    let mut count = 0_usize;
    while let Some(value) = stack.pop() {
        count = count.saturating_add(1);
        if count > remaining {
            return Err(ProtocolError::new(
                ProtocolErrorCode::ResourceLimit,
                "value exceeds protocol complexity limit",
            ));
        }
        match value {
            crate::ProtocolValue::Product(values) => stack.extend(values.values()),
            crate::ProtocolValue::Option(Some(value))
            | crate::ProtocolValue::Variant { value, .. } => stack.push(value),
            crate::ProtocolValue::Seq(values)
            | crate::ProtocolValue::Set {
                elements: values, ..
            } => stack.extend(values),
            crate::ProtocolValue::Bag { entries, .. } => {
                stack.extend(entries.iter().map(|(value, _)| value));
            }
            crate::ProtocolValue::Map { entries, .. } => {
                for (key, value) in entries {
                    stack.push(key);
                    stack.push(value);
                }
            }
            crate::ProtocolValue::Unit
            | crate::ProtocolValue::Bool(_)
            | crate::ProtocolValue::I64(_)
            | crate::ProtocolValue::F64Bits(_)
            | crate::ProtocolValue::Text(_)
            | crate::ProtocolValue::LiveEntityRef(_)
            | crate::ProtocolValue::HistoricalEntityRef(_)
            | crate::ProtocolValue::Option(None) => {}
        }
    }
    Ok(count)
}
