use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use cfmd_protocol::{
    HostedRequest, HostedResponse, HostedSession, ProtocolErrorCode, ProtocolQuery, QueryRequest,
    SnapshotTarget,
    wire::{
        CAP_QUERY, ProtocolHello, WIRE_HEADER_LEN, WireFrameKind, WireHostedSession, WireLimits,
        WireResponse, decode_header, decode_hello_ack_frame, decode_request_payload,
        decode_response_frame, encode_error_response_frame, encode_hello_frame,
        encode_request_frame, negotiate,
    },
};
use cfmd_runtime::{
    Database, EquivalenceId, Permission, PermissionSet, PrimitiveEquivalence, PrincipalId,
    RelationId, RelationSchema, Schema, Session, Type,
};

fn fixture() -> (std::path::PathBuf, Database, RelationId) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("cfmd-wire-{nonce}-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("fixture directory");
    let relation = RelationId::new(200);
    let equivalence = EquivalenceId::new(201);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("schema");
    let database = Database::create(&directory, schema).expect("database");
    (directory, database, relation)
}

#[test]
fn formation_proof_error_codes_roundtrip_without_collapsing_to_internal() {
    let limits = WireLimits::default();
    for code in [
        ProtocolErrorCode::FormationProofUnavailable,
        ProtocolErrorCode::FormationProofInvalidated,
        ProtocolErrorCode::ContractNotRepresentable,
    ] {
        let frame = encode_error_response_frame(91, code, "formation proof", limits)
            .expect("encode error response");
        let (request_id, response) =
            decode_response_frame(&frame, limits).expect("decode error response");
        assert_eq!(request_id, 91);
        assert_eq!(
            response,
            WireResponse::Error {
                code,
                message: "formation proof".to_owned(),
            }
        );
    }
}

fn wire_session(database: &Database) -> WireHostedSession {
    let permissions = PermissionSet::from([
        Permission::Read,
        Permission::HistoricalRead,
        Permission::HistoryRead,
        Permission::Watch,
        Permission::Write,
    ]);
    let hosted =
        HostedSession::new(database.session(Session::new(PrincipalId::new(9), permissions)));
    WireHostedSession::new(hosted)
}

#[test]
fn canonical_request_roundtrip_and_negotiation() {
    let limits = WireLimits::default();
    let request = HostedRequest::Query(QueryRequest {
        target: SnapshotTarget::Revision(42),
        query: ProtocolQuery::Project {
            input: Box::new(ProtocolQuery::Scan { relation: 7 }),
            columns: vec![0, 2],
        },
    });
    let frame = encode_request_frame(11, &request, limits).expect("encode request");
    let header = decode_header(&frame[..WIRE_HEADER_LEN], limits).expect("header");
    assert_eq!(header.kind, WireFrameKind::Request);
    assert_eq!(header.request_id, 11);
    let payload = &frame[WIRE_HEADER_LEN..];
    assert_eq!(
        decode_request_payload(payload, limits).expect("decode"),
        request
    );
    assert_eq!(
        frame,
        encode_request_frame(11, &request, limits).expect("canonical encode")
    );

    let contract_request = HostedRequest::Query(QueryRequest {
        target: SnapshotTarget::ContractHead {
            schema_revision: 539,
        },
        query: ProtocolQuery::Scan { relation: 7 },
    });
    let contract_frame =
        encode_request_frame(12, &contract_request, limits).expect("contract request");
    assert_eq!(
        decode_request_payload(&contract_frame[WIRE_HEADER_LEN..], limits)
            .expect("decode contract request"),
        contract_request
    );

    let ack = negotiate(ProtocolHello {
        min_protocol_version: 1,
        max_protocol_version: 2,
        capabilities: CAP_QUERY,
    })
    .expect("negotiation");
    assert_eq!(ack.protocol_version, 2);
    assert_eq!(ack.capabilities, CAP_QUERY);
}

#[test]
fn header_rejects_oversized_payload_before_payload_allocation() {
    let limits = WireLimits {
        max_payload_bytes: 8,
        ..WireLimits::default()
    };
    let mut header = [0_u8; WIRE_HEADER_LEN];
    header[..4].copy_from_slice(b"CFMD");
    header[4..6].copy_from_slice(&1_u16.to_be_bytes());
    header[6] = WireFrameKind::Request as u8;
    header[16..20].copy_from_slice(&9_u32.to_be_bytes());
    let error = decode_header(&header, limits).expect_err("oversized header must fail");
    assert_eq!(error.code(), ProtocolErrorCode::ResourceLimit);
}

#[test]
fn wire_session_requires_handshake_and_preserves_request_id() {
    let (directory, database, relation) = fixture();
    let wire = wire_session(&database);
    let limits = wire.limits();

    let request = HostedRequest::Query(QueryRequest {
        target: SnapshotTarget::Head,
        query: ProtocolQuery::Scan {
            relation: relation.raw(),
        },
    });
    let request_frame = encode_request_frame(77, &request, limits).expect("request frame");
    let denied = wire
        .handle_frame(&request_frame)
        .expect_err("handshake required");
    assert_eq!(denied.code(), ProtocolErrorCode::InvalidRequest);

    let hello = encode_hello_frame(1, ProtocolHello::current(), limits).expect("hello");
    let hello_ack = wire.handle_frame(&hello).expect("hello ack");
    let (hello_id, ack) = decode_hello_ack_frame(&hello_ack, limits).expect("decode hello ack");
    assert_eq!(hello_id, 1);
    assert_eq!(ack.protocol_version, 2);

    let response_frame = wire.handle_frame(&request_frame).expect("query response");
    let (request_id, response) =
        decode_response_frame(&response_frame, limits).expect("decode response");
    assert_eq!(request_id, 77);
    let WireResponse::Success(HostedResponse::Query(response)) = response else {
        panic!("expected query response")
    };
    assert_eq!(response.revision, 1);
    assert!(response.rows.is_empty());

    drop(wire);
    drop(database);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn decoder_rejects_unknown_variant_and_trailing_bytes() {
    let limits = WireLimits::default();
    let unknown = decode_request_payload(&[255], limits).expect_err("unknown tag");
    assert_eq!(unknown.code(), ProtocolErrorCode::InvalidRequest);

    let trailing = decode_request_payload(&[0, 0], limits).expect_err("trailing bytes");
    assert_eq!(trailing.code(), ProtocolErrorCode::InvalidRequest);
}
