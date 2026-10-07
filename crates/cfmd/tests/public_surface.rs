use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use cfmd::__private::{IntentJournal, TransactionId};
use cfmd::{
    CfmdEntity, CfmdSchema, CommitOutcome, Database, DiagnosticCode, EntitySet, ErrorDiagnosticExt,
    ErrorKind, Id, ModelRuleExpr, Object, ObjectPredicate, RecoveryAuthority, RecoveryOperation,
    RecoveryReason, RuleOrderComparison, RuleValueExpr, Schema, SemanticRuleExpr,
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

#[allow(dead_code)]
#[derive(CfmdSchema)]
#[cfmd(schema_revision = 539)]
struct VersionedTodoSchema {
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
#[cfmd(key = "example.binary-rule-task")]
struct BinaryRuleTask {
    #[cfmd(id)]
    pub id: Id<BinaryRuleTask>,
    pub minimum: i64,
    pub maximum: i64,
    pub enabled: bool,
    pub published: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.reference-rule-task")]
struct ReferenceRuleTask {
    #[cfmd(id)]
    pub id: Id<ReferenceRuleTask>,
    pub primary: cfmd::Ref<User>,
    pub secondary: cfmd::Ref<User>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.pattern-user")]
struct PatternUser {
    #[cfmd(id)]
    pub id: Id<PatternUser>,
    #[cfmd(matches = cfmd::TextPattern::concat([
        cfmd::TextPattern::literal("A"),
        cfmd::TextPattern::zero_or_more(cfmd::TextPattern::AnyScalar),
    ]))]
    pub name: String,
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

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.task")]
struct IdentityOnlyTask {
    #[cfmd(id)]
    id: Id<IdentityOnlyTask>,
}

#[derive(CfmdSchema)]
#[cfmd(schema_revision = 564)]
struct VersionedIdentityOnlyTaskSchema {
    tasks: EntitySet<IdentityOnlyTask>,
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
struct ParentReaderWriteSchema {
    parents: EntitySet<ParentReader>,
    children: EntitySet<Child>,
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

#[derive(CfmdSchema)]
struct OwnerSchema {
    owners: EntitySet<Owner>,
}

#[derive(Debug, Clone, PartialEq, CfmdEntity)]
#[cfmd(key = "example.ordered-f64")]
struct OrderedF64 {
    #[cfmd(id)]
    pub id: Id<OrderedF64>,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, CfmdEntity)]
#[cfmd(key = "example.filtered-metric")]
struct FilteredMetric {
    #[cfmd(id)]
    pub id: Id<FilteredMetric>,
    pub value: f64,
    pub lower: i64,
    pub upper: i64,
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

fn create_typed<S>(path: &std::path::Path) -> cfmd::Result<cfmd::Context<S>>
where
    S: cfmd::DatabaseDefinition,
{
    Database::builder(path)
        .create_authoritative::<S>()?
        .context::<S>()
}

fn open_typed<S>(path: &std::path::Path) -> cfmd::Result<cfmd::Context<S>>
where
    S: CfmdSchema,
{
    Database::open(path)?.context::<S>()
}

#[test]
fn schema_contract_can_carry_post_cutover_activation_revision() {
    assert_eq!(
        <VersionedTodoSchema as CfmdSchema>::contract_schema_revision(),
        Some(539)
    );
}

#[test]
fn authoritative_memory_database_persists_without_rebinding_typed_contexts() {
    let path = temp_path().with_extension("cfmd");
    let _ = fs::remove_file(&path);
    let database = Database::memory::<AppSchema>().expect("create authoritative memory database");
    let context = database
        .context::<AppSchema>()
        .expect("bind memory context");
    let revision = context
        .snapshot()
        .expect("memory context snapshot")
        .revision();

    database.persist(&path).expect("persist memory database");
    assert!(!database.is_memory());
    assert_eq!(
        context
            .snapshot()
            .expect("promoted context snapshot")
            .revision(),
        revision
    );

    drop(context);
    drop(database);
    let reopened = Database::open(&path).expect("reopen persisted typed database");
    reopened
        .context::<AppSchema>()
        .expect("typed contract remains valid after persistence transition");
    drop(reopened);
    fs::remove_file(path).expect("remove persisted typed database");
}

#[test]
fn adaptive_transaction_crud_keeps_database_and_payload_visible() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut transaction = IntentJournal::new();
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

    let mut seed = IntentJournal::new();
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
    let mut undo = IntentJournal::new();
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

    let mut redo = IntentJournal::new();
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

    let mut seed = IntentJournal::new();
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
    let mut transaction = IntentJournal::new();
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
fn scoped_context_keeps_one_admitted_formation_world_while_database_head_advances() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let context = database.context::<TodoSchema>().expect("context");
    let formation = context.formation_revision();
    let snapshot = context.snapshot().expect("formation snapshot");

    let mut external = IntentJournal::new();
    database
        .objects::<Todo>()
        .expect("live todos")
        .add(
            &mut external,
            Todo {
                id: Id::new(358_099),
                title: "after-admission".to_owned(),
                done: false,
            },
        )
        .expect("external add");
    database.commit(&external).expect("external commit");

    assert_eq!(context.formation_revision(), formation);
    assert_eq!(
        context.current_revision().expect("context revision"),
        formation
    );
    assert!(
        context
            .todos
            .get(Id::new(358_099))
            .expect("bounded context read")
            .is_none()
    );
    assert!(
        snapshot
            .todos
            .get(Id::new(358_099))
            .expect("snapshot read")
            .is_none()
    );
    assert!(
        database
            .context::<TodoSchema>()
            .expect("new context")
            .todos
            .get(Id::new(358_099))
            .expect("new scope read")
            .is_some()
    );

    drop(snapshot);
    drop(context);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn context_binding_has_stable_contract_not_representable_error() {
    let path = temp_path();
    let schema = Schema::builder().build().expect("empty schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let error = database
        .context::<TodoSchema>()
        .expect_err("missing consumer relation must fail closed");
    assert_eq!(error.kind(), ErrorKind::ContractNotRepresentable);
    assert!(error.message().contains("typed consumer contract"));
    assert!(error.message().contains("not representable"));

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn scoped_context_owns_basic_exact_intent_without_public_transaction() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let context = database.context::<TodoSchema>().expect("context");
    let formation = context.formation_revision();

    context
        .add(
            |schema| &schema.todos,
            Todo {
                id: Id::new(358_100),
                title: "scoped".to_owned(),
                done: false,
            },
        )
        .expect("stage scoped add");
    let preview = context.preview().expect("scoped preview");
    assert_eq!(preview.source_revision(), formation);
    assert_ne!(
        context.current_revision().expect("candidate revision"),
        formation
    );
    context.commit().expect("scoped commit");

    assert!(
        database
            .objects::<Todo>()
            .expect("live todos")
            .get(Id::new(358_100))
            .expect("committed read")
            .is_some()
    );

    drop(context);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn scoped_context_read_write_read_write_commit_reopens_with_prefix_observation() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let context = database.context::<TodoSchema>().expect("context");

    context
        .add(
            |schema| &schema.todos,
            Todo {
                id: Id::new(358_101),
                title: "candidate".to_owned(),
                done: false,
            },
        )
        .expect("stage scoped add");
    assert_eq!(
        context
            .todos
            .require(Id::new(358_101))
            .expect("read own staged row")
            .title,
        "candidate"
    );
    context
        .add(
            |schema| &schema.todos,
            Todo {
                id: Id::new(358_102),
                title: "after-read".to_owned(),
                done: true,
            },
        )
        .expect("stage second write after candidate observation");
    context.commit().expect("prefix-qualified scoped commit");

    drop(context);
    drop(database);
    let reopened = Database::open(&path).expect("reopen prefix-qualified scoped database");
    let stored = reopened
        .objects::<Todo>()
        .expect("reopened todos")
        .require(Id::new(358_101))
        .expect("reopened staged row");
    assert_eq!(stored.title, "candidate");
    assert!(!stored.done);
    assert!(
        reopened
            .objects::<Todo>()
            .expect("reopened todos")
            .get(Id::new(358_102))
            .expect("reopened second row")
            .is_some()
    );

    drop(reopened);
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
    let strict = snapshot.edit().expect("strict snapshot edit");
    strict
        .add(
            |schema| &schema.todos,
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

    assert_eq!(
        strict
            .commit()
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

    let mut transaction = IntentJournal::new()
        .with_idempotency_key(TransactionId::new(351_001))
        .expect("transaction");
    let todos = database.objects::<Todo>().expect("todo set");
    todos
        .add(
            &mut transaction,
            Todo {
                id: Id::new(1),
                title: "first".to_owned(),
                done: false,
            },
        )
        .expect("first insert");
    todos
        .add(
            &mut transaction,
            Todo {
                id: Id::new(2),
                title: "second".to_owned(),
                done: false,
            },
        )
        .expect("second insert");

    let preview = database.preview(&transaction).expect("preview");
    assert_eq!(preview.effects().inserted_rows(), 2);
    assert_eq!(
        preview.source_revision(),
        transaction.origin_revision().expect("bound transaction")
    );
    let first = database.commit(&transaction).expect("commit transaction");
    let revision = match first {
        CommitOutcome::Committed { revision } => revision,
        CommitOutcome::AlreadySatisfied { .. } | CommitOutcome::AlreadyCommitted { .. } => {
            panic!("first publication must commit")
        }
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

    let mut stale = IntentJournal::new()
        .with_idempotency_key(TransactionId::new(351_010))
        .expect("stale transaction");
    let stale_todos = database.objects::<Todo>().expect("stale todos");
    stale_todos
        .add(
            &mut stale,
            Todo {
                id: Id::new(10),
                title: "stale".to_owned(),
                done: false,
            },
        )
        .expect("stale insert");

    let mut winner = IntentJournal::new()
        .with_idempotency_key(TransactionId::new(351_011))
        .expect("winner transaction");
    let winner_todos = database.objects::<Todo>().expect("winner todos");
    winner_todos
        .add(
            &mut winner,
            Todo {
                id: Id::new(11),
                title: "winner".to_owned(),
                done: false,
            },
        )
        .expect("winner insert");
    database.commit(&winner).expect("winner commit");

    assert!(matches!(
        database
            .intent_readiness(&stale)
            .expect("stale transaction readiness"),
        cfmd::__private::IntentReadiness::Rebasable {
            base_revision,
            current_revision,
            ref intervening_effect_count,
        } if base_revision == stale.origin_revision().expect("bound stale transaction")
            && current_revision == database.current_revision().expect("current revision")
            && *intervening_effect_count == 1
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
        cfmd::CommitOutcome::AlreadySatisfied { .. }
        | cfmd::CommitOutcome::AlreadyCommitted { .. } => {
            panic!("first merged publish must commit")
        }
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
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete object carrier idempotency scenario together."
)]
fn independent_first_object_inserts_share_idempotent_carrier_presence() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut first = IntentJournal::new()
        .with_idempotency_key(TransactionId::new(356_001))
        .expect("first transaction");
    let first_todos = database.objects::<Todo>().expect("first todos");
    first_todos
        .add(
            &mut first,
            Todo {
                id: Id::new(1),
                title: "first".to_owned(),
                done: false,
            },
        )
        .expect("first insert");
    drop(first_todos);

    let mut second = IntentJournal::new()
        .with_idempotency_key(TransactionId::new(356_002))
        .expect("second transaction");
    let second_todos = database.objects::<Todo>().expect("second todos");
    second_todos
        .add(
            &mut second,
            Todo {
                id: Id::new(2),
                title: "second".to_owned(),
                done: false,
            },
        )
        .expect("second insert");
    drop(second_todos);

    database.commit(&first).expect("commit first");
    assert!(matches!(
        database
            .intent_readiness(&second)
            .expect("second readiness"),
        cfmd::__private::IntentReadiness::Rebasable { .. }
    ));
    database
        .preview(&second)
        .expect("same carrier creation must rebase through the first insert");
    let second_revision = match database
        .commit(&second)
        .expect("same carrier intent and distinct entity must merge")
    {
        cfmd::CommitOutcome::Committed { revision } => revision,
        cfmd::CommitOutcome::AlreadySatisfied { .. }
        | cfmd::CommitOutcome::AlreadyCommitted { .. } => {
            panic!("first merged publish must commit")
        }
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

    let mut undo = IntentJournal::new();
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
    let formation = database.snapshot().expect("formation snapshot");
    let mut transaction = IntentJournal::from(formation)
        .with_idempotency_key(TransactionId::new(355_010))
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
fn intent_readiness_reports_semantic_coordinate_conflict() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut first = IntentJournal::new()
        .with_idempotency_key(TransactionId::new(355_001))
        .expect("first transaction");
    let first_todos = database.objects::<Todo>().expect("first todos");
    first_todos
        .add(
            &mut first,
            Todo {
                id: Id::new(77),
                title: "first".to_owned(),
                done: false,
            },
        )
        .expect("first insert");

    let mut second = IntentJournal::new()
        .with_idempotency_key(TransactionId::new(355_002))
        .expect("second transaction");
    let second_todos = database.objects::<Todo>().expect("second todos");
    second_todos
        .add(
            &mut second,
            Todo {
                id: Id::new(77),
                title: "second".to_owned(),
                done: false,
            },
        )
        .expect("second insert");

    database.commit(&first).expect("commit first");
    assert!(matches!(
        database
            .intent_readiness(&second)
            .expect("second readiness"),
        cfmd::__private::IntentReadiness::Conflict {
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

    let mut transaction = IntentJournal::new()
        .with_idempotency_key(TransactionId::new(354_001))
        .expect("transaction");
    first
        .objects::<Todo>()
        .expect("todos")
        .add(
            &mut transaction,
            Todo {
                id: Id::new(1),
                title: "owned by first".to_owned(),
                done: false,
            },
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
    let recovery = diagnostic
        .recovery()
        .expect("structured recovery diagnostic");
    assert_eq!(recovery.operation(), RecoveryOperation::Open);
    assert_eq!(recovery.authority(), RecoveryAuthority::Storage);
    assert_eq!(recovery.reason(), RecoveryReason::PathUnavailable);
    assert_eq!(recovery.byte_offset(), None);
    assert_eq!(recovery.format_version(), None);
}

#[test]
fn backup_verification_projects_corruption_without_parsing_message() {
    let path = temp_path();
    fs::write(&path, b"not-a-cfmd-backup").expect("write invalid backup");
    let error = Database::verify_backup(&path, &cfmd::Encryption::None)
        .expect_err("invalid backup must fail closed");
    let diagnostic = error.diagnostic();
    let recovery = diagnostic
        .recovery()
        .expect("structured recovery diagnostic");
    assert_eq!(diagnostic.code(), DiagnosticCode::Recovery);
    assert_eq!(recovery.operation(), RecoveryOperation::VerifyBackup);
    assert!(matches!(
        recovery.reason(),
        RecoveryReason::Corruption | RecoveryReason::ProtocolViolation
    ));
    assert!(matches!(
        recovery.authority(),
        RecoveryAuthority::DurableBytes | RecoveryAuthority::SingleFileFormat
    ));
    fs::remove_file(path).expect("remove invalid backup");
}

#[test]
fn restore_projects_failure_operation_without_parsing_message() {
    let backup_path = temp_path();
    let target_path = temp_path();
    fs::write(&backup_path, b"not-a-cfmd-backup").expect("write invalid backup");
    let error = Database::restore_backup(
        &backup_path,
        &cfmd::Encryption::None,
        &target_path,
        cfmd::Encryption::None,
    )
    .expect_err("invalid restore source must fail closed");
    let recovery = error
        .diagnostic()
        .recovery()
        .expect("structured recovery diagnostic");
    assert_eq!(recovery.operation(), RecoveryOperation::RestoreBackup);
    assert!(matches!(
        recovery.reason(),
        RecoveryReason::Corruption | RecoveryReason::ProtocolViolation
    ));
    fs::remove_file(backup_path).expect("remove invalid backup");
    assert!(!target_path.exists());
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
fn schema_access_dx_lowers_typed_relationships_without_raw_coordinates() {
    use cfmd::{AccessCapability, Permission, Role, SchemaAccess};

    let capability = AccessCapability::new("example.cap.parent-editor")
        .read_field::<Parent, String, _>(ParentFields::name)
        .attach_relationship::<Parent, Child, _>(ParentFields::children)
        .detach_relationship::<Parent, Child, _>(ParentFields::children)
        .move_relationship::<Parent, Child, _>(ParentFields::children)
        .watch();
    let role = Role::new("example.role.parent-editor").capability(&capability);
    let policy = SchemaAccess::new()
        .capability(capability)
        .role(role.clone());
    let path = temp_path();
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .access(policy)
        .build()
        .expect("authorization schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create authorization database");
    let permissions = database
        .snapshot()
        .expect("snapshot")
        .schema()
        .expect("schema")
        .permissions_for_roles([role.id()])
        .expect("resolve role");
    assert!(
        permissions
            .iter()
            .any(|permission| matches!(permission, Permission::AttachRelationship(_)))
    );
    assert!(
        permissions
            .iter()
            .any(|permission| matches!(permission, Permission::DetachRelationship(_)))
    );
    assert!(
        permissions
            .iter()
            .any(|permission| matches!(permission, Permission::MoveRelationship(_)))
    );
    assert!(permissions.contains(Permission::Watch));
    assert!(!permissions.contains(Permission::Write));
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
#[allow(
    clippy::too_many_lines,
    reason = "Keep complete public semantic regression scenario together."
)]
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

    let mut add_zero = IntentJournal::new();
    database
        .objects::<Parent>()
        .expect("parents")
        .add(
            &mut add_zero,
            Parent::cfmd_new(Id::new(3), "zero".into(), cfmd::Many::empty()),
        )
        .expect("zero-degree parent");
    database
        .commit(&add_zero)
        .expect("commit zero-degree parent");

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

    let mut attach = IntentJournal::new();
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

    let conflict = database.context::<OwnerSchema>().expect("conflict context");
    let conflict_owner2 = conflict.owners.require(Id::new(2)).expect("two");
    let error = conflict
        .attach(&conflict_owner2.assets, Id::new(10))
        .expect_err("scoped OwnedMany must reject a second owner while forming Candidate");
    assert_eq!(error.kind(), ErrorKind::Cardinality);
    drop(conflict_owner2);
    drop(conflict);

    let move_context = database.context::<OwnerSchema>().expect("move context");
    let owner1 = move_context.owners.require(Id::new(1)).expect("one");
    let owner2 = move_context.owners.require(Id::new(2)).expect("two");
    move_context
        .move_to(&owner1.assets, Id::new(10), &owner2.assets)
        .expect("scoped owned move proposal");
    assert_eq!(
        move_context
            .owners
            .require(Id::new(2))
            .expect("candidate destination")
            .assets
            .count()
            .expect("candidate destination count"),
        1
    );
    move_context.commit().expect("scoped owned move");
    drop(owner1);
    drop(owner2);
    drop(move_context);

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
    let mut transaction = IntentJournal::new();
    selected
        .move_to(&mut transaction, &destination.children)
        .expect("filtered edge move");
    let mut invalid = IntentJournal::new();
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
    let mut transaction = IntentJournal::new();
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

    let mut seed = IntentJournal::new();
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
            .top(2, ChildFields::score)
            .all()
            .expect("top boundary")),
        vec![2, 3, 4]
    );
    assert_eq!(
        ids(children
            .bottom(2, ChildFields::score)
            .all()
            .expect("bottom boundary")),
        vec![1, 2, 3]
    );

    let mut watch = children
        .top(2, ChildFields::score)
        .watch()
        .expect("boundary watch");
    assert_eq!(ids(watch.initial().to_vec()), vec![2, 3, 4]);
    drop(children);

    let mut update = IntentJournal::new();
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
#[allow(
    clippy::too_many_lines,
    reason = "Keep complete public semantic regression scenario together."
)]
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

    let mut seed = IntentJournal::new();
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
    let ids = |rows: Vec<ColumnPair>| rows.into_iter().map(|row| row.id.raw()).collect::<Vec<_>>();
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

    let mut matching = IntentJournal::new();
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
    let event = watch
        .try_recv()
        .expect("watch read")
        .expect("matching event");
    assert_eq!(ids(event.inserted().to_vec()), vec![4]);
    assert!(event.removed().is_empty());

    let mut nonmatching = IntentJournal::new();
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

    let mut seed = IntentJournal::new();
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

    let mut matching = IntentJournal::new();
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

    let mut nonmatching = IntentJournal::new();
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
    let schema = Schema::builder()
        .object::<WideRow>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut seed = IntentJournal::new();
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

    let mut seed = IntentJournal::new();
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

    let children = database.objects::<Child>().expect("children");
    let projection = children.select(ChildFields::score);
    let mut scores = projection.all().expect("projected scores");
    scores.sort_unstable();
    assert_eq!(scores, vec![3, 7, 7, 11]);
    assert_eq!(projection.count().expect("projection count"), 4);

    let distinct = children.select(ChildFields::score).distinct();
    let mut unique_scores = distinct.all().expect("distinct scores");
    unique_scores.sort_unstable();
    assert_eq!(unique_scores, vec![3, 7, 11]);
    assert_eq!(distinct.count().expect("distinct count"), 3);

    let mut grouped = children
        .group_by(ChildFields::score)
        .count()
        .all()
        .expect("grouped count");
    grouped.sort_unstable_by_key(|(score, _)| *score);
    assert_eq!(grouped, vec![(3, 1), (7, 2), (11, 1)]);

    let mut watch = projection.watch().expect("projection watch");
    let mut initial = watch.initial().to_vec();
    initial.sort_unstable();
    assert_eq!(initial, vec![3, 7, 7, 11]);

    let mut duplicate = IntentJournal::new();
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
        .select(ChildFields::score)
        .distinct()
        .watch()
        .expect("distinct projection watch");
    let mut distinct_initial = distinct_watch.initial().to_vec();
    distinct_initial.sort_unstable();
    assert_eq!(distinct_initial, vec![3, 7, 11]);

    let mut another_duplicate = IntentJournal::new();
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

    let mut seed = IntentJournal::new();
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
    let count_query = metrics.group_by(GroupedMetricFields::bucket).count();
    let mut counts = count_query.all().expect("group counts");
    counts.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(counts, vec![(1, 2), (2, 1)]);

    let sum_query = metrics
        .group_by(GroupedMetricFields::bucket)
        .sum(GroupedMetricFields::value);
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
    let candidate_metrics = candidate
        .objects::<GroupedMetric>()
        .expect("candidate metrics");

    let mut future_counts = candidate_metrics
        .group_by(GroupedMetricFields::bucket)
        .count()
        .all()
        .expect("candidate group counts");
    future_counts.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(future_counts, vec![(1, 3), (2, 1)]);

    let mut future_sums = candidate_metrics
        .group_by(GroupedMetricFields::bucket)
        .sum(GroupedMetricFields::value)
        .all()
        .expect("candidate group sums");
    future_sums.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(future_sums, vec![(1, 4.5), (2, 10.0)]);

    let mut live_insert = IntentJournal::new();
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
    database
        .commit(&live_insert)
        .expect("live aggregate commit");

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
#[allow(
    clippy::too_many_lines,
    reason = "Keep complete public semantic regression scenario together."
)]
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

    let mut seed = IntentJournal::new();
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
    let count_top = metrics.group_by(GroupedMetricFields::bucket).count().top(1);
    let mut top_counts = count_top.all().expect("top grouped counts");
    top_counts.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(top_counts, vec![(1, 2), (2, 2)]);
    assert_eq!(
        metrics
            .group_by(GroupedMetricFields::bucket)
            .count()
            .bottom(1)
            .all()
            .expect("bottom grouped count"),
        vec![(3, 1)]
    );

    let sum_top = metrics
        .group_by(GroupedMetricFields::bucket)
        .sum(GroupedMetricFields::value)
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
    let candidate_metrics = candidate
        .objects::<GroupedMetric>()
        .expect("candidate metrics");
    let mut candidate_counts = candidate_metrics
        .group_by(GroupedMetricFields::bucket)
        .count()
        .top(1)
        .all()
        .expect("candidate top counts");
    candidate_counts.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(candidate_counts, vec![(1, 2), (2, 2), (3, 2)]);
    let mut candidate_sums = candidate_metrics
        .group_by(GroupedMetricFields::bucket)
        .sum(GroupedMetricFields::value)
        .top(1)
        .all()
        .expect("candidate top sums");
    candidate_sums.sort_unstable_by_key(|(bucket, _)| *bucket);
    assert_eq!(candidate_sums, vec![(1, 4.0), (2, 4.0), (3, 4.0)]);

    let mut live_insert = IntentJournal::new();
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
    database
        .commit(&live_insert)
        .expect("live aggregate commit");

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

    let mut seed = IntentJournal::new();
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

    let mut seed = IntentJournal::new();
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

    let mut nonmatching = IntentJournal::new();
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

    let mut matching = IntentJournal::new();
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

    let mut seed = IntentJournal::new();
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
        .sum(GroupedMetricFields::value)
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
        .sum(GroupedMetricFields::value)
        .top(1)
        .all()
        .expect("candidate composite top sums");
    assert_eq!(future_top_sum.len(), 3);
    assert!(future_top_sum.contains(&((2, 10.0), 20.0)));

    let mut tx = IntentJournal::new();
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
    let mut tx = IntentJournal::new();
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
        person.passport().region().code().eq("RU".to_owned()) & person.name().eq("Artem".to_owned())
    });
    let selected = query.one().expect("deep path match");
    assert_eq!(selected.id, person_id);

    let none = people
        .where_(|person| person.passport().region().code().eq("NL".to_owned()))
        .all()
        .expect("non-matching deep path");
    assert!(none.is_empty());

    let ergonomic = people
        .where_(|person| {
            person.passport().region().code().eq("NL".to_owned())
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

    let mut add_parent = IntentJournal::new();
    database
        .objects::<Parent>()
        .expect("parents")
        .add(
            &mut add_parent,
            Parent::cfmd_new(Id::new(3), "zero".into(), cfmd::Many::empty()),
        )
        .expect("add empty parent");
    database.commit(&add_parent).expect("parent commit");

    let mut first = IntentJournal::new()
        .with_idempotency_key(TransactionId::new(372_001))
        .expect("first transaction");
    let first_parent = database
        .objects::<Parent>()
        .expect("first parents")
        .require(Id::new(3))
        .expect("first parent");
    first_parent
        .children
        .attach(&mut first, Id::new(10))
        .expect("first attach");
    drop(first_parent);

    let mut second = IntentJournal::new()
        .with_idempotency_key(TransactionId::new(372_002))
        .expect("second transaction");
    let second_parent = database
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
        database.intent_readiness(&second).expect("readiness"),
        cfmd::__private::IntentReadiness::Rebasable { .. }
    ));
    let residual_revision = match database
        .commit(&second)
        .expect("same attach must publish certified residual")
    {
        CommitOutcome::Committed { revision } => revision,
        CommitOutcome::AlreadySatisfied { .. } | CommitOutcome::AlreadyCommitted { .. } => {
            panic!("first residual publication must commit")
        }
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
    let schema = Schema::builder()
        .object::<RuleUser>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut valid = IntentJournal::new();
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

    let mut invalid = IntentJournal::new();
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
    let error = database
        .preview(&invalid)
        .expect_err("rule must reject candidate");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(valid);
    drop(invalid);
    drop(database);
    let reopened = Database::builder(&path).open().expect("reopen database");
    let mut invalid_after_reopen = IntentJournal::new();
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
fn derived_text_pattern_rule_is_deterministic_and_database_owned() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<PatternUser>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut valid = IntentJournal::new();
    database
        .objects::<PatternUser>()
        .expect("pattern users")
        .add(
            &mut valid,
            PatternUser {
                id: Id::new(516_101),
                name: "Artem".to_owned(),
            },
        )
        .expect("valid pattern intent");
    database.commit(&valid).expect("valid pattern commit");

    let mut invalid = IntentJournal::new();
    database
        .objects::<PatternUser>()
        .expect("pattern users")
        .add(
            &mut invalid,
            PatternUser {
                id: Id::new(516_102),
                name: "Boris".to_owned(),
            },
        )
        .expect("invalid pattern may form intent");
    let error = database
        .preview(&invalid)
        .expect_err("deterministic text pattern must reject non-matching candidate");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn database_owned_model_rules_use_the_kernel_invariant_engine_and_survive_reopen() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<RuleUser>()
        .model_rule(ModelRuleExpr::RelationExactCountRange {
            relation: RuleUser::relation_id(),
            predicate: SemanticRuleExpr::True,
            min: 0,
            max: Some(1),
        })
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut first = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut first,
            RuleUser {
                id: Id::new(516_001),
                name: "Artem".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("first insert");
    database.commit(&first).expect("first commit");

    let mut second = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut second,
            RuleUser {
                id: Id::new(516_002),
                name: "Alice".to_owned(),
                age: 20,
                role: "admin".to_owned(),
            },
        )
        .expect("second intent");
    let error = database
        .preview(&second)
        .expect_err("model cardinality invariant must reject second row");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(second);
    drop(first);
    drop(database);
    let reopened = Database::builder(&path).open().expect("reopen database");
    let mut after_reopen = IntentJournal::new();
    reopened
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut after_reopen,
            RuleUser {
                id: Id::new(516_003),
                name: "Anya".to_owned(),
                age: 21,
                role: "user".to_owned(),
            },
        )
        .expect("post-reopen intent");
    let error = reopened
        .preview(&after_reopen)
        .expect_err("persisted model invariant must survive reopen");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(reopened);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn typed_object_rule_composition_lowers_to_database_owned_entity_invariants() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<RuleUser>()
        .object_rule::<RuleUser, _>(|user| {
            user.age()
                .rule()
                .range(Some(18), None)
                .and(user.role().rule().one_of(["user"]))
                .and(user.name().rule().length(3, Some(8)))
        })
        .build()
        .expect("typed object-rule schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut valid = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut valid,
            RuleUser {
                id: Id::new(517_001),
                name: "Artem".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("valid typed-rule intent");
    database.commit(&valid).expect("valid typed-rule commit");

    let mut invalid = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut invalid,
            RuleUser {
                id: Id::new(517_002),
                name: "Alice".to_owned(),
                age: 20,
                role: "admin".to_owned(),
            },
        )
        .expect("candidate may form before invariant validation");
    let error = database
        .preview(&invalid)
        .expect_err("typed role coordinate must reach the kernel invariant gate");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn typed_binary_rules_use_kernel_equivalence_and_ordering_for_scalar_and_bool_fields() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<BinaryRuleTask>()
        .object_rule::<BinaryRuleTask, _>(|task| {
            task.minimum()
                .rule()
                .less_than_or_equal_field(task.maximum().rule())
                .and(task.enabled().rule().equivalent_to(task.published().rule()))
        })
        .build()
        .expect("typed binary-rule schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut valid = IntentJournal::new();
    database
        .objects::<BinaryRuleTask>()
        .expect("tasks")
        .add(
            &mut valid,
            BinaryRuleTask {
                id: Id::new(518_010),
                minimum: 3,
                maximum: 7,
                enabled: true,
                published: true,
            },
        )
        .expect("valid binary-rule intent");
    database.commit(&valid).expect("valid binary-rule commit");

    drop(valid);
    drop(database);
    let database = Database::open(&path).expect("reopen binary-rule database");

    let mut invalid_order = IntentJournal::new();
    database
        .objects::<BinaryRuleTask>()
        .expect("tasks")
        .add(
            &mut invalid_order,
            BinaryRuleTask {
                id: Id::new(518_011),
                minimum: 9,
                maximum: 7,
                enabled: true,
                published: true,
            },
        )
        .expect("candidate may form before invariant validation");
    let error = database
        .preview(&invalid_order)
        .expect_err("semantic ordering must reject the future candidate");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    let mut invalid_bool = IntentJournal::new();
    database
        .objects::<BinaryRuleTask>()
        .expect("tasks")
        .add(
            &mut invalid_bool,
            BinaryRuleTask {
                id: Id::new(518_012),
                minimum: 3,
                maximum: 7,
                enabled: true,
                published: false,
            },
        )
        .expect("candidate may form before invariant validation");
    let error = database
        .preview(&invalid_bool)
        .expect_err("semantic bool equivalence must reject the future candidate");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn typed_reference_rules_expose_only_direct_root_coordinates() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<User>()
        .object::<ReferenceRuleTask>()
        .object_rule::<ReferenceRuleTask, _>(|task| {
            task.primary_rule().equivalent_to(task.secondary_rule())
        })
        .build()
        .expect("typed reference-rule schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let user_a = Id::<User>::new(518_100);
    let user_b = Id::<User>::new(518_101);
    let mut users = IntentJournal::new();
    database
        .objects::<User>()
        .expect("users")
        .add(
            &mut users,
            User {
                id: user_a,
                name: "A".to_owned(),
            },
        )
        .expect("add user A");
    database
        .objects::<User>()
        .expect("users")
        .add(
            &mut users,
            User {
                id: user_b,
                name: "B".to_owned(),
            },
        )
        .expect("add user B");
    database.commit(&users).expect("commit users");

    let mut valid = IntentJournal::new();
    database
        .objects::<ReferenceRuleTask>()
        .expect("tasks")
        .add(
            &mut valid,
            ReferenceRuleTask {
                id: Id::new(518_110),
                primary: user_a.reference(),
                secondary: user_a.reference(),
            },
        )
        .expect("valid reference-rule intent");
    database
        .commit(&valid)
        .expect("valid reference-rule commit");

    let mut invalid = IntentJournal::new();
    database
        .objects::<ReferenceRuleTask>()
        .expect("tasks")
        .add(
            &mut invalid,
            ReferenceRuleTask {
                id: Id::new(518_111),
                primary: user_a.reference(),
                secondary: user_b.reference(),
            },
        )
        .expect("candidate may form before invariant validation");
    let error = database
        .preview(&invalid)
        .expect_err("different references must violate semantic equivalence");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn object_first_model_rules_use_typed_relation_and_column_coordinates() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<OrderedF64>()
        .model_rule(ModelRuleExpr::object_cardinality::<OrderedF64>(0, Some(2)))
        .model_rule(ModelRuleExpr::object_exact_f64_sum_range::<OrderedF64, _>(
            OrderedF64Fields::value,
            Some(cfmd::FiniteF64::new(0.0).expect("finite lower bound")),
            Some(cfmd::FiniteF64::new(10.0).expect("finite upper bound")),
        ))
        .build()
        .expect("typed model-rule schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    for (id, value) in [(517_101, 4.0), (517_102, 5.0)] {
        let mut intent = IntentJournal::new();
        database
            .objects::<OrderedF64>()
            .expect("metrics")
            .add(
                &mut intent,
                OrderedF64 {
                    id: Id::new(id),
                    value,
                },
            )
            .expect("bounded sum intent");
        database.commit(&intent).expect("bounded sum commit");
    }

    let mut exceeds_sum = IntentJournal::new();
    database
        .objects::<OrderedF64>()
        .expect("metrics")
        .add(
            &mut exceeds_sum,
            OrderedF64 {
                id: Id::new(517_103),
                value: 2.0,
            },
        )
        .expect("overflowing sum may form intent");
    let error = database
        .preview(&exceeds_sum)
        .expect_err("typed exact-sum coordinate must reject the future candidate");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn exact_sum_where_is_delta_maintained_by_semantic_row_predicate_and_survives_reopen() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<FilteredMetric>()
        .model_rule(ModelRuleExpr::object_exact_f64_sum_where_range::<
            FilteredMetric,
            _,
            _,
        >(
            FilteredMetricFields::value,
            |metric| {
                metric
                    .lower()
                    .rule()
                    .less_than_or_equal_field(metric.upper().rule())
            },
            None,
            Some(cfmd::FiniteF64::new(10.0).expect("finite upper bound")),
        ))
        .build()
        .expect("filtered exact-sum schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    for (id, value, lower, upper) in [(519_001, 6.0, 1, 2), (519_002, 100.0, 9, 2)] {
        let mut intent = IntentJournal::new();
        database
            .objects::<FilteredMetric>()
            .expect("metrics")
            .add(
                &mut intent,
                FilteredMetric {
                    id: Id::new(id),
                    value,
                    lower,
                    upper,
                },
            )
            .expect("filtered aggregate intent");
        database
            .commit(&intent)
            .expect("only rows selected by the semantic predicate contribute");
    }

    drop(database);
    let database = Database::open(&path).expect("reopen filtered aggregate database");
    let mut violates = IntentJournal::new();
    database
        .objects::<FilteredMetric>()
        .expect("metrics")
        .add(
            &mut violates,
            FilteredMetric {
                id: Id::new(519_003),
                value: 5.0,
                lower: 3,
                upper: 4,
            },
        )
        .expect("violating candidate may form before invariant validation");
    let error = database
        .preview(&violates)
        .expect_err("selected exact sum must reject the future candidate after reopen");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn exact_count_where_unifies_quantifiers_and_survives_reopen() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<FilteredMetric>()
        .model_rule(ModelRuleExpr::object_exact_count_where_range::<
            FilteredMetric,
            _,
        >(
            |metric| {
                metric
                    .lower()
                    .rule()
                    .less_than_or_equal_field(metric.upper().rule())
            },
            0,
            Some(1),
        ))
        .build()
        .expect("selected exact-count schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    for (id, lower, upper) in [(520_001, 1, 2), (520_002, 9, 2)] {
        let mut intent = IntentJournal::new();
        database
            .objects::<FilteredMetric>()
            .expect("metrics")
            .add(
                &mut intent,
                FilteredMetric {
                    id: Id::new(id),
                    value: 0.0,
                    lower,
                    upper,
                },
            )
            .expect("selected count intent");
        database
            .commit(&intent)
            .expect("only predicate-selected rows contribute to exact count");
    }

    drop(database);
    let database = Database::open(&path).expect("reopen selected exact-count database");
    let mut violates = IntentJournal::new();
    database
        .objects::<FilteredMetric>()
        .expect("metrics")
        .add(
            &mut violates,
            FilteredMetric {
                id: Id::new(520_003),
                value: 0.0,
                lower: 3,
                upper: 4,
            },
        )
        .expect("violating candidate may form before invariant validation");
    let error = database
        .preview(&violates)
        .expect_err("second selected row must violate the exact-count upper bound after reopen");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
#[allow(clippy::too_many_lines)]
fn exact_aggregate_compare_maintains_cross_relation_count_product_after_reopen() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<User>()
        .object::<FilteredMetric>()
        .model_rule(ModelRuleExpr::object_exact_count_compare::<
            User,
            FilteredMetric,
            _,
            _,
        >(
            |_| SemanticRuleExpr::True,
            cfmd::RuleOrderComparison::LessOrEqual,
            |_| SemanticRuleExpr::True,
        ))
        .build()
        .expect("cross-relation exact-count comparison schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut metric = IntentJournal::new();
    database
        .objects::<FilteredMetric>()
        .expect("metrics")
        .add(
            &mut metric,
            FilteredMetric {
                id: Id::new(521_001),
                value: 1.0,
                lower: 0,
                upper: 0,
            },
        )
        .expect("metric intent");
    database.commit(&metric).expect("metric commit");

    let mut user = IntentJournal::new();
    database
        .objects::<User>()
        .expect("users")
        .add(
            &mut user,
            User {
                id: Id::new(521_010),
                name: "one".to_owned(),
            },
        )
        .expect("user intent");
    database.commit(&user).expect("balanced count commit");
    drop(metric);
    drop(user);
    drop(database);

    let database = Database::open(&path).expect("reopen aggregate comparison database");
    let mut excess_user = IntentJournal::new();
    database
        .objects::<User>()
        .expect("users")
        .add(
            &mut excess_user,
            User {
                id: Id::new(521_011),
                name: "two".to_owned(),
            },
        )
        .expect("excess user intent");
    let error = database
        .preview(&excess_user)
        .expect_err("user count must not exceed metric count");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);
    drop(excess_user);

    let mut second_metric = IntentJournal::new();
    database
        .objects::<FilteredMetric>()
        .expect("metrics")
        .add(
            &mut second_metric,
            FilteredMetric {
                id: Id::new(521_002),
                value: 1.0,
                lower: 0,
                upper: 0,
            },
        )
        .expect("second metric intent");
    database
        .commit(&second_metric)
        .expect("right-side measure update must satisfy comparator");
    drop(second_metric);
    let mut balanced_user = IntentJournal::new();
    database
        .objects::<User>()
        .expect("users")
        .add(
            &mut balanced_user,
            User {
                id: Id::new(521_011),
                name: "two".to_owned(),
            },
        )
        .expect("balanced user intent");
    database
        .commit(&balanced_user)
        .expect("left-side measure must observe exact right-side delta");
    drop(balanced_user);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn typed_rule_coordinate_uses_persisted_bind_identity_not_local_field_spelling() {
    let rule =
        LegacyRenameAccount::rule(|account| account.doctor_note().rule().length(0, Some(32)));
    let SemanticRuleExpr::TextLength {
        value: RuleValueExpr::Field(field),
        ..
    } = rule
    else {
        panic!("typed text rule must lower to one persisted field coordinate");
    };
    assert_eq!(field, RenamedAccount::__field_id("medical_note"));
    assert_ne!(field, RenamedAccount::__field_id("doctor_note"));
}

#[test]
fn typed_rule_frontend_preserves_kernel_schema_diagnostics() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<RuleUser>()
        .object_rule::<RuleUser, _>(|user| user.age().rule().range(Some(30), Some(18)))
        .build()
        .expect("frontend schema object");
    let error = Database::builder(&path)
        .schema(schema)
        .create()
        .expect_err("invalid typed bounds must fail in the existing kernel rule validator");
    assert_eq!(error.kind(), ErrorKind::InvalidSchema);
    assert!(error.message().contains("invalid model rule"));
    let _ = fs::remove_file(path);
}

#[test]
fn typed_transaction_requirement_uses_the_same_object_rule_frontend() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<RuleUser>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let id = Id::new(517_201);

    let mut create = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut create,
            RuleUser {
                id,
                name: "Artem".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("seed user");
    database.commit(&create).expect("seed commit");

    let mut invalid = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .set(&mut invalid, id, RuleUserFields::age, 17)
        .expect("patch intent");
    invalid
        .require::<RuleUser>(
            id,
            RuleUser::rule(|user| user.age().rule().range(Some(18), None)),
        )
        .expect("typed requirement");
    assert_eq!(
        database
            .preview(&invalid)
            .expect_err("typed requirement must observe exact future candidate")
            .kind(),
        ErrorKind::TransactionConflict,
    );

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn transaction_require_is_evaluated_on_the_exact_future_candidate() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<RuleUser>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let id = Id::new(446_001);

    let mut create = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut create,
            RuleUser {
                id,
                name: "Artem".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("create intent");
    database.commit(&create).expect("create commit");

    let adult = SemanticRuleExpr::I64Range {
        value: RuleValueExpr::Field(RuleUser::__field_id("age")),
        min: Some(18),
        max: None,
    };
    let users = database.objects::<RuleUser>().expect("users");
    let mut invalid = IntentJournal::new();
    users
        .set(&mut invalid, id, RuleUserFields::age, 17)
        .expect("patch intent");
    invalid
        .require::<RuleUser>(id, adult.clone())
        .expect("require intent");
    let error = database
        .preview(&invalid)
        .expect_err("future-world requirement must reject");
    assert_eq!(error.kind(), ErrorKind::TransactionConflict);

    let mut valid = IntentJournal::new();
    users
        .set(&mut valid, id, RuleUserFields::age, 20)
        .expect("valid patch intent");
    valid
        .require::<RuleUser>(id, adult)
        .expect("valid require intent");
    database.preview(&valid).expect("valid future candidate");
    database.commit(&valid).expect("require-backed commit");
    assert_eq!(
        database
            .objects::<RuleUser>()
            .expect("fresh users")
            .require(id)
            .expect("stored user")
            .age,
        20
    );

    drop(valid);
    drop(invalid);
    drop(create);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep complete public semantic regression scenario together."
)]
fn external_transaction_id_retains_canonical_requirement_identity() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<RuleUser>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let entity = Id::new(456_001);

    let mut seed = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut seed,
            RuleUser {
                id: entity,
                name: "Artem".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("seed intent");
    database.commit(&seed).expect("seed commit");

    let transaction_id = TransactionId::new(456_777);
    let adult = SemanticRuleExpr::I64Range {
        value: RuleValueExpr::Field(RuleUser::__field_id("age")),
        min: Some(18),
        max: None,
    };
    let narrower = SemanticRuleExpr::I64Range {
        value: RuleValueExpr::Field(RuleUser::__field_id("age")),
        min: Some(18),
        max: Some(99),
    };
    let users = database.objects::<RuleUser>().expect("users");

    let mut first = IntentJournal::new()
        .with_idempotency_key(transaction_id)
        .expect("external transaction");
    assert_eq!(first.origin_revision(), None);
    users
        .set(
            &mut first,
            entity,
            RuleUserFields::name,
            "Guarded".to_owned(),
        )
        .expect("first mutation");
    first
        .require::<RuleUser>(entity, adult.clone())
        .expect("first requirement");
    assert_eq!(first.id(), Some(transaction_id));

    let mut same = IntentJournal::new()
        .with_idempotency_key(transaction_id)
        .expect("same external transaction");
    users
        .set(
            &mut same,
            entity,
            RuleUserFields::name,
            "Guarded".to_owned(),
        )
        .expect("same mutation");
    same.require::<RuleUser>(entity, adult)
        .expect("same requirement");

    let mut different = IntentJournal::new()
        .with_idempotency_key(transaction_id)
        .expect("conflicting external transaction");
    users
        .set(
            &mut different,
            entity,
            RuleUserFields::name,
            "Guarded".to_owned(),
        )
        .expect("same mutation for conflicting requirement");
    different
        .require::<RuleUser>(entity, narrower)
        .expect("different requirement");

    assert!(matches!(
        database.commit(&first).expect("first commit"),
        CommitOutcome::Committed { .. }
    ));
    assert!(matches!(
        database.commit(&same).expect("same guarded retry"),
        CommitOutcome::AlreadyCommitted { .. }
    ));
    assert_eq!(
        database
            .commit(&different)
            .expect_err("same external id with another requirement must conflict")
            .kind(),
        ErrorKind::TransactionConflict,
    );

    drop(different);
    drop(same);
    drop(first);
    drop(users);
    drop(seed);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn generated_transaction_key_does_not_rotate_when_requirements_change() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<RuleUser>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let entity = Id::new(457_001);

    let mut seed = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut seed,
            RuleUser {
                id: entity,
                name: "Artem".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("seed intent");
    database.commit(&seed).expect("seed commit");

    let users = database.objects::<RuleUser>().expect("users");
    let mut transaction = IntentJournal::new();
    users
        .set(
            &mut transaction,
            entity,
            RuleUserFields::name,
            "Stable".to_owned(),
        )
        .expect("mutation");
    let key = transaction.id().expect("generated key");
    transaction
        .require::<RuleUser>(
            entity,
            SemanticRuleExpr::I64Range {
                value: RuleValueExpr::Field(RuleUser::__field_id("age")),
                min: Some(18),
                max: None,
            },
        )
        .expect("first guard");
    assert_eq!(transaction.id(), Some(key));
    database.commit(&transaction).expect("guarded commit");

    transaction
        .require::<RuleUser>(
            entity,
            SemanticRuleExpr::I64Range {
                value: RuleValueExpr::Field(RuleUser::__field_id("age")),
                min: Some(18),
                max: Some(99),
            },
        )
        .expect("second guard");
    assert_eq!(transaction.id(), Some(key));
    assert_eq!(
        database
            .commit(&transaction)
            .expect_err("changed guarded intent under the same key must conflict")
            .kind(),
        ErrorKind::TransactionConflict,
    );

    drop(transaction);
    drop(users);
    drop(seed);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn transaction_user_read_is_causal_but_mutation_lowering_is_not() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<RuleUser>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let id = Id::new(486_701);

    let mut create = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut create,
            RuleUser {
                id,
                name: "base".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("create intent");
    database.commit(&create).expect("create commit");

    let users = database.objects::<RuleUser>().expect("users");

    let mut retroactive = IntentJournal::new();
    users
        .set(&mut retroactive, id, RuleUserFields::age, 17)
        .expect("age intent");

    let mut observer = IntentJournal::new();
    users
        .set(&mut observer, id, RuleUserFields::name, "seen".to_owned())
        .expect("name intent");
    let adults = observer
        .objects::<RuleUser>()
        .expect("transaction users")
        .where_(|user| user.age().greater_than_or_equal(18))
        .count()
        .expect("transaction-bound count");
    assert_eq!(adults, 1);
    database.commit(&observer).expect("observer commit");

    let error = database
        .commit(&retroactive)
        .expect_err("retroactive effect must preserve later application-visible observation");
    assert_eq!(error.kind(), ErrorKind::TransactionConflict);

    drop(users);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn transaction_require_is_rechecked_on_the_rebased_candidate() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<RuleUser>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let id = Id::new(447_001);

    let mut create = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut create,
            RuleUser {
                id,
                name: "base".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("create intent");
    database.commit(&create).expect("create commit");

    let adult = SemanticRuleExpr::I64Range {
        value: RuleValueExpr::Field(RuleUser::__field_id("age")),
        min: Some(18),
        max: None,
    };
    let users = database.objects::<RuleUser>().expect("users");

    let mut survives = IntentJournal::new();
    users
        .set(
            &mut survives,
            id,
            RuleUserFields::name,
            "survives".to_owned(),
        )
        .expect("name intent");
    survives
        .require::<RuleUser>(id, adult.clone())
        .expect("adult requirement");
    let mut raise_age = IntentJournal::new();
    users
        .set(&mut raise_age, id, RuleUserFields::age, 21)
        .expect("raise age");
    database.commit(&raise_age).expect("independent age commit");
    database
        .commit(&survives)
        .expect("predicate-preserving rebase");
    let stored = database
        .objects::<RuleUser>()
        .expect("fresh users")
        .require(id)
        .expect("stored");
    assert_eq!(stored.age, 21);
    assert_eq!(stored.name, "survives");

    let fresh = database.objects::<RuleUser>().expect("users after rebase");
    let mut rejected = IntentJournal::new();
    fresh
        .set(&mut rejected, id, RuleUserFields::name, "reject".to_owned())
        .expect("name intent");
    rejected
        .require::<RuleUser>(id, adult)
        .expect("adult requirement");
    let mut lower_age = IntentJournal::new();
    fresh
        .set(&mut lower_age, id, RuleUserFields::age, 17)
        .expect("lower age");
    database
        .commit(&lower_age)
        .expect("independent falsifying commit");
    let error = database
        .commit(&rejected)
        .expect_err("rebased future world must fail requirement");
    assert_eq!(error.kind(), ErrorKind::TransactionConflict);
    let stored = database
        .objects::<RuleUser>()
        .expect("final users")
        .require(id)
        .expect("stored");
    assert_eq!(stored.age, 17);
    assert_eq!(stored.name, "survives");

    drop(stored);
    drop(fresh);
    drop(users);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn transaction_require_uses_current_read_authority_at_preview_and_commit() {
    use cfmd::{Object, Permission, PermissionSet, PrincipalId, Session};

    let path = temp_path();
    let schema = Schema::builder()
        .object::<RuleUser>()
        .build()
        .expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");
    let id = Id::new(447_010);
    let mut create = IntentJournal::new();
    database
        .objects::<RuleUser>()
        .expect("users")
        .add(
            &mut create,
            RuleUser {
                id,
                name: "base".to_owned(),
                age: 19,
                role: "user".to_owned(),
            },
        )
        .expect("create intent");
    database.commit(&create).expect("create commit");

    let adult = SemanticRuleExpr::I64Range {
        value: RuleValueExpr::Field(RuleUser::__field_id("age")),
        min: Some(18),
        max: None,
    };
    let session = Session::new(
        PrincipalId::new(447_010),
        PermissionSet::from([Permission::Write, Permission::Read]),
    );
    let restricted = database.session(session.clone());
    let mut tx = IntentJournal::new();
    restricted
        .objects::<RuleUser>()
        .expect("restricted users")
        .set(&mut tx, id, RuleUserFields::name, "auth".to_owned())
        .expect("write-only name patch");
    tx.require::<RuleUser>(id, adult).expect("require intent");
    restricted
        .preview(&tx)
        .expect("require read is initially authorized");

    session
        .refresh_permissions(PermissionSet::from([Permission::Write]))
        .expect("revoke requirement read authority");
    assert_eq!(
        restricted
            .preview(&tx)
            .expect_err("preview must use current read authority")
            .kind(),
        ErrorKind::PermissionDenied,
    );
    assert_eq!(
        restricted
            .commit(&tx)
            .expect_err("commit must use current read authority")
            .kind(),
        ErrorKind::PermissionDenied,
    );
    let stored = database
        .objects::<RuleUser>()
        .expect("fresh users")
        .require(id)
        .expect("stored");
    assert_eq!(stored.name, "base");

    drop(stored);
    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn typed_schema_root_binds_live_entity_sets_without_global_registration() {
    let path = temp_path();
    let db = create_typed::<AppSchema>(&path).expect("typed create");

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

    db.add(|schema| &schema.users, user.clone())
        .expect("add user");
    db.add(|schema| &schema.tasks, task.clone())
        .expect("add task");
    db.commit().expect("commit typed schema context");

    assert_eq!(db.users.require(user.id).expect("read user"), user);
    assert_eq!(db.tasks.require(task.id).expect("read task"), task);
    drop(db);

    let reopened = open_typed::<AppSchema>(&path).expect("typed reopen");
    assert_eq!(reopened.users.count().expect("reopened users"), 1);
    assert_eq!(reopened.tasks.count().expect("reopened tasks"), 1);
    drop(reopened);

    let subset = open_typed::<UserOnlySchema>(&path).expect(
        "consumer Context may bind an explicit subset of the authoritative database schema",
    );
    assert_eq!(subset.users.count().expect("subset users"), 1);
    drop(subset);

    let broken_path = temp_path();
    let broken = create_typed::<BrokenTaskSchema>(&broken_path)
        .expect_err("missing referenced entity must fail closed");
    assert_eq!(broken.kind(), ErrorKind::InvalidSchema);

    let isolated_path = temp_path();
    let isolated = create_typed::<UserOnlySchema>(&isolated_path)
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
    let db = create_typed::<HostileAccountSchema>(&path).expect("authoritative account schema");

    let account = HostileAccount {
        id: Id::new(378_001),
        name: "reader-visible".to_owned(),
        passport_secret: "SECRET".to_owned(),
        doctor_note: "editable".to_owned(),
    };
    db.add(|schema| &schema.accounts, account.clone())
        .expect("authoritative insert");
    db.commit().expect("authoritative commit");
    drop(db);
    let database = Database::open(&path).expect("reopen authoritative account database");

    // Raw object binding remains exact-shape by design. Partial contracts enter only through a
    // typed Context so low-level/tooling code cannot accidentally change semantics.
    let error = database
        .objects::<HostileReaderAccount>()
        .expect_err("raw object binding remains exact");
    assert_eq!(error.kind(), ErrorKind::TypeMismatch);

    let ctx = database
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
    let mut partial_tx = IntentJournal::new();
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

    ctx.set(
        |schema| &schema.accounts,
        Id::new(378_001),
        HostileReaderAccountFields::doctor_note,
        "changed-by-reader".to_owned(),
    )
    .expect("semantic scalar patch");
    ctx.commit().expect("commit partial scalar patch");

    let authoritative = database
        .context::<HostileAccountSchema>()
        .expect("fresh authoritative context");
    let stored = authoritative
        .accounts
        .require(account.id)
        .expect("authoritative reread");
    assert_eq!(stored.passport_secret, "SECRET");
    assert_eq!(stored.doctor_note, "changed-by-reader");

    // The reader descriptor deliberately does not repeat the authoritative field rule. The full
    // persisted candidate still owns validation and rejects a patch that violates it.
    let invalid_ctx = database
        .context::<HostileReaderSchema>()
        .expect("fresh reader context");
    invalid_ctx
        .set(
            |schema| &schema.accounts,
            Id::new(378_001),
            HostileReaderAccountFields::doctor_note,
            "x".repeat(64),
        )
        .expect_err("persisted semantic rules reject invalid Candidate formation");
    let stored = authoritative
        .accounts
        .require(account.id)
        .expect("reread after rejected patch");
    assert_eq!(stored.passport_secret, "SECRET");
    assert_eq!(stored.doctor_note, "changed-by-reader");

    drop(invalid_ctx);
    drop(partial_tx);
    drop(ctx);
    drop(authoritative);
    drop(database);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn partial_context_remove_where_lowers_selection_to_identity_owned_delete() {
    use cfmd::{Permission, PermissionSet, PrincipalId, Session};

    let path = temp_path();
    let seed = create_typed::<HostileAccountSchema>(&path).expect("authoritative account schema");
    for (id, name, secret) in [(565_001, "drop", "SECRET-A"), (565_002, "keep", "SECRET-B")] {
        seed.add(
            |schema| &schema.accounts,
            HostileAccount {
                id: Id::new(id),
                name: name.to_owned(),
                passport_secret: secret.to_owned(),
                doctor_note: "hidden-note".to_owned(),
            },
        )
        .expect("seed account");
    }
    seed.commit().expect("seed commit");
    drop(seed);

    let database = Database::open(&path).expect("reopen authoritative account database");
    let restricted = database.session(Session::new(
        PrincipalId::new(565_100),
        PermissionSet::from([
            Permission::ReadRelation(HostileAccount::relation_id()),
            Permission::DeleteObject(HostileAccount::relation_id()),
        ]),
    ));
    let ctx = restricted
        .context::<HostileReaderSchema>()
        .expect("partial reader context");
    ctx.remove_where(
        |schema| &schema.accounts,
        |account| account.name().eq("drop".to_owned()),
    )
    .expect("stage identity-lowered partial query delete");
    ctx.commit().expect("commit partial query delete");

    let authoritative = database
        .context::<HostileAccountSchema>()
        .expect("authoritative reread context");
    assert!(
        authoritative
            .accounts
            .get(Id::new(565_001))
            .expect("deleted lookup")
            .is_none()
    );
    let kept = authoritative
        .accounts
        .require(Id::new(565_002))
        .expect("kept authoritative row");
    assert_eq!(kept.passport_secret, "SECRET-B");
    assert_eq!(kept.doctor_note, "hidden-note");

    drop(authoritative);
    drop(ctx);
    drop(database);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn client_bind_preserves_old_local_name_without_polluting_authoritative_schema() {
    use cfmd::{Permission, PermissionSet, PrincipalId, Session};

    let path = temp_path();
    let db = create_typed::<RenamedAccountSchema>(&path).expect("authoritative renamed schema");

    let account = RenamedAccount {
        id: Id::new(379_001),
        name: "rename-safe".to_owned(),
        medical_note: "old-visible".to_owned(),
    };
    db.add(|schema| &schema.accounts, account.clone())
        .expect("authoritative insert");
    db.commit().expect("authoritative commit");
    drop(db);

    // The authoritative schema has only `medical_note`; compatibility belongs to the client.
    let reopened = Database::open(&path).expect("raw reopen without Rust schema authority");
    let schema = reopened
        .snapshot()
        .expect("schema snapshot")
        .schema()
        .expect("schema metadata");
    let relation = RenamedAccount::relation_id();
    let medical_note = schema
        .relation(relation)
        .expect("renamed account relation")
        .column_ids()[2];
    let old = reopened
        .session(Session::new(
            PrincipalId::new(503_100),
            PermissionSet::from([
                Permission::Read,
                Permission::WriteField {
                    relation,
                    field: medical_note,
                },
            ]),
        ))
        .context::<LegacyRenameSchema>()
        .expect("legacy local field explicitly binds to the authorized persisted coordinate");
    let legacy = old.accounts.require(Id::new(379_001)).expect("legacy read");
    assert_eq!(legacy.doctor_note, "old-visible");

    old.set(
        |schema| &schema.accounts,
        Id::new(379_001),
        LegacyRenameAccountFields::doctor_note,
        "changed-through-old-name".to_owned(),
    )
    .expect("legacy patch resolves through the explicit client-side bind");
    old.commit().expect("legacy patch commit");

    let current = reopened
        .objects::<RenamedAccount>()
        .expect("authoritative shape remains exact and legacy-free")
        .require(account.id)
        .expect("authoritative reread");
    assert_eq!(current.medical_note, "changed-through-old-name");

    let invalid = reopened
        .context::<LegacyRenameSchema>()
        .expect("fresh legacy context");
    let error = invalid
        .set(
            |schema| &schema.accounts,
            Id::new(379_001),
            LegacyRenameAccountFields::doctor_note,
            "x".repeat(64),
        )
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
    let db = create_typed::<RenamedReferenceSchema>(&path)
        .expect("current authoritative reference schema");
    let doctor = RenameDoctor {
        id: Id::new(379_101),
        name: "Dr Semantic".to_owned(),
    };
    let patient = RenamedPatient {
        id: Id::new(379_102),
        primary_doctor: cfmd::Ref::new(doctor.id),
    };
    db.add(|schema| &schema.doctors, doctor.clone())
        .expect("doctor insert");
    db.add(|schema| &schema.patients, patient)
        .expect("patient insert");
    db.commit().expect("reference commit");
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
    let db = create_typed::<AppSchema>(&path).expect("authoritative app schema");
    let alice = User {
        id: Id::new(438_001),
        name: "Alice".into(),
    };
    let bob = User {
        id: Id::new(438_002),
        name: "Bob".into(),
    };
    db.add(|schema| &schema.users, alice.clone())
        .expect("alice");
    db.add(|schema| &schema.users, bob.clone()).expect("bob");
    db.add(
        |schema| &schema.tasks,
        Task {
            id: Id::new(438_010),
            title: "hidden-title".into(),
            owner: cfmd::Ref::new(alice.id),
            reviewer: None,
        },
    )
    .expect("task");
    db.commit().expect("seed commit");
    drop(db);
    let database = Database::open(&path).expect("reopen app database");

    let ctx = database
        .context::<PartialTaskSchema>()
        .expect("partial task context");
    ctx.set(
        |schema| &schema.tasks,
        Id::new(438_010),
        PartialTaskFields::owner,
        cfmd::Ref::new(bob.id),
    )
    .expect("required scoped reference patch");
    let candidate = ctx
        .tasks
        .require(Id::new(438_010))
        .expect("candidate reference reread");
    assert_eq!(candidate.owner.id(), bob.id);
    ctx.set(
        |schema| &schema.tasks,
        Id::new(438_010),
        PartialTaskFields::reviewer,
        Some(cfmd::Ref::new(alice.id)),
    )
    .expect("optional scoped reference patch after candidate observation");
    ctx.commit().expect("scoped reference patch commit");

    let authoritative = database
        .context::<AppSchema>()
        .expect("fresh authoritative context");
    let stored = authoritative
        .tasks
        .require(Id::new(438_010))
        .expect("authoritative reread");
    assert_eq!(stored.title, "hidden-title");
    assert_eq!(stored.owner.id(), bob.id);
    assert_eq!(stored.reviewer.as_ref().map(cfmd::Ref::id), Some(alice.id));

    drop(ctx);
    drop(database);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn bridged_partial_identity_delete_preserves_hidden_reference_lifecycle_and_delete_authority() {
    use cfmd::dynamic::{MigrationHistoryPolicy, MigrationModel};
    use cfmd::{Permission, PermissionSet, PrincipalId, Session};

    let path = temp_path();
    let source = Schema::builder()
        .revisions(564, 1)
        .object::<User>()
        .object::<Task>()
        .build()
        .expect("source object schema");
    let database = Database::create(&path, source).expect("source database");
    let user = User {
        id: Id::new(564_001),
        name: "owner".into(),
    };
    let task = Task {
        id: Id::new(564_010),
        title: "hidden".into(),
        owner: cfmd::Ref::new(user.id),
        reviewer: Some(cfmd::Ref::new(user.id)),
    };
    let snapshot = database.snapshot().expect("source snapshot");
    let mut seed = IntentJournal::new();
    snapshot
        .objects::<User>()
        .expect("users")
        .add(&mut seed, user.clone())
        .expect("user seed");
    snapshot
        .objects::<Task>()
        .expect("tasks")
        .add(&mut seed, task.clone())
        .expect("task seed");
    database.commit(&seed).expect("seed commit");
    drop(snapshot);

    let target = Schema::builder()
        .revisions(565, 1)
        .object::<User>()
        .object::<Task>()
        .build()
        .expect("target object schema");
    let migration = MigrationModel::new(564_565, target);
    let prepared = database
        .prepare_migration(&migration)
        .expect("prepare identity migration");
    database
        .execute_migration(
            &prepared,
            TransactionId::new(564_565),
            MigrationHistoryPolicy::Forget,
        )
        .expect("publish identity migration");

    let denied = database.session(Session::new(
        PrincipalId::new(564_100),
        PermissionSet::from([Permission::Read]),
    ));
    let denied_ctx = denied
        .context::<VersionedIdentityOnlyTaskSchema>()
        .expect("bridged partial context");
    let error = denied_ctx
        .remove_id(|schema| &schema.tasks, Id::new(564_010))
        .expect_err("read authority cannot delete through bridge");
    assert_eq!(error.kind(), ErrorKind::PermissionDenied);

    let delete_only = database.session(Session::new(
        PrincipalId::new(564_101),
        PermissionSet::from([Permission::DeleteObject(Task::relation_id())]),
    ));
    let ctx = delete_only
        .context::<VersionedIdentityOnlyTaskSchema>()
        .expect("bridged delete-only partial context");
    ctx.remove_id(|schema| &schema.tasks, Id::new(564_010))
        .expect("stage bridged identity delete");
    ctx.commit().expect("commit bridged identity delete");

    let check = database.snapshot().expect("post-delete snapshot");
    assert_eq!(
        check
            .objects::<Task>()
            .expect("tasks")
            .count()
            .expect("task count"),
        0
    );
    assert_eq!(
        check
            .objects::<User>()
            .expect("users")
            .count()
            .expect("user count"),
        1
    );

    drop(check);
    drop(database);
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
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("database");
    seed_object_many(&database);

    let ctx = database
        .context::<ParentReaderWriteSchema>()
        .expect("partial parent context");
    let parent = ctx.parents.require(Id::new(1)).expect("partial owner");
    assert_eq!(parent.children.count().expect("initial count"), 2);
    ctx.add(
        |schema| &schema.children,
        Child {
            id: Id::new(13),
            score: 130,
        },
    )
    .expect("stage child before relationship mutation");
    ctx.attach(&parent.children, Id::new(13))
        .expect("scoped attach staged child");
    assert_eq!(
        ctx.parents
            .require(Id::new(1))
            .expect("candidate owner")
            .children
            .count()
            .expect("candidate relationship count"),
        3
    );
    // `parent.children` was materialized before the first stage. Scoped mutation must rebind its
    // semantic owner identity to the current Candidate rather than read the stale snapshot.
    ctx.detach(&parent.children, Id::new(10))
        .expect("scoped detach from current Candidate");
    ctx.commit().expect("scoped relationship mutation commit");

    drop(parent);
    drop(ctx);
    drop(database);
    let database = Database::open(&path).expect("reopen relationship mutation database");
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
    assert_eq!(ids, vec![11, 13]);

    drop(snapshot);
    drop(database);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn context_field_coordinates_rebase_independent_fields_and_conflict_same_field() {
    let path = temp_path();
    let seed = create_typed::<AppSchema>(&path).expect("authoritative app schema");
    let alice = User {
        id: Id::new(438_101),
        name: "Alice".into(),
    };
    let bob = User {
        id: Id::new(438_102),
        name: "Bob".into(),
    };
    seed.add(|schema| &schema.users, alice.clone())
        .expect("alice");
    seed.add(|schema| &schema.users, bob.clone()).expect("bob");
    seed.add(
        |schema| &schema.tasks,
        Task {
            id: Id::new(438_110),
            title: "base".into(),
            owner: cfmd::Ref::new(alice.id),
            reviewer: None,
        },
    )
    .expect("task");
    seed.commit().expect("seed commit");
    drop(seed);

    let database = Database::open(&path).expect("reopen field-coordinate database");
    let title = database.context::<AppSchema>().expect("title context");
    let owner = database
        .context::<PartialTaskSchema>()
        .expect("owner partial context");
    title
        .set(
            |schema| &schema.tasks,
            Id::new(438_110),
            TaskFields::title,
            "title-a".to_string(),
        )
        .expect("title patch");
    owner
        .set(
            |schema| &schema.tasks,
            Id::new(438_110),
            PartialTaskFields::owner,
            cfmd::Ref::new(bob.id),
        )
        .expect("owner patch");

    title.commit().expect("first independent field commit");
    owner.commit().expect("stale independent field rebase");

    let check = database.context::<AppSchema>().expect("fresh check");
    let stored = check.tasks.require(Id::new(438_110)).expect("reread");
    assert_eq!(stored.title, "title-a");
    assert_eq!(stored.owner.id(), bob.id);

    let left = database.context::<AppSchema>().expect("left context");
    let right = database.context::<AppSchema>().expect("right context");
    left.set(
        |schema| &schema.tasks,
        Id::new(438_110),
        TaskFields::title,
        "left".to_string(),
    )
    .expect("left title");
    right
        .set(
            |schema| &schema.tasks,
            Id::new(438_110),
            TaskFields::title,
            "right".to_string(),
        )
        .expect("right title");
    left.commit().expect("left commit");
    let conflict = right
        .commit()
        .expect_err("same semantic field must conflict");
    assert_eq!(conflict.kind(), ErrorKind::TransactionConflict);

    drop(stored);
    drop(check);
    drop(left);
    drop(right);
    drop(owner);
    drop(title);
    drop(database);
    let reopened = Database::open(&path).expect("reopen");
    let ctx = reopened
        .context::<AppSchema>()
        .expect("reopen authoritative context");
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
    let session = Session::new(
        PrincipalId::new(440_001),
        PermissionSet::from([Permission::Read, Permission::MoveRelationship(relationship)]),
    );
    let mover = database
        .session(session.clone())
        .context::<ParentReaderWriteSchema>()
        .expect("move-authorized scoped Context");
    let source = mover.parents.require(Id::new(1)).expect("source parent");
    let destination = mover
        .parents
        .require(Id::new(2))
        .expect("destination parent");
    mover
        .move_to(&source.children, Id::new(10), &destination.children)
        .expect("semantic move planning");
    mover.commit().expect("move-only grant commit");

    let attach = database
        .session(session)
        .context::<ParentReaderWriteSchema>()
        .expect("fresh restricted Context");
    let destination = attach
        .parents
        .require(Id::new(2))
        .expect("destination parent");
    attach
        .attach(&destination.children, Id::new(11))
        .expect("attach planning remains structurally valid");
    assert_eq!(
        attach
            .commit()
            .expect_err("move grant must not authorize relationship attach")
            .kind(),
        ErrorKind::PermissionDenied
    );

    drop(source);
    drop(destination);
    drop(mover);
    drop(attach);
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
        PermissionSet::from([
            Permission::Read,
            Permission::DetachRelationship(relationship),
        ]),
    );
    let restricted = database.session(session.clone());
    let owner = restricted
        .objects::<Owner>()
        .expect("owners")
        .require(Id::new(1))
        .expect("owner");
    let mut tx = IntentJournal::new()
        .with_idempotency_key(TransactionId::new(440_011))
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

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep complete public semantic regression scenario together."
)]
fn transaction_reference_and_deep_path_reads_are_exact_causal_observations() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<PathRegion>()
        .object::<PathPassport>()
        .object::<PathPerson>()
        .build()
        .expect("path schema");
    let database = Database::create(&path, schema).expect("create path database");

    let region_id = Id::new(495_001);
    let passport_id = Id::new(495_010);
    let person_id = Id::new(495_100);
    let mut seed = IntentJournal::new();
    database
        .objects::<PathRegion>()
        .expect("regions")
        .add(
            &mut seed,
            PathRegion {
                id: region_id,
                code: "RU".to_owned(),
            },
        )
        .expect("region");
    database
        .objects::<PathPassport>()
        .expect("passports")
        .add(
            &mut seed,
            PathPassport {
                id: passport_id,
                number: "42".to_owned(),
                region: cfmd::Ref::new(region_id),
            },
        )
        .expect("passport");
    database
        .objects::<PathPerson>()
        .expect("people")
        .add(
            &mut seed,
            PathPerson {
                id: person_id,
                name: "Artem".to_owned(),
                passport: cfmd::Ref::new(passport_id),
            },
        )
        .expect("person");
    database.commit(&seed).expect("seed commit");

    let mut retroactive_ref = IntentJournal::new();
    database
        .objects::<PathRegion>()
        .expect("regions")
        .set(
            &mut retroactive_ref,
            region_id,
            PathRegionFields::code,
            "NL".to_owned(),
        )
        .expect("retroactive region change");

    let mut ref_observer = IntentJournal::new();
    database
        .objects::<PathPerson>()
        .expect("people")
        .set(
            &mut ref_observer,
            person_id,
            PathPersonFields::name,
            "RefSeen".to_owned(),
        )
        .expect("observer binding write");
    let person = ref_observer
        .objects::<PathPerson>()
        .expect("transaction people")
        .require(person_id)
        .expect("transaction person");
    let passport = person.passport.load().expect("explicit passport traversal");
    let region = passport.region.load().expect("explicit region traversal");
    assert_eq!(region.code, "RU");
    database
        .commit(&ref_observer)
        .expect("reference observer commit");
    let ref_conflict = database
        .commit(&retroactive_ref)
        .expect_err("reference traversal observation must block changed target value");
    assert_eq!(ref_conflict.kind(), ErrorKind::TransactionConflict);

    let mut retroactive_path = IntentJournal::new();
    database
        .objects::<PathRegion>()
        .expect("regions")
        .set(
            &mut retroactive_path,
            region_id,
            PathRegionFields::code,
            "DE".to_owned(),
        )
        .expect("second retroactive region change");

    let mut path_observer = IntentJournal::new();
    database
        .objects::<PathPerson>()
        .expect("people")
        .set(
            &mut path_observer,
            person_id,
            PathPersonFields::name,
            "PathSeen".to_owned(),
        )
        .expect("path observer binding write");
    let matching = path_observer
        .objects::<PathPerson>()
        .expect("transaction people")
        .where_(|person| person.passport().region().code().eq("RU".to_owned()))
        .count()
        .expect("deep path count");
    assert_eq!(matching, 1);
    database
        .commit(&path_observer)
        .expect("path observer commit");
    let path_conflict = database
        .commit(&retroactive_path)
        .expect_err("deep traversal observation must block changed joined value");
    assert_eq!(path_conflict.kind(), ErrorKind::TransactionConflict);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn transaction_many_count_is_exact_causal_observation() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .build()
        .expect("relationship schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create relationship database");
    seed_object_many(&database);

    let parent = database
        .objects::<Parent>()
        .expect("parents")
        .require(Id::new(1))
        .expect("parent");
    let mut retroactive = IntentJournal::new();
    parent
        .children
        .detach(&mut retroactive, Id::new(11))
        .expect("retroactive detach");
    drop(parent);

    let mut observer = IntentJournal::new();
    database
        .objects::<Parent>()
        .expect("parents")
        .set(
            &mut observer,
            Id::new(1),
            ParentFields::name,
            "observed".to_owned(),
        )
        .expect("observer binding write");
    let observed_parent = observer
        .objects::<Parent>()
        .expect("transaction parents")
        .require(Id::new(1))
        .expect("transaction parent");
    assert_eq!(observed_parent.children.count().expect("children count"), 2);
    database
        .commit(&observer)
        .expect("relationship observer commit");

    let error = database
        .commit(&retroactive)
        .expect_err("relationship count observation must block retroactive detach");
    assert_eq!(error.kind(), ErrorKind::TransactionConflict);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep complete public semantic regression scenario together."
)]
fn scoped_context_carries_session_authority_without_raw_database_escape() {
    use cfmd::{Permission, PermissionSet, PrincipalId, Session};

    let path = temp_path();
    let database = Database::builder(&path)
        .create_authoritative::<AppSchema>()
        .expect("authoritative app database");

    let seed = database.context::<AppSchema>().expect("seed context");
    let alice = User {
        id: Id::new(503_001),
        name: "Alice".into(),
    };
    seed.add(|schema| &schema.users, alice.clone())
        .expect("seed user");
    seed.add(
        |schema| &schema.tasks,
        Task {
            id: Id::new(503_010),
            title: "before".into(),
            owner: cfmd::Ref::new(alice.id),
            reviewer: None,
        },
    )
    .expect("seed task");
    seed.commit().expect("seed commit");

    let model = database
        .snapshot()
        .expect("model snapshot")
        .schema()
        .expect("model schema");
    let task_relation = Task::relation_id();
    let title_field = model
        .relation(task_relation)
        .expect("task relation")
        .column_ids()[1];

    let session = Session::new(
        PrincipalId::new(503_001),
        PermissionSet::from([Permission::WriteField {
            relation: task_relation,
            field: title_field,
        }]),
    );
    let scoped = database
        .session(session)
        .context::<AppSchema>()
        .expect("write-only session Context admission");

    assert_eq!(
        scoped
            .tasks
            .require(Id::new(503_010))
            .expect_err("write authority must not imply read authority")
            .kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(
        scoped
            .at(scoped.formation_revision())
            .expect_err("Context::at must require dedicated historical authority")
            .kind(),
        ErrorKind::PermissionDenied
    );

    scoped
        .set(
            |schema| &schema.tasks,
            Id::new(503_010),
            TaskFields::title,
            "after".to_owned(),
        )
        .expect("field-authorized Context patch");
    scoped.commit().expect("field-authorized Context commit");

    let revocable = Session::new(
        PrincipalId::new(503_002),
        PermissionSet::from([Permission::WriteField {
            relation: task_relation,
            field: title_field,
        }]),
    );
    let guarded = database
        .session(revocable.clone())
        .context::<AppSchema>()
        .expect("revocable session Context");
    guarded
        .set(
            |schema| &schema.tasks,
            Id::new(503_010),
            TaskFields::title,
            "must-not-publish".to_owned(),
        )
        .expect("stage before revocation");
    revocable.revoke().expect("revoke publication authority");
    assert_eq!(
        guarded
            .commit()
            .expect_err("revocation must be checked at publication")
            .kind(),
        ErrorKind::SessionRevoked
    );

    let no_history = database
        .session(Session::new(
            PrincipalId::new(503_003),
            PermissionSet::from([Permission::WriteField {
                relation: task_relation,
                field: title_field,
            }]),
        ))
        .context::<AppSchema>()
        .expect("write-only history test Context");
    assert_eq!(
        no_history
            .undo_latest()
            .expect_err("write permission must not bypass HistoryRead")
            .kind(),
        ErrorKind::PermissionDenied
    );

    let check = database.context::<AppSchema>().expect("unrestricted check");
    assert_eq!(
        check
            .tasks
            .require(Id::new(503_010))
            .expect("task after authorized patch")
            .title,
        "after"
    );

    drop(check);
    drop(database);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn grouped_exact_count_uses_gamma_keyed_sparse_witness_and_survives_reopen() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<FilteredMetric>()
        .model_rule(ModelRuleExpr::object_group_exact_count_where_range::<
            FilteredMetric,
            _,
            _,
            _,
        >(
            FilteredMetricFields::lower,
            |metric| {
                metric
                    .lower()
                    .rule()
                    .less_than_or_equal_field(metric.upper().rule())
            },
            1,
            Some(1),
        ))
        .build()
        .expect("grouped exact-count schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let add = |database: &Database, id, lower, upper| {
        let mut intent = IntentJournal::new();
        database
            .objects::<FilteredMetric>()
            .expect("metrics")
            .add(
                &mut intent,
                FilteredMetric {
                    id: Id::new(id),
                    value: 0.0,
                    lower,
                    upper,
                },
            )
            .expect("grouped count intent");
        database.commit(&intent)
    };

    add(&database, 522_001, 1, 2).expect("first selected row establishes group");
    add(&database, 522_002, 1, 0).expect("unselected row may join an already-satisfied group");

    let error = add(&database, 522_003, 9, 2)
        .expect_err("a new existing group with zero selected rows must violate min=1");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    add(&database, 522_004, 9, 10).expect("selected row establishes second group");
    drop(database);

    let database = Database::open(&path).expect("reopen grouped exact-count database");
    let error = add(&database, 522_005, 1, 3)
        .expect_err("second selected row in one semantic group must violate max=1");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn grouped_exact_f64_sum_reuses_gamma_sparse_domain_and_survives_reopen() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<FilteredMetric>()
        .model_rule(ModelRuleExpr::object_group_exact_f64_sum_where_range::<
            FilteredMetric,
            _,
            _,
            _,
            _,
        >(
            FilteredMetricFields::lower,
            FilteredMetricFields::value,
            |metric| {
                metric
                    .lower()
                    .rule()
                    .less_than_or_equal_field(metric.upper().rule())
            },
            Some(cfmd::FiniteF64::new(5.0).expect("finite lower bound")),
            Some(cfmd::FiniteF64::new(10.0).expect("finite upper bound")),
        ))
        .build()
        .expect("grouped exact-sum schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let add = |database: &Database, id, value, lower, upper| {
        let mut intent = IntentJournal::new();
        database
            .objects::<FilteredMetric>()
            .expect("metrics")
            .add(
                &mut intent,
                FilteredMetric {
                    id: Id::new(id),
                    value,
                    lower,
                    upper,
                },
            )
            .expect("grouped sum intent");
        database.commit(&intent)
    };

    add(&database, 523_001, 6.0, 1, 2).expect("selected value establishes valid group sum");
    add(&database, 523_002, 100.0, 1, 0).expect("unselected value must not affect grouped sum");
    let error = add(&database, 523_003, 100.0, 9, 2)
        .expect_err("new live group with zero selected sum violates min=5");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);
    add(&database, 523_004, 5.0, 9, 10).expect("selected value establishes second valid group");
    drop(database);

    let database = Database::open(&path).expect("reopen grouped exact-sum database");
    let error = add(&database, 523_005, 5.0, 1, 3)
        .expect_err("group-local exact sum 11 must violate max=10 after reopen");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn grouped_exact_count_product_compare_uses_one_gamma_domain_and_survives_reopen() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<FilteredMetric>()
        .model_rule(ModelRuleExpr::object_group_exact_count_compare::<
            FilteredMetric,
            _,
            _,
            _,
            _,
        >(
            FilteredMetricFields::lower,
            |_| SemanticRuleExpr::True,
            RuleOrderComparison::LessOrEqual,
            |metric| {
                metric
                    .lower()
                    .rule()
                    .less_than_or_equal_field(metric.upper().rule())
            },
        ))
        .build()
        .expect("grouped exact-count product schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let add = |database: &Database, id, lower, upper| {
        let mut intent = IntentJournal::new();
        database
            .objects::<FilteredMetric>()
            .expect("metrics")
            .add(
                &mut intent,
                FilteredMetric {
                    id: Id::new(id),
                    value: 1.0,
                    lower,
                    upper,
                },
            )
            .expect("grouped count product intent");
        database.commit(&intent)
    };

    add(&database, 524_001, 1, 2).expect("one valid row satisfies count(all)<=count(valid)");
    let error = add(&database, 524_002, 1, 0)
        .expect_err("invalid row must violate its local group comparator");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);
    add(&database, 524_003, 9, 10).expect("second canonical group has independent measures");
    drop(database);

    let database = Database::open(&path).expect("reopen grouped count product database");
    let error = add(&database, 524_004, 9, 1)
        .expect_err("reopened second group must retain exact product comparator");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn grouped_exact_f64_sum_product_compare_is_exact_and_survives_reopen() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<FilteredMetric>()
        .model_rule(ModelRuleExpr::object_group_exact_f64_sum_compare::<
            FilteredMetric,
            _,
            _,
            _,
            _,
            _,
            _,
        >(
            FilteredMetricFields::lower,
            FilteredMetricFields::value,
            |_| SemanticRuleExpr::True,
            RuleOrderComparison::LessOrEqual,
            FilteredMetricFields::value,
            |metric| {
                metric
                    .lower()
                    .rule()
                    .less_than_or_equal_field(metric.upper().rule())
            },
        ))
        .build()
        .expect("grouped exact-sum product schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let add = |database: &Database, id, value, lower, upper| {
        let mut intent = IntentJournal::new();
        database
            .objects::<FilteredMetric>()
            .expect("metrics")
            .add(
                &mut intent,
                FilteredMetric {
                    id: Id::new(id),
                    value,
                    lower,
                    upper,
                },
            )
            .expect("grouped sum product intent");
        database.commit(&intent)
    };

    add(&database, 524_101, 6.0, 1, 2).expect("equal exact group sums satisfy comparator");
    let error = add(&database, 524_102, 100.0, 1, 0)
        .expect_err("unselected right-side value must make exact left sum exceed right");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);
    add(&database, 524_103, 5.0, 9, 10).expect("independent exact-sum group");
    drop(database);

    let database = Database::open(&path).expect("reopen grouped exact-sum product database");
    let error = add(&database, 524_104, 7.0, 9, 0)
        .expect_err("reopened group must preserve exact sum-to-sum comparison");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

fn ordered_extrema_compare_rule() -> ModelRuleExpr {
    ModelRuleExpr::object_exact_extremum_compare::<FilteredMetric, FilteredMetric, f64, _, _, _, _>(
        FilteredMetricFields::value,
        |metric| {
            metric
                .lower()
                .rule()
                .less_than_or_equal_field(metric.upper().rule())
        },
        cfmd::OrderedExtremumKind::Min,
        RuleOrderComparison::LessOrEqual,
        FilteredMetricFields::value,
        |_| SemanticRuleExpr::True,
        cfmd::OrderedExtremumKind::Max,
    )
}

#[test]
fn ordered_extrema_use_semantic_multiset_delete_and_support_aligned_partiality() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<FilteredMetric>()
        .model_rule(ordered_extrema_compare_rule())
        .build()
        .expect("ordered extrema schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("empty support is aligned and valid");

    let invalid = FilteredMetric {
        id: Id::new(527_001),
        value: 100.0,
        lower: 9,
        upper: 1,
    };
    let mut intent = IntentJournal::new();
    database
        .objects::<FilteredMetric>()
        .expect("metrics")
        .add(&mut intent, invalid)
        .expect("invalid-side intent");
    assert_eq!(
        database
            .commit(&intent)
            .expect_err("one undefined extremum side must fail")
            .kind(),
        ErrorKind::InvariantViolation
    );
    drop(intent);

    let rows = [
        FilteredMetric {
            id: Id::new(527_010),
            value: 1.0,
            lower: 1,
            upper: 2,
        },
        FilteredMetric {
            id: Id::new(527_011),
            value: 2.0,
            lower: 1,
            upper: 2,
        },
        FilteredMetric {
            id: Id::new(527_012),
            value: 3.0,
            lower: 1,
            upper: 2,
        },
    ];
    for row in &rows {
        let mut intent = IntentJournal::new();
        database
            .objects::<FilteredMetric>()
            .expect("metrics")
            .add(&mut intent, row.clone())
            .expect("ordered extrema insert");
        database.commit(&intent).expect("min <= max");
    }

    let mut remove_min = IntentJournal::new();
    database
        .objects::<FilteredMetric>()
        .expect("metrics")
        .remove(&mut remove_min, rows[0].clone())
        .expect("remove current min intent");
    database
        .commit(&remove_min)
        .expect("deleting current min must advance from maintained ordered multiplicities");
    drop(remove_min);
    drop(database);

    let database = Database::open(&path).expect("reopen extrema database");
    let mut remove_max = IntentJournal::new();
    database
        .objects::<FilteredMetric>()
        .expect("metrics")
        .remove(&mut remove_max, rows[2].clone())
        .expect("remove current max intent");
    database
        .commit(&remove_max)
        .expect("reopened ordered witness must delete current max exactly");

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn ordered_lower_quantile_uses_exact_rank_and_survives_reopen() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<FilteredMetric>()
        .model_rule(ModelRuleExpr::object_exact_order_statistic_compare::<
            FilteredMetric,
            FilteredMetric,
            f64,
            _,
            _,
            _,
            _,
        >(
            FilteredMetricFields::value,
            |_| SemanticRuleExpr::True,
            cfmd::OrderedStatisticSelector::LowerQuantile {
                numerator: 1,
                denominator: 2,
            },
            RuleOrderComparison::LessOrEqual,
            FilteredMetricFields::value,
            |_| SemanticRuleExpr::True,
            cfmd::OrderedStatisticSelector::FromStart(0),
        ))
        .build()
        .expect("ordered statistic schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create ordered statistic database");

    let add = |database: &Database, id, value| {
        let mut intent = IntentJournal::new();
        database
            .objects::<FilteredMetric>()
            .expect("metrics")
            .add(
                &mut intent,
                FilteredMetric {
                    id: Id::new(id),
                    value,
                    lower: 0,
                    upper: 0,
                },
            )
            .expect("ordered statistic intent");
        database.commit(&intent)
    };

    add(&database, 528_001, 1.0).expect("single value has median == min");
    add(&database, 528_002, 3.0).expect("lower median of two values remains min");
    let error = add(&database, 528_003, 2.0)
        .expect_err("lower median of three distinct values must exceed min");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);
    drop(database);

    let database = Database::open(&path).expect("reopen ordered statistic database");
    let error = add(&database, 528_004, 2.0)
        .expect_err("reopened lower-quantile selector must preserve exact rank law");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn ordered_statistic_range_uses_semantic_bounds_and_survives_reopen() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<FilteredMetric>()
        .model_rule(ModelRuleExpr::object_exact_order_statistic_range::<
            FilteredMetric,
            f64,
            _,
            _,
        >(
            FilteredMetricFields::value,
            |_| SemanticRuleExpr::True,
            cfmd::OrderedStatisticSelector::LowerQuantile {
                numerator: 1,
                denominator: 2,
            },
            Some(1.0),
            Some(2.0),
        ))
        .build()
        .expect("ordered statistic range schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create ordered statistic range database");

    let add = |database: &Database, id, value| {
        let mut intent = IntentJournal::new();
        database
            .objects::<FilteredMetric>()
            .expect("metrics")
            .add(
                &mut intent,
                FilteredMetric {
                    id: Id::new(id),
                    value,
                    lower: 0,
                    upper: 0,
                },
            )
            .expect("ordered statistic range intent");
        database.commit(&intent)
    };

    add(&database, 529_001, 1.0).expect("single lower quantile lies on lower bound");
    add(&database, 529_002, 5.0).expect("lower quantile of two remains first value");
    let error = add(&database, 529_003, 3.0)
        .expect_err("lower quantile crossing semantic upper bound must fail");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);
    drop(database);

    let database = Database::open(&path).expect("reopen ordered statistic range database");
    let error = add(&database, 529_004, 4.0)
        .expect_err("reopened range must retain semantic canonical bound");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn grouped_ordered_statistic_range_is_bucket_local_and_survives_reopen() {
    let path = temp_path();
    let schema = Schema::builder()
        .object::<FilteredMetric>()
        .model_rule(ModelRuleExpr::object_group_exact_order_statistic_range::<
            FilteredMetric,
            _,
            f64,
            _,
            _,
            _,
        >(
            FilteredMetricFields::lower,
            FilteredMetricFields::value,
            |_| SemanticRuleExpr::True,
            cfmd::OrderedStatisticSelector::LowerQuantile {
                numerator: 1,
                denominator: 2,
            },
            Some(1.0),
            Some(2.0),
        ))
        .build()
        .expect("grouped ordered-statistic range schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create grouped ordered-statistic database");

    let add = |database: &Database, id, value, group| {
        let mut intent = IntentJournal::new();
        database
            .objects::<FilteredMetric>()
            .expect("metrics")
            .add(
                &mut intent,
                FilteredMetric {
                    id: Id::new(id),
                    value,
                    lower: group,
                    upper: 0,
                },
            )
            .expect("grouped ordered-statistic intent");
        database.commit(&intent)
    };

    add(&database, 530_001, 1.0, 1).expect("first group lower quantile is in range");
    add(&database, 530_002, 5.0, 1).expect("lower median of two stays at first value");
    add(&database, 530_003, 2.0, 9).expect("independent Γ bucket is valid");
    let error = add(&database, 530_004, 3.0, 1)
        .expect_err("only the touched Γ bucket may cross its statistic bound");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);
    drop(database);

    let database = Database::open(&path).expect("reopen grouped ordered-statistic database");
    let error = add(&database, 530_005, 4.0, 1)
        .expect_err("reopened bucket must preserve ordered multiplicity and bound authority");
    assert_eq!(error.kind(), ErrorKind::InvariantViolation);

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn migratable_watch_is_public_schema_neutral_surface() {
    use cfmd::dynamic::Query;

    fn open(database: &Database, query: &Query) -> cfmd::Result<cfmd::MigratableQueryWatch> {
        database.migratable_watch(query)
    }

    fn late_materialize(event: &cfmd::MigratableWatchEvent) -> cfmd::Result<usize> {
        event.materialize_schema(event.schema_revision(), |inserted, removed| {
            Ok(inserted.len() + removed.len())
        })
    }

    let _ = open as fn(&Database, &Query) -> cfmd::Result<cfmd::MigratableQueryWatch>;
    let _ = cfmd::MigratableWatchEvent::schema_revision as fn(&cfmd::MigratableWatchEvent) -> u64;
    let _ = late_materialize as fn(&cfmd::MigratableWatchEvent) -> cfmd::Result<usize>;
}

#[test]
fn migration_workflow_is_one_public_prepared_artifact() {
    let path = temp_path();
    let source = Schema::builder()
        .revisions(543, 1)
        .build()
        .expect("source schema");
    let database = Database::builder(&path)
        .schema(source)
        .create()
        .expect("create database");
    let target = Schema::builder()
        .revisions(544, 1)
        .build()
        .expect("target schema");
    let model = cfmd::dynamic::MigrationModel::new(543_544, target);

    let prepared = database
        .prepare_migration(&model)
        .expect("prepare migration");
    assert_eq!(prepared.plan().source_schema_revision(), 543);
    assert_eq!(prepared.plan().target_schema_revision(), 544);
    assert_eq!(
        prepared.plan().cost_class(),
        cfmd::MigrationCostClass::MetadataOnly
    );
    assert_eq!(prepared.preview().validation(), prepared.validation());

    let before = database
        .observe_migration(&prepared)
        .expect("observe prepared migration");
    assert_eq!(before.state(), cfmd::MigrationObservationState::Prepared);
    assert!(before.cutover().is_none());

    database
        .execute_migration(
            &prepared,
            TransactionId::new(543_544),
            cfmd::dynamic::MigrationHistoryPolicy::Forget,
        )
        .expect("execute prepared migration");
    let after = database
        .observe_migration(&prepared)
        .expect("observe migration cutover");
    assert_eq!(after.state(), cfmd::MigrationObservationState::CutOver);
    assert!(
        after
            .cutover()
            .expect("public cutover projection")
            .target_schema_is_current()
    );

    let pins = database
        .history_retention_pins()
        .expect("retained migration source epoch");
    assert_eq!(pins.len(), 1);
    let pin = pins[0];
    assert_eq!(pin.reason(), cfmd::HistoryRetentionReason::SchemaMigration);
    assert_eq!(pin.source_schema_revision(), 543);
    database
        .release_history_retention(&pin)
        .expect("release retained source epoch");
    assert!(database.history_retention_pins().unwrap().is_empty());
    database
        .release_history_retention(&pin)
        .expect("release is idempotent");

    drop(database);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn backup_verify_and_fresh_restore_are_public_format_v1_operations() {
    let source_path = temp_path();
    let backup_path = temp_path();
    let restore_path = temp_path();
    let schema = Schema::builder()
        .revisions(559, 1)
        .build()
        .expect("backup schema");
    let database = Database::builder(&source_path)
        .schema(schema)
        .create()
        .expect("create backup source");
    let source_revision = database.snapshot().unwrap().revision();

    let backup = database
        .backup_to(&backup_path, &cfmd::Encryption::None)
        .expect("create strict backup");
    assert_eq!(backup.revision(), source_revision);
    let verified = Database::verify_backup(&backup_path, &cfmd::Encryption::None)
        .expect("verify strict backup");
    assert_eq!(verified.revision(), source_revision);

    let restored = Database::restore_backup(
        &backup_path,
        &cfmd::Encryption::None,
        &restore_path,
        cfmd::Encryption::None,
    )
    .expect("fresh restore");
    assert_eq!(restored.snapshot().unwrap().revision(), source_revision);

    drop(restored);
    drop(database);
    fs::remove_file(source_path).expect("remove backup source");
    fs::remove_file(backup_path).expect("remove backup artifact");
    fs::remove_file(restore_path).expect("remove restored database");
}

#[test]
fn value_object_query_mutation_does_not_invent_semantic_identity() {
    cfmd::cfmd_object! {
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct ValueUser => ValueUserFields("pass580.value-user") {
            pub id: i64,
            pub name: String,
        }
    }

    let path = temp_path();
    let schema = Schema::builder()
        .object::<ValueUser>()
        .build()
        .expect("value-object schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("value-object database");

    let snapshot = database.snapshot().expect("snapshot");
    let values = snapshot.objects::<ValueUser>().expect("value users");
    let insert = values
        .insert(ValueUser {
            id: 1,
            name: "before".into(),
        })
        .expect("insert value object");
    drop(values);
    drop(snapshot);
    database
        .commit_plan(&insert, TransactionId::new(580_001))
        .expect("commit insert");

    let snapshot = database.snapshot().expect("snapshot");
    let values = snapshot.objects::<ValueUser>().expect("value users");
    let update = values
        .where_(|value| value.id().eq(1))
        .update_plan(|mut value| {
            value.name = "after".into();
            value
        })
        .expect("value-object rewrite");
    drop(values);
    drop(snapshot);
    database
        .commit_plan(&update, TransactionId::new(580_002))
        .expect("commit rewrite");

    let snapshot = database.snapshot().expect("snapshot");
    let values = snapshot.objects::<ValueUser>().expect("value users");
    assert_eq!(
        values
            .where_(|value| value.id().eq(1))
            .one()
            .expect("updated value object")
            .name,
        "after"
    );
    let delete = values
        .where_(|value| value.id().eq(1))
        .delete_plan()
        .expect("value-object delete");
    drop(values);
    drop(snapshot);
    database
        .commit_plan(&delete, TransactionId::new(580_003))
        .expect("commit delete");

    assert!(
        database
            .snapshot()
            .expect("snapshot")
            .objects::<ValueUser>()
            .expect("value users")
            .all()
            .expect("all values")
            .is_empty()
    );
    drop(database);
    fs::remove_file(path).expect("remove database");
}
