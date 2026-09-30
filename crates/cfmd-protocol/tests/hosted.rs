use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use cfmd_protocol::{
    CommitRequest, CommitResponse, HostedRequest, HostedResponse, HostedSession, OpenWatchRequest,
    ProtocolErrorCode, ProtocolLimits, ProtocolQuery, ProtocolValue, QueryRequest,
    RelationMutation, SnapshotTarget, WatchStatusDto,
};
use cfmd_runtime::{
    Database, EquivalenceId, Permission, PermissionSet, PrimitiveEquivalence, PrincipalId,
    RelationId, RelationSchema, Schema, Session, Type,
};

fn temp_directory() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("cfmd-protocol-{nonce}-{}", std::process::id()))
}

fn fixture() -> (std::path::PathBuf, Database, RelationId, EquivalenceId) {
    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("fixture directory");
    let relation = RelationId::new(100);
    let equivalence = EquivalenceId::new(101);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("schema");
    let database = Database::create(&directory, schema).expect("database");
    (directory, database, relation, equivalence)
}

fn full_session(database: &Database) -> HostedSession {
    let permissions = PermissionSet::from([
        Permission::Read,
        Permission::HistoricalRead,
        Permission::HistoryRead,
        Permission::Watch,
        Permission::Write,
    ]);
    HostedSession::new(database.session(Session::new(PrincipalId::new(7), permissions)))
}

fn commit_i64(
    hosted: &HostedSession,
    relation: RelationId,
    base_revision: u64,
    transaction: u128,
    value: i64,
) {
    hosted
        .execute(HostedRequest::Commit(CommitRequest {
            base_revision,
            transaction,
            mutations: vec![RelationMutation {
                relation: relation.raw(),
                inserted: vec![vec![ProtocolValue::I64(value)]],
                removed: vec![],
            }],
        }))
        .expect("commit");
}

#[test]
fn protocol_dispatches_through_restricted_product_authority() {
    let (directory, database, relation, equivalence) = fixture();
    let hosted = full_session(&database);

    assert_eq!(HostedSession::protocol_version(), 2);
    assert_eq!(hosted.principal(), PrincipalId::new(7));
    assert_eq!(
        hosted
            .execute(HostedRequest::CurrentRevision)
            .expect("head"),
        HostedResponse::CurrentRevision { revision: 1 }
    );

    let committed = hosted
        .execute(HostedRequest::Commit(CommitRequest {
            base_revision: 1,
            transaction: 500,
            mutations: vec![RelationMutation {
                relation: relation.raw(),
                inserted: vec![vec![ProtocolValue::I64(1)], vec![ProtocolValue::I64(2)]],
                removed: vec![],
            }],
        }))
        .expect("commit");
    assert_eq!(
        committed,
        HostedResponse::Commit(CommitResponse::Committed { revision: 2 })
    );

    let query = ProtocolQuery::Scan {
        relation: relation.raw(),
    };
    let response = hosted
        .execute(HostedRequest::Query(QueryRequest {
            target: SnapshotTarget::Head,
            query: ProtocolQuery::FilterEq {
                input: Box::new(query.clone()),
                column: 0,
                value: ProtocolValue::I64(2),
                equivalence: equivalence.raw(),
            },
        }))
        .expect("query");
    let HostedResponse::Query(response) = response else {
        panic!("query response")
    };
    assert_eq!(response.revision, 2);
    assert_eq!(response.rows, vec![vec![ProtocolValue::I64(2)]]);

    let historical = hosted
        .execute(HostedRequest::Query(QueryRequest {
            target: SnapshotTarget::Revision(1),
            query,
        }))
        .expect("historical query");
    let HostedResponse::Query(historical) = historical else {
        panic!("query response")
    };
    assert_eq!(historical.revision, 1);
    assert!(historical.rows.is_empty());

    let history = hosted
        .execute(HostedRequest::History {
            target: SnapshotTarget::Head,
        })
        .expect("history");
    let HostedResponse::History {
        anchor_revision,
        entries,
    } = history
    else {
        panic!("history response")
    };
    assert_eq!(anchor_revision, 2);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].transaction, 500);
    assert_eq!(entries[0].source_revision, 1);
    assert_eq!(entries[0].target_revision, 2);

    let stale = hosted
        .execute(HostedRequest::Commit(CommitRequest {
            base_revision: 1,
            transaction: 501,
            mutations: vec![RelationMutation {
                relation: relation.raw(),
                inserted: vec![vec![ProtocolValue::I64(3)]],
                removed: vec![],
            }],
        }))
        .expect_err("stale base must fail closed");
    assert_eq!(stale.code(), ProtocolErrorCode::StaleRevision);

    drop(hosted);
    drop(database);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn protocol_cannot_expand_session_grants() {
    let (directory, database, relation, _) = fixture();
    let hosted = HostedSession::new(database.session(Session::new(
        PrincipalId::new(8),
        PermissionSet::from([Permission::Read]),
    )));

    let denied = hosted
        .execute(HostedRequest::Commit(CommitRequest {
            base_revision: 1,
            transaction: 600,
            mutations: vec![RelationMutation {
                relation: relation.raw(),
                inserted: vec![vec![ProtocolValue::I64(1)]],
                removed: vec![],
            }],
        }))
        .expect_err("protocol must preserve Write denial");
    assert_eq!(denied.code(), ProtocolErrorCode::PermissionDenied);

    let history_denied = hosted
        .execute(HostedRequest::History {
            target: SnapshotTarget::Head,
        })
        .expect_err("protocol must preserve HistoryRead denial");
    assert_eq!(history_denied.code(), ProtocolErrorCode::PermissionDenied);

    drop(hosted);
    drop(database);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn protocol_watch_stream_is_exact_revision_tagged_and_cancellable() {
    let (directory, database, relation, equivalence) = fixture();
    let hosted = full_session(&database);
    let watch_query = ProtocolQuery::FilterEq {
        input: Box::new(ProtocolQuery::Scan {
            relation: relation.raw(),
        }),
        column: 0,
        value: ProtocolValue::I64(1),
        equivalence: equivalence.raw(),
    };

    let opened = hosted
        .execute(HostedRequest::OpenWatch(OpenWatchRequest {
            query: watch_query,
        }))
        .expect("open watch");
    let HostedResponse::WatchOpened(opened) = opened else {
        panic!("watch opened response")
    };
    assert_eq!(opened.initial.revision, 1);
    assert!(opened.initial.rows.is_empty());
    let subscription = opened.subscription;

    commit_i64(&hosted, relation, 1, 700, 2);
    commit_i64(&hosted, relation, 2, 701, 1);
    let event = hosted
        .execute(HostedRequest::NextWatch { subscription })
        .expect("matching watch transition");
    let HostedResponse::WatchEvent(event) = event else {
        panic!("watch event response")
    };
    assert_eq!(event.source_revision, 1);
    assert_eq!(event.target_revision, 3);
    assert_eq!(event.inserted, vec![vec![ProtocolValue::I64(1)]]);
    assert!(event.removed.is_empty());

    let status = hosted
        .execute(HostedRequest::WatchStatus { subscription })
        .expect("watch status");
    assert_eq!(
        status,
        HostedResponse::WatchStatus {
            subscription,
            status: WatchStatusDto::Current { revision: 3 },
        }
    );

    let barrier = Arc::new(Barrier::new(2));
    let waiter_session = hosted.clone();
    let waiter_barrier = Arc::clone(&barrier);
    let waiter = thread::spawn(move || {
        waiter_barrier.wait();
        waiter_session.execute(HostedRequest::NextWatch { subscription })
    });
    barrier.wait();
    assert_eq!(
        hosted
            .execute(HostedRequest::CancelWatch { subscription })
            .expect("cancel watch"),
        HostedResponse::WatchCancelled { subscription }
    );
    let cancelled = waiter
        .join()
        .expect("watch waiter thread")
        .expect_err("cancelled next must fail closed");
    assert_eq!(cancelled.code(), ProtocolErrorCode::WatchClosed);

    let cancelled_status = hosted
        .execute(HostedRequest::WatchStatus { subscription })
        .expect("cancelled status");
    assert_eq!(
        cancelled_status,
        HostedResponse::WatchStatus {
            subscription,
            status: WatchStatusDto::Cancelled { revision: 3 },
        }
    );
    assert_eq!(
        hosted
            .execute(HostedRequest::CloseWatch { subscription })
            .expect("close watch"),
        HostedResponse::WatchClosed { subscription }
    );
    let missing = hosted
        .execute(HostedRequest::WatchStatus { subscription })
        .expect_err("closed subscription must disappear");
    assert_eq!(missing.code(), ProtocolErrorCode::NotFound);

    drop(hosted);
    drop(database);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn protocol_watch_preserves_permissions_and_subscription_bounds() {
    let (directory, database, relation, _) = fixture();
    let read_only = HostedSession::new(database.session(Session::new(
        PrincipalId::new(9),
        PermissionSet::from([Permission::Read]),
    )));
    let denied = read_only
        .execute(HostedRequest::OpenWatch(OpenWatchRequest {
            query: ProtocolQuery::Scan {
                relation: relation.raw(),
            },
        }))
        .expect_err("Watch permission must be preserved");
    assert_eq!(denied.code(), ProtocolErrorCode::PermissionDenied);

    let limits = ProtocolLimits {
        max_watch_subscriptions: 1,
        ..ProtocolLimits::default()
    };
    let bounded = HostedSession::with_limits(
        database.session(Session::new(
            PrincipalId::new(10),
            PermissionSet::from([Permission::Read, Permission::Watch]),
        )),
        limits,
    );
    let first = bounded
        .execute(HostedRequest::OpenWatch(OpenWatchRequest {
            query: ProtocolQuery::Scan {
                relation: relation.raw(),
            },
        }))
        .expect("first watch");
    let HostedResponse::WatchOpened(first) = first else {
        panic!("watch opened response")
    };
    let resource_limit = bounded
        .execute(HostedRequest::OpenWatch(OpenWatchRequest {
            query: ProtocolQuery::Scan {
                relation: relation.raw(),
            },
        }))
        .expect_err("second watch must hit hosted limit");
    assert_eq!(resource_limit.code(), ProtocolErrorCode::ResourceLimit);
    bounded
        .execute(HostedRequest::CloseWatch {
            subscription: first.subscription,
        })
        .expect("close first watch");
    bounded
        .execute(HostedRequest::OpenWatch(OpenWatchRequest {
            query: ProtocolQuery::Scan {
                relation: relation.raw(),
            },
        }))
        .expect("slot is reusable after close");

    drop(bounded);
    drop(read_only);
    drop(database);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn protocol_session_close_cancels_blocked_watch_and_rejects_future_requests() {
    let (directory, database, relation, _) = fixture();
    let hosted = full_session(&database);
    let opened = hosted
        .execute(HostedRequest::OpenWatch(OpenWatchRequest {
            query: ProtocolQuery::Scan {
                relation: relation.raw(),
            },
        }))
        .expect("open watch");
    let HostedResponse::WatchOpened(opened) = opened else {
        panic!("watch opened response")
    };
    let subscription = opened.subscription;

    let barrier = Arc::new(Barrier::new(2));
    let waiter_session = hosted.clone();
    let waiter_barrier = Arc::clone(&barrier);
    let waiter = thread::spawn(move || {
        waiter_barrier.wait();
        waiter_session.execute(HostedRequest::NextWatch { subscription })
    });
    barrier.wait();
    let mut consumer_is_blocked = false;
    for _ in 0..10_000 {
        match hosted.execute(HostedRequest::WatchStatus { subscription }) {
            Err(error) if error.code() == ProtocolErrorCode::InvalidRequest => {
                consumer_is_blocked = true;
                break;
            }
            Ok(_) => thread::yield_now(),
            Err(error) => panic!("unexpected watch-status error: {error}"),
        }
    }
    assert!(
        consumer_is_blocked,
        "NextWatch did not enter its consumer section"
    );
    assert_eq!(
        hosted
            .execute(HostedRequest::CloseSession)
            .expect("close hosted session"),
        HostedResponse::SessionClosed
    );
    let closed = waiter
        .join()
        .expect("watch waiter thread")
        .expect_err("session close must wake blocked watch");
    assert_eq!(closed.code(), ProtocolErrorCode::WatchClosed);
    assert!(hosted.is_closed().expect("closed state"));
    let rejected = hosted
        .execute(HostedRequest::CurrentRevision)
        .expect_err("closed session must reject requests");
    assert_eq!(rejected.code(), ProtocolErrorCode::SessionClosed);

    drop(hosted);
    drop(database);
    fs::remove_dir_all(directory).expect("cleanup");
}
