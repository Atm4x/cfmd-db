use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use cfmd::{
    CfmdEntity, CommitOutcome, Database, DiagnosticCode, ErrorDiagnosticExt, ErrorKind, Id, Schema,
    TransactionId,
};

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.todo")]
struct Todo {
    #[cfmd(id)]
    pub id: Id<Todo>,
    pub title: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.user")]
struct User {
    #[cfmd(id)]
    pub id: Id<User>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "example.task")]
struct Task {
    #[cfmd(id)]
    pub id: Id<Task>,
    pub title: String,
    pub owner: cfmd::Ref<User>,
    pub reviewer: Option<cfmd::Ref<User>>,
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

fn temp_path() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("cfmd-public-{nonce}-{}", std::process::id()))
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
            .commit(&plan, TransactionId::new(337_001))
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
        .transaction(TransactionId::new(351_001))
        .expect("transaction");
    let todos = transaction.objects::<Todo>().expect("todo set");
    transaction
        .apply(
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
        .apply(
            todos
                .insert(Todo {
                    id: Id::new(2),
                    title: "second".to_owned(),
                    done: false,
                })
                .expect("second insert"),
        )
        .expect("compose second");

    let preview = transaction.preview().expect("preview");
    assert_eq!(preview.effects().inserted_rows(), 2);
    assert_eq!(preview.source_revision(), transaction.base_revision());
    let first = transaction.commit().expect("commit transaction");
    let revision = match first {
        CommitOutcome::Committed { revision } => revision,
        CommitOutcome::AlreadyCommitted { .. } => panic!("first publication must commit"),
    };
    assert_eq!(
        transaction.commit().expect("idempotent transaction retry"),
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
fn snapshot_bound_transaction_rejects_cross_snapshot_plan_and_stale_commit() {
    let path = temp_path();
    let schema = Schema::builder().object::<Todo>().build().expect("schema");
    let database = Database::builder(&path)
        .schema(schema)
        .create()
        .expect("create database");

    let mut stale = database
        .transaction(TransactionId::new(351_010))
        .expect("stale transaction");
    let stale_todos = stale.objects::<Todo>().expect("stale todos");
    stale
        .apply(
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
        .transaction(TransactionId::new(351_011))
        .expect("winner transaction");
    let winner_todos = winner.objects::<Todo>().expect("winner todos");
    winner
        .apply(
            winner_todos
                .insert(Todo {
                    id: Id::new(11),
                    title: "winner".to_owned(),
                    done: false,
                })
                .expect("winner insert"),
        )
        .expect("compose winner");
    winner.commit().expect("winner commit");

    let stale_error = stale.commit().expect_err("stale transaction must fail");
    assert_eq!(stale_error.kind(), ErrorKind::StaleRevision);

    let mut old_transaction = database
        .transaction(TransactionId::new(351_012))
        .expect("old transaction");

    let mut advance = database
        .transaction(TransactionId::new(351_013))
        .expect("advance transaction");
    let advance_todos = advance.objects::<Todo>().expect("advance todos");
    advance
        .apply(
            advance_todos
                .insert(Todo {
                    id: Id::new(12),
                    title: "advance".to_owned(),
                    done: false,
                })
                .expect("advance insert"),
        )
        .expect("compose advance");
    advance.commit().expect("advance commit");

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
    let error = old_transaction
        .apply(foreign_plan)
        .expect_err("cross-snapshot plan must fail");
    assert_eq!(error.kind(), ErrorKind::InvalidPlan);

    drop(current);
    drop(database);
    fs::remove_file(path).expect("remove database");
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
        .commit(&plan, TransactionId::new(338_001))
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
        .commit(&plan, TransactionId::new(338_002))
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
        .commit(&plan, TransactionId::new(342_001))
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
        .update(|mut parent| {
            parent.name = "renamed".into();
            parent
        })
        .expect("scalar rewrite");
    drop(snapshot);
    database
        .commit(&plan, TransactionId::new(342_005))
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
        .update(|parent| {
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
        .commit(&plan, TransactionId::new(342_004))
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
        .delete()
        .expect("delete child plan");
    drop(snapshot);
    database
        .commit(&delete_child, TransactionId::new(342_002))
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
        .delete()
        .expect("delete parent plan");
    drop(snapshot);
    database
        .commit(&delete_parent, TransactionId::new(342_003))
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
        .commit(&plan, TransactionId::new(343_001))
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

    let conflict = owner2
        .assets
        .attach(Id::new(10))
        .expect("proposal")
        .candidate();
    assert!(conflict.is_err(), "OwnedMany must reject a second owner");

    let move_plan = owner1
        .assets
        .move_to(Id::new(10), &owner2.assets)
        .expect("move plan");
    drop(snapshot);
    database
        .commit(&move_plan, TransactionId::new(343_002))
        .expect("move");

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
        .commit(&plan, TransactionId::new(343_010))
        .expect("seed commit");

    let snapshot = database.snapshot().expect("snapshot");
    let delete = snapshot
        .objects::<Owner>()
        .expect("owners")
        .where_(|owner| owner.id().eq(Id::new(1)))
        .delete()
        .expect("delete owner");
    drop(snapshot);
    database
        .commit(&delete, TransactionId::new(343_011))
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
    let plan = selected
        .move_to(&destination.children)
        .expect("filtered edge move");
    assert!(
        source
            .children
            .move_ids_to([Id::new(999)], &destination.children)
            .is_err(),
        "move-by-id must not degrade into attach when the source edge is absent"
    );
    drop(snapshot);
    database
        .commit(&plan, TransactionId::new(344_001))
        .expect("move commit");

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
        .commit(&plan, TransactionId::new(344_010))
        .expect("seed commit");

    let snapshot = database.snapshot().expect("snapshot");
    let owner = snapshot
        .objects::<Owner>()
        .expect("owners")
        .require(Id::new(1))
        .expect("owner");
    let detach = owner
        .assets
        .where_(|asset| asset.label().eq("drop".to_owned()))
        .expect("selection")
        .detach_all()
        .expect("detach plan");
    let candidate = detach.candidate().expect("candidate");
    let preview = candidate.preview();
    assert_eq!(preview.derived().orphan_entities_deleted(), 1);
    assert!(preview.derived().normalized_rows_removed() >= 1);
    assert!(
        candidate
            .objects::<Asset>()
            .expect("candidate assets")
            .get(Id::new(11))
            .expect("candidate lookup")
            .is_none(),
        "orphan deletion must be visible in the candidate world before commit"
    );
    assert!(
        candidate
            .objects::<Asset>()
            .expect("candidate assets")
            .get(Id::new(10))
            .expect("candidate lookup")
            .is_some()
    );
    drop(snapshot);
    database
        .commit(&detach, TransactionId::new(344_011))
        .expect("detach commit");

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
        .commit(&plan, TransactionId::new(352_001))
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
        .commit(&seed, TransactionId::new(352_010))
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
        .commit(&below, TransactionId::new(352_011))
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
        .commit(&above, TransactionId::new(352_012))
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
