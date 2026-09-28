#![cfg(unix)]

use std::{
    error::Error as StdError,
    fmt, fs,
    io::Write,
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::PathBuf,
    sync::Arc,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use cfmd_host::{
    AuthenticationContext, AuthenticationDecision, Authenticator, AuthorizationDecision,
    AuthorizationGrant, Authorizer, HostedServer, ServerLimits,
};
use cfmd_protocol::{
    HostedRequest, HostedResponse, OpenWatchRequest, ProtocolQuery,
    wire::{
        ProtocolHello, WireResponse, decode_hello_ack_frame, decode_response_frame,
        encode_hello_frame, encode_request_frame,
    },
};
use cfmd_runtime::{
    Database, EquivalenceId, Permission, PermissionSet, PrimitiveEquivalence, PrincipalId,
    RelationId, RelationSchema, Schema, Type,
};
use cfmd_transport_local::{
    LOCAL_AUTH_HEADER_LEN, LOCAL_AUTH_MAGIC, LOCAL_AUTH_VERSION, LocalIpcErrorCode, LocalIpcLimits,
    unix::{UnixLocalIpcClient, UnixLocalIpcServer},
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

impl Authenticator<Vec<u8>> for TokenAuthenticator {
    type Error = ProviderError;

    fn authenticate(
        &self,
        context: AuthenticationContext<'_, Vec<u8>>,
    ) -> std::result::Result<AuthenticationDecision, Self::Error> {
        Ok(if context.evidence().as_slice() == b"secret" {
            AuthenticationDecision::Authenticated(PrincipalId::new(77))
        } else {
            AuthenticationDecision::Reject
        })
    }
}

#[derive(Clone)]
struct FixedAuthorizer;

impl Authorizer for FixedAuthorizer {
    type Error = ProviderError;

    fn authorize(
        &self,
        principal: PrincipalId,
    ) -> std::result::Result<AuthorizationDecision, Self::Error> {
        Ok(if principal == PrincipalId::new(77) {
            AuthorizationDecision::Grant(AuthorizationGrant::new(PermissionSet::from([
                Permission::Read,
                Permission::HistoricalRead,
                Permission::HistoryRead,
                Permission::Watch,
                Permission::Write,
            ])))
        } else {
            AuthorizationDecision::Reject
        })
    }
}

struct Fixture {
    root: PathBuf,
    socket: PathBuf,
    database: Database,
    relation: RelationId,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "cfmd-local-ipc-{name}-{nonce}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("fixture root");
        let database_dir = root.join("db");
        fs::create_dir_all(&database_dir).expect("database directory");
        let socket = root.join("cfmd.sock");
        let relation = RelationId::new(501);
        let equivalence = EquivalenceId::new(502);
        let schema = Schema::builder()
            .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
            .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
            .build()
            .expect("schema");
        let database = Database::create(&database_dir, schema).expect("database");
        Self {
            root,
            socket,
            database,
            relation,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn host(
    database: Database,
    max_in_flight: usize,
) -> HostedServer<Vec<u8>, TokenAuthenticator, FixedAuthorizer> {
    HostedServer::with_limits(
        database,
        TokenAuthenticator,
        FixedAuthorizer,
        ServerLimits {
            max_in_flight_requests_per_connection: max_in_flight,
            ..ServerLimits::default()
        },
    )
}

fn negotiate(client: &mut UnixLocalIpcClient, limits: cfmd_protocol::wire::WireLimits) {
    let hello = encode_hello_frame(1, ProtocolHello::current(), limits).expect("hello");
    let response = client.exchange(&hello).expect("hello exchange");
    let (request_id, ack) = decode_hello_ack_frame(&response, limits).expect("hello ack");
    assert_eq!(request_id, 1);
    assert_eq!(ack.protocol_version, 2);
}

#[test]
fn unix_socket_is_private_and_roundtrips_hosted_wire() {
    let fixture = Fixture::new("roundtrip");
    let host = host(fixture.database.clone(), 4);
    let wire_limits = host.limits().wire;
    let local_limits = LocalIpcLimits::default();
    let server = Arc::new(
        UnixLocalIpcServer::bind(&fixture.socket, host, local_limits).expect("bind local IPC"),
    );
    let mode = fs::metadata(&fixture.socket)
        .expect("socket metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);

    let accepting = Arc::clone(&server);
    let worker = thread::spawn(move || accepting.accept_one());
    let mut client =
        UnixLocalIpcClient::connect(&fixture.socket, b"secret", local_limits, wire_limits)
            .expect("client connect");
    negotiate(&mut client, wire_limits);

    let frame = encode_request_frame(2, &HostedRequest::CurrentRevision, wire_limits)
        .expect("current revision request");
    let response = client.exchange(&frame).expect("current revision response");
    let (request_id, response) = decode_response_frame(&response, wire_limits).expect("decode");
    assert_eq!(request_id, 2);
    let WireResponse::Success(HostedResponse::CurrentRevision { revision }) = response else {
        panic!("current revision response expected")
    };
    assert_eq!(
        revision,
        fixture
            .database
            .snapshot()
            .expect("snapshot")
            .revision()
            .raw()
    );

    client.shutdown().expect("client shutdown");
    drop(client);
    worker.join().expect("server join").expect("server result");
}

#[test]
fn local_endpoint_never_turns_locality_into_authentication() {
    let fixture = Fixture::new("auth");
    let host = host(fixture.database.clone(), 2);
    let wire_limits = host.limits().wire;
    let local_limits = LocalIpcLimits::default();
    let server = Arc::new(
        UnixLocalIpcServer::bind(&fixture.socket, host, local_limits).expect("bind local IPC"),
    );
    let accepting = Arc::clone(&server);
    let worker = thread::spawn(move || accepting.accept_one());
    let mut client =
        UnixLocalIpcClient::connect(&fixture.socket, b"127.0.0.1", local_limits, wire_limits)
            .expect("transport connect");
    let hello = encode_hello_frame(1, ProtocolHello::current(), wire_limits).expect("hello");
    let denied = client
        .exchange(&hello)
        .expect_err("unauthenticated local peer denied");
    assert!(matches!(
        denied.code(),
        LocalIpcErrorCode::Io | LocalIpcErrorCode::InvalidAuthenticationPrelude
    ));
    let server_error = worker
        .join()
        .expect("server join")
        .expect_err("server rejects evidence");
    assert_eq!(server_error.code(), LocalIpcErrorCode::AccessDenied);
}

#[test]
fn socket_disconnect_cancels_a_blocked_watch_without_polling() {
    let fixture = Fixture::new("disconnect");
    let host = host(fixture.database.clone(), 4);
    let wire_limits = host.limits().wire;
    let local_limits = LocalIpcLimits {
        max_workers_per_connection: 4,
        ..LocalIpcLimits::default()
    };
    let server = Arc::new(
        UnixLocalIpcServer::bind(&fixture.socket, host, local_limits).expect("bind local IPC"),
    );
    let accepting = Arc::clone(&server);
    let worker = thread::spawn(move || accepting.accept_one());
    let mut client =
        UnixLocalIpcClient::connect(&fixture.socket, b"secret", local_limits, wire_limits)
            .expect("client connect");
    negotiate(&mut client, wire_limits);

    let open = HostedRequest::OpenWatch(OpenWatchRequest {
        query: ProtocolQuery::Scan {
            relation: fixture.relation.raw(),
        },
    });
    let response = client
        .exchange(&encode_request_frame(2, &open, wire_limits).expect("open frame"))
        .expect("open response");
    let (_, response) = decode_response_frame(&response, wire_limits).expect("decode open");
    let WireResponse::Success(HostedResponse::WatchOpened(opened)) = response else {
        panic!("watch opened response expected")
    };

    let next = HostedRequest::NextWatch {
        subscription: opened.subscription,
    };
    client
        .send_frame(&encode_request_frame(3, &next, wire_limits).expect("next frame"))
        .expect("send blocking next");
    let status = HostedRequest::WatchStatus {
        subscription: opened.subscription,
    };
    client
        .send_frame(&encode_request_frame(4, &status, wire_limits).expect("status frame"))
        .expect("send status");
    let status_response = client.receive_frame().expect("status response");
    let (request_id, _) =
        decode_response_frame(&status_response, wire_limits).expect("decode status");
    assert_eq!(request_id, 4, "request 3 must still be blocked");

    client.shutdown().expect("disconnect");
    drop(client);
    worker
        .join()
        .expect("server join")
        .expect("disconnect must cancel blocked watch and finish serving");
}

#[test]
fn oversized_authentication_evidence_is_rejected_from_the_fixed_header() {
    let fixture = Fixture::new("oversized-auth");
    let host = host(fixture.database.clone(), 2);
    let local_limits = LocalIpcLimits {
        max_auth_evidence_bytes: 8,
        ..LocalIpcLimits::default()
    };
    let server = Arc::new(
        UnixLocalIpcServer::bind(&fixture.socket, host, local_limits).expect("bind local IPC"),
    );
    let accepting = Arc::clone(&server);
    let worker = thread::spawn(move || accepting.accept_one());

    let mut stream = UnixStream::connect(&fixture.socket).expect("connect raw local IPC");
    let mut header = [0_u8; LOCAL_AUTH_HEADER_LEN];
    header[..4].copy_from_slice(&LOCAL_AUTH_MAGIC);
    header[4..6].copy_from_slice(&LOCAL_AUTH_VERSION.to_be_bytes());
    header[8..12].copy_from_slice(&9_u32.to_be_bytes());
    stream.write_all(&header).expect("write auth header");
    drop(stream);

    let error = worker
        .join()
        .expect("server join")
        .expect_err("oversized evidence must fail before body read");
    assert_eq!(
        error.code(),
        LocalIpcErrorCode::AuthenticationEvidenceTooLarge
    );
}
