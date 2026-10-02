use std::{
    error::Error as StdError,
    fmt, fs,
    sync::Arc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use cfmd_host::{
    AuthenticationContext, AuthenticationDecision, Authenticator, AuthorizationDecision,
    AuthorizationGrant, Authorizer, ChannelBinding, DatabaseHostingExt, HostErrorCode,
    HostedServer, ServerLimits,
};
use cfmd_protocol::{
    CommitRequest, HostedRequest, OpenWatchRequest, ProtocolErrorCode, ProtocolQuery,
    ProtocolValue, RelationMutation,
    wire::{
        ProtocolHello, WireLimits, WireResponse, decode_hello_ack_frame, decode_response_frame,
        encode_hello_frame, encode_request_frame,
    },
};
use cfmd_runtime::{
    Database, EquivalenceId, Permission, PermissionSet, PrimitiveEquivalence, PrincipalId,
    RelationId, RelationSchema, Schema, Type,
};

#[derive(Debug)]
struct ProviderError;

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("provider failure")
    }
}

impl StdError for ProviderError {}

struct TokenAuthenticator;

impl Authenticator<&'static str> for TokenAuthenticator {
    type Error = ProviderError;

    fn authenticate(
        &self,
        context: AuthenticationContext<'_, &'static str>,
    ) -> std::result::Result<AuthenticationDecision, Self::Error> {
        Ok(if *context.evidence() == "secret" {
            AuthenticationDecision::Authenticated(PrincipalId::new(41))
        } else {
            AuthenticationDecision::Reject
        })
    }
}

#[derive(Clone)]
struct FixedAuthorizer {
    permissions: PermissionSet,
}

impl Authorizer for FixedAuthorizer {
    type Error = ProviderError;

    fn authorize(
        &self,
        principal: PrincipalId,
    ) -> std::result::Result<AuthorizationDecision, Self::Error> {
        Ok(if principal == PrincipalId::new(41) {
            AuthorizationDecision::Grant(AuthorizationGrant::new(self.permissions.clone()))
        } else {
            AuthorizationDecision::Reject
        })
    }
}

fn fixture() -> (std::path::PathBuf, Database, RelationId) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("cfmd-host-{nonce}-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("fixture directory");
    let relation = RelationId::new(400);
    let equivalence = EquivalenceId::new(401);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("schema");
    let database = Database::create(&directory, schema).expect("database");
    (directory, database, relation)
}

fn permissions() -> PermissionSet {
    PermissionSet::from([
        Permission::Read,
        Permission::HistoricalRead,
        Permission::HistoryRead,
        Permission::Watch,
        Permission::Write,
    ])
}

fn server(
    database: &Database,
    permissions: PermissionSet,
    limits: ServerLimits,
) -> HostedServer<&'static str, TokenAuthenticator, FixedAuthorizer> {
    database.host_with_limits(TokenAuthenticator, FixedAuthorizer { permissions }, limits)
}

fn negotiate(connection: &cfmd_host::HostedConnection, limits: WireLimits) {
    let hello = encode_hello_frame(1, ProtocolHello::current(), limits).expect("hello");
    let response = connection.handle_frame(&hello).expect("hello response");
    let (_, ack) = decode_hello_ack_frame(&response, limits).expect("hello ack");
    assert_eq!(ack.protocol_version, 2);
}

#[test]
fn authentication_and_authorization_are_host_owned() {
    let (directory, database, relation) = fixture();
    let server = server(
        &database,
        PermissionSet::from([Permission::Read]),
        ServerLimits::default(),
    );

    let denied = server
        .connect(&"wrong")
        .err()
        .expect("invalid evidence denied");
    assert_eq!(denied.code(), HostErrorCode::AccessDenied);
    assert_eq!(server.active_connections(), 0);

    let connection = server.connect(&"secret").expect("authenticated connection");
    assert_eq!(connection.principal(), PrincipalId::new(41));
    assert_eq!(
        connection.permissions().expect("permissions"),
        PermissionSet::from([Permission::Read])
    );
    let wire_limits = server.limits().wire;
    negotiate(&connection, wire_limits);

    let commit = HostedRequest::Commit(CommitRequest {
        base_revision: 1,
        transaction: 900,
        mutations: vec![RelationMutation {
            relation: relation.raw(),
            inserted: vec![vec![ProtocolValue::I64(1)]],
            removed: vec![],
        }],
    });
    let request = encode_request_frame(7, &commit, wire_limits).expect("commit frame");
    let response = connection.handle_frame(&request).expect("wire response");
    let (_, response) = decode_response_frame(&response, wire_limits).expect("decode response");
    let WireResponse::Error { code, .. } = response else {
        panic!("read-only principal must not commit")
    };
    assert_eq!(code, ProtocolErrorCode::PermissionDenied);

    connection.close();
    assert_eq!(server.active_connections(), 0);
    drop(connection);
    drop(server);
    drop(directory);
}

#[test]
fn connection_admission_is_bounded_and_close_releases_capacity() {
    let (directory, database, _) = fixture();
    let limits = ServerLimits {
        max_connections: 1,
        ..ServerLimits::default()
    };
    let server = server(&database, permissions(), limits);
    let first = server.connect(&"secret").expect("first connection");
    assert_eq!(server.active_connections(), 1);

    let denied = server.connect(&"secret").err().expect("connection limit");
    assert_eq!(denied.code(), HostErrorCode::ConnectionLimit);
    first.close();
    assert_eq!(server.active_connections(), 0);

    let replacement = server.connect(&"secret").expect("replacement connection");
    assert_eq!(server.active_connections(), 1);
    server.close();
    assert!(server.is_closed());
    assert!(replacement.is_closed());
    assert_eq!(server.active_connections(), 0);
    let denied = server
        .connect(&"secret")
        .err()
        .expect("closed host rejects");
    assert_eq!(denied.code(), HostErrorCode::Closed);

    drop(replacement);
    drop(directory);
}

#[test]
fn disconnect_cancels_blocked_watch_and_in_flight_work_is_bounded() {
    let (directory, database, relation) = fixture();
    let limits = ServerLimits {
        max_in_flight_requests_per_connection: 1,
        ..ServerLimits::default()
    };
    let server = server(&database, permissions(), limits);
    let connection = server.connect(&"secret").expect("connection");
    let wire_limits = server.limits().wire;
    negotiate(&connection, wire_limits);

    let open = HostedRequest::OpenWatch(OpenWatchRequest {
        query: ProtocolQuery::Scan {
            relation: relation.raw(),
        },
    });
    let frame = encode_request_frame(2, &open, wire_limits).expect("open watch frame");
    let response = connection
        .handle_frame(&frame)
        .expect("open watch response");
    let (_, response) = decode_response_frame(&response, wire_limits).expect("decode watch open");
    let WireResponse::Success(cfmd_protocol::HostedResponse::WatchOpened(opened)) = response else {
        panic!("watch opened response")
    };

    let next = HostedRequest::NextWatch {
        subscription: opened.subscription,
    };
    let next_frame = encode_request_frame(3, &next, wire_limits).expect("next watch frame");
    let blocked_connection = Arc::clone(&connection);
    let waiter = thread::spawn(move || blocked_connection.handle_frame(&next_frame));

    for _ in 0..10_000 {
        if connection.in_flight_requests() == 1 {
            break;
        }
        thread::yield_now();
    }
    assert_eq!(connection.in_flight_requests(), 1);

    let status = HostedRequest::WatchStatus {
        subscription: opened.subscription,
    };
    let status_frame = encode_request_frame(4, &status, wire_limits).expect("status frame");
    let error = connection
        .handle_frame(&status_frame)
        .expect_err("second in-flight request must be bounded");
    assert_eq!(error.code(), ProtocolErrorCode::ResourceLimit);

    connection.close();
    let response = waiter
        .join()
        .expect("waiter thread")
        .expect("watch error frame");
    let (_, response) =
        decode_response_frame(&response, wire_limits).expect("decode close response");
    let WireResponse::Error { code, .. } = response else {
        panic!("blocked watch must end with error")
    };
    assert!(matches!(
        code,
        ProtocolErrorCode::WatchClosed | ProtocolErrorCode::SessionClosed
    ));
    assert_eq!(server.active_connections(), 0);

    thread::sleep(Duration::from_millis(1));
    drop(connection);
    drop(server);
    drop(directory);
}

#[test]
fn transport_metadata_never_grants_identity_or_permissions() {
    let (directory, database, _) = fixture();
    let server = server(&database, permissions(), ServerLimits::default());

    // A transport may decide what evidence type it can supply, but only the
    // configured authenticator maps that evidence to a PrincipalId and only
    // the authorizer maps the principal to permissions.
    let denied = server
        .connect(&"127.0.0.1")
        .err()
        .expect("localhost is not trusted");
    assert_eq!(denied.code(), HostErrorCode::AccessDenied);
    assert_eq!(server.active_connections(), 0);

    drop(server);
    drop(directory);
}

#[derive(Clone)]
struct MutableAuthorizer {
    grant: Arc<std::sync::Mutex<AuthorizationGrant>>,
}

impl Authorizer for MutableAuthorizer {
    type Error = ProviderError;

    fn authorize(
        &self,
        principal: PrincipalId,
    ) -> std::result::Result<AuthorizationDecision, Self::Error> {
        if principal != PrincipalId::new(41) {
            return Ok(AuthorizationDecision::Reject);
        }
        Ok(AuthorizationDecision::Grant(
            self.grant.lock().expect("grant lock").clone(),
        ))
    }
}

struct BoundTokenAuthenticator;

impl Authenticator<&'static str> for BoundTokenAuthenticator {
    type Error = ProviderError;

    fn authenticate(
        &self,
        context: AuthenticationContext<'_, &'static str>,
    ) -> std::result::Result<AuthenticationDecision, Self::Error> {
        let expected = ChannelBinding::bound([7; 32]);
        Ok(
            if *context.evidence() == "secret" && context.channel_binding() == expected {
                AuthenticationDecision::Authenticated(PrincipalId::new(41))
            } else {
                AuthenticationDecision::Reject
            },
        )
    }
}

#[test]
fn channel_binding_is_explicit_and_authenticator_owned() {
    let (directory, database, _) = fixture();
    let server = database.host(
        BoundTokenAuthenticator,
        FixedAuthorizer {
            permissions: permissions(),
        },
    );

    let denied = server
        .connect(&"secret")
        .err()
        .expect("unbound evidence rejected");
    assert_eq!(denied.code(), HostErrorCode::AccessDenied);

    let connection = server
        .connect_bound(&"secret", [7; 32])
        .expect("bound authentication");
    assert_eq!(connection.channel_binding(), ChannelBinding::bound([7; 32]));
    connection.close();
    drop(connection);
    drop(server);
    drop(directory);
}

#[test]
fn authorization_refresh_revokes_watch_when_exact_read_footprint_disappears() {
    let (directory, database, relation) = fixture();
    let grant = Arc::new(std::sync::Mutex::new(
        AuthorizationGrant::new(permissions()),
    ));
    let authorizer = MutableAuthorizer {
        grant: Arc::clone(&grant),
    };
    let server = HostedServer::new(database, TokenAuthenticator, authorizer);
    let connection = server.connect(&"secret").expect("connection");
    let wire_limits = server.limits().wire;
    negotiate(&connection, wire_limits);

    let open = HostedRequest::OpenWatch(OpenWatchRequest {
        query: ProtocolQuery::Scan {
            relation: relation.raw(),
        },
    });
    let frame = encode_request_frame(50, &open, wire_limits).expect("open watch frame");
    let response = connection
        .handle_frame(&frame)
        .expect("open watch response");
    let (_, response) = decode_response_frame(&response, wire_limits).expect("decode open");
    let WireResponse::Success(cfmd_protocol::HostedResponse::WatchOpened(opened)) = response else {
        panic!("watch opened response")
    };

    let next = HostedRequest::NextWatch {
        subscription: opened.subscription,
    };
    let next_frame = encode_request_frame(51, &next, wire_limits).expect("next frame");
    let blocked = Arc::clone(&connection);
    let waiter = thread::spawn(move || blocked.handle_frame(&next_frame));
    for _ in 0..10_000 {
        if connection.in_flight_requests() == 1 {
            break;
        }
        thread::yield_now();
    }
    assert_eq!(connection.in_flight_requests(), 1);

    *grant.lock().expect("grant lock") = AuthorizationGrant::new(PermissionSet::from([
        Permission::Watch,
        Permission::HistoryRead,
    ]));
    server
        .refresh_authorization(&connection)
        .expect("refresh authorization");
    assert!(connection.permissions().expect("permissions").contains(Permission::Watch));
    assert!(!connection.is_closed());

    let response = waiter
        .join()
        .expect("waiter")
        .expect("watch close response");
    let (_, response) = decode_response_frame(&response, wire_limits).expect("decode watch close");
    let WireResponse::Error { code, .. } = response else {
        panic!("watch must terminate after exact read-footprint revocation")
    };
    assert_eq!(code, ProtocolErrorCode::PermissionDenied);

    connection.close();
    drop(connection);
    drop(server);
    drop(directory);
}

#[test]
fn expiring_grant_can_be_scheduled_without_polling_and_wakes_waiters() {
    let (directory, database, relation) = fixture();
    let deadline = SystemTime::now() + Duration::from_secs(3_600);
    let grant = Arc::new(std::sync::Mutex::new(AuthorizationGrant::expiring(
        permissions(),
        deadline,
    )));
    let server = HostedServer::new(database, TokenAuthenticator, MutableAuthorizer { grant });
    let connection = server.connect(&"secret").expect("connection");
    assert_eq!(server.next_expiration(), Some(deadline));
    let wire_limits = server.limits().wire;
    negotiate(&connection, wire_limits);

    let open = HostedRequest::OpenWatch(OpenWatchRequest {
        query: ProtocolQuery::Scan {
            relation: relation.raw(),
        },
    });
    let frame = encode_request_frame(60, &open, wire_limits).expect("open frame");
    let response = connection.handle_frame(&frame).expect("open response");
    let (_, response) = decode_response_frame(&response, wire_limits).expect("decode open");
    let WireResponse::Success(cfmd_protocol::HostedResponse::WatchOpened(opened)) = response else {
        panic!("watch opened response")
    };
    let next = HostedRequest::NextWatch {
        subscription: opened.subscription,
    };
    let next_frame = encode_request_frame(61, &next, wire_limits).expect("next frame");
    let blocked = Arc::clone(&connection);
    let waiter = thread::spawn(move || blocked.handle_frame(&next_frame));
    for _ in 0..10_000 {
        if connection.in_flight_requests() == 1 {
            break;
        }
        thread::yield_now();
    }
    assert_eq!(connection.in_flight_requests(), 1);

    assert_eq!(server.expire_due(deadline), 1);
    assert!(connection.is_closed());
    let response = waiter
        .join()
        .expect("waiter")
        .expect("expiry response frame");
    let (_, response) = decode_response_frame(&response, wire_limits).expect("decode expiry");
    let WireResponse::Error { code, .. } = response else {
        panic!("expired watch must end with error")
    };
    assert!(matches!(
        code,
        ProtocolErrorCode::WatchClosed | ProtocolErrorCode::SessionClosed
    ));
    assert_eq!(server.active_connections(), 0);

    drop(connection);
    drop(server);
    drop(directory);
}

#[test]
fn graceful_drain_rejects_new_connections_but_preserves_existing_work() {
    let (directory, database, _) = fixture();
    let server = server(&database, permissions(), ServerLimits::default());
    let connection = server.connect(&"secret").expect("connection");
    let wire_limits = server.limits().wire;
    negotiate(&connection, wire_limits);

    server.drain();
    assert!(server.is_draining());
    assert!(!server.is_drained());
    let denied = server
        .connect(&"secret")
        .err()
        .expect("draining host rejects new connection");
    assert_eq!(denied.code(), HostErrorCode::Draining);

    let request = encode_request_frame(70, &HostedRequest::CurrentRevision, wire_limits)
        .expect("current revision frame");
    let response = connection
        .handle_frame(&request)
        .expect("existing connection remains usable");
    let (_, response) = decode_response_frame(&response, wire_limits).expect("decode response");
    assert!(matches!(
        response,
        WireResponse::Success(cfmd_protocol::HostedResponse::CurrentRevision { .. })
    ));

    connection.close();
    assert!(server.is_drained());
    server.close();
    assert!(server.is_closed());

    drop(connection);
    drop(server);
    drop(directory);
}
