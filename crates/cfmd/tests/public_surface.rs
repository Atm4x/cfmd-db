use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use cfmd::{
    CfmdEntity, CfmdSchema, CommitOutcome, Database, DiagnosticCode, EntitySet, ErrorDiagnosticExt, ErrorKind, Id, Schema,
    Object, ObjectPredicate, RuleValueExpr, SemanticRuleExpr, Transaction, TransactionId,
};

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.todo")]
struct Todo {
    #[cfmd(id)]
    pub id: Id<Todo>,
    pub title: String,
    pub done: bool,
}

#[allow(dead_code)]
#[derive(CfmdSchema)]
struct TodoSchema {
    todos: EntitySet<Todo>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.user", authoritative)]
struct User {
    #[cfmd(id)]
    pub id: Id<User>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.rule-user")]
struct RuleUser {
    #[cfmd(id)]
    pub id: Id<RuleUser>,
    #[cfmd(length(min = 3, max = 8))]
    pub name: String,
    #[cfmd(range(min = 0, max = 150))]
    pub age: i64,
    #[cfmd(one_of("user", "admin"))]
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.task", authoritative)]
struct Task {
    #[cfmd(id)]
    pub id: Id<Task>,
    pub title: String,
    pub owner: cfmd::Ref<User>,
    pub reviewer: Option<cfmd::Ref<User>>,
}

#[derive(CfmdSchema)]
struct AppSchema {
    // Intentionally reversed: schema assembly must not depend on declaration order.
    tasks: EntitySet<Task>,
    users: EntitySet<User>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.task")]
struct PartialTask {
    #[cfmd(id)]
    id: Id<PartialTask>,
    owner: cfmd::Ref<User>,
    reviewer: Option<cfmd::Ref<User>>,
}

#[derive(CfmdSchema)]
struct PartialTaskSchema {
    tasks: EntitySet<PartialTask>,
}

#[derive(CfmdSchema)]
struct UserOnlySchema {
    users: EntitySet<User>,
}

#[allow(dead_code)]
#[derive(CfmdSchema)]
struct BrokenTaskSchema {
    tasks: EntitySet<Task>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.hostile-account", authoritative)]
struct HostileAccount {
    #[cfmd(id)]
    id: Id<HostileAccount>,
    name: String,
    passport_secret: String,
    #[cfmd(length(max = 32))]
    doctor_note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.hostile-account")]
struct HostileReaderAccount {
    #[cfmd(id)]
    id: Id<HostileReaderAccount>,
    name: String,
    doctor_note: String,
}

#[derive(CfmdSchema)]
struct HostileAccountSchema {
    accounts: EntitySet<HostileAccount>,
}

#[allow(dead_code)]
#[derive(CfmdSchema)]
struct HostileReaderSchema {
    accounts: EntitySet<HostileReaderAccount>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.rename-account", authoritative)]
struct RenamedAccount {
    #[cfmd(id)]
    id: Id<RenamedAccount>,
    name: String,
    #[cfmd(length(max = 32))]
    medical_note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.rename-account")]
struct LegacyRenameAccount {
    #[cfmd(id)]
    id: Id<LegacyRenameAccount>,
    #[cfmd(bind = "medical_note")]
    doctor_note: String,
}

#[derive(CfmdSchema)]
struct RenamedAccountSchema {
    accounts: EntitySet<RenamedAccount>,
}

#[derive(CfmdSchema)]
struct LegacyRenameSchema {
    accounts: EntitySet<LegacyRenameAccount>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.rename-doctor", authoritative)]
struct RenameDoctor {
    #[cfmd(id)]
    id: Id<RenameDoctor>,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.rename-patient", authoritative)]
struct RenamedPatient {
    #[cfmd(id)]
    id: Id<RenamedPatient>,
    primary_doctor: cfmd::Ref<RenameDoctor>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.rename-patient")]
struct LegacyPatient {
    #[cfmd(id)]
    id: Id<LegacyPatient>,
    #[cfmd(bind = "primary_doctor")]
    doctor: cfmd::Ref<RenameDoctor>,
}

#[derive(CfmdSchema)]
struct RenamedReferenceSchema {
    doctors: EntitySet<RenameDoctor>,
    patients: EntitySet<RenamedPatient>,
}

#[derive(CfmdSchema)]
struct LegacyReferenceSchema {
    patients: EntitySet<LegacyPatient>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.parent")]
struct Parent {
    #[cfmd(id)]
    pub id: Id<Parent>,
    pub name: String,
    pub children: cfmd::Many<Child>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.parent")]
struct ParentReader {
    #[cfmd(id)]
    pub id: Id<ParentReader>,
    pub children: cfmd::Many<Child>,
}

#[derive(CfmdSchema)]
struct ParentReaderSchema {
    parents: EntitySet<ParentReader>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.child")]
struct Child {
    #[cfmd(id)]
    pub id: Id<Child>,
    pub score: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.owner")]
struct Owner {
    #[cfmd(id)]
    pub id: Id<Owner>,
    pub name: String,
    #[cfmd(orphan = "delete")]
    pub assets: cfmd::OwnedMany<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.asset")]
struct Asset {
    #[cfmd(id)]
    pub id: Id<Asset>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, CfmdEntity)]
#[cfmd(key = "example.ordered-f64")]
struct OrderedF64 {
    #[cfmd(id)]
    pub id: Id<OrderedF64>,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, CfmdEntity)]
#[cfmd(key = "example.grouped-metric")]
struct GroupedMetric {
    #[cfmd(id)]
    pub id: Id<GroupedMetric>,
    pub bucket: i64,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.column-pair")]
struct ColumnPair {
    #[cfmd(id)]
    pub id: Id<ColumnPair>,
    pub left: i64,
    pub right: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.wide-row")]
struct WideRow {
    #[cfmd(id)]
    pub id: Id<WideRow>,
    pub a: i64,
    pub b: i64,
    pub c: i64,
}

static TEMP_PATH_NONCE: AtomicU64 = AtomicU64::new(0);

fn temp_path() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let sequence = TEMP_PATH_NONCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "cfmd-public-{nonce}-{}-{sequence}",
        std::process::id()
    ))
}

#[test]
fn adaptive_transaction_crud_keeps_database_and_payload_visible() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut transaction = Transaction::new();
    database
        .objects::<Todo>()
        .expect("todos")
        .add(
            &mut transaction,
            Todo {
                id: Id::new(358_001),
                title: "database-owned CRUD".to_owned(),
                done: false,
            },
        )
        .expect("add todo");

    assert!(transaction.id().is_some());
    assert!(transaction.origin_revision().is_some());
    assert!(!transaction.is_snapshot_bound());
    let preview = database.preview(&transaction).expect("preview");
    assert_eq!(preview.effects().inserted_rows(), 1);
    database.commit(&transaction).expect("commit");
    assert!(
        database
            .objects::<Todo>()
            .expect("todos")
            .get(Id::new(358_001))
            .expect("get")
            .is_some()
    );

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn history_undo_accumulates_into_the_same_visible_transaction_language() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut seed = Transaction::new();
    database
        .objects::<Todo>()
        .expect("todos")
        .add(
            &mut seed,
            Todo {
                id: Id::new(360_001),
                title: "history through transaction".to_owned(),
                done: false,
            },
        )
        .expect("seed intent");
    database.commit(&seed).expect("seed commit");

    let history = database.history().expect("history");
    let entry = history.latest().expect("latest history");
    let mut undo = Transaction::new();
    database.undo(&mut undo, entry).expect("append undo");
    let preview = database.preview(&undo).expect("undo preview");
    assert!(preview.effects().changes_lifecycle());
    database.commit(&undo).expect("undo commit");
    assert!(
        database
            .objects::<Todo>()
            .expect("todos")
            .get(Id::new(360_001))
            .expect("lookup after undo")
            .is_none()
    );

    let mut redo = Transaction::new();
    database
        .undo_latest(&mut redo)
        .expect("append undo-of-undo");
    database.commit(&redo).expect("redo commit");
    assert!(
        database
            .objects::<Todo>()
            .expect("todos")
            .get(Id::new(360_001))
            .expect("lookup after redo")
            .is_some()
    );

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn history_undo_composes_atomically_with_ordinary_crud() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut seed = Transaction::new();
    database
        .objects::<Todo>()
        .expect("todos")
        .add(
            &mut seed,
            Todo {
                id: Id::new(360_010),
                title: "undo me".to_owned(),
                done: false,
            },
        )
        .expect("seed intent");
    database.commit(&seed).expect("seed commit");

    let history = database.history().expect("history");
    let entry = history.latest().expect("latest history");
    let mut transaction = Transaction::new();
    database.undo(&mut transaction, entry).expect("append undo");
    database
        .objects::<Todo>()
        .expect("todos")
        .add(
            &mut transaction,
            Todo {
                id: Id::new(360_011),
                title: "same atomic intent".to_owned(),
                done: false,
            },
        )
        .expect("append ordinary insert");

    let preview = database.preview(&transaction).expect("combined preview");
    assert!(preview.effects().changes_lifecycle());
    database.commit(&transaction).expect("combined commit");
    let todos = database.objects::<Todo>().expect("todos");
    assert!(
        todos
            .get(Id::new(360_010))
            .expect("undone lookup")
            .is_none()
    );
    assert!(
        todos
            .get(Id::new(360_011))
            .expect("inserted lookup")
            .is_some()
    );

    drop(todos);
    drop(history);
    drop(database);
    fs::remove_file(path).expect("remove database");
}


#[test]
fn typed_context_snapshot_keeps_one_exact_revision_while_context_tracks_head() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path).schema(schema).create().expect("create database");
    let context = database.context::<TodoSchema>().expect("context");
    let snapshot = context.snapshot().expect("snapshot");

    let mut transaction = Transaction::new();
    context.todos.add(&mut transaction, Todo {
        id: Id::new(358_099),
        title: "after-snapshot".to_owned(),
        done: false,
    }).expect("add");
    context.commit(&transaction).expect("commit");

    assert!(context.todos.get(Id::new(358_099)).expect("current read").is_some());
    assert!(snapshot.todos.get(Id::new(358_099)).expect("snapshot read").is_none());

    drop(snapshot);
    drop(context);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn snapshot_transaction_refuses_silent_time_transport() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let context = database.context::<TodoSchema>().expect("context");
    let snapshot = context.snapshot().expect("snapshot");
    let mut strict = Transaction::from(snapshot);
    database
        .objects::<Todo>()
        .expect("todos")
        .add(
            &mut strict,
            Todo {
                id: Id::new(358_010),
                title: "strict".to_owned(),
                done: false,
            },
        )
        .expect("strict add");

    let advance = database
        .objects::<Todo>()
        .expect("todos")
        .insert_plan(Todo {
            id: Id::new(358_011),
            title: "advance".to_owned(),
            done: false,
        })
        .expect("advance plan");
    database
        .commit_plan(&advance, TransactionId::new(358_011))
        .expect("advance commit");

    assert!(matches!(
        database.transaction_readiness(&strict).expect("readiness"),
        cfmd::TransactionReadiness::SnapshotChanged { .. }
    ));
    assert_eq!(
        database
            .commit(&strict)
            .expect_err("strict snapshot transaction must not move")
            .kind(),
        ErrorKind::StaleRevision
    );

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn object_first_application_uses_only_cfmd_crate() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let snapshot = database.snapshot().expect("snapshot");
    let todos = snapshot.objects::<Todo>().expect("todo set");
    let plan = todos
        .insert(Todo {
            id: Id::new(1),
            title: "ship public facade".to_owned(),
            done: false,
        })
        .expect("insert plan");
    drop(todos);
    drop(snapshot);

    assert!(matches!(
        database
            .commit_plan(&plan, TransactionId::new(337_001))
            .expect("commit"),
        CommitOutcome::Committed { .. }
    ));

    let snapshot = database.snapshot().expect("snapshot");
    let todo = snapshot
        .objects::<Todo>()
        .expect("todo set")
        .where_(|todo| todo.id().eq(Id::new(1)))
        .one()
        .expect("todo");
    assert_eq!(todo.title, "ship public facade");
    drop(snapshot);
    drop(database);

    let reopened = Database::open(&path).expect("reopen through public facade");
    let snapshot = reopened.snapshot().expect("reopened snapshot");
    assert_eq!(
        snapshot
            .objects::<Todo>()
            .expect("reopened todo set")
            .require(Id::new(1))
            .expect("persisted todo")
            .title,
        "ship public facade"
    );
    drop(snapshot);
    drop(reopened);
    fs::remove_file(path).expect("remove single-file database");
}

#[test]
fn snapshot_bound_transaction_composes_preview_and_commit_without_manual_plan_plumbing() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut transaction = database
        .transaction_with_id(TransactionId::new(351_001))
        .expect("transaction");
    let todos = transaction.objects::<Todo>().expect("todo set");
    transaction
        .add_plan(
            todos
                .insert(Todo {
                    id: Id::new(1),
                    title: "first".to_owned(),
                    done: false,
                })
                .expect("first insert"),
        )
        .expect("compose first");
    transaction
        .add_plan(
            todos
                .insert(Todo {
                    id: Id::new(2),
                    title: "second".to_owned(),
                    done: false,
                })
                .expect("second insert"),
        )
        .expect("compose second");

    let preview = database.preview(&transaction).expect("preview");
    assert_eq!(preview.effects().inserted_rows(), 2);
    assert_eq!(
        preview.source_revision(),
        transaction.origin_revision().expect("bound transaction")
    );
    let first = database.commit(&transaction).expect("commit transaction");
    let revision = match first {
        CommitOutcome::Committed { revision } => revision,
        CommitOutcome::AlreadyCommitted { .. } => panic!("first publication must commit"),
    };
    assert_eq!(
        database
            .commit(&transaction)
            .expect("idempotent transaction retry"),
        CommitOutcome::AlreadyCommitted { revision }
    );

    let snapshot = database.snapshot().expect("snapshot");
    assert_eq!(
        snapshot
            .objects::<Todo>()
            .expect("todos")
            .all()
            .expect("all todos")
            .len(),
        2
    );
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn snapshot_bound_transaction_rejects_cross_snapshot_plan_and_auto_merges_independent_head() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let seed_snapshot = database.snapshot().expect("seed snapshot");
    let seed = seed_snapshot
        .objects::<Todo>()
        .expect("seed todos")
        .insert(Todo {
            id: Id::new(1),
            title: "seed".to_owned(),
            done: false,
        })
        .expect("seed insert");
    database
        .commit_plan(&seed, TransactionId::new(351_009))
        .expect("seed commit");
    drop(seed_snapshot);

    let mut stale = database
        .transaction_with_id(TransactionId::new(351_010))
        .expect("stale transaction");
    let stale_todos = stale.objects::<Todo>().expect("stale todos");
    stale
        .add_plan(
            stale_todos
                .insert(Todo {
                    id: Id::new(10),
                    title: "stale".to_owned(),
                    done: false,
                })
                .expect("stale insert"),
        )
        .expect("compose stale");

    let mut winner = database
        .transaction_with_id(TransactionId::new(351_011))
        .expect("winner transaction");
    let winner_todos = winner.objects::<Todo>().expect("winner todos");
    winner
        .add_plan(
            winner_todos
                .insert(Todo {
                    id: Id::new(11),
                    title: "winner".to_owned(),
                    done: false,
                })
                .expect("winner insert"),
        )
        .expect("compose winner");
    database.commit(&winner).expect("winner commit");

    assert!(matches!(
        database
            .transaction_readiness(&stale)
            .expect("stale transaction readiness"),
        cfmd::TransactionReadiness::Rebasable {
            base_revision,
            current_revision,
            ref intervening_effects,
        } if base_revision == stale.origin_revision().expect("bound stale transaction")
            && current_revision == database.current_revision().expect("current revision")
            && intervening_effects.len() == 1
    ));
    let merged_preview = database.preview(&stale).expect("certified merged preview");
    assert_eq!(
        merged_preview.source_revision(),
        database
            .current_revision()
            .expect("preview current revision")
    );

    let merged = database
        .commit(&stale)
        .expect("certified stale transaction must publish on the current head");
    let merged_revision = match merged {
        cfmd::CommitOutcome::Committed { revision } => revision,
        cfmd::CommitOutcome::AlreadyCommitted { .. } => panic!("first merged publish must commit"),
    };
    assert!(matches!(
        database
            .commit(&stale)
            .expect("retry after merged publication must remain idempotent"),
        cfmd::CommitOutcome::AlreadyCommitted { revision } if revision == merged_revision
    ));

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn independent_first_object_inserts_share_idempotent_carrier_presence() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut first = database
        .transaction_with_id(TransactionId::new(356_001))
        .expect("first transaction");
    let first_todos = first.objects::<Todo>().expect("first todos");
    first
        .add_plan(
            first_todos
                .insert(Todo {
                    id: Id::new(1),
                    title: "first".to_owned(),
                    done: false,
                })
                .expect("first insert"),
        )
        .expect("compose first");
    drop(first_todos);

    let mut second = database
        .transaction_with_id(TransactionId::new(356_002))
        .expect("second transaction");
    let second_todos = second.objects::<Todo>().expect("second todos");
    second
        .add_plan(
            second_todos
                .insert(Todo {
                    id: Id::new(2),
                    title: "second".to_owned(),
                    done: false,
                })
                .expect("second insert"),
        )
        .expect("compose second");
    drop(second_todos);

    database.commit(&first).expect("commit first");
    assert!(matches!(
        database
            .transaction_readiness(&second)
            .expect("second readiness"),
        cfmd::TransactionReadiness::Rebasable { .. }
    ));
    database
        .preview(&second)
        .expect("same carrier creation must rebase through the first insert");
    let second_revision = match database
        .commit(&second)
        .expect("same carrier intent and distinct entity must merge")
    {
        cfmd::CommitOutcome::Committed { revision } => revision,
        cfmd::CommitOutcome::AlreadyCommitted { .. } => panic!("first merged publish must commit"),
    };
    assert!(matches!(
        database
            .commit(&second)
            .expect("merged transaction retry must be durable-idempotent"),
        cfmd::CommitOutcome::AlreadyCommitted { revision } if revision == second_revision
    ));

    let snapshot = database.snapshot().expect("merged snapshot");
    assert_eq!(
        snapshot
            .objects::<Todo>()
            .expect("merged todos")
            .all()
            .expect("merged todo rows")
            .len(),
        2
    );
    drop(snapshot);

    let mut undo = Transaction::new();
    database.undo_latest(&mut undo).expect("undo merged insert");
    database.commit(&undo).expect("commit merged undo");
    let snapshot = database.snapshot().expect("undo snapshot");
    let remaining = snapshot
        .objects::<Todo>()
        .expect("remaining todos")
        .all()
        .expect("remaining rows");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, Id::new(1));
    drop(snapshot);

    drop(first);
    drop(second);
    drop(undo);
    drop(database);
    let reopened = Database::open(&path).expect("reopen mixed residual database");
    let snapshot = reopened.snapshot().expect("reopened snapshot");
    let remaining = snapshot
        .objects::<Todo>()
        .expect("reopened todos")
        .all()
        .expect("reopened rows");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, Id::new(1));
    drop(snapshot);
    drop(reopened);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn transaction_rejects_plan_from_a_different_snapshot() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let mut transaction = database
        .transaction_with_id(TransactionId::new(355_010))
        .expect("transaction");

    let advance_snapshot = database.snapshot().expect("advance snapshot");
    let advance = advance_snapshot
        .objects::<Todo>()
        .expect("advance todos")
        .insert(Todo {
            id: Id::new(12),
            title: "advance".to_owned(),
            done: false,
        })
        .expect("advance insert");
    database
        .commit_plan(&advance, TransactionId::new(355_011))
        .expect("advance commit");
    drop(advance_snapshot);

    let current = database.snapshot().expect("current snapshot");
    let foreign_plan = current
        .objects::<Todo>()
        .expect("current todos")
        .insert(Todo {
            id: Id::new(13),
            title: "foreign".to_owned(),
            done: false,
        })
        .expect("foreign insert");
    let error = transaction
        .add_plan(foreign_plan)
        .expect_err("cross-snapshot plan must fail");
    assert_eq!(error.kind(), ErrorKind::InvalidPlan);

    drop(current);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn transaction_readiness_reports_semantic_coordinate_conflict() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut first = database
        .transaction_with_id(TransactionId::new(355_001))
        .expect("first transaction");
    let first_todos = first.objects::<Todo>().expect("first todos");
    first
        .add_plan(
            first_todos
                .insert(Todo {
                    id: Id::new(77),
                    title: "first".to_owned(),
                    done: false,
                })
                .expect("first insert"),
        )
        .expect("compose first");

    let mut second = database
        .transaction_with_id(TransactionId::new(355_002))
        .expect("second transaction");
    let second_todos = second.objects::<Todo>().expect("second todos");
    second
        .add_plan(
            second_todos
                .insert(Todo {
                    id: Id::new(77),
                    title: "second".to_owned(),
                    done: false,
                })
                .expect("second insert"),
        )
        .expect("compose second");

    database.commit(&first).expect("commit first");
    assert!(matches!(
        database
            .transaction_readiness(&second)
            .expect("second readiness"),
        cfmd::TransactionReadiness::Conflict {
            conflicting_coordinates,
            ..
        } if conflicting_coordinates > 0
    ));
    assert_eq!(
        database
            .preview(&second)
            .expect_err("conflicting stale preview must fail")
            .kind(),
        ErrorKind::TransactionConflict
    );
    assert_eq!(
        database
            .commit(&second)
            .expect_err("conflicting stale transaction must fail closed")
            .kind(),
        ErrorKind::TransactionConflict
    );

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn database_owned_transaction_control_rejects_foreign_database() {
    let first_path = temp_path();
    let second_path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let first = Database::builder(&first_path)
        .schema(schema.clone())
        .create()
        .expect("first database");
    let second = Database::builder(&second_path)
        .schema(schema)
        .create()
        .expect("second database");

    let mut transaction = first
        .transaction_with_id(TransactionId::new(354_001))
        .expect("transaction");
    let todos = transaction.objects::<Todo>().expect("todos");
    transaction
        .add_plan(
            todos
                .insert(Todo {
                    id: Id::new(1),
                    title: "owned by first".to_owned(),
                    done: false,
                })
                .expect("insert plan"),
        )
        .expect("compose transaction");

    assert_eq!(
        second
            .preview(&transaction)
            .expect_err("foreign preview must fail")
            .kind(),
        ErrorKind::InvalidPlan
    );
    assert_eq!(
        second
            .commit(&transaction)
            .expect_err("foreign commit must fail")
            .kind(),
        ErrorKind::InvalidPlan
    );
    first.commit(&transaction).expect("owner commit");

    drop(first);
    drop(second);
    fs::remove_file(first_path).expect("remove first database");
    fs::remove_file(second_path).expect("remove second database");
}

#[test]
fn diagnostics_are_stable_at_public_facade() {
    let path = temp_path();
    let error = Database::open(&path).expect_err("missing database must fail");
    let diagnostic = error.diagnostic();
    assert_eq!(diagnostic.code(), DiagnosticCode::Recovery);
    assert!(!diagnostic.message().is_empty());
}

#[test]
fn query_identity_survives_clone_and_reaches_diagnostics() {
    use cfmd::dynamic::{Query, RelationId};

    let expected_line = line!() + 1;
    let query = Query::scan(RelationId::new(0xfeed));
    let cloned = query.clone();
    assert_eq!(query.node_id(), cloned.node_id());
    assert_eq!(query.nodes(), cloned.nodes());
    assert_eq!(query.source().line(), expected_line);
    assert!(query.source().file().ends_with("public_surface.rs"));

    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let snapshot = database.snapshot().expect("snapshot");
    let error = snapshot
        .execute(&query)
        .expect_err("unknown relation must fail");
    let diagnostic = error.diagnostic();
    assert_eq!(diagnostic.code(), DiagnosticCode::Query);
    assert_eq!(diagnostic.query_node(), Some(query.node_id()));
    assert_eq!(diagnostic.query_source(), Some(query.source()));
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn derive_generates_typed_reference_navigation() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<User>()
        .object::<Task>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let snapshot = database.snapshot().expect("snapshot");
    let plan = snapshot
        .objects::<User>()
        .expect("users")
        .insert(User {
            id: Id::new(1),
            name: "Alice".to_owned(),
        })
        .expect("insert user");
    drop(snapshot);
    database
        .commit_plan(&plan, TransactionId::new(338_001))
        .expect("commit user");

    let snapshot = database.snapshot().expect("snapshot");
    let plan = snapshot
        .objects::<Task>()
        .expect("tasks")
        .insert(Task {
            id: Id::new(7),
            title: "typed path".to_owned(),
            owner: cfmd::Ref::new(Id::new(1)),
            reviewer: None,
        })
        .expect("insert task");
    drop(snapshot);
    database
        .commit_plan(&plan, TransactionId::new(338_002))
        .expect("commit task");

    let snapshot = database.snapshot().expect("snapshot");
    let tasks = snapshot
        .objects::<Task>()
        .expect("tasks")
        .where_(|task| {
            task.owner()
                .matches(|user| user.name().eq("Alice".to_owned()))
        })
        .all()
        .expect("deep reference query");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "typed path");
    assert!(tasks[0].owner.is_bound());
    assert_eq!(
        tasks[0]
            .owner
            .load()
            .expect("explicit ref materialization")
            .name,
        "Alice"
    );
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

fn seed_object_many(database: &Database) {
    let snapshot = database.snapshot().expect("snapshot");
    let parents = snapshot.objects::<Parent>().expect("parents");
    let plan = parents
        .insert(Parent::cfmd_new(
            Id::new(1),
            "one".into(),
            cfmd::Many::new([
                Child {
                    id: Id::new(10),
                    score: 5,
                },
                Child {
                    id: Id::new(11),
                    score: 9,
                },
            ]),
        ))
        .expect("p1 graph")
        .and(
            parents
                .insert(Parent::cfmd_new(
                    Id::new(2),
                    "two".into(),
                    cfmd::Many::new([Child {
                        id: Id::new(12),
                        score: 3,
                    }]),
                ))
                .expect("p2 graph"),
        )
        .expect("parents");
    drop(parents);
    drop(snapshot);
    database
        .commit_plan(&plan, TransactionId::new(342_001))
        .expect("commit object graph");
}

#[test]
fn derive_many_is_first_class_object_relation() {
    use cfmd::{Object, ObjectRelationshipCardinality};

    let many = Parent::many_fields();
    assert_eq!(many.len(), 1);
    assert_eq!(many[0].name(), "children");
    assert_eq!(many[0].cardinality(), ObjectRelationshipCardinality::Many);

    let path = temp_path();
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .build()
        .expect("object relationship schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create object relationship database");
    seed_object_many(&database);

    let snapshot = database.snapshot().expect("snapshot");
    let parents = snapshot.objects::<Parent>().expect("parents");
    assert_eq!(
        parents
            .clone()
            .where_(|p| p.children().any(|c| c.score().eq(9)))
            .all()
            .expect("any")
            .len(),
        1
    );
    assert_eq!(
        parents
            .clone()
            .where_(|p| p.children().all(|c| c.score().eq(3)))
            .all()
            .expect("all")
            .len(),
        1
    );
    assert_eq!(
        parents
            .where_(|p| p.children().count().eq(2))
            .all()
            .expect("count")
            .len(),
        1
    );

    let parent = snapshot
        .objects::<Parent>()
        .expect("parents")
        .require(Id::new(1))
        .expect("parent");
    assert!(parent.children.is_bound());
    assert_eq!(parent.children.count().expect("edge count"), 2);
    let loaded = parent
        .children
        .load()
        .expect("explicit relationship materialization");
    assert_eq!(loaded.len(), 2);
    assert!(loaded.iter().any(|child| child.score == 9));
    assert_eq!(
        parent
            .children
            .where_(|child| child.score().eq(9))
            .expect("relationship query")
            .all()
            .expect("filtered relationship")
            .len(),
        1
    );
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn many_count_predicates_preserve_zero_degree_in_exact_candidate_and_watch() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("database");
    seed_object_many(&database);

    let mut add_zero = Transaction::new();
    database
        .objects::<Parent>()
        .expect("parents")
        .add(
            &mut add_zero,
            Parent::cfmd_new(Id::new(3), "zero".into(), cfmd::Many::empty()),
        )
        .expect("zero-degree parent");
    database.commit(&add_zero).expect("commit zero-degree parent");

    let ids = |rows: Vec<Parent>| {
        let mut ids = rows
            .into_iter()
            .map(|parent| parent.id.raw())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    };

    let parents = database.objects::<Parent>().expect("parents");
    assert_eq!(
        ids(parents
            .clone()
            .where_(|p| p.children().count().less_than(1))
            .all()
            .expect("count < 1")),
        vec![3]
    );
    assert_eq!(
        ids(parents
            .clone()
            .where_(|p| p.children().count().less_than_or_equal(1))
            .all()
            .expect("count <= 1")),
        vec![2, 3]
    );
    assert_eq!(
        ids(parents
            .clone()
            .where_(|p| p.children().count().greater_than(1))
            .all()
            .expect("count > 1")),
        vec![1]
    );
    assert_eq!(
        ids(parents
            .clone()
            .where_(|p| p.children().count().greater_than_or_equal(1))
            .all()
            .expect("count >= 1")),
        vec![1, 2]
    );
    assert_eq!(
        ids(parents
            .clone()
            .where_(|p| p.children().count().ne(1))
            .all()
            .expect("count != 1")),
        vec![1, 3]
    );
    assert_eq!(
        ids(parents
            .clone()
            .where_(|p| p.children().count().between(0, 1))
            .all()
            .expect("0 <= count <= 1")),
        vec![2, 3]
    );
    assert_eq!(
        ids(parents
            .clone()
            .where_(|p| p.children().count().greater_than(-1))
            .all()
            .expect("count > -1")),
        vec![1, 2, 3]
    );

    let zero_query = parents.where_(|p| p.children().count().eq(0));
    let mut watch = zero_query.watch().expect("zero-degree watch");
    assert_eq!(ids(watch.initial().to_vec()), vec![3]);

    let snapshot = database.snapshot().expect("snapshot");
    let zero_parent = snapshot
        .objects::<Parent>()
        .expect("parents")
        .require(Id::new(3))
        .expect("zero parent");

    let candidate = zero_parent
        .children
        .attach_plan(Id::new(10))
        .expect("candidate attach plan")
        .candidate()
        .expect("candidate");
    assert!(
        candidate
            .objects::<Parent>()
            .expect("candidate parents")
            .where_(|p| p.children().count().eq(0))
            .all()
            .expect("candidate zero-degree parents")
            .is_empty()
    );

    let mut attach = Transaction::new();
    zero_parent
        .children
        .attach(&mut attach, Id::new(10))
        .expect("attach shared child");
    drop(zero_parent);
    drop(snapshot);

    database.preview(&attach).expect("preview");

    database.commit(&attach).expect("attach commit");
    let event = watch.try_recv().expect("watch event").expect("event");
    assert!(event.inserted().is_empty());
    assert_eq!(ids(event.removed().to_vec()), vec![3]);

    drop(watch);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn object_update_preserves_bound_many_when_only_scalar_fields_change() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("database");
    seed_object_many(&database);

    let snapshot = database.snapshot().expect("snapshot");
    let plan = snapshot
        .objects::<Parent>()
        .expect("parents")
        .where_(|parent| parent.id().eq(Id::new(1)))
        .update_plan(|mut parent| {
            parent.name = "renamed".into();
            parent
        })
        .expect("scalar rewrite");
    drop(snapshot);
    database
        .commit_plan(&plan, TransactionId::new(342_005))
        .expect("commit scalar rewrite");

    let snapshot = database.snapshot().expect("snapshot");
    let parent = snapshot
        .objects::<Parent>()
        .expect("parents")
        .require(Id::new(1))
        .expect("parent");
    assert_eq!(parent.name, "renamed");
    assert_eq!(parent.children.count().expect("preserved relationship"), 2);
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn object_update_replaces_many_relation_value() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("database");
    seed_object_many(&database);

    let snapshot = database.snapshot().expect("snapshot");
    let plan = snapshot
        .objects::<Parent>()
        .expect("parents")
        .where_(|parent| parent.id().eq(Id::new(1)))
        .update_plan(|parent| {
            Parent::cfmd_new(
                parent.id,
                parent.name,
                cfmd::Many::new([Child {
                    id: Id::new(13),
                    score: 7,
                }]),
            )
        })
        .expect("replace object relationship");
    drop(snapshot);
    database
        .commit_plan(&plan, TransactionId::new(342_004))
        .expect("commit relationship replacement");

    let snapshot = database.snapshot().expect("snapshot");
    let parent = snapshot
        .objects::<Parent>()
        .expect("parents")
        .require(Id::new(1))
        .expect("parent");
    assert_eq!(parent.children.count().expect("count"), 1);
    let children = parent.children.load().expect("load");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].id.raw(), 13);
    assert_eq!(children[0].score, 7);
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn many_edges_follow_object_lifecycle_without_manual_cascade() {
    use cfmd::Object;

    let path = temp_path();
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("database");
    seed_object_many(&database);

    let snapshot = database.snapshot().expect("snapshot");
    let delete_child = snapshot
        .objects::<Child>()
        .expect("children")
        .where_(|child| child.id().eq(Id::new(11)))
        .delete_plan()
        .expect("delete child plan");
    drop(snapshot);
    database
        .commit_plan(&delete_child, TransactionId::new(342_002))
        .expect("delete child");

    let snapshot = database.snapshot().expect("snapshot");
    let parent = snapshot
        .objects::<Parent>()
        .expect("parents")
        .require(Id::new(1))
        .expect("parent");
    assert_eq!(parent.children.count().expect("normalized edge count"), 1);
    let delete_parent = snapshot
        .objects::<Parent>()
        .expect("parents")
        .where_(|parent| parent.id().eq(Id::new(1)))
        .delete_plan()
        .expect("delete parent plan");
    drop(snapshot);
    database
        .commit_plan(&delete_parent, TransactionId::new(342_003))
        .expect("delete parent");

    let snapshot = database.snapshot().expect("snapshot");
    let relation = Parent::many_fields()[0].relation();
    assert!(
        snapshot
            .execute(&cfmd::dynamic::Query::scan(relation))
            .expect("internal edge relation")
            .rows()
            .iter()
            .all(|row| {
                !matches!(
                    row.first(),
                    Some(cfmd::dynamic::Value::HistoricalEntityRef(reference))
                        if reference.id == 1
                )
            })
    );
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn many_requires_target_registration() {
    let error = Schema::builder()
        .object::<Parent>()
        .build()
        .expect_err("declared object relationship target must be registered");
    assert_eq!(error.kind(), cfmd::ErrorKind::InvalidSchema);
    assert!(error.to_string().contains("example.parent.children"));
    assert!(
        error
            .to_string()
            .contains("requires target object relation")
    );
}

#[test]
fn object_query_reports_application_callsite() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let snapshot = database.snapshot().expect("snapshot");
    let todos = snapshot.objects::<Todo>().expect("todos");
    let expected_line = line!() + 1;
    let query = todos.where_(|todo| todo.done().eq(false));
    assert_eq!(query.source().line(), expected_line);
    assert!(query.source().file().ends_with("public_surface.rs"));
    drop(query);
    drop(todos);
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn owned_many_enforces_exclusive_owner_and_supports_atomic_move() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<Owner>()
        .object::<Asset>()
        .build()
        .expect("owned schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("db");
    let snapshot = database.snapshot().expect("snapshot");
    let owner_set = snapshot.objects::<Owner>().expect("owners");
    let plan = owner_set
        .insert(Owner::cfmd_new(
            Id::new(1),
            "one".into(),
            cfmd::OwnedMany::new([Asset {
                id: Id::new(10),
                label: "share".into(),
            }]),
        ))
        .expect("owner one")
        .and(
            owner_set
                .insert(Owner::cfmd_new(
                    Id::new(2),
                    "two".into(),
                    cfmd::OwnedMany::empty(),
                ))
                .expect("owner two"),
        )
        .expect("compose");
    drop(owner_set);
    drop(snapshot);
    database
        .commit_plan(&plan, TransactionId::new(343_001))
        .expect("seed");

    let snapshot = database.snapshot().expect("snapshot");
    let owner1 = snapshot
        .objects::<Owner>()
        .expect("owners")
        .require(Id::new(1))
        .expect("one");
    let owner2 = snapshot
        .objects::<Owner>()
        .expect("owners")
        .require(Id::new(2))
        .expect("two");

    let mut conflict_tx = cfmd::Transaction::new();
    owner2
        .assets
        .attach(&mut conflict_tx, Id::new(10))
        .expect("proposal");
    assert!(
        database.preview(&conflict_tx).is_err(),
        "OwnedMany must reject a second owner"
    );

    let mut move_tx = cfmd::Transaction::new();
    owner1
        .assets
        .move_to(&mut move_tx, Id::new(10), &owner2.assets)
        .expect("move proposal");
    drop(owner1);
    drop(owner2);
    drop(snapshot);
    database.commit(&move_tx).expect("move");

    let snapshot = database.snapshot().expect("snapshot");
    let owner1 = snapshot
        .objects::<Owner>()
        .expect("owners")
        .require(Id::new(1))
        .expect("one");
    let owner2 = snapshot
        .objects::<Owner>()
        .expect("owners")
        .require(Id::new(2))
        .expect("two");
    assert_eq!(owner1.assets.count().expect("one count"), 0);
    assert_eq!(owner2.assets.count().expect("two count"), 1);
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove db");
}

#[test]
fn owned_many_delete_if_unowned_applies_after_owner_lifecycle_cleanup() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<Owner>()
        .object::<Asset>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("db");
    let snapshot = database.snapshot().expect("snapshot");
    let plan = snapshot
        .objects::<Owner>()
        .expect("owners")
        .insert(Owner::cfmd_new(
            Id::new(1),
            "one".into(),
            cfmd::OwnedMany::new([Asset {
                id: Id::new(10),
                label: "ephemeral".into(),
            }]),
        ))
        .expect("seed");
    drop(snapshot);
    database
        .commit_plan(&plan, TransactionId::new(343_010))
        .expect("seed commit");

    let snapshot = database.snapshot().expect("snapshot");
    let delete = snapshot
        .objects::<Owner>()
        .expect("owners")
        .where_(|owner| owner.id().eq(Id::new(1)))
        .delete_plan()
        .expect("delete owner");
    drop(snapshot);
    database
        .commit_plan(&delete, TransactionId::new(343_011))
        .expect("delete commit");

    let snapshot = database.snapshot().expect("snapshot");
    assert!(
        snapshot
            .objects::<Asset>()
            .expect("assets")
            .get(Id::new(10))
            .expect("lookup")
            .is_none()
    );
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove db");
}

#[test]
fn many_selection_moves_only_matching_edges_without_materializing_targets() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("database");
    seed_object_many(&database);

    let snapshot = database.snapshot().expect("snapshot");
    let parents = snapshot.objects::<Parent>().expect("parents");
    let source = parents.require(Id::new(1)).expect("source");
    let destination = parents.require(Id::new(2)).expect("destination");
    let selected = source
        .children
        .where_(|child| child.score().eq(9))
        .expect("selection");
    assert_eq!(
        selected.ids().expect("identity projection"),
        vec![Id::new(11)]
    );
    let mut transaction = cfmd::Transaction::new();
    selected
        .move_to(&mut transaction, &destination.children)
        .expect("filtered edge move");
    let mut invalid = cfmd::Transaction::new();
    assert!(
        source
            .children
            .move_ids_to(&mut invalid, [Id::new(999)], &destination.children)
            .is_err(),
        "move-by-id must not degrade into attach when the source edge is absent"
    );
    drop(selected);
    drop(source);
    drop(destination);
    drop(parents);
    drop(snapshot);
    database.commit(&transaction).expect("move commit");

    let snapshot = database.snapshot().expect("snapshot");
    let parents = snapshot.objects::<Parent>().expect("parents");
    assert_eq!(
        parents
            .require(Id::new(1))
            .expect("source")
            .children
            .count()
            .expect("source count"),
        1
    );
    assert_eq!(
        parents
            .require(Id::new(2))
            .expect("destination")
            .children
            .count()
            .expect("destination count"),
        2
    );
    assert_eq!(
        snapshot
            .objects::<Child>()
            .expect("children")
            .all()
            .expect("all")
            .len(),
        3,
        "moving an edge must not rewrite/delete target objects"
    );
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn owned_selection_preview_exposes_orphan_deletion_before_commit() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<Owner>()
        .object::<Asset>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("database");
    let snapshot = database.snapshot().expect("snapshot");
    let plan = snapshot
        .objects::<Owner>()
        .expect("owners")
        .insert(Owner::cfmd_new(
            Id::new(1),
            "one".into(),
            cfmd::OwnedMany::new([
                Asset {
                    id: Id::new(10),
                    label: "keep".into(),
                },
                Asset {
                    id: Id::new(11),
                    label: "drop".into(),
                },
            ]),
        ))
        .expect("seed");
    drop(snapshot);
    database
        .commit_plan(&plan, TransactionId::new(344_010))
        .expect("seed commit");

    let snapshot = database.snapshot().expect("snapshot");
    let owner = snapshot
        .objects::<Owner>()
        .expect("owners")
        .require(Id::new(1))
        .expect("owner");
    let selection = owner
        .assets
        .where_(|asset| asset.label().eq("drop".to_owned()))
        .expect("selection");
    let mut transaction = cfmd::Transaction::new();
    selection
        .detach_all(&mut transaction)
        .expect("detach proposal");
    let preview = database.preview(&transaction).expect("preview");
    assert_eq!(preview.derived().orphan_entities_deleted(), 1);
    assert!(preview.derived().normalized_rows_removed() >= 1);
    drop(selection);
    drop(owner);
    drop(snapshot);
    database.commit(&transaction).expect("detach commit");

    let snapshot = database.snapshot().expect("post-commit snapshot");
    assert!(
        snapshot
            .objects::<Asset>()
            .expect("candidate assets")
            .get(Id::new(11))
            .expect("candidate lookup")
            .is_none(),
        "orphan deletion must be published with the relationship detach"
    );
    assert!(
        snapshot
            .objects::<Asset>()
            .expect("candidate assets")
            .get(Id::new(10))
            .expect("candidate lookup")
            .is_some()
    );

    let snapshot = database.snapshot().expect("snapshot");
    let owner = snapshot
        .objects::<Owner>()
        .expect("owners")
        .require(Id::new(1))
        .expect("owner");
    assert_eq!(owner.assets.count().expect("asset count"), 1);
    assert!(
        snapshot
            .objects::<Asset>()
            .expect("assets")
            .get(Id::new(11))
            .expect("lookup")
            .is_none()
    );
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn object_order_predicates_use_declared_gamma_ordering() {
    let path = temp_path();
    let schema = Schema::builder().object::<Child>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let snapshot = database.snapshot().expect("snapshot");
    let children = snapshot.objects::<Child>().expect("children");
    let plan = children
        .insert(Child {
            id: Id::new(1),
            score: 3,
        })
        .expect("first")
        .and(
            children
                .insert(Child {
                    id: Id::new(2),
                    score: 7,
                })
                .expect("second"),
        )
        .expect("compose")
        .and(
            children
                .insert(Child {
                    id: Id::new(3),
                    score: 11,
                })
                .expect("third"),
        )
        .expect("compose");
    drop(children);
    drop(snapshot);
    database
        .commit_plan(&plan, TransactionId::new(352_001))
        .expect("seed");

    let snapshot = database.snapshot().expect("snapshot");
    let children = snapshot.objects::<Child>().expect("children");
    let ids = |values: Vec<Child>| {
        values
            .into_iter()
            .map(|child| child.id.raw())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(children
            .where_(|child| child.score().greater_than(3))
            .all()
            .expect("gt")),
        vec![2, 3]
    );
    assert_eq!(
        ids(children
            .where_(|child| child.score().greater_than_or_equal(7))
            .all()
            .expect("ge")),
        vec![2, 3]
    );
    assert_eq!(
        ids(children
            .where_(|child| child.score().less_than(7))
            .all()
            .expect("lt")),
        vec![1]
    );
    assert_eq!(
        ids(children
            .where_(|child| child.score().less_than_or_equal(7))
            .all()
            .expect("le")),
        vec![1, 2]
    );
    assert_eq!(
        ids(children
            .where_(|child| child.score().between(4, 10))
            .all()
            .expect("between")),
        vec![2]
    );
    drop(children);
    drop(snapshot);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn object_query_composition_and_ordered_boundaries_stay_gamma_native() {
    let path = temp_path();
    let schema = Schema::builder().object::<Child>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut seed = Transaction::new();
    for (id, score) in [(1, 3), (2, 7), (3, 7), (4, 11)] {
        database
            .objects::<Child>()
            .expect("children")
            .add(
                &mut seed,
                Child {
                    id: Id::new(id),
                    score,
                },
            )
            .expect("seed child");
    }
    database.commit(&seed).expect("seed commit");

    let ids = |rows: Vec<Child>| {
        let mut ids = rows
            .into_iter()
            .map(|child| child.id.raw())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    };
    let children = database.objects::<Child>().expect("children");
    assert_eq!(
        ids(children
            .where_(|child| {
                child
                    .score()
                    .greater_than(3)
                    .and(child.score().less_than_or_equal(7))
            })
            .all()
            .expect("conjunction")),
        vec![2, 3]
    );
    assert_eq!(
        ids(children
            .top(2, |child| child.score())
            .all()
            .expect("top boundary")),
        vec![2, 3, 4]
    );
    assert_eq!(
        ids(children
            .bottom(2, |child| child.score())
            .all()
            .expect("bottom boundary")),
        vec![1, 2, 3]
    );

    let mut watch = children
        .top(2, |child| child.score())
        .watch()
        .expect("boundary watch");
    assert_eq!(ids(watch.initial().to_vec()), vec![2, 3, 4]);
    drop(children);

    let mut update = Transaction::new();
    database
        .objects::<Child>()
        .expect("children")
        .add(
            &mut update,
            Child {
                id: Id::new(5),
                score: 20,
            },
        )
        .expect("new maximum");
    database.commit(&update).expect("commit new maximum");
    let event = watch.try_recv().expect("boundary event").expect("event");
    assert_eq!(ids(event.inserted().to_vec()), vec![5]);
    assert_eq!(ids(event.removed().to_vec()), vec![2, 3]);

    drop(watch);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn field_equality_uses_kernel_filter_eq_columns_for_exact_candidate_and_watch() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<ColumnPair>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut seed = Transaction::new();
    for (id, left, right) in [(1, 3, 3), (2, 3, 4), (3, 7, 7)] {
        database
            .objects::<ColumnPair>()
            .expect("pairs")
            .add(
                &mut seed,
                ColumnPair {
                    id: Id::new(id),
                    left,
                    right,
                },
            )
            .expect("seed pair");
    }
    database.commit(&seed).expect("seed commit");

    let query = database
        .objects::<ColumnPair>()
        .expect("pairs")
        .where_(|row| row.left().eq(row.right()));
    let ids = |rows: Vec<ColumnPair>| {
        rows.into_iter()
            .map(|row| row.id.raw())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(query.all().expect("exact field equality")), vec![1, 3]);
    assert_eq!(
        ids(database
            .objects::<ColumnPair>()
            .expect("pairs")
            .where_(|row| row.left().ne(row.right()))
            .all()
            .expect("exact field inequality")),
        vec![2]
    );

    let future = database
        .objects::<ColumnPair>()
        .expect("pairs")
        .insert(ColumnPair {
            id: Id::new(4),
            left: 9,
            right: 9,
        })
        .expect("future pair")
        .candidate()
        .expect("candidate");
    assert_eq!(
        ids(future
            .objects::<ColumnPair>()
            .expect("candidate pairs")
            .where_(|row| row.left().eq(row.right()))
            .all()
            .expect("candidate field equality")),
        vec![1, 3, 4]
    );

    let mut watch = query.watch().expect("field equality watch");
    assert_eq!(ids(watch.initial().to_vec()), vec![1, 3]);

    let mut matching = Transaction::new();
    database
        .objects::<ColumnPair>()
        .expect("pairs")
        .add(
            &mut matching,
            ColumnPair {
                id: Id::new(4),
                left: 9,
                right: 9,
            },
        )
        .expect("matching pair");
    database.commit(&matching).expect("matching commit");
    let event = watch.try_recv().expect("watch read").expect("matching event");
    assert_eq!(ids(event.inserted().to_vec()), vec![4]);
    assert!(event.removed().is_empty());

    let mut nonmatching = Transaction::new();
    database
        .objects::<ColumnPair>()
        .expect("pairs")
        .add(
            &mut nonmatching,
            ColumnPair {
                id: Id::new(5),
                left: 10,
                right: 11,
            },
        )
        .expect("nonmatching pair");
    database.commit(&nonmatching).expect("nonmatching commit");
    assert!(watch.try_recv().expect("nonmatching watch read").is_none());

    drop(watch);
    drop(query);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn predicate_or_uses_native_gamma_union_for_exact_candidate_and_watch() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<ColumnPair>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut seed = Transaction::new();
    for (id, left, right) in [(1, 3, 3), (2, 3, 4), (3, 7, 7), (4, 8, 9)] {
        database
            .objects::<ColumnPair>()
            .expect("pairs")
            .add(
                &mut seed,
                ColumnPair {
                    id: Id::new(id),
                    left,
                    right,
                },
            )
            .expect("seed pair");
    }
    database.commit(&seed).expect("seed commit");

    let query = database
        .objects::<ColumnPair>()
        .expect("pairs")
        .where_(|row| row.left().eq(row.right()).or(row.left().eq(3)));
    let ids = |rows: Vec<ColumnPair>| {
        let mut ids = rows.into_iter().map(|row| row.id.raw()).collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    };
    assert_eq!(ids(query.all().expect("exact disjunction")), vec![1, 2, 3]);

    let future = database
        .objects::<ColumnPair>()
        .expect("pairs")
        .insert(ColumnPair {
            id: Id::new(5),
            left: 3,
            right: 10,
        })
        .expect("future pair")
        .candidate()
        .expect("candidate");
    assert_eq!(
        ids(future
            .objects::<ColumnPair>()
            .expect("candidate pairs")
            .where_(|row| row.left().eq(row.right()).or(row.left().eq(3)))
            .all()
            .expect("candidate disjunction")),
        vec![1, 2, 3, 5]
    );

    let mut watch = query.watch().expect("disjunction watch");
    assert_eq!(ids(watch.initial().to_vec()), vec![1, 2, 3]);

    let mut matching = Transaction::new();
    database
        .objects::<ColumnPair>()
        .expect("pairs")
        .add(
            &mut matching,
            ColumnPair {
                id: Id::new(5),
                left: 9,
                right: 9,
            },
        )
        .expect("matching pair");
    database.commit(&matching).expect("matching commit");
    let event = watch.try_recv().expect("watch read").expect("union event");
    assert_eq!(ids(event.inserted().to_vec()), vec![5]);
    assert!(event.removed().is_empty());

    let mut nonmatching = Transaction::new();
    database
        .objects::<ColumnPair>()
        .expect("pairs")
        .add(
            &mut nonmatching,
            ColumnPair {
                id: Id::new(6),
                left: 10,
                right: 11,
            },
        )
        .expect("nonmatching pair");
    database.commit(&nonmatching).expect("nonmatching commit");
    assert!(watch.try_recv().expect("nonmatching watch read").is_none());

    drop(watch);
    drop(query);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn projection_and_group_keys_do_not_stop_at_three_columns() {
    let path = temp_path();
    let schema = Schema::builder().object::<WideRow>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut seed = Transaction::new();
    for (id, a, b, c) in [(1, 10, 20, 30), (2, 10, 20, 30)] {
        database
            .objects::<WideRow>()
            .expect("rows")
            .add(
                &mut seed,
                WideRow {
                    id: Id::new(id),
                    a,
                    b,
                    c,
                },
            )
            .expect("seed row");
    }
    database.commit(&seed).expect("seed commit");

    let rows = database.objects::<WideRow>().expect("rows");
    let projected = rows
        .select(|row| (row.id(), row.a(), row.b(), row.c()))
        .all()
        .expect("four-column projection");
    assert_eq!(projected.len(), 2);

    let grouped = rows
        .group_by(|row| (row.id(), row.a(), row.b(), row.c()))
        .count()
        .all()
        .expect("four-column group key");
    assert_eq!(grouped.len(), 2);
    assert!(grouped.iter().all(|(_, count)| *count == 1));

    let homogeneous = rows
        .select(|row| [row.a(), row.b(), row.c()])
        .all()
        .expect("array projection");
    assert_eq!(homogeneous, vec![[10, 20, 30], [10, 20, 30]]);

    drop(rows);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn projection_preserves_multiplicity_and_distinct_is_explicit_kernel_semantics() {
    let path = temp_path();
    let schema = Schema::builder().object::<Child>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut seed = Transaction::new();
    for (id, score) in [(1, 3), (2, 7), (3, 7), (4, 11)] {
        database
            .objects::<Child>()
            .expect("children")
            .add(&mut seed, Child { id: Id::new(id), score })
            .expect("seed child");
    }
    database.commit(&seed).expect("seed commit");

    let children = database.objects::<Child>().expect("children");
    let projection = children.select(|child| child.score());
    let mut scores = projection.all().expect("projected scores");
    scores.sort_unstable();
    assert_eq!(scores, vec![3, 7, 7, 11]);
    assert_eq!(projection.count().expect("projection count"), 4);

    let distinct = children.select(|child| child.score()).distinct();
    let mut unique_scores = distinct.all().expect("distinct scores");
    unique_scores.sort_unstable();
    assert_eq!(unique_scores, vec![3, 7, 11]);
    assert_eq!(distinct.count().expect("distinct count"), 3);

    let mut grouped = children
        .group_by(|child| child.score())
        .count()
        .all()
        .expect("grouped count");
    grouped.sort_unstable_by_key(|(score, _)| *score);
    assert_eq!(grouped, vec![(3, 1), (7, 2), (11, 1)]);

    let mut watch = projection.watch().expect("projection watch");
    let mut initial = watch.initial().to_vec();
    initial.sort_unstable();
    assert_eq!(initial, vec![3, 7, 7, 11]);

    let mut duplicate = Transaction::new();
    database
        .objects::<Child>()
        .expect("children")
        .add(
            &mut duplicate,
            Child {
                id: Id::new(5),
                score: 7,
            },
        )
        .expect("duplicate projected value");
    database.commit(&duplicate).expect("duplicate commit");
    let event = watch.try_recv().expect("projection event").expect("event");
    assert_eq!(event.inserted(), &[7]);
    assert!(event.removed().is_empty());

    let mut distinct_watch = children
        .select(|child| child.score())
        .distinct()
        .watch()
        .expect("distinct projection watch");
    let mut distinct_initial = distinct_watch.initial().to_vec();
    distinct_initial.sort_unstable();
    assert_eq!(distinct_initial, vec![3, 7, 11]);

    let mut another_duplicate = Transaction::new();
    database
        .objects::<Child>()
        .expect("children")
        .add(
            &mut another_duplicate,
            Child {
                id: Id::new(6),
                score: 7,
            },
        )
        .expect("another duplicate projected value");
    database
        .commit(&another_duplicate)
        .expect("another duplicate commit");
    assert!(
        distinct_watch
            .try_recv()
            .expect("distinct duplicate event")
            .is_none(),
        "explicit Distinct must suppress an additional equivalent projection"
    );

    drop(distinct_watch);
    drop(watch);
    drop(children);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn grouped_count_and_exact_sum_use_kernel_group_for_live_and_candidate_worlds() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<GroupedMetric>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut seed = Transaction::new();
    for (id, bucket, value) in [(1, 1, 1.25), (2, 1, 2.75), (3, 2, 10.0)] {
        database
            .objects::<GroupedMetric>()
            .expect("metrics")
            .add(
                &mut seed,
                GroupedMetric {
                    id: Id::new(id),
                    bucket,
                    value,
                },
            )
            .expect("seed metric");
    }
    database.commit(&seed).expect("seed commit");

    let metrics = database.objects::<GroupedMetric>().expect("metrics");
    let count_query = metrics.group_by(|metric| metric.bucket()).count();
    let mut counts = count_query.all().expect("group counts");
    counts.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(counts, vec![(1, 2), (2, 1)]);

    let sum_query = metrics
        .group_by(|metric| metric.bucket())
        .sum(|metric| metric.value());
    let mut sums = sum_query.all().expect("group sums");
    sums.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(sums, vec![(1, 4.0), (2, 10.0)]);

    let mut count_watch = count_query.watch().expect("group count watch");
    let mut count_initial = count_watch.initial().to_vec();
    count_initial.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(count_initial, vec![(1, 2), (2, 1)]);

    let mut sum_watch = sum_query.watch().expect("group sum watch");
    let mut sum_initial = sum_watch.initial().to_vec();
    sum_initial.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(sum_initial, vec![(1, 4.0), (2, 10.0)]);

    let future = metrics
        .insert(GroupedMetric {
            id: Id::new(4),
            bucket: 1,
            value: 0.5,
        })
        .expect("future metric plan");
    let candidate = future.candidate().expect("candidate");
    let candidate_metrics = candidate.objects::<GroupedMetric>().expect("candidate metrics");

    let mut future_counts = candidate_metrics
        .group_by(|metric| metric.bucket())
        .count()
        .all()
        .expect("candidate group counts");
    future_counts.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(future_counts, vec![(1, 3), (2, 1)]);

    let mut future_sums = candidate_metrics
        .group_by(|metric| metric.bucket())
        .sum(|metric| metric.value())
        .all()
        .expect("candidate group sums");
    future_sums.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(future_sums, vec![(1, 4.5), (2, 10.0)]);

    let mut live_insert = Transaction::new();
    database
        .objects::<GroupedMetric>()
        .expect("live metrics")
        .add(
            &mut live_insert,
            GroupedMetric {
                id: Id::new(4),
                bucket: 1,
                value: 0.5,
            },
        )
        .expect("live aggregate update");
    database.commit(&live_insert).expect("live aggregate commit");

    let count_event = count_watch
        .try_recv()
        .expect("count watch event")
        .expect("count event");
    assert_eq!(count_event.removed(), &[(1, 2)]);
    assert_eq!(count_event.inserted(), &[(1, 3)]);

    let sum_event = sum_watch
        .try_recv()
        .expect("sum watch event")
        .expect("sum event");
    assert_eq!(sum_event.removed(), &[(1, 4.0)]);
    assert_eq!(sum_event.inserted(), &[(1, 4.5)]);

    drop(metrics);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn grouped_aggregate_boundaries_preserve_ties_for_exact_candidate_and_watch() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<GroupedMetric>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut seed = Transaction::new();
    for (id, bucket, value) in [
        (1, 1, 1.0),
        (2, 1, 3.0),
        (3, 2, 1.5),
        (4, 2, 2.5),
        (5, 3, 1.0),
    ] {
        database
            .objects::<GroupedMetric>()
            .expect("metrics")
            .add(
                &mut seed,
                GroupedMetric {
                    id: Id::new(id),
                    bucket,
                    value,
                },
            )
            .expect("seed metric");
    }
    database.commit(&seed).expect("seed commit");

    let metrics = database.objects::<GroupedMetric>().expect("metrics");
    let count_top = metrics.group_by(|metric| metric.bucket()).count().top(1);
    let mut top_counts = count_top.all().expect("top grouped counts");
    top_counts.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(top_counts, vec![(1, 2), (2, 2)]);
    assert_eq!(
        metrics
            .group_by(|metric| metric.bucket())
            .count()
            .bottom(1)
            .all()
            .expect("bottom grouped count"),
        vec![(3, 1)]
    );

    let sum_top = metrics
        .group_by(|metric| metric.bucket())
        .sum(|metric| metric.value())
        .top(1);
    let mut top_sums = sum_top.all().expect("top grouped sums");
    top_sums.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(top_sums, vec![(1, 4.0), (2, 4.0)]);

    let mut count_watch = count_top.watch().expect("top count watch");
    let mut count_initial = count_watch.initial().to_vec();
    count_initial.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(count_initial, vec![(1, 2), (2, 2)]);

    let mut sum_watch = sum_top.watch().expect("top sum watch");
    let mut sum_initial = sum_watch.initial().to_vec();
    sum_initial.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(sum_initial, vec![(1, 4.0), (2, 4.0)]);

    let future = metrics
        .insert(GroupedMetric {
            id: Id::new(6),
            bucket: 3,
            value: 3.0,
        })
        .expect("future metric plan");
    let candidate = future.candidate().expect("candidate");
    let candidate_metrics = candidate.objects::<GroupedMetric>().expect("candidate metrics");
    let mut candidate_counts = candidate_metrics
        .group_by(|metric| metric.bucket())
        .count()
        .top(1)
        .all()
        .expect("candidate top counts");
    candidate_counts.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(candidate_counts, vec![(1, 2), (2, 2), (3, 2)]);
    let mut candidate_sums = candidate_metrics
        .group_by(|metric| metric.bucket())
        .sum(|metric| metric.value())
        .top(1)
        .all()
        .expect("candidate top sums");
    candidate_sums.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(candidate_sums, vec![(1, 4.0), (2, 4.0), (3, 4.0)]);

    let mut live_insert = Transaction::new();
    database
        .objects::<GroupedMetric>()
        .expect("live metrics")
        .add(
            &mut live_insert,
            GroupedMetric {
                id: Id::new(6),
                bucket: 3,
                value: 3.0,
            },
        )
        .expect("live aggregate boundary update");
    database.commit(&live_insert).expect("live aggregate commit");

    let count_event = count_watch
        .try_recv()
        .expect("top count watch event")
        .expect("top count event");
    assert!(count_event.removed().is_empty());
    assert_eq!(count_event.inserted(), &[(3, 2)]);

    let sum_event = sum_watch
        .try_recv()
        .expect("top sum watch event")
        .expect("top sum event");
    assert!(sum_event.removed().is_empty());
    assert_eq!(sum_event.inserted(), &[(3, 4.0)]);

    drop(metrics);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn ordered_object_watch_maintains_exact_delta_without_empty_events() {
    let path = temp_path();
    let schema = Schema::builder().object::<Child>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("db");

    let snapshot = database.snapshot().expect("snapshot");
    let children = snapshot.objects::<Child>().expect("children");
    let seed = children
        .insert(Child {
            id: Id::new(1),
            score: 7,
        })
        .expect("seed");
    drop(children);
    drop(snapshot);
    database
        .commit_plan(&seed, TransactionId::new(352_010))
        .expect("seed commit");

    let snapshot = database.snapshot().expect("snapshot");
    let query = snapshot
        .objects::<Child>()
        .expect("children")
        .where_(|child| child.score().greater_than(5));
    let mut watch = query.watch().expect("watch");
    assert_eq!(
        watch
            .initial()
            .iter()
            .map(|child| child.id.raw())
            .collect::<Vec<_>>(),
        vec![1]
    );
    drop(query);
    drop(snapshot);

    let snapshot = database.snapshot().expect("snapshot");
    let below = snapshot
        .objects::<Child>()
        .expect("children")
        .insert(Child {
            id: Id::new(2),
            score: 4,
        })
        .expect("below");
    drop(snapshot);
    database
        .commit_plan(&below, TransactionId::new(352_011))
        .expect("below commit");
    assert!(watch.try_recv().expect("drain below").is_none());

    let snapshot = database.snapshot().expect("snapshot");
    let above = snapshot
        .objects::<Child>()
        .expect("children")
        .insert(Child {
            id: Id::new(3),
            score: 9,
        })
        .expect("above");
    drop(snapshot);
    database
        .commit_plan(&above, TransactionId::new(352_012))
        .expect("above commit");
    let event = watch.try_recv().expect("event").expect("relevant event");
    assert_eq!(
        event
            .inserted()
            .iter()
            .map(|child| child.id.raw())
            .collect::<Vec<_>>(),
        vec![3]
    );
    assert!(event.removed().is_empty());

    drop(watch);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn ordered_f64_predicates_follow_total_order_hostile_edges() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<OrderedF64>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("database");

    let mut seed = Transaction::new();
    let values = [
        (1, f64::NEG_INFINITY),
        (2, -0.0),
        (3, 0.0),
        (4, f64::INFINITY),
        (5, f64::NAN),
    ];
    for (id, value) in values {
        database
            .objects::<OrderedF64>()
            .expect("ordered values")
            .add(
                &mut seed,
                OrderedF64 {
                    id: Id::new(id),
                    value,
                },
            )
            .expect("seed value");
    }
    database.commit(&seed).expect("seed commit");

    let ids = |mut rows: Vec<OrderedF64>| {
        let mut ids = rows.drain(..).map(|row| row.id.raw()).collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    };
    let objects = database.objects::<OrderedF64>().expect("ordered values");
    assert_eq!(
        ids(objects
            .where_(|row| row.value().less_than(-0.0))
            .all()
            .expect("below negative zero")),
        vec![1]
    );
    assert_eq!(
        ids(objects
            .where_(|row| row.value().between(-0.0, 0.0))
            .all()
            .expect("signed zero range")),
        vec![2, 3]
    );
    assert_eq!(
        ids(objects
            .where_(|row| row.value().greater_than(-0.0))
            .all()
            .expect("above negative zero")),
        vec![3, 4, 5]
    );
    assert_eq!(
        ids(objects
            .where_(|row| row.value().greater_than(f64::INFINITY))
            .all()
            .expect("above infinity")),
        vec![5]
    );

    drop(objects);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn ordered_f64_watch_matches_exact_total_order_and_suppresses_empty_revisions() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<OrderedF64>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("database");

    let mut seed = Transaction::new();
    database
        .objects::<OrderedF64>()
        .expect("ordered values")
        .add(
            &mut seed,
            OrderedF64 {
                id: Id::new(10),
                value: f64::INFINITY,
            },
        )
        .expect("seed infinity");
    database.commit(&seed).expect("seed commit");

    let query = database
        .objects::<OrderedF64>()
        .expect("ordered values")
        .where_(|row| row.value().greater_than(0.0));
    let mut watch = query.watch().expect("watch");
    assert_eq!(watch.initial()[0].id.raw(), 10);

    let mut nonmatching = Transaction::new();
    database
        .objects::<OrderedF64>()
        .expect("ordered values")
        .add(
            &mut nonmatching,
            OrderedF64 {
                id: Id::new(11),
                value: -0.0,
            },
        )
        .expect("negative zero");
    database.commit(&nonmatching).expect("nonmatching commit");
    assert!(watch.try_recv().expect("nonmatching watch read").is_none());

    let mut matching = Transaction::new();
    database
        .objects::<OrderedF64>()
        .expect("ordered values")
        .add(
            &mut matching,
            OrderedF64 {
                id: Id::new(12),
                value: f64::NAN,
            },
        )
        .expect("nan intent");
    database.commit(&matching).expect("nan commit");
    let event = watch
        .try_recv()
        .expect("watch read")
        .expect("matching event");
    assert_eq!(event.inserted().len(), 1);
    assert_eq!(event.inserted()[0].id.raw(), 12);
    assert!(event.removed().is_empty());

    drop(watch);
    drop(query);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn composite_group_keys_use_kernel_group_for_exact_candidate_and_watch() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<GroupedMetric>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut seed = Transaction::new();
    for (id, bucket, value) in [(1, 1, 10.0), (2, 1, 10.0), (3, 1, 20.0), (4, 2, 10.0)] {
        database
            .objects::<GroupedMetric>()
            .expect("metrics")
            .add(
                &mut seed,
                GroupedMetric {
                    id: Id::new(id),
                    bucket,
                    value,
                },
            )
            .expect("seed metric");
    }
    database.commit(&seed).expect("seed commit");

    let metrics = database.objects::<GroupedMetric>().expect("metrics");
    let grouped = metrics
        .group_by(|metric| (metric.bucket(), metric.value()))
        .count();
    let exact = grouped.all().expect("composite group counts");
    assert_eq!(exact.len(), 3);
    assert!(exact.contains(&((1, 10.0), 2)));
    assert!(exact.contains(&((1, 20.0), 1)));
    assert!(exact.contains(&((2, 10.0), 1)));

    let top_sum = metrics
        .group_by(|metric| (metric.bucket(), metric.value()))
        .sum(|metric| metric.value())
        .top(1);
    let top_sum_exact = top_sum.all().expect("composite top sums");
    assert_eq!(top_sum_exact.len(), 2);
    assert!(top_sum_exact.contains(&((1, 10.0), 20.0)));
    assert!(top_sum_exact.contains(&((1, 20.0), 20.0)));

    let mut watch = grouped.watch().expect("composite group watch");
    let mut sum_watch = top_sum.watch().expect("composite top sum watch");
    let candidate = metrics
        .insert(GroupedMetric {
            id: Id::new(5),
            bucket: 2,
            value: 10.0,
        })
        .expect("future metric plan")
        .candidate()
        .expect("candidate");
    let future = candidate
        .objects::<GroupedMetric>()
        .expect("candidate metrics")
        .group_by(|metric| (metric.bucket(), metric.value()))
        .count()
        .all()
        .expect("candidate composite counts");
    assert!(future.contains(&((2, 10.0), 2)));
    let future_top_sum = candidate
        .objects::<GroupedMetric>()
        .expect("candidate metrics")
        .group_by(|metric| (metric.bucket(), metric.value()))
        .sum(|metric| metric.value())
        .top(1)
        .all()
        .expect("candidate composite top sums");
    assert_eq!(future_top_sum.len(), 3);
    assert!(future_top_sum.contains(&((2, 10.0), 20.0)));

    let mut tx = Transaction::new();
    database
        .objects::<GroupedMetric>()
        .expect("live metrics")
        .add(
            &mut tx,
            GroupedMetric {
                id: Id::new(5),
                bucket: 2,
                value: 10.0,
            },
        )
        .expect("live insert");
    database.commit(&tx).expect("live commit");

    let event = watch
        .try_recv()
        .expect("composite group event")
        .expect("composite group delta");
    assert_eq!(event.removed(), &[((2, 10.0), 1)]);
    assert_eq!(event.inserted(), &[((2, 10.0), 2)]);

    let sum_event = sum_watch
        .try_recv()
        .expect("composite top sum event")
        .expect("composite top sum delta");
    assert!(sum_event.removed().is_empty());
    assert_eq!(sum_event.inserted(), &[((2, 10.0), 20.0)]);

    drop(metrics);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.path-region")]
struct PathRegion {
    #[cfmd(id)]
    pub id: Id<PathRegion>,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.path-passport")]
struct PathPassport {
    #[cfmd(id)]
    pub id: Id<PathPassport>,
    pub number: String,
    pub region: cfmd::Ref<PathRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.path-person")]
struct PathPerson {
    #[cfmd(id)]
    pub id: Id<PathPerson>,
    pub name: String,
    pub passport: cfmd::Ref<PathPassport>,
}

#[test]
fn deep_typed_reference_path_lowers_without_nested_matches_or_hidden_io() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<PathRegion>()
        .object::<PathPassport>()
        .object::<PathPerson>()
        .build()
        .expect("path schema");
    let db = Database::create(&path, schema).expect("create path database");

    let region_id = Id::new(1);
    let passport_id = Id::new(10);
    let person_id = Id::new(100);
    let snapshot = db.snapshot().expect("seed snapshot");
    let mut tx = Transaction::new();
    snapshot
        .objects::<PathRegion>()
        .expect("regions")
        .add(
            &mut tx,
            PathRegion {
                id: region_id,
                code: "RU".to_owned(),
            },
        )
        .expect("add region");
    snapshot
        .objects::<PathPassport>()
        .expect("passports")
        .add(
            &mut tx,
            PathPassport {
                id: passport_id,
                number: "42".to_owned(),
                region: cfmd::Ref::new(region_id),
            },
        )
        .expect("add passport");
    snapshot
        .objects::<PathPerson>()
        .expect("people")
        .add(
            &mut tx,
            PathPerson {
                id: person_id,
                name: "Artem".to_owned(),
                passport: cfmd::Ref::new(passport_id),
            },
        )
        .expect("add person");
    db.commit(&tx).expect("commit path fixture");
    drop(snapshot);

    let snapshot = db.snapshot().expect("path snapshot");
    let people = snapshot.objects::<PathPerson>().expect("people");
    let query = people.where_(|person| {
        person
            .passport()
            .region()
            .code()
            .eq("RU".to_owned())
            & person.name().eq("Artem".to_owned())
    });
    let selected = query.one().expect("deep path match");
    assert_eq!(selected.id, person_id);

    let none = people
        .where_(|person| {
            person
                .passport()
                .region()
                .code()
                .eq("NL".to_owned())
        })
        .all()
        .expect("non-matching deep path");
    assert!(none.is_empty());

    let ergonomic = people
        .where_(|person| {
            person
                .passport()
                .region()
                .code()
                .eq("NL".to_owned())
                | (!person.name().eq("Nobody".to_owned()))
        })
        .one()
        .expect("native operator predicate composition");
    assert_eq!(ergonomic.id, person_id);

    drop(snapshot);
    drop(db);
    fs::remove_file(path).expect("remove path database");
}

#[test]
fn same_relationship_attach_rebases_as_durable_residual_and_retries_by_client_intent() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("database");
    seed_object_many(&database);

    let mut add_parent = Transaction::new();
    database
        .objects::<Parent>()
        .expect("parents")
        .add(
            &mut add_parent,
            Parent::cfmd_new(Id::new(3), "zero".into(), cfmd::Many::empty()),
        )
        .expect("add empty parent");
    database.commit(&add_parent).expect("parent commit");

    let mut first = database
        .transaction_with_id(TransactionId::new(372_001))
        .expect("first transaction");
    let first_parent = first
        .objects::<Parent>()
        .expect("first parents")
        .require(Id::new(3))
        .expect("first parent");
    first_parent
        .children
        .attach(&mut first, Id::new(10))
        .expect("first attach");
    drop(first_parent);

    let mut second = database
        .transaction_with_id(TransactionId::new(372_002))
        .expect("second transaction");
    let second_parent = second
        .objects::<Parent>()
        .expect("second parents")
        .require(Id::new(3))
        .expect("second parent");
    second_parent
        .children
        .attach(&mut second, Id::new(10))
        .expect("second attach");
    drop(second_parent);

    database.commit(&first).expect("first commit");
    assert!(matches!(
        database.transaction_readiness(&second).expect("readiness"),
        cfmd::TransactionReadiness::Rebasable { .. }
    ));
    let residual_revision = match database
        .commit(&second)
        .expect("same attach must publish certified residual")
    {
        CommitOutcome::Committed { revision } => revision,
        CommitOutcome::AlreadyCommitted { .. } => panic!("first residual publication must commit"),
    };
    assert!(matches!(
        database.commit(&second).expect("retry by original client intent"),
        CommitOutcome::AlreadyCommitted { revision } if revision == residual_revision
    ));
    let history = database.history().expect("history");
    let residual_entry = history.latest().expect("residual history entry");
    assert_eq!(residual_entry.transaction(), TransactionId::new(372_002));
    assert!(
        residual_entry.changes().is_empty(),
        "causal/history replay must retain the realized no-op, not the original attach"
    );

    let parent = database
        .objects::<Parent>()
        .expect("parents")
        .require(Id::new(3))
        .expect("parent");
    assert_eq!(parent.children.count().expect("children count"), 1);
    drop(parent);
    drop(add_parent);
    drop(first);
    drop(second);
    drop(database);

    let reopened = Database::open(&path).expect("reopen residual database");
    let parent = reopened
        .objects::<Parent>()
        .expect("reopened parents")
        .require(Id::new(3))
        .expect("reopened parent");
    assert_eq!(parent.children.count().expect("reopened children count"), 1);
    drop(parent);
    drop(reopened);
    fs::remove_file(path).expect("remove database");
}


#[test]
fn database_owned_object_field_rules_reject_invalid_candidates_and_survive_reopen() {
    let path = temp_path();
    let schema = Schema::builder().object::<RuleUser>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut valid = Transaction::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut valid,
            RuleUser {
                id: Id::new(374_001),
                name: "Artem".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("valid intent");
    database.commit(&valid).expect("valid commit");

    let mut invalid = Transaction::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut invalid,
            RuleUser {
                id: Id::new(374_002),
                name: "X".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("invalid value may form an intent");
    let error = database.preview(&invalid).expect_err("rule must reject candidate");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(valid);
    drop(invalid);
    drop(database);
    let reopened = Database::builder(&path).open().expect("reopen database");
    let mut invalid_after_reopen = Transaction::new();
    reopened
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut invalid_after_reopen,
            RuleUser {
                id: Id::new(374_003),
                name: "No".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("invalid value may form an intent after reopen");
    let error = reopened
        .preview(&invalid_after_reopen)
        .expect_err("persisted rule must reject candidate after reopen");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(reopened);
    fs::remove_file(path).expect("remove database");
}


#[test]
fn transaction_require_is_evaluated_on_the_exact_future_candidate() {
    let path = temp_path();
    let schema = Schema::builder().object::<RuleUser>().build().expect("schema");
    let database = Database::builder(&path).schema(schema).create().expect("create database");
    let id = Id::new(446_001);

    let mut create = Transaction::new();
    database.objects::<RuleUser>().expect("users").add(&mut create, RuleUser {
        id,
        name: "Artem".to_owned(),
        age: 19,
        role: "user".to_owned(),
    }).expect("create intent");
    database.commit(&create).expect("create commit");

    let adult = SemanticRuleExpr::I64Range {
        value: RuleValueExpr::Field(RuleUser::__field_id("age")),
        min: Some(18),
        max: None,
    };
    let users = database.objects::<RuleUser>().expect("users");
    let mut invalid = Transaction::new();
    users.set(&mut invalid, id, |user| user.age(), 17).expect("patch intent");
    invalid.require::<RuleUser>(id, adult.clone()).expect("require intent");
    let error = database.preview(&invalid).expect_err("future-world requirement must reject");
    assert_eq!(error.kind(), ErrorKind::TransactionConflict);

    let mut valid = Transaction::new();
    users.set(&mut valid, id, |user| user.age(), 20).expect("valid patch intent");
    valid.require::<RuleUser>(id, adult).expect("valid require intent");
    database.preview(&valid).expect("valid future candidate");
    database.commit(&valid).expect("require-backed commit");
    assert_eq!(database.objects::<RuleUser>().expect("fresh users").require(id).expect("stored user").age, 20);

    drop(valid);
    drop(invalid);
    drop(create);
    drop(database);
    fs::remove_file(path).expect("remove database");
}


#[test]
fn typed_schema_root_binds_live_entity_sets_without_global_registration() {
    let path = temp_path();
    let db = AppSchema::database(&path).create().expect("typed create");

    assert_eq!(db.users.count().expect("empty users"), 0);
    assert_eq!(db.tasks.count().expect("empty tasks"), 0);

    let user = User {
        id: Id::new(7),
        name: "Artem".to_owned(),
    };
    let task = Task {
        id: Id::new(9),
        title: "typed schema".to_owned(),
        owner: cfmd::Ref::new(user.id),
        reviewer: None,
    };

    let mut tx = Transaction::new();
    db.users.add(&mut tx, user.clone()).expect("add user");
    db.tasks.add(&mut tx, task.clone()).expect("add task");
    db.commit(&tx).expect("commit typed schema transaction");

    assert_eq!(db.users.require(user.id).expect("read user"), user);
    assert_eq!(db.tasks.require(task.id).expect("read task"), task);
    drop(tx);
    drop(db);

    let reopened = AppSchema::database(&path).open().expect("typed reopen");
    assert_eq!(reopened.users.count().expect("reopened users"), 1);
    assert_eq!(reopened.tasks.count().expect("reopened tasks"), 1);
    drop(reopened);

    let mismatch = UserOnlySchema::database(&path)
        .open()
        .expect_err("typed open must reject a different persisted schema");
    assert_eq!(mismatch.kind(), ErrorKind::InvalidSchema);

    let broken_path = temp_path();
    let broken = BrokenTaskSchema::database(&broken_path)
        .create()
        .expect_err("missing referenced entity must fail closed");
    assert_eq!(broken.kind(), ErrorKind::InvalidSchema);

    let isolated_path = temp_path();
    let isolated = UserOnlySchema::database(&isolated_path)
        .create()
        .expect("same process may host a different explicit schema");
    assert_eq!(isolated.users.count().expect("isolated users"), 0);

    drop(isolated);
    let _ = fs::remove_dir_all(path);
    let _ = fs::remove_dir_all(broken_path);
    let _ = fs::remove_dir_all(isolated_path);
}

#[test]
fn partial_context_binds_by_semantic_fields_and_blocks_full_row_mutation() {
    let path = temp_path();
    let db = HostileAccountSchema::database(&path)
        .create()
        .expect("authoritative account schema");

    let account = HostileAccount {
        id: Id::new(378_001),
        name: "reader-visible".to_owned(),
        passport_secret: "SECRET".to_owned(),
        doctor_note: "editable".to_owned(),
    };
    let mut tx = Transaction::new();
    db.accounts
        .add(&mut tx, account.clone())
        .expect("authoritative insert");
    db.commit(&tx).expect("authoritative commit");

    // Raw object binding remains exact-shape by design. Partial contracts enter only through a
    // typed Context so low-level/tooling code cannot accidentally change semantics.
    let error = db
        .raw()
        .objects::<HostileReaderAccount>()
        .expect_err("raw object binding remains exact");
    assert_eq!(error.kind(), ErrorKind::TypeMismatch);

    let ctx = db
        .raw()
        .context::<HostileReaderSchema>()
        .expect("partial context binds by semantic field identity");
    let visible = ctx.accounts.all().expect("projected read");
    assert_eq!(
        visible,
        vec![HostileReaderAccount {
            id: Id::new(378_001),
            name: "reader-visible".to_owned(),
            doctor_note: "editable".to_owned(),
        }]
    );

    // A truncated Rust value is never interpreted as a replacement persisted row.
    let mut partial_tx = Transaction::new();
    let error = ctx
        .accounts
        .add(
            &mut partial_tx,
            HostileReaderAccount {
                id: Id::new(378_002),
                name: "unsafe-create".to_owned(),
                doctor_note: "would omit secret".to_owned(),
            },
        )
        .expect_err("partial create must fail before plan construction");
    assert_eq!(error.kind(), ErrorKind::InvalidPlan);

    let error = ctx
        .accounts
        .query()
        .expect("partial query")
        .update(&mut partial_tx, |mut row| {
            row.doctor_note = "changed".to_owned();
            row
        })
        .expect_err("partial full-row rewrite must fail closed");
    assert_eq!(error.kind(), ErrorKind::InvalidPlan);

    ctx.accounts
        .set(
            &mut partial_tx,
            Id::new(378_001),
            |account| account.doctor_note(),
            "changed-by-reader".to_owned(),
        )
        .expect("semantic scalar patch");
    ctx.commit(&partial_tx).expect("commit partial scalar patch");

    let stored = db.accounts.require(account.id).expect("authoritative reread");
    assert_eq!(stored.passport_secret, "SECRET");
    assert_eq!(stored.doctor_note, "changed-by-reader");

    // The reader descriptor deliberately does not repeat the authoritative field rule. The full
    // persisted candidate still owns validation and rejects a patch that violates it.
    let mut invalid_tx = Transaction::new();
    ctx.accounts
        .set(
            &mut invalid_tx,
            Id::new(378_001),
            |account| account.doctor_note(),
            "x".repeat(64),
        )
        .expect("reader can form a patch without knowing the hidden authoritative rule");
    let error = ctx
        .commit(&invalid_tx)
        .expect_err("persisted semantic rules must reject the invalid full candidate");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);
    let stored = db.accounts.require(account.id).expect("reread after rejected patch");
    assert_eq!(stored.passport_secret, "SECRET");
    assert_eq!(stored.doctor_note, "changed-by-reader");

    drop(invalid_tx);
    drop(partial_tx);
    drop(tx);
    drop(ctx);
    drop(db);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn client_bind_preserves_old_local_name_without_polluting_authoritative_schema() {
    let path = temp_path();
    let db = RenamedAccountSchema::database(&path)
        .create()
        .expect("authoritative renamed schema");

    let account = RenamedAccount {
        id: Id::new(379_001),
        name: "rename-safe".to_owned(),
        medical_note: "old-visible".to_owned(),
    };
    let mut create = Transaction::new();
    db.accounts
        .add(&mut create, account.clone())
        .expect("authoritative insert");
    db.commit(&create).expect("authoritative commit");
    drop(create);
    drop(db);

    // The authoritative schema has only `medical_note`; compatibility belongs to the client.
    let reopened = Database::open(&path).expect("raw reopen without Rust schema authority");
    let old = reopened
        .context::<LegacyRenameSchema>()
        .expect("legacy local field explicitly binds to the current persisted coordinate");
    let legacy = old.accounts.require(Id::new(379_001)).expect("legacy read");
    assert_eq!(legacy.doctor_note, "old-visible");

    let mut patch = Transaction::new();
    old.accounts
        .set(
            &mut patch,
            Id::new(379_001),
            |row| row.doctor_note(),
            "changed-through-old-name".to_owned(),
        )
        .expect("legacy patch resolves through the explicit client-side bind");
    old.commit(&patch).expect("legacy patch commit");

    let current = reopened
        .objects::<RenamedAccount>()
        .expect("authoritative shape remains exact and legacy-free")
        .require(account.id)
        .expect("authoritative reread");
    assert_eq!(current.medical_note, "changed-through-old-name");

    let mut invalid = Transaction::new();
    old.accounts
        .set(
            &mut invalid,
            Id::new(379_001),
            |row| row.doctor_note(),
            "x".repeat(64),
        )
        .expect("legacy client can form patch without copying authoritative rules");
    let error = old
        .commit(&invalid)
        .expect_err("persisted authoritative rule still governs the bound field");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(old);
    drop(reopened);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn client_bind_can_keep_old_reference_name_without_authoritative_alias() {
    let path = temp_path();
    let db = RenamedReferenceSchema::database(&path)
        .create()
        .expect("current authoritative reference schema");
    let doctor = RenameDoctor {
        id: Id::new(379_101),
        name: "Dr Semantic".to_owned(),
    };
    let patient = RenamedPatient {
        id: Id::new(379_102),
        primary_doctor: cfmd::Ref::new(doctor.id),
    };
    let mut tx = Transaction::new();
    db.doctors.add(&mut tx, doctor.clone()).expect("doctor insert");
    db.patients.add(&mut tx, patient).expect("patient insert");
    db.commit(&tx).expect("reference commit");
    drop(tx);
    drop(db);

    let reopened = Database::open(&path).expect("reopen");
    let legacy = reopened
        .context::<LegacyReferenceSchema>()
        .expect("old local reference name binds explicitly after reopen");
    let rows = legacy
        .patients
        .where_(|patient| patient.doctor().eq(doctor.id))
        .expect("legacy reference predicate")
        .all()
        .expect("legacy reference query");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].doctor.id(), doctor.id);

    drop(legacy);
    drop(reopened);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn partial_context_patches_required_and_optional_references_without_hidden_row_rewrite() {
    let path = temp_path();
    let db = AppSchema::database(&path).create().expect("authoritative app schema");
    let alice = User { id: Id::new(438_001), name: "Alice".into() };
    let bob = User { id: Id::new(438_002), name: "Bob".into() };
    let mut seed = Transaction::new();
    db.users.add(&mut seed, alice.clone()).expect("alice");
    db.users.add(&mut seed, bob.clone()).expect("bob");
    db.tasks
        .add(
            &mut seed,
            Task {
                id: Id::new(438_010),
                title: "hidden-title".into(),
                owner: cfmd::Ref::new(alice.id),
                reviewer: None,
            },
        )
        .expect("task");
    db.commit(&seed).expect("seed commit");

    let ctx = db.raw().context::<PartialTaskSchema>().expect("partial task context");
    let mut patch = Transaction::new();
    ctx.tasks
        .set(&mut patch, Id::new(438_010), |task| task.owner(), cfmd::Ref::new(bob.id))
        .expect("required reference patch");
    ctx.tasks
        .set(
            &mut patch,
            Id::new(438_010),
            |task| task.reviewer(),
            Some(cfmd::Ref::new(alice.id)),
        )
        .expect("optional reference patch");
    ctx.commit(&patch).expect("reference patch commit");

    let stored = db.tasks.require(Id::new(438_010)).expect("authoritative reread");
    assert_eq!(stored.title, "hidden-title");
    assert_eq!(stored.owner.id(), bob.id);
    assert_eq!(stored.reviewer.as_ref().map(cfmd::Ref::id), Some(alice.id));

    drop(ctx);
    drop(db);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn partial_context_many_mutation_preserves_hidden_owner_fields() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .build()
        .expect("relationship schema");
    let database = Database::builder(&path).schema(schema).create().expect("database");
    seed_object_many(&database);

    let ctx = database.context::<ParentReaderSchema>().expect("partial parent context");
    let parent = ctx.parents.require(Id::new(1)).expect("partial owner");
    assert_eq!(parent.children.count().expect("initial count"), 2);
    let mut tx = Transaction::new();
    parent.children.attach(&mut tx, Id::new(12)).expect("attach existing child");
    parent.children.detach(&mut tx, Id::new(10)).expect("detach child");
    ctx.commit(&tx).expect("relationship mutation commit");

    let snapshot = database.snapshot().expect("snapshot");
    let authoritative = snapshot
        .objects::<Parent>()
        .expect("parents")
        .require(Id::new(1))
        .expect("owner reread");
    assert_eq!(authoritative.name, "one");
    let mut ids = authoritative
        .children
        .all()
        .expect("children")
        .into_iter()
        .map(|child| child.id.raw())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    assert_eq!(ids, vec![11, 12]);

    drop(snapshot);
    drop(ctx);
    drop(database);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn context_field_coordinates_rebase_independent_fields_and_conflict_same_field() {
    let path = temp_path();
    let db = AppSchema::database(&path).create().expect("authoritative app schema");
    let alice = User { id: Id::new(438_101), name: "Alice".into() };
    let bob = User { id: Id::new(438_102), name: "Bob".into() };
    let mut seed = Transaction::new();
    db.users.add(&mut seed, alice.clone()).expect("alice");
    db.users.add(&mut seed, bob.clone()).expect("bob");
    db.tasks.add(&mut seed, Task {
        id: Id::new(438_110),
        title: "base".into(),
        owner: cfmd::Ref::new(alice.id),
        reviewer: None,
    }).expect("task");
    db.commit(&seed).expect("seed commit");

    let partial = db.raw().context::<PartialTaskSchema>().expect("partial task context");
    let mut title_tx = Transaction::new();
    db.tasks.set(&mut title_tx, Id::new(438_110), |task| task.title(), "title-a".to_string()).expect("title patch");
    let mut owner_tx = Transaction::new();
    partial.tasks.set(&mut owner_tx, Id::new(438_110), |task| task.owner(), cfmd::Ref::new(bob.id)).expect("owner patch");

    db.commit(&title_tx).expect("first independent field commit");
    partial.commit(&owner_tx).expect("stale independent field rebase");
    let stored = db.tasks.require(Id::new(438_110)).expect("reread");
    assert_eq!(stored.title, "title-a");
    assert_eq!(stored.owner.id(), bob.id);

    let mut left = Transaction::new();
    db.tasks.set(&mut left, Id::new(438_110), |task| task.title(), "left".to_string()).expect("left title");
    let mut right = Transaction::new();
    db.tasks.set(&mut right, Id::new(438_110), |task| task.title(), "right".to_string()).expect("right title");
    db.commit(&left).expect("left commit");
    let conflict = db.commit(&right).expect_err("same semantic field must conflict");
    assert_eq!(conflict.kind(), ErrorKind::TransactionConflict);

    let retry = partial.commit(&owner_tx).expect("committed field intent retry");
    assert!(matches!(retry, cfmd::CommitOutcome::AlreadyCommitted { .. }));

    drop(stored);
    drop(seed);
    drop(title_tx);
    drop(owner_tx);
    drop(left);
    drop(right);
    drop(partial);
    drop(db);
    let reopened = Database::open(&path).expect("reopen");
    let ctx = reopened.context::<AppSchema>().expect("reopen authoritative context");
    let stored = ctx.tasks.require(Id::new(438_110)).expect("reopen reread");
    assert_eq!(stored.title, "left");
    assert_eq!(stored.owner.id(), bob.id);
    drop(ctx);
    drop(reopened);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn relationship_authorization_preserves_semantic_actions() {
    use cfmd::{Object, Permission, PermissionSet, PrincipalId, Session};

    let path = temp_path();
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .build()
        .expect("relationship authorization schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("relationship authorization db");
    seed_object_many(&database);

    let relationship = Parent::many_fields()[0].relation();
    let mover = database.session(Session::new(
        PrincipalId::new(440_001),
        PermissionSet::from([Permission::Read, Permission::MoveRelationship(relationship)]),
    ));
    let parents = mover.objects::<Parent>().expect("parents");
    let source = parents.require(Id::new(1)).expect("source parent");
    let destination = parents.require(Id::new(2)).expect("destination parent");

    let mut move_tx = mover
        .transaction_with_id(TransactionId::new(440_001))
        .expect("move transaction");
    source
        .children
        .move_to(&mut move_tx, Id::new(10), &destination.children)
        .expect("semantic move planning");
    mover.commit(&move_tx).expect("move-only grant commit");

    let mut attach_tx = mover
        .transaction_with_id(TransactionId::new(440_002))
        .expect("attach transaction");
    destination
        .children
        .attach(&mut attach_tx, Id::new(11))
        .expect("attach planning remains structurally valid");
    assert_eq!(
        mover
            .commit(&attach_tx)
            .expect_err("move grant must not authorize relationship attach")
            .kind(),
        ErrorKind::PermissionDenied
    );

    drop(parents);
    drop(source);
    drop(destination);
    drop(database);
    fs::remove_file(path).expect("remove relationship authorization db");
}

#[test]
fn owned_detach_cannot_bypass_object_delete_authority() {
    use cfmd::{Object, Permission, PermissionSet, PrincipalId, Session};

    let path = temp_path();
    let schema = Schema::builder()
        .object::<Owner>()
        .object::<Asset>()
        .build()
        .expect("owned authorization schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("owned authorization db");
    let snapshot = database.snapshot().expect("seed snapshot");
    let owners = snapshot.objects::<Owner>().expect("owners");
    let seed = owners
        .insert(Owner::cfmd_new(
            Id::new(1),
            "one".into(),
            cfmd::OwnedMany::new([Asset {
                id: Id::new(10),
                label: "owned".into(),
            }]),
        ))
        .expect("owned seed");
    drop(owners);
    drop(snapshot);
    database
        .commit_plan(&seed, TransactionId::new(440_010))
        .expect("owned seed commit");

    let relationship = Owner::many_fields()[0].relation();
    let session = Session::new(
        PrincipalId::new(440_010),
        PermissionSet::from([Permission::Read, Permission::DetachRelationship(relationship)]),
    );
    let restricted = database.session(session.clone());
    let owner = restricted
        .objects::<Owner>()
        .expect("owners")
        .require(Id::new(1))
        .expect("owner");
    let mut tx = restricted
        .transaction_with_id(TransactionId::new(440_011))
        .expect("detach transaction");
    owner
        .assets
        .detach(&mut tx, Id::new(10))
        .expect("detach planning");
    assert_eq!(
        restricted
            .commit(&tx)
            .expect_err("orphan delete must require target delete authority")
            .kind(),
        ErrorKind::PermissionDenied
    );

    session
        .refresh_permissions(PermissionSet::from([
            Permission::Read,
            Permission::DetachRelationship(relationship),
            Permission::DeleteObject(Asset::relation_id()),
        ]))
        .expect("grant induced delete authority");
    restricted
        .commit(&tx)
        .expect("same semantic detach commits once induced delete is authorized");

    drop(owner);
    drop(database);
    fs::remove_file(path).expect("remove owned authorization db");
}
