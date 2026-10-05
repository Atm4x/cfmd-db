use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
    time::Instant,
    time::{SystemTime, UNIX_EPOCH},
};

use cfmd_protocol::{
    CommitRequest, CommitResponse, HostedRequest, HostedResponse, HostedSession, IdempotencyKey,
    OpenWatchRequest, ProtocolErrorCode, ProtocolLimits, ProtocolQuery, ProtocolValue,
    QueryRequest, RelationMutation, SemanticRevision, SnapshotTarget, WatchStatusDto,
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

#[test]
fn stale_set_removal_uses_historical_gamma_support_and_current_exact_representative() {
    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("fixture directory");
    let relation = RelationId::new(130);
    let equivalence = EquivalenceId::new(131);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::TextAsciiCaseInsensitive)
        .relation(RelationSchema::set(relation, [Type::text()], [equivalence]))
        .build()
        .expect("schema");
    let database = Database::create(&directory, schema).expect("database");
    let hosted = full_session(&database);

    let commit = |base_revision, key, inserted: &[&str], removed: &[&str]| {
        hosted.execute(HostedRequest::Commit(CommitRequest {
            base_revision,
            formation_semantic_revision: SemanticRevision::new(1, 1),
            idempotency_key: IdempotencyKey::new(key),
            mutations: vec![RelationMutation {
                relation: relation.raw(),
                inserted: inserted
                    .iter()
                    .map(|value| vec![ProtocolValue::Text((*value).into())])
                    .collect(),
                removed: removed
                    .iter()
                    .map(|value| vec![ProtocolValue::Text((*value).into())])
                    .collect(),
            }],
        }))
    };

    assert_eq!(
        commit(1, 700, &["Alpha"], &[]).expect("seed set"),
        HostedResponse::Commit(CommitResponse::Committed { revision: 2 })
    );
    assert_eq!(
        commit(2, 701, &["Beta"], &[]).expect("concurrent set insert"),
        HostedResponse::Commit(CommitResponse::Committed { revision: 3 })
    );
    assert_eq!(
        commit(2, 702, &[], &["aLpHa"]).expect("stale Γ-equivalent removal"),
        HostedResponse::Commit(CommitResponse::Committed { revision: 4 })
    );

    let result = hosted
        .execute(HostedRequest::Query(QueryRequest {
            target: SnapshotTarget::Head,
            query: ProtocolQuery::Scan {
                relation: relation.raw(),
            },
        }))
        .expect("query set after stale removal");
    let HostedResponse::Query(result) = result else {
        panic!("query response")
    };
    assert_eq!(result.revision, 4);
    assert_eq!(result.rows, vec![vec![ProtocolValue::Text("Beta".into())]]);

    drop(hosted);
    drop(database);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
#[ignore = "diagnostic release benchmark for stale exact-effect history depth"]
fn benchmark_stale_exact_effect_history_depth() {
    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("fixture directory");
    let relation = RelationId::new(132);
    let equivalence = EquivalenceId::new(133);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::set(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("schema");
    let database = Database::create(&directory, schema).expect("database");
    let hosted = full_session(&database);

    let mut head = 1_u64;
    let depths = [32_u64, 128, 512, 2048];
    for depth in depths {
        while head < depth + 1 {
            let value = i64::try_from(head).expect("benchmark value");
            let response = hosted
                .execute(HostedRequest::Commit(CommitRequest {
                    base_revision: head,
                    formation_semantic_revision: SemanticRevision::new(1, 1),
                    idempotency_key: IdempotencyKey::new(u128::from(head) + 10_000),
                    mutations: vec![RelationMutation {
                        relation: relation.raw(),
                        inserted: vec![vec![ProtocolValue::I64(value)]],
                        removed: vec![],
                    }],
                }))
                .expect("history append");
            let HostedResponse::Commit(CommitResponse::Committed { revision }) = response else {
                panic!("history append must commit")
            };
            head = revision;
        }

        let started = Instant::now();
        let response = hosted
            .execute(HostedRequest::Commit(CommitRequest {
                base_revision: 1,
                formation_semantic_revision: SemanticRevision::new(1, 1),
                idempotency_key: IdempotencyKey::new(u128::from(depth) + 1_000_000),
                mutations: vec![RelationMutation {
                    relation: relation.raw(),
                    inserted: vec![vec![ProtocolValue::I64(-i64::try_from(depth).unwrap())]],
                    removed: vec![],
                }],
            }))
            .expect("stale exact effect");
        let elapsed = started.elapsed();
        let HostedResponse::Commit(CommitResponse::Committed { revision }) = response else {
            panic!("stale exact effect must commit")
        };
        head = revision;
        eprintln!(
            "stale_history_depth={depth} elapsed_ns={}",
            elapsed.as_nanos()
        );
    }

    drop(hosted);
    drop(database);
    fs::remove_dir_all(directory).expect("cleanup");
}

fn commit_i64(
    hosted: &HostedSession,
    relation: RelationId,
    base_revision: u64,
    idempotency_key: u128,
    value: i64,
) {
    hosted
        .execute(HostedRequest::Commit(CommitRequest {
            base_revision,
            formation_semantic_revision: SemanticRevision::new(1, 1),
            idempotency_key: IdempotencyKey::new(idempotency_key),
            mutations: vec![RelationMutation {
                relation: relation.raw(),
                inserted: vec![vec![ProtocolValue::I64(value)]],
                removed: vec![],
            }],
        }))
        .expect("commit");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep complete hosted authorization protocol scenario together."
)]
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
            formation_semantic_revision: SemanticRevision::new(1, 1),
            idempotency_key: IdempotencyKey::new(500),
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

    let retry = hosted
        .execute(HostedRequest::Commit(CommitRequest {
            base_revision: 1,
            formation_semantic_revision: SemanticRevision::new(1, 1),
            idempotency_key: IdempotencyKey::new(500),
            mutations: vec![RelationMutation {
                relation: relation.raw(),
                inserted: vec![vec![ProtocolValue::I64(1)], vec![ProtocolValue::I64(2)]],
                removed: vec![],
            }],
        }))
        .expect("uncertain hosted retry");
    assert_eq!(
        retry,
        HostedResponse::Commit(CommitResponse::AlreadyCommitted { revision: 2 })
    );

    let changed_retry = hosted
        .execute(HostedRequest::Commit(CommitRequest {
            base_revision: 1,
            formation_semantic_revision: SemanticRevision::new(1, 1),
            idempotency_key: IdempotencyKey::new(500),
            mutations: vec![RelationMutation {
                relation: relation.raw(),
                inserted: vec![vec![ProtocolValue::I64(9)]],
                removed: vec![],
            }],
        }))
        .expect_err("same hosted key with a different effect must conflict");
    assert_eq!(changed_retry.code(), ProtocolErrorCode::TransactionConflict);

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
    assert_eq!(entries[0].idempotency_key.raw(), 500);
    assert_eq!(entries[0].source_revision, 1);
    assert_eq!(entries[0].target_revision, 2);

    let transported = hosted
        .execute(HostedRequest::Commit(CommitRequest {
            base_revision: 1,
            formation_semantic_revision: SemanticRevision::new(1, 1),
            idempotency_key: IdempotencyKey::new(501),
            mutations: vec![RelationMutation {
                relation: relation.raw(),
                inserted: vec![vec![ProtocolValue::I64(3)]],
                removed: vec![],
            }],
        }))
        .expect("disjoint stale relation intent must transport through exact history");
    assert_eq!(
        transported,
        HostedResponse::Commit(CommitResponse::Committed { revision: 3 })
    );

    let overlapping = hosted
        .execute(HostedRequest::Commit(CommitRequest {
            base_revision: 1,
            formation_semantic_revision: SemanticRevision::new(1, 1),
            idempotency_key: IdempotencyKey::new(502),
            mutations: vec![RelationMutation {
                relation: relation.raw(),
                inserted: vec![vec![ProtocolValue::I64(1)]],
                removed: vec![],
            }],
        }))
        .expect_err("overlapping stale bag intent requires coordination");
    assert_eq!(overlapping.code(), ProtocolErrorCode::TransactionConflict);

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
            formation_semantic_revision: SemanticRevision::new(1, 1),
            idempotency_key: IdempotencyKey::new(600),
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

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep complete hosted schema-aware authority scenario together."
)]
fn hosted_schema_aware_commit_uses_explicit_formation_identity_and_current_authority() {
    use cfmd_runtime::{
        MigrationColumnRule, MigrationHistoryPolicy, MigrationModel, MigrationRelationRule,
        MigrationValueExpr, PermissionSet, PrimitiveEquivalence, RelationColumnId, RelationSchema,
        TransactionId,
    };

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("fixture directory");
    let source_relation = RelationId::new(507_001);
    let target_relation = RelationId::new(507_002);
    let source_column = RelationColumnId::new(507_003);
    let target_column = RelationColumnId::new(507_004);
    let equivalence = EquivalenceId::new(507_005);

    let source = Schema::builder()
        .revisions(507, 1)
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::set_with_column_ids(
            source_relation,
            [(source_column, Type::i64())],
            [equivalence],
        ))
        .build()
        .expect("source schema");
    let database = Database::create(&directory, source).expect("database");
    let source_hosted = HostedSession::new(database.session(Session::new(
        PrincipalId::new(507_010),
        PermissionSet::from([Permission::WriteRelation(source_relation)]),
    )));
    let target_hosted = HostedSession::new(database.session(Session::new(
        PrincipalId::new(507_011),
        PermissionSet::from([Permission::WriteRelation(target_relation)]),
    )));

    let target = Schema::builder()
        .revisions(508, 1)
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::set_with_column_ids(
            target_relation,
            [(target_column, Type::i64())],
            [equivalence],
        ))
        .build()
        .expect("target schema");
    let migration = MigrationModel::new(507_508, target).relation(MigrationRelationRule::Rows {
        source: source_relation,
        target: target_relation,
        columns: vec![MigrationColumnRule {
            source_columns: vec![0],
            target_column: 0,
            value: MigrationValueExpr::Column(0),
        }],
    });
    database
        .migrate(
            &migration,
            TransactionId::new(9_507_001),
            MigrationHistoryPolicy::Forget,
        )
        .expect("migration");

    let stale = CommitRequest {
        base_revision: 1,
        formation_semantic_revision: SemanticRevision::new(507, 1),
        idempotency_key: IdempotencyKey::new(9_507_002),
        mutations: vec![RelationMutation {
            relation: source_relation.raw(),
            inserted: vec![vec![ProtocolValue::I64(7)]],
            removed: vec![],
        }],
    };
    assert_eq!(
        source_hosted
            .execute(HostedRequest::Commit(stale.clone()))
            .expect_err("source-world grant must not authorize target-world publication")
            .code(),
        ProtocolErrorCode::PermissionDenied
    );
    assert_eq!(
        target_hosted
            .execute(HostedRequest::Commit(stale))
            .expect("target-world grant publishes transported intent"),
        HostedResponse::Commit(CommitResponse::Committed { revision: 3 })
    );

    let bad_identity = CommitRequest {
        base_revision: 1,
        formation_semantic_revision: SemanticRevision::new(999, 1),
        idempotency_key: IdempotencyKey::new(9_507_003),
        mutations: vec![RelationMutation {
            relation: source_relation.raw(),
            inserted: vec![vec![ProtocolValue::I64(8)]],
            removed: vec![],
        }],
    };
    assert_eq!(
        target_hosted
            .execute(HostedRequest::Commit(bad_identity))
            .expect_err("explicit semantic identity mismatch must fail closed")
            .code(),
        ProtocolErrorCode::InvalidRequest
    );

    drop(source_hosted);
    drop(target_hosted);
    drop(database);
    fs::remove_dir_all(directory).expect("cleanup");
}
