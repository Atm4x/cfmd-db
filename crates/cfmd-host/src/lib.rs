//! Transport-neutral hosted CFMD server composition.
//!
//! This crate owns connection admission, authentication/authorization
//! composition and hosted connection lifecycle. It deliberately owns no
//! listener, socket, TLS implementation, IPC protocol or background thread.

use std::{
    collections::BTreeMap,
    error::Error as StdError,
    fmt,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering},
    },
    time::SystemTime,
};

use cfmd_protocol::{
    HostedSession, ProtocolError, ProtocolErrorCode, ProtocolLimits,
    wire::{WireHostedSession, WireLimits},
};
use cfmd_runtime::{Database, PermissionSet, PrincipalId, Session};

pub type Result<T> = std::result::Result<T, HostError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostErrorCode {
    AccessDenied,
    ProviderUnavailable,
    ConnectionLimit,
    Draining,
    Closed,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    code: HostErrorCode,
    message: &'static str,
}

impl HostError {
    #[must_use]
    pub const fn new(code: HostErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }

    #[must_use]
    pub const fn code(&self) -> HostErrorCode {
        self.code
    }

    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl StdError for HostError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelBinding {
    Unbound,
    Bound([u8; 32]),
}

impl ChannelBinding {
    #[must_use]
    pub const fn bound(exporter: [u8; 32]) -> Self {
        Self::Bound(exporter)
    }

    #[must_use]
    pub const fn is_bound(self) -> bool {
        matches!(self, Self::Bound(_))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AuthenticationContext<'a, Evidence> {
    evidence: &'a Evidence,
    channel_binding: ChannelBinding,
}

impl<'a, Evidence> AuthenticationContext<'a, Evidence> {
    #[must_use]
    pub const fn new(evidence: &'a Evidence, channel_binding: ChannelBinding) -> Self {
        Self {
            evidence,
            channel_binding,
        }
    }

    #[must_use]
    pub const fn evidence(&self) -> &'a Evidence {
        self.evidence
    }

    #[must_use]
    pub const fn channel_binding(&self) -> ChannelBinding {
        self.channel_binding
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthenticationDecision {
    Authenticated(PrincipalId),
    Reject,
}

pub trait Authenticator<Evidence>: Send + Sync + 'static {
    type Error: StdError + Send + Sync + 'static;

    fn authenticate(
        &self,
        context: AuthenticationContext<'_, Evidence>,
    ) -> std::result::Result<AuthenticationDecision, Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationGrant {
    permissions: PermissionSet,
    expires_at: Option<SystemTime>,
}

impl AuthorizationGrant {
    #[must_use]
    pub const fn new(permissions: PermissionSet) -> Self {
        Self {
            permissions,
            expires_at: None,
        }
    }

    #[must_use]
    pub const fn expiring(permissions: PermissionSet, expires_at: SystemTime) -> Self {
        Self {
            permissions,
            expires_at: Some(expires_at),
        }
    }

    #[must_use]
    pub const fn permissions(&self) -> &PermissionSet {
        &self.permissions
    }

    #[must_use]
    pub const fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationDecision {
    Grant(AuthorizationGrant),
    Reject,
}

pub trait Authorizer: Send + Sync + 'static {
    type Error: StdError + Send + Sync + 'static;

    fn authorize(
        &self,
        principal: PrincipalId,
    ) -> std::result::Result<AuthorizationDecision, Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerLimits {
    pub max_connections: usize,
    pub max_in_flight_requests_per_connection: usize,
    pub protocol: ProtocolLimits,
    pub wire: WireLimits,
}

impl Default for ServerLimits {
    fn default() -> Self {
        Self {
            max_connections: 1_024,
            max_in_flight_requests_per_connection: 16,
            protocol: ProtocolLimits::default(),
            wire: WireLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum ServerLifecycle {
    Running = 0,
    Draining = 1,
    Closed = 2,
}

impl ServerLifecycle {
    fn load(value: u8) -> Self {
        match value {
            0 => Self::Running,
            1 => Self::Draining,
            _ => Self::Closed,
        }
    }
}

#[derive(Debug)]
struct ServerState {
    lifecycle: AtomicU8,
    active_connections: AtomicUsize,
    next_connection_id: AtomicU64,
    connections: Mutex<BTreeMap<u64, Weak<HostedConnection>>>,
}

impl ServerState {
    fn new() -> Self {
        Self {
            lifecycle: AtomicU8::new(ServerLifecycle::Running as u8),
            active_connections: AtomicUsize::new(0),
            next_connection_id: AtomicU64::new(1),
            connections: Mutex::new(BTreeMap::new()),
        }
    }

    fn lifecycle(&self) -> ServerLifecycle {
        ServerLifecycle::load(self.lifecycle.load(Ordering::Acquire))
    }

    fn reserve_connection(&self, limit: usize) -> Result<()> {
        match self.lifecycle() {
            ServerLifecycle::Running => {}
            ServerLifecycle::Draining => return Err(host_draining()),
            ServerLifecycle::Closed => return Err(host_closed()),
        }
        if limit == 0 {
            return Err(connection_limit());
        }
        let result =
            self.active_connections
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    (current < limit).then_some(current + 1)
                });
        if result.is_err() {
            return Err(connection_limit());
        }
        if self.lifecycle() != ServerLifecycle::Running {
            self.active_connections.fetch_sub(1, Ordering::AcqRel);
            return Err(match self.lifecycle() {
                ServerLifecycle::Draining => host_draining(),
                _ => host_closed(),
            });
        }
        Ok(())
    }

    fn release_connection(&self, id: u64) {
        self.active_connections.fetch_sub(1, Ordering::AcqRel);
        if let Ok(mut connections) = self.connections.lock() {
            connections.remove(&id);
        }
    }

    fn live_connections(&self) -> Vec<Arc<HostedConnection>> {
        self.connections
            .lock()
            .map(|connections| connections.values().filter_map(Weak::upgrade).collect())
            .unwrap_or_default()
    }
}

pub struct HostedServer<Evidence, A, Z>
where
    A: Authenticator<Evidence>,
    Z: Authorizer,
{
    database: Database,
    authenticator: Arc<A>,
    authorizer: Arc<Z>,
    limits: ServerLimits,
    state: Arc<ServerState>,
    evidence: std::marker::PhantomData<fn(&Evidence)>,
}

impl<Evidence, A, Z> Clone for HostedServer<Evidence, A, Z>
where
    A: Authenticator<Evidence>,
    Z: Authorizer,
{
    fn clone(&self) -> Self {
        Self {
            database: self.database.clone(),
            authenticator: Arc::clone(&self.authenticator),
            authorizer: Arc::clone(&self.authorizer),
            limits: self.limits,
            state: Arc::clone(&self.state),
            evidence: std::marker::PhantomData,
        }
    }
}

impl<Evidence, A, Z> HostedServer<Evidence, A, Z>
where
    A: Authenticator<Evidence>,
    Z: Authorizer,
{
    #[must_use]
    pub fn new(database: Database, authenticator: A, authorizer: Z) -> Self {
        Self::with_limits(database, authenticator, authorizer, ServerLimits::default())
    }

    #[must_use]
    pub fn with_limits(
        database: Database,
        authenticator: A,
        authorizer: Z,
        limits: ServerLimits,
    ) -> Self {
        Self {
            database,
            authenticator: Arc::new(authenticator),
            authorizer: Arc::new(authorizer),
            limits,
            state: Arc::new(ServerState::new()),
            evidence: std::marker::PhantomData,
        }
    }

    #[must_use]
    pub const fn limits(&self) -> ServerLimits {
        self.limits
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.state.lifecycle() == ServerLifecycle::Closed
    }

    #[must_use]
    pub fn is_draining(&self) -> bool {
        self.state.lifecycle() == ServerLifecycle::Draining
    }

    #[must_use]
    pub fn is_drained(&self) -> bool {
        self.is_draining() && self.active_connections() == 0
    }

    #[must_use]
    pub fn active_connections(&self) -> usize {
        self.state.active_connections.load(Ordering::Acquire)
    }

    pub fn connect(&self, evidence: &Evidence) -> Result<Arc<HostedConnection>> {
        self.connect_with_context(AuthenticationContext::new(
            evidence,
            ChannelBinding::Unbound,
        ))
    }

    pub fn connect_bound(
        &self,
        evidence: &Evidence,
        binding: [u8; 32],
    ) -> Result<Arc<HostedConnection>> {
        self.connect_with_context(AuthenticationContext::new(
            evidence,
            ChannelBinding::bound(binding),
        ))
    }

    pub fn connect_with_context(
        &self,
        context: AuthenticationContext<'_, Evidence>,
    ) -> Result<Arc<HostedConnection>> {
        self.state.reserve_connection(self.limits.max_connections)?;
        let channel_binding = context.channel_binding();
        let principal = match self.authenticator.authenticate(context) {
            Ok(AuthenticationDecision::Authenticated(principal)) => principal,
            Ok(AuthenticationDecision::Reject) => {
                self.state.active_connections.fetch_sub(1, Ordering::AcqRel);
                return Err(access_denied());
            }
            Err(_) => {
                self.state.active_connections.fetch_sub(1, Ordering::AcqRel);
                return Err(provider_unavailable());
            }
        };
        let grant = match self.authorizer.authorize(principal) {
            Ok(AuthorizationDecision::Grant(grant)) => grant,
            Ok(AuthorizationDecision::Reject) => {
                self.state.active_connections.fetch_sub(1, Ordering::AcqRel);
                return Err(access_denied());
            }
            Err(_) => {
                self.state.active_connections.fetch_sub(1, Ordering::AcqRel);
                return Err(provider_unavailable());
            }
        };
        if grant
            .expires_at()
            .is_some_and(|deadline| deadline <= SystemTime::now())
        {
            self.state.active_connections.fetch_sub(1, Ordering::AcqRel);
            return Err(access_denied());
        }
        if self.state.lifecycle() != ServerLifecycle::Running {
            self.state.active_connections.fetch_sub(1, Ordering::AcqRel);
            return Err(match self.state.lifecycle() {
                ServerLifecycle::Draining => host_draining(),
                _ => host_closed(),
            });
        }

        let session = Session::new(principal, grant.permissions().clone());
        let hosted = HostedSession::with_limits(
            self.database.session(session.clone()),
            self.limits.protocol,
        );
        let wire = WireHostedSession::with_limits(hosted, self.limits.wire);
        let id = self
            .state
            .next_connection_id
            .fetch_add(1, Ordering::Relaxed);
        let connection = Arc::new(HostedConnection {
            id,
            session,
            channel_binding,
            expires_at: Mutex::new(grant.expires_at()),
            wire,
            max_in_flight: self.limits.max_in_flight_requests_per_connection,
            in_flight: AtomicUsize::new(0),
            closed: AtomicU8::new(0),
            lease_released: AtomicU8::new(0),
            server: Arc::downgrade(&self.state),
        });
        if self
            .state
            .connections
            .lock()
            .map(|mut connections| connections.insert(id, Arc::downgrade(&connection)))
            .is_err()
        {
            connection.close();
            return Err(host_internal());
        }
        if self.state.lifecycle() != ServerLifecycle::Running {
            connection.close();
            return Err(match self.state.lifecycle() {
                ServerLifecycle::Draining => host_draining(),
                _ => host_closed(),
            });
        }
        Ok(connection)
    }

    pub fn refresh_authorization(&self, connection: &Arc<HostedConnection>) -> Result<()> {
        if !connection.belongs_to(&self.state) || connection.is_closed() {
            return Err(host_closed());
        }
        let grant = match self.authorizer.authorize(connection.principal()) {
            Ok(AuthorizationDecision::Grant(grant)) => grant,
            Ok(AuthorizationDecision::Reject) => {
                connection.revoke();
                return Err(access_denied());
            }
            Err(_) => return Err(provider_unavailable()),
        };
        if grant
            .expires_at()
            .is_some_and(|deadline| deadline <= SystemTime::now())
        {
            connection.revoke();
            return Err(access_denied());
        }
        connection
            .session
            .refresh_permissions(grant.permissions().clone())
            .map_err(|_| host_internal())?;
        *connection.expires_at.lock().map_err(|_| host_internal())? = grant.expires_at();
        if !grant
            .permissions()
            .contains(cfmd_runtime::Permission::Watch)
        {
            let _ = connection.wire.cancel_all_watches();
        }
        Ok(())
    }

    #[must_use]
    pub fn revoke_connection(&self, id: u64) -> bool {
        let connection = self
            .state
            .connections
            .lock()
            .ok()
            .and_then(|connections| connections.get(&id).and_then(Weak::upgrade));
        connection.is_some_and(|connection| {
            connection.revoke();
            true
        })
    }

    #[must_use]
    pub fn revoke_principal(&self, principal: PrincipalId) -> usize {
        let mut revoked = 0;
        for connection in self.state.live_connections() {
            if connection.principal() == principal && !connection.is_closed() {
                connection.revoke();
                revoked += 1;
            }
        }
        revoked
    }

    #[must_use]
    pub fn next_expiration(&self) -> Option<SystemTime> {
        self.state
            .live_connections()
            .into_iter()
            .filter_map(|connection| connection.expires_at())
            .min()
    }

    #[must_use]
    pub fn expire_due(&self, now: SystemTime) -> usize {
        let mut expired = 0;
        for connection in self.state.live_connections() {
            if connection.is_expired_at(now) {
                connection.revoke();
                expired += 1;
            }
        }
        expired
    }

    pub fn drain(&self) {
        let _ = self.state.lifecycle.compare_exchange(
            ServerLifecycle::Running as u8,
            ServerLifecycle::Draining as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub fn close(&self) {
        let previous = self
            .state
            .lifecycle
            .swap(ServerLifecycle::Closed as u8, Ordering::AcqRel);
        if ServerLifecycle::load(previous) == ServerLifecycle::Closed {
            return;
        }
        for connection in self.state.live_connections() {
            connection.close();
        }
    }
}

pub struct HostedConnection {
    id: u64,
    session: Session,
    channel_binding: ChannelBinding,
    expires_at: Mutex<Option<SystemTime>>,
    wire: WireHostedSession,
    max_in_flight: usize,
    in_flight: AtomicUsize,
    closed: AtomicU8,
    lease_released: AtomicU8,
    server: Weak<ServerState>,
}

impl HostedConnection {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub const fn principal(&self) -> PrincipalId {
        self.session.principal()
    }

    pub fn permissions(&self) -> cfmd_runtime::Result<PermissionSet> {
        self.session
            .snapshot()
            .map(|snapshot| snapshot.permissions().clone())
    }

    #[must_use]
    pub const fn channel_binding(&self) -> ChannelBinding {
        self.channel_binding
    }

    #[must_use]
    pub fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at.lock().ok().and_then(|deadline| *deadline)
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire) != 0 || self.wire.is_closed().unwrap_or(true)
    }

    #[must_use]
    pub fn in_flight_requests(&self) -> usize {
        self.in_flight.load(Ordering::Acquire)
    }

    pub fn handle_frame(&self, frame: &[u8]) -> cfmd_protocol::Result<Vec<u8>> {
        if self.is_expired_at(SystemTime::now()) {
            self.revoke();
        }
        if self.closed.load(Ordering::Acquire) != 0 {
            return Err(session_closed());
        }
        let _permit = InFlightPermit::acquire(&self.in_flight, self.max_in_flight)?;
        let response = self.wire.handle_frame(frame);
        if self.wire.is_closed().unwrap_or(true) {
            self.mark_closed();
        }
        response
    }

    pub fn close(&self) {
        self.revoke();
    }

    fn revoke(&self) {
        let _ = self.session.revoke();
        self.mark_closed();
        let _ = self.wire.close();
    }

    fn mark_closed(&self) {
        self.closed.store(1, Ordering::Release);
        if self.lease_released.swap(1, Ordering::AcqRel) == 0
            && let Some(server) = self.server.upgrade()
        {
            server.release_connection(self.id);
        }
    }

    fn belongs_to(&self, state: &Arc<ServerState>) -> bool {
        self.server
            .upgrade()
            .is_some_and(|server| Arc::ptr_eq(&server, state))
    }

    fn is_expired_at(&self, now: SystemTime) -> bool {
        self.expires_at().is_some_and(|deadline| deadline <= now)
    }
}

impl Drop for HostedConnection {
    fn drop(&mut self) {
        self.close();
    }
}

struct InFlightPermit<'a> {
    counter: &'a AtomicUsize,
}

impl<'a> InFlightPermit<'a> {
    fn acquire(counter: &'a AtomicUsize, limit: usize) -> cfmd_protocol::Result<Self> {
        if limit == 0 {
            return Err(in_flight_limit());
        }
        if counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < limit).then_some(current + 1)
            })
            .is_err()
        {
            return Err(in_flight_limit());
        }
        Ok(Self { counter })
    }
}

impl Drop for InFlightPermit<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

fn access_denied() -> HostError {
    HostError::new(HostErrorCode::AccessDenied, "connection access denied")
}

fn provider_unavailable() -> HostError {
    HostError::new(
        HostErrorCode::ProviderUnavailable,
        "connection security provider unavailable",
    )
}

fn connection_limit() -> HostError {
    HostError::new(
        HostErrorCode::ConnectionLimit,
        "hosted connection limit reached",
    )
}

fn host_draining() -> HostError {
    HostError::new(HostErrorCode::Draining, "hosted server is draining")
}

fn host_closed() -> HostError {
    HostError::new(HostErrorCode::Closed, "hosted server is closed")
}

fn host_internal() -> HostError {
    HostError::new(HostErrorCode::Internal, "internal hosted server error")
}

fn in_flight_limit() -> ProtocolError {
    ProtocolError::new(
        ProtocolErrorCode::ResourceLimit,
        "connection in-flight request limit reached",
    )
}

fn session_closed() -> ProtocolError {
    ProtocolError::new(
        ProtocolErrorCode::SessionClosed,
        "hosted connection is closed",
    )
}

/// Product-level hosting composition sugar. The method lives in `cfmd-host`
/// rather than `cfmd-runtime`, so opening an embedded database never acquires
/// a hosting/networking dependency or trust policy.
pub trait DatabaseHostingExt {
    #[must_use]
    fn host<Evidence, A, Z>(self, authenticator: A, authorizer: Z) -> HostedServer<Evidence, A, Z>
    where
        A: Authenticator<Evidence>,
        Z: Authorizer;

    #[must_use]
    fn host_with_limits<Evidence, A, Z>(
        &self,
        authenticator: A,
        authorizer: Z,
        limits: ServerLimits,
    ) -> HostedServer<Evidence, A, Z>
    where
        A: Authenticator<Evidence>,
        Z: Authorizer;
}

impl DatabaseHostingExt for Database {
    fn host<Evidence, A, Z>(self, authenticator: A, authorizer: Z) -> HostedServer<Evidence, A, Z>
    where
        A: Authenticator<Evidence>,
        Z: Authorizer,
    {
        HostedServer::new(self.clone(), authenticator, authorizer)
    }

    fn host_with_limits<Evidence, A, Z>(
        &self,
        authenticator: A,
        authorizer: Z,
        limits: ServerLimits,
    ) -> HostedServer<Evidence, A, Z>
    where
        A: Authenticator<Evidence>,
        Z: Authorizer,
    {
        HostedServer::with_limits(self.clone(), authenticator, authorizer, limits)
    }
}
