use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use cfmd_runtime::{
    CommitOutcome, Database, EquivalenceId, PrimitiveEquivalence, Query, RelationId,
    RelationResult, RelationSchema, Schema, TransactionId, Type, Value,
};

fn temp_directory() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("cfmd-runtime-{nonce}-{}", std::process::id()))
}

#[test]
fn product_api_creates_opens_queries_and_commits_without_kernel_imports() {
    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(100);
    let equivalence = EquivalenceId::new(101);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("build public schema");

    let database = Database::create(&directory, schema).expect("create through product facade");
    let initial = database.snapshot().expect("snapshot");
    assert_eq!(initial.revision().raw(), 1);
    let schema = initial.schema().expect("model read");
    assert_eq!(schema.revision(), 1);
    assert_eq!(
        schema.relation(relation).expect("relation").columns(),
        &[Type::i64()]
    );
    drop(initial);

    let mut seed = database.plan().expect("seed plan");
    seed.insert(relation, vec![Value::I64(1)])
        .insert(relation, vec![Value::I64(2)]);
    assert_eq!(
        database
            .commit_plan(&seed, TransactionId::new(699))
            .expect("seed commit"),
        CommitOutcome::Committed {
            revision: cfmd_runtime::RevisionId::new(2)
        }
    );

    let snapshot = database.snapshot().expect("snapshot after seed");
    let query = Query::scan(relation).filter_eq(0, Value::I64(2), equivalence);
    let prepared = snapshot.prepare(&query).expect("prepare query");
    assert_eq!(
        prepared.execute(&snapshot).expect("execute prepared"),
        RelationResult::Bag(vec![vec![Value::I64(2)]])
    );

    let mut plan = database.plan().expect("plan");
    plan.insert(relation, vec![Value::I64(3)]);
    let outcome = database
        .commit_plan(&plan, TransactionId::new(700))
        .expect("commit product plan");
    assert_eq!(
        outcome,
        CommitOutcome::Committed {
            revision: cfmd_runtime::RevisionId::new(3)
        }
    );
    assert_eq!(
        database
            .commit_plan(&plan, TransactionId::new(700))
            .expect("idempotent retry"),
        CommitOutcome::AlreadyCommitted {
            revision: cfmd_runtime::RevisionId::new(3)
        }
    );
    let mut conflicting = plan.clone();
    conflicting.insert(relation, vec![Value::I64(4)]);
    assert_eq!(
        database
            .commit_plan(&conflicting, TransactionId::new(700))
            .expect_err("transaction identity must bind intent")
            .kind(),
        cfmd_runtime::ErrorKind::TransactionConflict
    );

    drop(snapshot);
    drop(database);
    let reopened = Database::open(&directory).expect("reopen product database");
    let next = reopened.snapshot().expect("new snapshot");
    assert_eq!(next.revision().raw(), 3);
    assert_eq!(
        next.execute(&Query::scan(relation))
            .expect("scan after commit"),
        RelationResult::Bag(vec![
            vec![Value::I64(1)],
            vec![Value::I64(2)],
            vec![Value::I64(3)]
        ])
    );
    drop(next);
    drop(reopened);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn schema_rejects_semantic_id_reuse_across_namespaces() {
    use cfmd_runtime::{OrderingId, PrimitiveOrdering};

    let error = Schema::builder()
        .equivalence(EquivalenceId::new(77), PrimitiveEquivalence::I64Exact)
        .ordering(OrderingId::new(77), PrimitiveOrdering::I64Ascending)
        .build()
        .expect_err("one semantic id cannot name two facade contracts");
    assert_eq!(error.kind(), cfmd_runtime::ErrorKind::InvalidSchema);
}

#[test]
fn typed_rust_query_uses_domain_fields_and_typed_projection() {
    struct Numbers;

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation_id = RelationId::new(900);
    let equivalence = EquivalenceId::new(901);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(
            relation_id,
            [Type::i64(), Type::text()],
            [equivalence, EquivalenceId::new(902)],
        ))
        .equivalence(EquivalenceId::new(902), PrimitiveEquivalence::TextExact)
        .build()
        .expect("build schema");
    let database = Database::create(&directory, schema).expect("create database");
    let initial = database.snapshot().expect("initial snapshot");
    let numbers = initial
        .relation::<Numbers>(relation_id)
        .expect("typed relation");
    let mut seed = database.plan().expect("plan");
    seed.insert_typed(&numbers, (1_i64, "one".to_owned()))
        .expect("typed insert")
        .insert_typed(&numbers, (2_i64, "two".to_owned()))
        .expect("typed insert");
    database
        .commit_plan(&seed, TransactionId::new(902))
        .expect("seed data");
    drop(initial);

    let snapshot = database.snapshot().expect("snapshot");
    let numbers = snapshot
        .relation::<Numbers>(relation_id)
        .expect("typed relation");
    let number = numbers.field::<i64>(0).expect("number field");
    let name = numbers.field::<String>(1).expect("name field");

    let query = numbers.query().filter(number.eq(2)).select((number, name));
    let prepared = query.prepare(&snapshot).expect("prepare typed query");
    assert_eq!(
        prepared.all(&snapshot).expect("execute typed query"),
        vec![(2, "two".to_owned())]
    );

    drop(snapshot);
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn typed_fields_fail_fast_on_schema_type_mismatch() {
    struct Numbers;

    let relation_id = RelationId::new(910);
    let equivalence = EquivalenceId::new(911);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(
            relation_id,
            [Type::i64()],
            [equivalence],
        ))
        .build()
        .expect("schema");
    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let database = Database::create(&directory, schema).expect("create database");
    let snapshot = database.snapshot().expect("snapshot");
    let numbers = snapshot.relation::<Numbers>(relation_id).expect("relation");
    let Err(error) = numbers.field::<String>(0) else {
        panic!("wrong Rust type must be rejected");
    };
    assert_eq!(error.kind(), cfmd_runtime::ErrorKind::TypeMismatch);
    drop(snapshot);
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

cfmd_runtime::cfmd_object! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct User => UserFields("example.user") {
        pub id: i64,
        pub name: String,
        pub active: bool,
    }
}

#[test]
fn object_first_schema_query_and_plan_results_need_no_relation_plumbing() {
    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<User>()
        .build()
        .expect("object schema");
    let database = Database::create(&directory, schema).expect("create object database");

    let snapshot = database.snapshot().expect("snapshot");
    let users = snapshot.objects::<User>().expect("users object set");
    let insert = users
        .insert(User {
            id: 1,
            name: "Artem".to_owned(),
            active: true,
        })
        .expect("insert plan");
    database
        .commit_plan(&insert, TransactionId::new(2_840))
        .expect("commit object insert");
    drop(users);
    drop(snapshot);

    let snapshot = database.snapshot().expect("snapshot after insert");
    let users = snapshot.objects::<User>().expect("users object set");
    assert_eq!(
        users
            .where_(|u| u.id().eq(1))
            .select(|u| (u.name(), u.active()))
            .one()
            .expect("typed object projection"),
        ("Artem".to_owned(), true)
    );

    let update = users
        .where_(|u| u.id().eq(1))
        .update_plan(|mut user| {
            user.name = "Artem II".to_owned();
            user
        })
        .expect("query produces update plan");
    database
        .commit_plan(&update, TransactionId::new(2_841))
        .expect("commit object update");
    drop(users);
    drop(snapshot);

    let snapshot = database.snapshot().expect("snapshot after update");
    let users = snapshot.objects::<User>().expect("users object set");
    assert_eq!(
        users
            .where_(|u| u.id().eq(1))
            .one()
            .expect("updated object"),
        User {
            id: 1,
            name: "Artem II".to_owned(),
            active: true,
        }
    );

    let delete = users
        .where_(|u| u.id().eq(1))
        .delete_plan()
        .expect("query produces delete plan");
    database
        .commit_plan(&delete, TransactionId::new(2_842))
        .expect("commit object delete");
    drop(users);
    drop(snapshot);

    let snapshot = database.snapshot().expect("snapshot after delete");
    assert!(
        snapshot
            .objects::<User>()
            .expect("users")
            .all()
            .expect("all users")
            .is_empty()
    );
    drop(snapshot);
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn plans_are_bound_to_one_open_database_instance() {
    cfmd_runtime::cfmd_object! {
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct BoundRow => BoundRowFields("example.bound-row") {
            pub value: i64,
        }
    }

    let left_dir = temp_directory();
    let right_dir = left_dir.with_extension("other");
    fs::create_dir_all(&left_dir).expect("left dir");
    fs::create_dir_all(&right_dir).expect("right dir");
    let left = Database::create(
        &left_dir,
        Schema::builder()
            .object::<BoundRow>()
            .build()
            .expect("schema"),
    )
    .expect("left database");
    let right = Database::create(
        &right_dir,
        Schema::builder()
            .object::<BoundRow>()
            .build()
            .expect("schema"),
    )
    .expect("right database");
    let snapshot = left.snapshot().expect("snapshot");
    let rows = snapshot.objects::<BoundRow>().expect("rows");
    let plan = rows.insert(BoundRow { value: 7 }).expect("plan");
    let error = right
        .commit_plan(&plan, TransactionId::new(2_843))
        .expect_err("cross-database plan must fail closed");
    assert_eq!(error.kind(), cfmd_runtime::ErrorKind::InvalidPlan);
    drop(rows);
    drop(snapshot);
    drop(left);
    drop(right);
    fs::remove_dir_all(left_dir).expect("remove left");
    fs::remove_dir_all(right_dir).expect("remove right");
}

cfmd_runtime::cfmd_entity! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Country => CountryFields("example.country") {
        id pub id;
        fields { pub code: String }
        refs { }
    }
}

cfmd_runtime::cfmd_entity! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Passport => PassportFields("example.passport") {
        id pub id;
        fields { pub number: String }
        refs { pub country: Country }
    }
}

cfmd_runtime::cfmd_entity! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Person => PersonFields("example.person") {
        id pub id;
        fields { pub name: String }
        refs { pub passport: Passport }
    }
}

#[test]
fn entity_refs_validate_atomically_and_deep_predicates_preserve_root_shape() {
    use cfmd_runtime::{Id, Ref};

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .object::<Passport>()
        .object::<Person>()
        .build()
        .expect("entity schema");
    let database = Database::create(&directory, schema).expect("create entity database");
    let snapshot = database.snapshot().expect("snapshot");
    let countries = snapshot.objects::<Country>().expect("countries");
    let passports = snapshot.objects::<Passport>().expect("passports");
    let people = snapshot.objects::<Person>().expect("people");

    let country_id = Id::<Country>::new(1);
    let passport_id = Id::<Passport>::new(10);
    let person_id = Id::<Person>::new(100);
    let plan = countries
        .insert(Country {
            id: country_id,
            code: "RU".into(),
        })
        .expect("country plan")
        .and(
            passports
                .insert(Passport {
                    id: passport_id,
                    number: "42".into(),
                    country: Ref::new(country_id),
                })
                .expect("passport plan"),
        )
        .expect("compose")
        .and(
            people
                .insert(Person {
                    id: person_id,
                    name: "Artem".into(),
                    passport: Ref::new(passport_id),
                })
                .expect("person plan"),
        )
        .expect("compose");
    database
        .commit_plan(&plan, TransactionId::new(2_850))
        .expect("atomic strong-ref commit");
    drop(people);
    drop(passports);
    drop(countries);
    drop(snapshot);

    let snapshot = database.snapshot().expect("snapshot after insert");
    let people = snapshot.objects::<Person>().expect("people");
    let selected = people
        .where_(|person| {
            person.passport().matches(|passport| {
                passport
                    .country()
                    .matches(|country| country.code().eq("RU".to_owned()))
            })
        })
        .one()
        .expect("deep reference predicate");
    assert_eq!(selected.id, person_id);
    assert_eq!(selected.name, "Artem");

    let dangling = people
        .insert(Person {
            id: Id::new(101),
            name: "Broken".into(),
            passport: Ref::new(Id::new(999)),
        })
        .expect("dangling plan is inspectable before commit");
    let error = database
        .commit_plan(&dangling, TransactionId::new(2_851))
        .expect_err("strong reference must fail closed at commit");
    assert_eq!(error.kind(), cfmd_runtime::ErrorKind::InvariantViolation);

    let duplicate = people
        .insert(Person {
            id: person_id,
            name: "Duplicate".into(),
            passport: Ref::new(passport_id),
        })
        .expect("duplicate plan is inspectable before commit");
    let error = database
        .commit_plan(&duplicate, TransactionId::new(2_852))
        .expect_err("identity must be unique");
    assert_eq!(error.kind(), cfmd_runtime::ErrorKind::InvariantViolation);

    drop(people);
    drop(snapshot);
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn kernel_lifecycle_rejects_deleting_a_referenced_target_after_reopen() {
    use cfmd_runtime::{Id, Ref};

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .object::<Passport>()
        .build()
        .expect("entity schema");
    let database = Database::create(&directory, schema).expect("create database");
    let snapshot = database.snapshot().expect("snapshot");
    let countries = snapshot.objects::<Country>().expect("countries");
    let passports = snapshot.objects::<Passport>().expect("passports");
    let country_id = Id::<Country>::new(7);
    let passport_id = Id::<Passport>::new(7); // type-local ids may intentionally match.

    let plan = countries
        .insert(Country {
            id: country_id,
            code: "NL".into(),
        })
        .expect("country plan")
        .and(
            passports
                .insert(Passport {
                    id: passport_id,
                    number: "P7".into(),
                    country: Ref::new(country_id),
                })
                .expect("passport plan"),
        )
        .expect("compose");
    database
        .commit_plan(&plan, TransactionId::new(2_855))
        .expect("seed related entities");
    drop(passports);
    drop(countries);
    drop(snapshot);
    drop(database);

    let database = Database::open(&directory).expect("reopen");
    let snapshot = database.snapshot().expect("reopened snapshot");
    let countries = snapshot.objects::<Country>().expect("countries");
    let delete = countries
        .where_(|country| country.id().eq(country_id))
        .delete_plan()
        .expect("delete plan");
    let error = database
        .commit_plan(&delete, TransactionId::new(2_856))
        .expect_err("kernel lifecycle must reject a dangling strong reference");
    assert_eq!(error.kind(), cfmd_runtime::ErrorKind::InvariantViolation);

    drop(countries);
    drop(snapshot);
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

cfmd_runtime::cfmd_entity! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Parent => ParentFields("example.parent") {
        id pub id;
        fields { pub name: String }
        refs { }
        optional_refs { }
        many { }
    }
}

cfmd_runtime::cfmd_entity! {
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Child => ChildFields("example.child") {
        id pub id;
        fields { pub score: i64 }
        refs { }
        optional_refs { pub mentor: Parent }
        many { }
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn optional_refs_have_explicit_cardinality_semantics() {
    use cfmd_runtime::{Id, Ref};

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Parent>()
        .object::<Child>()
        .build()
        .expect("cardinality schema");
    let database = Database::create(&directory, schema).expect("create cardinality database");
    let snapshot = database.snapshot().expect("snapshot");
    let parents = snapshot.objects::<Parent>().expect("parents");
    let children = snapshot.objects::<Child>().expect("children");

    let p1 = Id::<Parent>::new(1);
    let p2 = Id::<Parent>::new(2);
    let p3 = Id::<Parent>::new(3);
    let plan = parents
        .insert(Parent {
            id: p1,
            name: "one".into(),
        })
        .expect("p1")
        .and(
            parents
                .insert(Parent {
                    id: p2,
                    name: "two".into(),
                })
                .expect("p2"),
        )
        .expect("compose")
        .and(
            parents
                .insert(Parent {
                    id: p3,
                    name: "three".into(),
                })
                .expect("p3"),
        )
        .expect("compose")
        .and(
            children
                .insert(Child {
                    id: Id::new(10),
                    score: 10,
                    mentor: None,
                })
                .expect("c1"),
        )
        .expect("compose")
        .and(
            children
                .insert(Child {
                    id: Id::new(11),
                    score: 10,
                    mentor: Some(Ref::new(p2)),
                })
                .expect("c2"),
        )
        .expect("compose")
        .and(
            children
                .insert(Child {
                    id: Id::new(12),
                    score: 20,
                    mentor: None,
                })
                .expect("c3"),
        )
        .expect("compose");
    database
        .commit_plan(&plan, TransactionId::new(2_860))
        .expect("cardinality commit");
    drop(children);
    drop(parents);
    drop(snapshot);

    let snapshot = database.snapshot().expect("snapshot after commit");
    let parents = snapshot.objects::<Parent>().expect("parents");
    let children = snapshot.objects::<Child>().expect("children");

    assert_eq!(
        children
            .where_(|c| c.mentor().is_some())
            .one()
            .expect("optional is some")
            .id,
        Id::new(11)
    );
    assert_eq!(
        children
            .where_(|c| c.mentor().eq(p2))
            .one()
            .expect("optional equality")
            .id,
        Id::new(11)
    );
    assert_eq!(
        children
            .where_(|c| c.mentor().is_none())
            .all()
            .expect("optional none")
            .len(),
        2
    );

    let dangling = children
        .insert(Child {
            id: Id::new(13),
            score: 30,
            mentor: Some(Ref::new(Id::new(999))),
        })
        .expect("dangling optional ref plan");
    let error = database
        .commit_plan(&dangling, TransactionId::new(2_861))
        .expect_err("optional strong ref must validate when present");
    assert_eq!(error.kind(), cfmd_runtime::ErrorKind::InvariantViolation);

    drop(children);
    drop(parents);
    drop(snapshot);
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
#[allow(clippy::too_many_lines)] // One end-to-end scenario intentionally keeps source, candidate, commit, and closed-runtime assertions contiguous.
fn candidate_previews_object_future_and_commits_the_same_plan() {
    use cfmd_runtime::{ErrorKind, Id};

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .build()
        .expect("country schema");
    let database = Database::create(&directory, schema).expect("create database");
    let id = Id::<Country>::new(42);

    let snapshot = database.snapshot().expect("initial snapshot");
    let countries = snapshot.objects::<Country>().expect("countries");
    let seed = countries
        .insert(Country {
            id,
            code: "NL".into(),
        })
        .expect("seed plan");
    database
        .commit_plan(&seed, TransactionId::new(2_900))
        .expect("seed commit");
    drop(countries);
    drop(snapshot);

    let snapshot = database.snapshot().expect("source snapshot");
    let countries = snapshot.objects::<Country>().expect("countries");
    assert_eq!(
        countries
            .get(id)
            .expect("get source")
            .expect("present")
            .code,
        "NL"
    );
    assert_eq!(
        countries
            .require(Id::new(999))
            .expect_err("missing identity")
            .kind(),
        ErrorKind::NotFound
    );

    let plan = countries
        .where_(|country| country.id().eq(id))
        .update_plan(|mut country| {
            country.code = "DE".into();
            country
        })
        .expect("update plan");
    let candidate = plan.candidate().expect("candidate");
    assert_eq!(candidate.source_revision(), snapshot.revision());
    assert_eq!(candidate.revision().raw(), snapshot.revision().raw() + 1);
    assert_eq!(candidate.changes().len(), 1);
    assert_eq!(candidate.changes()[0].inserted().len(), 1);
    assert_eq!(candidate.changes()[0].removed().len(), 1);
    let effects = candidate.effects();
    assert_eq!(effects.touched_relations(), 1);
    assert_eq!(effects.inserted_rows(), 1);
    assert_eq!(effects.removed_rows(), 1);
    assert_eq!(effects.entity_relations(), 1);
    assert!(effects.changes_lifecycle());
    let diagnostics = candidate.diagnostics();
    assert!(diagnostics.invariants_validated());
    assert_eq!(
        diagnostics.readiness(),
        cfmd_runtime::CandidateReadiness::Ready
    );
    let preview = candidate.preview();
    assert_eq!(preview.source_revision(), candidate.source_revision());
    assert_eq!(preview.target_revision(), candidate.revision());
    assert_eq!(preview.effects(), effects);
    assert_eq!(preview.diagnostics(), diagnostics);

    // The source snapshot is immutable; only the candidate sees the proposed future.
    assert_eq!(countries.require(id).expect("source object").code, "NL");
    let proposed = candidate.objects::<Country>().expect("candidate countries");
    assert_eq!(proposed.require(id).expect("candidate object").code, "DE");
    assert_eq!(
        proposed
            .where_(|country| country.code().eq("DE".to_owned()))
            .one()
            .expect("candidate predicate")
            .id,
        id
    );
    assert_eq!(
        proposed
            .where_(|country| country.id().eq(id))
            .select(CountryFields::code)
            .one()
            .expect("typed candidate projection"),
        "DE".to_owned()
    );

    assert_eq!(
        database
            .commit_plan(&plan, TransactionId::new(2_901))
            .expect("candidate commit"),
        CommitOutcome::Committed {
            revision: candidate.revision()
        }
    );
    assert_eq!(
        database
            .commit_plan(&plan, TransactionId::new(2_901))
            .expect("candidate retry"),
        CommitOutcome::AlreadyCommitted {
            revision: candidate.revision()
        }
    );
    assert_eq!(
        candidate.diagnostics().readiness(),
        cfmd_runtime::CandidateReadiness::Stale {
            current_revision: candidate.revision()
        }
    );

    drop(proposed);
    drop(countries);
    drop(snapshot);
    let snapshot = database.snapshot().expect("committed snapshot");
    assert_eq!(
        snapshot
            .objects::<Country>()
            .expect("countries")
            .require(id)
            .expect("committed object")
            .code,
        "DE"
    );
    drop(snapshot);
    drop(database);
    assert_eq!(
        candidate.diagnostics().readiness(),
        cfmd_runtime::CandidateReadiness::RuntimeClosed
    );
    assert_eq!(
        candidate
            .objects::<Country>()
            .expect("closed-runtime candidate remains previewable")
            .require(id)
            .expect("candidate object")
            .code,
        "DE"
    );
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn relation_only_plan_candidate_uses_the_same_derived_endpoint_as_commit() {
    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(9_100);
    let equivalence = EquivalenceId::new(9_101);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("schema");
    let database = Database::create(&directory, schema).expect("database");
    let mut plan = database.plan().expect("plan");
    plan.insert(relation, vec![Value::I64(7)]);
    let candidate = plan.candidate().expect("candidate");
    assert_eq!(
        candidate.execute(&Query::scan(relation)).expect("preview"),
        RelationResult::Bag(vec![vec![Value::I64(7)]])
    );
    database
        .commit_plan(&plan, TransactionId::new(2_902))
        .expect("commit exact candidate intent");
    let snapshot = database.snapshot().expect("snapshot");
    assert_eq!(
        snapshot.execute(&Query::scan(relation)).expect("committed"),
        candidate
            .execute(&Query::scan(relation))
            .expect("candidate remains readable")
    );
    drop(snapshot);
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn durable_history_derives_undo_and_redo_as_ordinary_plans() {
    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(7_700);
    let equivalence = EquivalenceId::new(7_701);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("build schema");
    let database = Database::create(&directory, schema).expect("create database");

    let mut seed = database.plan().expect("seed plan");
    seed.insert(relation, vec![Value::I64(1)]);
    database
        .commit_plan(&seed, TransactionId::new(7_710))
        .expect("seed commit");

    let mut change = database.plan().expect("change plan");
    change.insert(relation, vec![Value::I64(2)]);
    database
        .commit_plan(&change, TransactionId::new(7_711))
        .expect("change commit");

    let history = database.history().expect("durable history");
    assert_eq!(history.revision(), cfmd_runtime::RevisionId::new(3));
    assert_eq!(history.entries().len(), 2);
    let latest = history.latest().expect("head history entry");
    assert_eq!(latest.transaction(), TransactionId::new(7_711));
    assert_eq!(latest.source_revision(), cfmd_runtime::RevisionId::new(2));
    assert_eq!(latest.target_revision(), cfmd_runtime::RevisionId::new(3));
    assert_eq!(
        latest.reversibility(),
        cfmd_runtime::HistoryReversibility::ExactPlanInverse
    );
    assert_eq!(latest.changes().len(), 1);
    assert_eq!(latest.changes()[0].inserted(), &[vec![Value::I64(2)]]);

    let undo = history.undo_latest().expect("derive undo plan");
    let preview = undo.candidate().expect("preview undo");
    assert_eq!(preview.source_revision(), cfmd_runtime::RevisionId::new(3));
    assert_eq!(preview.revision(), cfmd_runtime::RevisionId::new(4));
    database
        .commit_plan(&undo, TransactionId::new(7_712))
        .expect("commit ordinary undo plan");
    assert_eq!(
        database
            .snapshot()
            .expect("snapshot after undo")
            .execute(&Query::scan(relation))
            .expect("scan after undo"),
        RelationResult::Bag(vec![vec![Value::I64(1)]])
    );

    let redo_history = database.history().expect("history after undo");
    let undo_entry = redo_history.latest().expect("undo history entry");
    assert_eq!(undo_entry.transaction(), TransactionId::new(7_712));
    let redo = undo_entry.undo_plan().expect("undo-of-undo is redo plan");
    let _redo_candidate = redo.candidate().expect("preview redo");
    database
        .commit_plan(&redo, TransactionId::new(7_713))
        .expect("commit redo");
    assert_eq!(
        database
            .snapshot()
            .expect("snapshot after redo")
            .execute(&Query::scan(relation))
            .expect("scan after redo"),
        RelationResult::Bag(vec![vec![Value::I64(1)], vec![Value::I64(2)]])
    );

    drop(database);
    let reopened = Database::open(&directory).expect("reopen database");
    let reopened_history = reopened.history().expect("history survives reopen");
    assert_eq!(reopened_history.entries().len(), 4);
    assert_eq!(
        reopened_history
            .latest()
            .expect("reopened head")
            .transaction(),
        TransactionId::new(7_713)
    );
    drop(reopened);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn object_history_persists_exact_lifecycle_complement_for_undo_and_redo() {
    use cfmd_runtime::{HistoryEffectKind, HistoryReversibility, Id};

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .build()
        .expect("country schema");
    let database = Database::create(&directory, schema).expect("create database");
    let id = Id::<Country>::new(77);

    let snapshot = database.snapshot().expect("initial snapshot");
    let countries = snapshot.objects::<Country>().expect("countries");
    let seed = countries
        .insert(Country {
            id,
            code: "NL".into(),
        })
        .expect("seed plan");
    database
        .commit_plan(&seed, TransactionId::new(7_720))
        .expect("seed entity");
    drop(countries);
    drop(snapshot);
    drop(database);

    let database = Database::open(&directory).expect("reopen before undo");
    let history = database.history().expect("durable history");
    let entry = history.latest().expect("seed entry");
    assert_eq!(entry.kind(), HistoryEffectKind::MixedRevision);
    assert_eq!(
        entry.reversibility(),
        HistoryReversibility::ExactPlanInverse
    );

    let undo = entry.undo_plan().expect("derive lifecycle-aware undo");
    let candidate = undo.candidate().expect("preview lifecycle-aware undo");
    assert!(candidate.effects().changes_lifecycle());
    assert!(
        candidate
            .objects::<Country>()
            .expect("candidate countries")
            .get(id)
            .expect("candidate lookup")
            .is_none()
    );
    database
        .commit_plan(&undo, TransactionId::new(7_721))
        .expect("commit entity removal through ordinary plan");
    drop(database);

    let database = Database::open(&directory).expect("reopen before redo");
    let history = database.history().expect("history after undo");
    let undo_entry = history.latest().expect("undo entry");
    assert_eq!(
        undo_entry.reversibility(),
        HistoryReversibility::ExactPlanInverse
    );
    let redo = undo_entry.undo_plan().expect("undo-of-undo is exact redo");
    let candidate = redo.candidate().expect("preview redo");
    assert_eq!(
        candidate
            .objects::<Country>()
            .expect("candidate countries")
            .require(id)
            .expect("country restored in proposed future")
            .code,
        "NL"
    );
    database
        .commit_plan(&redo, TransactionId::new(7_722))
        .expect("commit redo");
    drop(database);

    let reopened = Database::open(&directory).expect("final reopen");
    assert_eq!(
        reopened
            .snapshot()
            .expect("snapshot")
            .objects::<Country>()
            .expect("countries")
            .require(id)
            .expect("restored country")
            .code,
        "NL"
    );
    drop(reopened);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn object_history_persists_reference_field_complement() {
    use cfmd_runtime::{HistoryReversibility, Id, Ref};

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .object::<Passport>()
        .build()
        .expect("reference schema");
    let database = Database::create(&directory, schema).expect("create database");
    let first = Id::<Country>::new(1);
    let second = Id::<Country>::new(2);
    let passport_id = Id::<Passport>::new(10);
    let snapshot = database.snapshot().expect("snapshot");
    let countries = snapshot.objects::<Country>().expect("countries");
    let passports = snapshot.objects::<Passport>().expect("passports");
    let seed = countries
        .insert(Country {
            id: first,
            code: "NL".into(),
        })
        .expect("first country")
        .and(
            countries
                .insert(Country {
                    id: second,
                    code: "DE".into(),
                })
                .expect("second country"),
        )
        .expect("compose countries")
        .and(
            passports
                .insert(Passport {
                    id: passport_id,
                    number: "P10".into(),
                    country: Ref::new(first),
                })
                .expect("passport"),
        )
        .expect("compose passport");
    database
        .commit_plan(&seed, TransactionId::new(7_730))
        .expect("seed referenced entities");
    drop(passports);
    drop(countries);
    drop(snapshot);

    let snapshot = database.snapshot().expect("snapshot for update");
    let passports = snapshot.objects::<Passport>().expect("passports");
    let update = passports
        .where_(|passport| passport.id().eq(passport_id))
        .update_plan(|mut passport| {
            passport.country = Ref::new(second);
            passport
        })
        .expect("reference update");
    database
        .commit_plan(&update, TransactionId::new(7_731))
        .expect("commit reference update");
    drop(passports);
    drop(snapshot);
    drop(database);

    let database = Database::open(&directory).expect("reopen");
    let entry = database
        .history()
        .expect("history")
        .latest()
        .expect("reference entry")
        .clone();
    assert_eq!(
        entry.reversibility(),
        HistoryReversibility::ExactPlanInverse
    );
    let undo = entry.undo_plan().expect("reference undo plan");
    let candidate = undo.candidate().expect("reference undo candidate");
    assert!(!candidate.effects().changes_lifecycle());
    assert_eq!(
        candidate
            .objects::<Passport>()
            .expect("passports")
            .require(passport_id)
            .expect("passport")
            .country,
        Ref::new(first)
    );
    database
        .commit_plan(&undo, TransactionId::new(7_732))
        .expect("commit reference undo");
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn historical_at_reuses_typed_reads_and_anchors_history() {
    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(7_800);
    let equivalence = EquivalenceId::new(7_801);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("build schema");
    let database = Database::create(&directory, schema).expect("create database");

    let mut first = database.plan().expect("first plan");
    first.insert(relation, vec![Value::I64(10)]);
    database
        .commit_plan(&first, TransactionId::new(7_810))
        .expect("first commit");
    let first_revision = database.current_revision().expect("first revision");

    let mut second = database.plan().expect("second plan");
    second.insert(relation, vec![Value::I64(20)]);
    database
        .commit_plan(&second, TransactionId::new(7_811))
        .expect("second commit");

    let past = database.at(first_revision).expect("historical snapshot");
    assert_eq!(past.revision(), first_revision);
    assert_eq!(
        past.execute(&Query::scan(relation))
            .expect("historical query"),
        RelationResult::Bag(vec![vec![Value::I64(10)]])
    );
    let anchored = past
        .history()
        .expect("history anchored at historical revision");
    assert_eq!(anchored.revision(), first_revision);
    assert_eq!(anchored.entries().len(), 1);
    assert_eq!(
        anchored.latest().expect("historical head").transaction(),
        TransactionId::new(7_810)
    );

    drop(anchored);
    drop(past);
    drop(database);
    let reopened = Database::open(&directory).expect("reopen database");
    let past = reopened
        .at(first_revision)
        .expect("reconstruct after reopen");
    assert_eq!(
        past.execute(&Query::scan(relation))
            .expect("reopened historical query"),
        RelationResult::Bag(vec![vec![Value::I64(10)]])
    );
    drop(reopened);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn historical_at_reconstructs_object_lifecycle_and_is_read_only() {
    use cfmd_runtime::Id;

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .build()
        .expect("country schema");
    let database = Database::create(&directory, schema).expect("create database");
    let empty_revision = database.current_revision().expect("empty revision");
    let id = Id::<Country>::new(88);

    let live = database.snapshot().expect("live snapshot");
    let insert = live
        .objects::<Country>()
        .expect("countries")
        .insert(Country {
            id,
            code: "NL".into(),
        })
        .expect("insert plan");
    database
        .commit_plan(&insert, TransactionId::new(7_820))
        .expect("insert country");
    let inserted_revision = database.current_revision().expect("inserted revision");
    drop(live);

    let live = database.snapshot().expect("update snapshot");
    let update = live
        .objects::<Country>()
        .expect("countries")
        .where_(|country| country.id().eq(id))
        .update_plan(|mut country| {
            country.code = "DE".into();
            country
        })
        .expect("update plan");
    database
        .commit_plan(&update, TransactionId::new(7_821))
        .expect("update country");
    drop(live);

    let inserted = database.at(inserted_revision).expect("inserted world");
    let countries = inserted.objects::<Country>().expect("historical countries");
    assert_eq!(
        countries.require(id).expect("historical country").code,
        "NL"
    );
    let mutation_error = countries
        .insert(Country {
            id: Id::new(99),
            code: "FR".into(),
        })
        .expect_err("historical world must not create a Plan");
    assert_eq!(mutation_error.kind(), cfmd_runtime::ErrorKind::InvalidPlan);

    let empty = database.at(empty_revision).expect("empty world");
    assert!(
        empty
            .objects::<Country>()
            .expect("historical countries")
            .get(id)
            .expect("lookup")
            .is_none()
    );

    drop(countries);
    drop(inserted);
    drop(empty);
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn non_head_history_undo_rebases_over_disjoint_canonical_relation_classes() {
    use cfmd_runtime::HistoryUndoReadiness;

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(7_900);
    let equivalence = EquivalenceId::new(7_901);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("build schema");
    let database = Database::create(&directory, schema).expect("create database");

    let mut first = database.plan().expect("first plan");
    first.insert(relation, vec![Value::I64(1)]);
    database
        .commit_plan(&first, TransactionId::new(7_910))
        .expect("first commit");
    let mut second = database.plan().expect("second plan");
    second.insert(relation, vec![Value::I64(2)]);
    database
        .commit_plan(&second, TransactionId::new(7_911))
        .expect("second commit");
    drop(database);

    let database = Database::open(&directory).expect("reopen before non-head undo");
    let history = database.history().expect("reopened history");
    let first_entry = history
        .entries()
        .iter()
        .find(|entry| entry.transaction() == TransactionId::new(7_910))
        .expect("first entry after reopen")
        .clone();

    assert!(matches!(
        first_entry.undo_readiness(),
        HistoryUndoReadiness::Rebased {
            intervening_effects,
            ..
        } if intervening_effects.len() == 1
    ));
    let rebased = first_entry
        .undo_plan()
        .expect("certified rebased undo plan");
    let _rebased_candidate = rebased.candidate().expect("rebased candidate");
    database
        .commit_plan(&rebased, TransactionId::new(7_912))
        .expect("commit rebased undo");
    assert_eq!(
        database
            .snapshot()
            .expect("snapshot")
            .execute(&Query::scan(relation))
            .expect("scan"),
        RelationResult::Bag(vec![vec![Value::I64(2)]])
    );

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn non_head_history_undo_reports_same_coordinate_conflict() {
    use cfmd_runtime::HistoryUndoReadiness;

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(7_920);
    let equivalence = EquivalenceId::new(7_921);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("build schema");
    let database = Database::create(&directory, schema).expect("create database");

    let mut first = database.plan().expect("first plan");
    first.insert(relation, vec![Value::I64(1)]);
    database
        .commit_plan(&first, TransactionId::new(7_930))
        .expect("first commit");
    let first_entry = database
        .history()
        .expect("history")
        .latest()
        .expect("first entry")
        .clone();

    let mut second = database.plan().expect("second plan");
    second.remove(relation, vec![Value::I64(1)]);
    database
        .commit_plan(&second, TransactionId::new(7_931))
        .expect("second commit");

    assert!(matches!(
        first_entry.undo_readiness(),
        HistoryUndoReadiness::Conflict {
            conflicting_effects,
            conflicting_coordinates: 1,
            ..
        } if conflicting_effects.len() == 1
    ));
    let error = first_entry
        .undo_plan()
        .expect_err("same canonical class must conflict");
    assert_eq!(error.kind(), cfmd_runtime::ErrorKind::HistoryRebaseConflict);

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn non_head_object_undo_rebases_over_disjoint_entity_coordinates() {
    use cfmd_runtime::{HistoryUndoReadiness, Id};

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .build()
        .expect("country schema");
    let database = Database::create(&directory, schema).expect("create database");
    let first_id = Id::<Country>::new(201);
    let second_id = Id::<Country>::new(202);

    let snapshot = database.snapshot().expect("snapshot");
    let countries = snapshot.objects::<Country>().expect("countries");
    let seed = countries
        .insert(Country {
            id: first_id,
            code: "NL".into(),
        })
        .expect("first insert")
        .and(
            countries
                .insert(Country {
                    id: second_id,
                    code: "US".into(),
                })
                .expect("second insert"),
        )
        .expect("compose seed");
    database
        .commit_plan(&seed, TransactionId::new(7_940))
        .expect("seed countries");
    drop(countries);
    drop(snapshot);

    let snapshot = database.snapshot().expect("first update snapshot");
    let first_change = snapshot
        .objects::<Country>()
        .expect("countries")
        .where_(|country| country.id().eq(first_id))
        .update_plan(|mut country| {
            country.code = "DE".into();
            country
        })
        .expect("first update");
    database
        .commit_plan(&first_change, TransactionId::new(7_941))
        .expect("first update commit");
    drop(snapshot);

    let snapshot = database.snapshot().expect("second update snapshot");
    let second_change = snapshot
        .objects::<Country>()
        .expect("countries")
        .where_(|country| country.id().eq(second_id))
        .update_plan(|mut country| {
            country.code = "CA".into();
            country
        })
        .expect("second update");
    database
        .commit_plan(&second_change, TransactionId::new(7_942))
        .expect("second update commit");
    drop(snapshot);

    let history = database.history().expect("history");
    let first_update = history
        .entries()
        .iter()
        .find(|entry| entry.transaction() == TransactionId::new(7_941))
        .expect("first update entry");
    assert!(matches!(
        first_update.undo_readiness(),
        HistoryUndoReadiness::Rebased { .. }
    ));
    let rebased_undo = first_update.undo_plan().expect("rebased object undo");
    let _candidate = rebased_undo.candidate().expect("candidate");
    database
        .commit_plan(&rebased_undo, TransactionId::new(7_943))
        .expect("commit object undo");

    let snapshot = database.snapshot().expect("final snapshot");
    let countries = snapshot.objects::<Country>().expect("countries");
    assert_eq!(countries.require(first_id).expect("first").code, "NL");
    assert_eq!(countries.require(second_id).expect("second").code, "CA");

    drop(countries);
    drop(snapshot);
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn watch_emits_exact_revision_tagged_object_delta_without_recompute() {
    use cfmd_runtime::Id;

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .build()
        .expect("country schema");
    let database = Database::create(&directory, schema).expect("create database");
    let id = Id::<Country>::new(301);

    let snapshot = database.snapshot().expect("seed snapshot");
    let seed = snapshot
        .objects::<Country>()
        .expect("countries")
        .insert(Country {
            id,
            code: "NL".into(),
        })
        .expect("seed plan");
    database
        .commit_plan(&seed, TransactionId::new(8_010))
        .expect("seed commit");
    drop(snapshot);

    let snapshot = database.snapshot().expect("watch snapshot");
    let query = snapshot
        .objects::<Country>()
        .expect("countries")
        .where_(|country| country.id().eq(id));
    let mut watch = query.watch().expect("exact object watch");
    assert_eq!(watch.initial().len(), 1);
    assert_eq!(watch.initial()[0].code, "NL");
    let source_revision = snapshot.revision();

    let update_snapshot = database.snapshot().expect("update snapshot");
    let update = update_snapshot
        .objects::<Country>()
        .expect("countries")
        .where_(|country| country.id().eq(id))
        .update_plan(|mut country| {
            country.code = "DE".into();
            country
        })
        .expect("update plan");
    database
        .commit_plan(&update, TransactionId::new(8_011))
        .expect("update commit");
    let target_revision = database.current_revision().expect("target revision");

    let event = watch
        .try_recv()
        .expect("watch receive")
        .expect("watch event");
    assert_eq!(event.source_revision(), source_revision);
    assert_eq!(event.target_revision(), target_revision);
    assert_eq!(event.removed().len(), 1);
    assert_eq!(event.removed()[0].code, "NL");
    assert_eq!(event.inserted().len(), 1);
    assert_eq!(event.inserted()[0].code, "DE");
    assert!(watch.try_recv().expect("no duplicate event").is_none());

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn projection_watch_filters_irrelevant_changes_and_decodes_exact_delta() {
    use cfmd_runtime::Id;

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .build()
        .expect("country schema");
    let database = Database::create(&directory, schema).expect("create database");
    let watched = Id::<Country>::new(311);
    let other = Id::<Country>::new(312);

    let snapshot = database.snapshot().expect("seed snapshot");
    let countries = snapshot.objects::<Country>().expect("countries");
    let seed = countries
        .insert(Country {
            id: watched,
            code: "NL".into(),
        })
        .expect("watched seed")
        .and(
            countries
                .insert(Country {
                    id: other,
                    code: "US".into(),
                })
                .expect("other seed"),
        )
        .expect("compose seed");
    database
        .commit_plan(&seed, TransactionId::new(8_020))
        .expect("seed commit");
    drop(countries);
    drop(snapshot);

    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot
        .objects::<Country>()
        .expect("countries")
        .where_(|country| country.id().eq(watched))
        .select(CountryFields::code)
        .watch()
        .expect("projection watch");
    assert_eq!(watch.initial(), &[String::from("NL")]);

    let other_snapshot = database.snapshot().expect("other update snapshot");
    let other_update = other_snapshot
        .objects::<Country>()
        .expect("countries")
        .where_(|country| country.id().eq(other))
        .update_plan(|mut country| {
            country.code = "CA".into();
            country
        })
        .expect("other update");
    database
        .commit_plan(&other_update, TransactionId::new(8_021))
        .expect("other commit");
    assert!(
        watch
            .try_recv()
            .expect("irrelevant transition quotient")
            .is_none(),
        "an output-equivalent transition must not fabricate an empty public event"
    );

    let watched_snapshot = database.snapshot().expect("watched update snapshot");
    let watched_update = watched_snapshot
        .objects::<Country>()
        .expect("countries")
        .where_(|country| country.id().eq(watched))
        .update_plan(|mut country| {
            country.code = "DE".into();
            country
        })
        .expect("watched update");
    database
        .commit_plan(&watched_update, TransactionId::new(8_022))
        .expect("watched commit");
    let event = watch
        .try_recv()
        .expect("projection event")
        .expect("revision event");
    assert_eq!(event.removed(), &[String::from("NL")]);
    assert_eq!(event.inserted(), &[String::from("DE")]);

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn watch_recv_blocks_on_runtime_publication_signal_and_wakes_on_commit() {
    use std::sync::{Arc, Barrier};

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(8_100);
    let equivalence = EquivalenceId::new(8_101);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("watch schema");
    let database = Database::create(&directory, schema).expect("create database");
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot
        .watch(&Query::scan(relation))
        .expect("raw query watch");

    let writer = database.clone();
    let barrier = Arc::new(Barrier::new(2));
    let writer_barrier = Arc::clone(&barrier);
    let handle = std::thread::spawn(move || {
        writer_barrier.wait();
        let mut plan = writer.plan().expect("writer plan");
        plan.insert(relation, vec![Value::I64(42)]);
        writer
            .commit_plan(&plan, TransactionId::new(8_110))
            .expect("writer commit");
    });
    barrier.wait();
    let event = watch.recv().expect("blocking watch event");
    handle.join().expect("writer thread");

    assert_eq!(event.inserted(), &[vec![Value::I64(42)]]);
    assert!(event.removed().is_empty());
    assert_eq!(watch.revision(), event.target_revision());

    drop(snapshot);
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn custom_publication_notifier_is_wake_only_and_cannot_fabricate_watch_state() {
    use std::sync::Arc;

    use cfmd_runtime::{InProcessPublicationNotifier, PublicationNotifier};

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(8_200);
    let equivalence = EquivalenceId::new(8_201);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("notification schema");
    let notifier = Arc::new(InProcessPublicationNotifier::default());
    let database = Database::create_with_publication_notifier(
        &directory,
        schema,
        Arc::clone(&notifier) as Arc<dyn PublicationNotifier>,
    )
    .expect("create database with custom notifier");

    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot
        .watch(&Query::scan(relation))
        .expect("raw query watch");

    notifier.notify_revision_published();
    assert!(
        watch
            .try_recv()
            .expect("spurious notification is harmless")
            .is_none()
    );

    let mut plan = database.plan().expect("writer plan");
    plan.insert(relation, vec![Value::I64(77)]);
    database
        .commit_plan(&plan, TransactionId::new(8_210))
        .expect("writer commit");

    let event = watch.recv().expect("watch event from custom notifier");
    assert_eq!(event.inserted(), &[vec![Value::I64(77)]]);
    assert!(event.removed().is_empty());

    drop(snapshot);
    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn watch_cancellation_interrupts_blocking_recv_without_polling() {
    use std::sync::{Arc, Barrier};

    use cfmd_runtime::{ErrorKind, InProcessPublicationNotifier, PublicationNotifier};

    #[derive(Debug)]
    struct BarrierNotifier {
        inner: InProcessPublicationNotifier,
        entered_wait: Arc<Barrier>,
    }

    impl PublicationNotifier for BarrierNotifier {
        fn generation(&self) -> u64 {
            self.inner.generation()
        }

        fn wait_after(&self, observed: u64) -> u64 {
            self.entered_wait.wait();
            self.inner.wait_after(observed)
        }

        fn register_waker_after(
            &self,
            waiter_id: u64,
            observed: u64,
            waker: &std::task::Waker,
        ) -> u64 {
            self.inner.register_waker_after(waiter_id, observed, waker)
        }

        fn unregister_waker(&self, waiter_id: u64) {
            self.inner.unregister_waker(waiter_id);
        }

        fn notify_waiters(&self) {
            self.inner.notify_waiters();
        }
    }

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(8_300);
    let equivalence = EquivalenceId::new(8_301);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("watch schema");
    let entered_wait = Arc::new(Barrier::new(2));
    let notifier = Arc::new(BarrierNotifier {
        inner: InProcessPublicationNotifier::default(),
        entered_wait: Arc::clone(&entered_wait),
    });
    let database = Database::create_with_publication_notifier(
        &directory,
        schema,
        Arc::clone(&notifier) as Arc<dyn PublicationNotifier>,
    )
    .expect("create database");
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot
        .watch(&Query::scan(relation))
        .expect("raw query watch");
    let cancellation = watch.cancellation();
    drop(snapshot);

    let handle = std::thread::spawn(move || watch.recv());
    entered_wait.wait();
    cancellation.cancel();
    let error = handle
        .join()
        .expect("watch thread")
        .expect_err("cancelled recv must stop");
    assert_eq!(error.kind(), ErrorKind::WatchClosed);
    assert!(cancellation.is_cancelled());

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn watch_recv_wakes_when_last_runtime_owner_closes() {
    use std::sync::{Arc, Barrier};

    use cfmd_runtime::{ErrorKind, InProcessPublicationNotifier, PublicationNotifier};

    #[derive(Debug)]
    struct BarrierNotifier {
        inner: InProcessPublicationNotifier,
        entered_wait: Arc<Barrier>,
    }

    impl PublicationNotifier for BarrierNotifier {
        fn generation(&self) -> u64 {
            self.inner.generation()
        }

        fn wait_after(&self, observed: u64) -> u64 {
            self.entered_wait.wait();
            self.inner.wait_after(observed)
        }

        fn register_waker_after(
            &self,
            waiter_id: u64,
            observed: u64,
            waker: &std::task::Waker,
        ) -> u64 {
            self.inner.register_waker_after(waiter_id, observed, waker)
        }

        fn unregister_waker(&self, waiter_id: u64) {
            self.inner.unregister_waker(waiter_id);
        }

        fn notify_waiters(&self) {
            self.inner.notify_waiters();
        }
    }

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(8_320);
    let equivalence = EquivalenceId::new(8_321);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("watch schema");
    let entered_wait = Arc::new(Barrier::new(2));
    let notifier = Arc::new(BarrierNotifier {
        inner: InProcessPublicationNotifier::default(),
        entered_wait: Arc::clone(&entered_wait),
    });
    let database = Database::create_with_publication_notifier(
        &directory,
        schema,
        Arc::clone(&notifier) as Arc<dyn PublicationNotifier>,
    )
    .expect("create database");
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot
        .watch(&Query::scan(relation))
        .expect("raw query watch");
    drop(snapshot);

    let handle = std::thread::spawn(move || watch.recv());
    entered_wait.wait();
    drop(database);
    let error = handle
        .join()
        .expect("watch thread")
        .expect_err("closed runtime must stop recv");
    assert_eq!(error.kind(), ErrorKind::WatchClosed);

    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn watch_lag_is_durable_and_catches_up_one_transition_at_a_time() {
    use cfmd_runtime::WatchStatus;

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(8_340);
    let equivalence = EquivalenceId::new(8_341);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("watch schema");
    let database = Database::create(&directory, schema).expect("create database");
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot
        .watch(&Query::scan(relation))
        .expect("raw query watch");
    let sibling_watch = snapshot
        .watch(&Query::scan(relation))
        .expect("sibling raw query watch");
    assert_ne!(watch.subscription_id(), sibling_watch.subscription_id());
    assert_eq!(
        watch.readiness().source_id(),
        sibling_watch.readiness().source_id()
    );
    let anchor_revision = watch.revision();
    drop(sibling_watch);
    drop(snapshot);

    for (offset, value) in [11_i64, 22, 33].into_iter().enumerate() {
        let mut plan = database.plan().expect("writer plan");
        plan.insert(relation, vec![Value::I64(value)]);
        database
            .commit_plan(&plan, TransactionId::new(8_350 + offset as u128))
            .expect("writer commit");
    }
    let head_revision = database.current_revision().expect("head revision");

    assert_eq!(
        watch.status().expect("lag status"),
        WatchStatus::Lagging {
            anchor_revision,
            head_revision,
            pending_transitions: 3,
        }
    );

    let first_drain = watch.drain_ready(2).expect("bounded catch-up drain");
    assert_eq!(first_drain.events().len(), 2);
    assert_eq!(first_drain.events()[0].inserted(), &[vec![Value::I64(11)]]);
    assert_eq!(first_drain.events()[1].inserted(), &[vec![Value::I64(22)]]);
    assert!(
        first_drain
            .events()
            .iter()
            .all(|event| event.removed().is_empty())
    );
    assert!(first_drain.has_more());
    assert!(matches!(
        first_drain.status(),
        WatchStatus::Lagging {
            pending_transitions: 1,
            ..
        }
    ));

    let final_drain = watch.drain_ready(8).expect("final catch-up drain");
    assert_eq!(final_drain.events().len(), 1);
    assert_eq!(final_drain.events()[0].inserted(), &[vec![Value::I64(33)]]);
    assert!(!final_drain.has_more());
    assert_eq!(
        final_drain.status(),
        WatchStatus::Current {
            revision: head_revision,
        }
    );
    assert!(watch.try_recv().expect("no buffered duplicate").is_none());

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
#[allow(clippy::too_many_lines)]
fn hosted_session_permissions_follow_product_values_and_fail_closed() {
    use cfmd_runtime::{
        ErrorKind, Id, Object, Permission, PermissionSet, PrincipalId, RowCodec, Session,
    };

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .build()
        .expect("country schema");
    let database = Database::create(&directory, schema).expect("create database");

    let seed_snapshot = database.snapshot().expect("seed snapshot");
    let countries = seed_snapshot.objects::<Country>().expect("countries");
    let seed = countries
        .insert(Country {
            id: Id::new(1),
            code: "NL".to_owned(),
        })
        .expect("seed plan");
    database
        .commit_plan(&seed, TransactionId::new(9_001))
        .expect("seed commit");
    drop(countries);
    drop(seed_snapshot);

    let read_only = database.session(Session::new(
        PrincipalId::new(101),
        PermissionSet::from([Permission::Read]),
    ));
    let read_view = read_only.snapshot().expect("read snapshot");
    assert_eq!(
        read_view
            .objects::<Country>()
            .expect("countries")
            .require(Id::new(1))
            .expect("country")
            .code,
        "NL"
    );
    assert_eq!(
        read_only
            .plan()
            .expect_err("read-only cannot create plans")
            .kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(
        read_view
            .history()
            .expect_err("history requires explicit permission")
            .kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(
        read_view
            .objects::<Country>()
            .expect("countries")
            .where_(|country| country.id().eq(Id::new(1)))
            .watch()
            .expect_err("watch requires explicit permission")
            .kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(
        read_only
            .at(cfmd_runtime::RevisionId::new(1))
            .expect_err("historical reads require explicit permission")
            .kind(),
        ErrorKind::PermissionDenied
    );

    let history_reader = database.session(Session::new(
        PrincipalId::new(102),
        PermissionSet::from([Permission::Read, Permission::HistoryRead]),
    ));
    let history = history_reader.history().expect("authorized history");
    assert_eq!(
        history
            .undo_latest()
            .expect_err("history read does not imply write authority")
            .kind(),
        ErrorKind::PermissionDenied
    );

    let historical_reader = database.session(Session::new(
        PrincipalId::new(103),
        PermissionSet::from([Permission::Read, Permission::HistoricalRead]),
    ));
    assert_eq!(
        historical_reader
            .at(cfmd_runtime::RevisionId::new(1))
            .expect("authorized historical view")
            .revision(),
        cfmd_runtime::RevisionId::new(1)
    );

    let watcher = database.session(Session::new(
        PrincipalId::new(104),
        PermissionSet::from([Permission::Read, Permission::Watch]),
    ));
    let watcher_view = watcher.snapshot().expect("watch view");
    let watch = watcher_view
        .objects::<Country>()
        .expect("countries")
        .where_(|country| country.id().eq(Id::new(1)))
        .watch()
        .expect("authorized watch");
    watch.close();

    let writer = database.session(Session::new(
        PrincipalId::new(105),
        PermissionSet::from([Permission::Write]),
    ));
    let mut plan = writer.plan().expect("authorized writer plan");
    plan.insert(
        Country::relation_id(),
        Country {
            id: Id::new(2),
            code: "DE".to_owned(),
        }
        .into_row(),
    );
    let candidate = plan.candidate().expect("candidate");
    assert_eq!(
        candidate
            .objects::<Country>()
            .expect_err("write permission does not grant candidate reads")
            .kind(),
        ErrorKind::PermissionDenied
    );
    writer
        .commit_plan(&plan, TransactionId::new(9_002))
        .expect("plan keeps write authority");

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn session_authority_refresh_and_revoke_reach_existing_product_values() {
    use cfmd_runtime::{
        ErrorKind, Id, Object, Permission, PermissionSet, PrincipalId, RowCodec, Session,
    };

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .build()
        .expect("country schema");
    let database = Database::create(&directory, schema).expect("create database");
    let session = Session::new(
        PrincipalId::new(201),
        PermissionSet::from([Permission::Write]),
    );
    let writer = database.session(session.clone());

    let mut plan = writer.plan().expect("initial write plan");
    plan.insert(
        Country::relation_id(),
        Country {
            id: Id::new(1),
            code: "NL".to_owned(),
        }
        .into_row(),
    );
    let _candidate = plan.candidate().expect("candidate before grant refresh");

    session
        .refresh_permissions(PermissionSet::from([Permission::Read]))
        .expect("downgrade grants");
    assert_eq!(
        writer
            .commit_plan(&plan, TransactionId::new(9_101))
            .expect_err("existing plan must observe downgraded authority")
            .kind(),
        ErrorKind::PermissionDenied
    );

    session
        .refresh_permissions(PermissionSet::from([Permission::Write]))
        .expect("restore write grant");
    writer
        .commit_plan(&plan, TransactionId::new(9_101))
        .expect("existing plan observes refreshed write authority");

    let mut second = writer.plan().expect("second write plan");
    second.insert(
        Country::relation_id(),
        Country {
            id: Id::new(2),
            code: "DE".to_owned(),
        }
        .into_row(),
    );
    let _second_candidate = second.candidate().expect("candidate before revoke");
    assert!(session.revoke().expect("revoke session"));
    assert_eq!(
        writer
            .commit_plan(&second, TransactionId::new(9_102))
            .expect_err("revoked authority invalidates existing plan")
            .kind(),
        ErrorKind::SessionRevoked
    );
    assert_eq!(
        writer
            .plan()
            .expect_err("revoked session cannot derive fresh plan")
            .kind(),
        ErrorKind::SessionRevoked
    );

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn database_builder_unifies_single_file_default_and_explicit_directory_storage() {
    use cfmd_runtime::Storage;

    let relation = RelationId::new(9_100);
    let equivalence = EquivalenceId::new(9_101);
    let schema = || {
        Schema::builder()
            .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
            .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
            .build()
            .expect("schema")
    };

    let file_path = temp_directory().with_extension("cfmd");
    let database = Database::builder(&file_path)
        .schema(schema())
        .create()
        .expect("default single-file create");
    assert!(file_path.is_file());
    let mut plan = database.plan().expect("plan");
    plan.insert(relation, vec![Value::I64(41)]);
    database
        .commit_plan(&plan, TransactionId::new(9_102))
        .expect("commit");
    drop(database);

    let reopened = Database::builder(&file_path)
        .open()
        .expect("auto reopen file");
    assert_eq!(reopened.current_revision().expect("revision").raw(), 2);
    drop(reopened);
    fs::remove_file(&file_path).expect("remove single-file fixture");

    let directory = temp_directory();
    let database = Database::builder(&directory)
        .storage(Storage::Directory)
        .schema(schema())
        .create()
        .expect("explicit directory create");
    assert!(directory.is_dir());
    drop(database);
    let reopened = Database::builder(&directory)
        .open()
        .expect("auto reopen directory");
    assert_eq!(reopened.current_revision().expect("revision").raw(), 1);
    drop(reopened);
    fs::remove_dir_all(directory).expect("remove directory fixture");
}

#[test]
fn encrypted_single_file_builder_encrypts_wal_and_sections_and_requires_exact_key() {
    use cfmd_runtime::{Encryption, EncryptionKey};

    let relation = RelationId::new(9_200);
    let equivalence = EquivalenceId::new(9_201);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("schema");
    let key_bytes = [0x31_u8; 32];
    let encryption = || Encryption::aes256_gcm_siv(EncryptionKey::from_bytes(key_bytes).unwrap());
    let file_path = temp_directory().with_extension("cfmd");

    let database = Database::builder(&file_path)
        .schema(schema)
        .encryption(encryption())
        .create()
        .expect("encrypted single-file create");
    let mut plan = database.plan().expect("plan");
    plan.insert(relation, vec![Value::I64(0x1122_3344_5566_7788)]);
    database
        .commit_plan(&plan, TransactionId::new(9_202))
        .expect("encrypted commit");
    drop(database);

    let bytes = fs::read(&file_path).expect("read encrypted fixture");
    assert!(bytes.windows(4).any(|window| window == b"CFAE"));

    assert!(Database::builder(&file_path).open().is_err());
    assert!(
        Database::builder(&file_path)
            .encryption(Encryption::aes256_gcm_siv(
                EncryptionKey::from_bytes([0x32_u8; 32]).unwrap()
            ))
            .open()
            .is_err()
    );

    let reopened = Database::builder(&file_path)
        .encryption(encryption())
        .open()
        .expect("reopen encrypted database with exact key");
    assert_eq!(reopened.current_revision().expect("revision").raw(), 2);
    let snapshot = reopened.snapshot().expect("snapshot");
    let query = Query::scan(relation);
    let prepared = snapshot.prepare(&query).expect("prepare encrypted scan");
    assert_eq!(
        prepared
            .execute(&snapshot)
            .expect("scan encrypted relation"),
        RelationResult::Bag(vec![vec![Value::I64(0x1122_3344_5566_7788)]])
    );
    drop(snapshot);
    drop(reopened);
    fs::remove_file(&file_path).expect("remove encrypted fixture");
}

#[test]
fn encryption_key_provider_is_resolved_for_create_and_open() {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use cfmd_runtime::{
        Encryption, EncryptionKeyId, EncryptionKeyOperation, EncryptionKeyProvider,
        EncryptionProviderKeyMetadata,
    };

    #[derive(Debug)]
    struct Provider {
        key: [u8; 32],
        operations: Arc<Mutex<Vec<EncryptionKeyOperation>>>,
    }

    impl EncryptionKeyProvider for Provider {
        fn provide_key(
            &self,
            _path: &Path,
            operation: EncryptionKeyOperation,
            destination: &mut cfmd_runtime::EncryptionKeyDestination<'_>,
        ) -> cfmd_runtime::Result<EncryptionProviderKeyMetadata> {
            self.operations
                .lock()
                .expect("provider lock")
                .push(operation);
            destination
                .fill_with(|bytes| {
                    bytes.copy_from_slice(&self.key);
                    Ok::<(), std::convert::Infallible>(())
                })
                .expect("infallible direct fill");
            Ok(EncryptionProviderKeyMetadata::new(
                EncryptionKeyId::from_bytes([0xA1; 16]),
                1,
            ))
        }
    }

    let operations = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn EncryptionKeyProvider> = Arc::new(Provider {
        key: [0x63; 32],
        operations: Arc::clone(&operations),
    });
    let relation = RelationId::new(9_300);
    let equivalence = EquivalenceId::new(9_301);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("schema");
    let path = temp_directory().with_extension("cfmd");
    let database = Database::builder(&path)
        .schema(schema)
        .encryption(Encryption::aes256_gcm_siv_with_provider(Arc::clone(
            &provider,
        )))
        .create()
        .expect("provider-backed create");
    drop(database);
    let reopened = Database::builder(&path)
        .encryption(Encryption::aes256_gcm_siv_with_provider(provider))
        .open()
        .expect("provider-backed open");
    drop(reopened);
    assert_eq!(
        *operations.lock().expect("provider lock"),
        vec![EncryptionKeyOperation::Create, EncryptionKeyOperation::Open]
    );
    fs::remove_file(path).expect("remove provider fixture");
}

#[test]
fn encryption_key_provider_must_initialize_secure_destination() {
    use std::path::Path;
    use std::sync::Arc;

    use cfmd_runtime::{
        Encryption, EncryptionKeyId, EncryptionKeyOperation, EncryptionKeyProvider,
        EncryptionProviderKeyMetadata, ErrorKind,
    };

    #[derive(Debug)]
    struct EmptyProvider;

    impl EncryptionKeyProvider for EmptyProvider {
        fn provide_key(
            &self,
            _path: &Path,
            _operation: EncryptionKeyOperation,
            _destination: &mut cfmd_runtime::EncryptionKeyDestination<'_>,
        ) -> cfmd_runtime::Result<EncryptionProviderKeyMetadata> {
            Ok(EncryptionProviderKeyMetadata::new(
                EncryptionKeyId::from_bytes([0xA2; 16]),
                1,
            ))
        }
    }

    let relation = RelationId::new(9_325);
    let equivalence = EquivalenceId::new(9_326);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("schema");
    let path = temp_directory().with_extension("cfmd");
    let error = Database::builder(&path)
        .schema(schema)
        .encryption(Encryption::aes256_gcm_siv_with_provider(Arc::new(
            EmptyProvider,
        )))
        .create()
        .expect_err("provider success without key initialization must fail closed");
    assert_eq!(error.kind(), ErrorKind::Recovery);
    assert!(error.message().contains("without initializing"));
}

#[test]
fn provider_database_key_epoch_floor_rejects_complete_header_rollback() {
    use std::io::{Seek, SeekFrom, Write};
    use std::path::Path;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    use cfmd_runtime::{
        Encryption, EncryptionKeyAcknowledgement, EncryptionKeyId, EncryptionKeyOperation,
        EncryptionKeyProvider, EncryptionProviderKeyMetadata,
    };

    #[derive(Debug)]
    struct Provider {
        floor: Arc<AtomicU64>,
        provider_epoch: u64,
    }

    impl EncryptionKeyProvider for Provider {
        fn provide_key(
            &self,
            _path: &Path,
            _operation: EncryptionKeyOperation,
            destination: &mut cfmd_runtime::EncryptionKeyDestination<'_>,
        ) -> cfmd_runtime::Result<EncryptionProviderKeyMetadata> {
            destination.write(&[0x73; 32]);
            Ok(EncryptionProviderKeyMetadata::new(
                EncryptionKeyId::from_bytes([0x83; 16]),
                self.provider_epoch,
            )
            .with_minimum_database_key_epoch(self.floor.load(Ordering::SeqCst)))
        }

        fn acknowledge_database_key_epoch(
            &self,
            _path: &Path,
            acknowledgement: EncryptionKeyAcknowledgement,
        ) -> cfmd_runtime::Result<()> {
            self.floor
                .store(acknowledgement.database_key_epoch(), Ordering::SeqCst);
            Ok(())
        }
    }

    let floor = Arc::new(AtomicU64::new(1));
    let provider: Arc<dyn EncryptionKeyProvider> = Arc::new(Provider {
        floor: Arc::clone(&floor),
        provider_epoch: 4,
    });
    let next_provider: Arc<dyn EncryptionKeyProvider> = Arc::new(Provider {
        floor: Arc::clone(&floor),
        provider_epoch: 5,
    });
    let relation = RelationId::new(9_350);
    let equivalence = EquivalenceId::new(9_351);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("schema");
    let path = temp_directory().with_extension("cfmd");
    let database = Database::builder(&path)
        .schema(schema)
        .encryption(Encryption::aes256_gcm_siv_with_provider(Arc::clone(
            &provider,
        )))
        .create()
        .expect("create wrapped database");

    let before = fs::read(&path).expect("read original database");
    let old_header = before[..4096].to_vec();
    assert_eq!(
        database
            .rewrap_encryption(&Encryption::aes256_gcm_siv_with_provider(Arc::clone(
                &next_provider,
            )))
            .expect("rewrap provider authority"),
        2
    );
    assert_eq!(floor.load(Ordering::SeqCst), 2);
    drop(database);

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open fixture for rollback");
    file.seek(SeekFrom::Start(0)).expect("seek header");
    file.write_all(&old_header).expect("restore old header");
    file.sync_all().expect("sync rolled-back header");
    drop(file);

    assert!(
        Database::builder(&path)
            .encryption(Encryption::aes256_gcm_siv_with_provider(provider))
            .open()
            .is_err()
    );
    fs::remove_file(path).expect("remove key-floor fixture");
}

#[test]
#[allow(clippy::too_many_lines)]
fn provider_acknowledgement_failure_recovers_and_retries_pending_handoff() {
    use std::path::Path;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    use cfmd_runtime::{
        Encryption, EncryptionKeyAcknowledgement, EncryptionKeyId, EncryptionKeyOperation,
        EncryptionKeyProvider, EncryptionProviderKeyMetadata,
    };

    #[derive(Debug)]
    struct Provider {
        id: [u8; 16],
        epoch: u64,
        key: [u8; 32],
        floor: Arc<AtomicU64>,
        acknowledge: bool,
    }

    impl EncryptionKeyProvider for Provider {
        fn provide_key(
            &self,
            _path: &Path,
            _operation: EncryptionKeyOperation,
            destination: &mut cfmd_runtime::EncryptionKeyDestination<'_>,
        ) -> cfmd_runtime::Result<EncryptionProviderKeyMetadata> {
            destination.write(&self.key);
            Ok(
                EncryptionProviderKeyMetadata::new(
                    EncryptionKeyId::from_bytes(self.id),
                    self.epoch,
                )
                .with_minimum_database_key_epoch(self.floor.load(Ordering::SeqCst)),
            )
        }

        fn acknowledge_database_key_epoch(
            &self,
            path: &Path,
            acknowledgement: EncryptionKeyAcknowledgement,
        ) -> cfmd_runtime::Result<()> {
            if !self.acknowledge {
                return Database::open(path.with_extension("acknowledgement-unavailable"))
                    .map(|_| ());
            }
            self.floor
                .store(acknowledgement.database_key_epoch(), Ordering::SeqCst);
            Ok(())
        }
    }

    let floor = Arc::new(AtomicU64::new(1));
    let old: Arc<dyn EncryptionKeyProvider> = Arc::new(Provider {
        id: [0x91; 16],
        epoch: 1,
        key: [0x81; 32],
        floor: Arc::clone(&floor),
        acknowledge: true,
    });
    let pending: Arc<dyn EncryptionKeyProvider> = Arc::new(Provider {
        id: [0x92; 16],
        epoch: 2,
        key: [0x82; 32],
        floor: Arc::clone(&floor),
        acknowledge: false,
    });
    let next: Arc<dyn EncryptionKeyProvider> = Arc::new(Provider {
        id: [0x92; 16],
        epoch: 2,
        key: [0x82; 32],
        floor: Arc::clone(&floor),
        acknowledge: true,
    });

    let relation = RelationId::new(9_375);
    let equivalence = EquivalenceId::new(9_376);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("schema");
    let path = temp_directory().with_extension("cfmd");
    let database = Database::builder(&path)
        .schema(schema)
        .encryption(Encryption::aes256_gcm_siv_with_provider(Arc::clone(&old)))
        .create()
        .expect("create old provider database");

    assert!(
        database
            .rewrap_encryption(&Encryption::aes256_gcm_siv_with_provider(pending))
            .is_err(),
        "external acknowledgement failure must be surfaced after local pending publication"
    );
    assert_eq!(floor.load(Ordering::SeqCst), 1);
    drop(database);

    let recovered = Database::builder(&path)
        .encryption(Encryption::aes256_gcm_siv_with_provider(Arc::clone(&old)))
        .open()
        .expect("old acknowledged authority remains recoverable while handoff is pending");
    assert_eq!(
        recovered
            .rewrap_encryption(&Encryption::aes256_gcm_siv_with_provider(Arc::clone(&next)))
            .expect("retry pending handoff"),
        2
    );
    assert_eq!(floor.load(Ordering::SeqCst), 2);
    drop(recovered);

    assert!(
        Database::builder(&path)
            .encryption(Encryption::aes256_gcm_siv_with_provider(old))
            .open()
            .is_err()
    );
    let reopened = Database::builder(&path)
        .encryption(Encryption::aes256_gcm_siv_with_provider(next))
        .open()
        .expect("acknowledged successor authority opens after predecessor retirement");
    drop(reopened);
    fs::remove_file(path).expect("remove pending handoff fixture");
}

#[test]
#[allow(clippy::too_many_lines)]
fn provider_rotation_rewraps_dmk_without_rewriting_database_ciphertext() {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use cfmd_runtime::{
        Encryption, EncryptionKeyAcknowledgement, EncryptionKeyId, EncryptionKeyOperation,
        EncryptionKeyProvider, EncryptionProviderKeyMetadata,
    };

    #[derive(Debug)]
    struct Provider {
        id: [u8; 16],
        epoch: u64,
        key: [u8; 32],
        operations: Arc<Mutex<Vec<EncryptionKeyOperation>>>,
        acknowledgements: Arc<Mutex<Vec<EncryptionKeyAcknowledgement>>>,
    }

    impl EncryptionKeyProvider for Provider {
        fn provide_key(
            &self,
            _path: &Path,
            operation: EncryptionKeyOperation,
            destination: &mut cfmd_runtime::EncryptionKeyDestination<'_>,
        ) -> cfmd_runtime::Result<EncryptionProviderKeyMetadata> {
            self.operations
                .lock()
                .expect("provider lock")
                .push(operation);
            destination.write(&self.key);
            Ok(EncryptionProviderKeyMetadata::new(
                EncryptionKeyId::from_bytes(self.id),
                self.epoch,
            ))
        }

        fn acknowledge_database_key_epoch(
            &self,
            _path: &Path,
            acknowledgement: EncryptionKeyAcknowledgement,
        ) -> cfmd_runtime::Result<()> {
            self.acknowledgements
                .lock()
                .expect("provider acknowledgement lock")
                .push(acknowledgement);
            Ok(())
        }
    }

    let old_ops = Arc::new(Mutex::new(Vec::new()));
    let new_ops = Arc::new(Mutex::new(Vec::new()));
    let old_acks = Arc::new(Mutex::new(Vec::new()));
    let new_acks = Arc::new(Mutex::new(Vec::new()));
    let old_provider: Arc<dyn EncryptionKeyProvider> = Arc::new(Provider {
        id: [0x11; 16],
        epoch: 1,
        key: [0x41; 32],
        operations: Arc::clone(&old_ops),
        acknowledgements: Arc::clone(&old_acks),
    });
    let new_provider: Arc<dyn EncryptionKeyProvider> = Arc::new(Provider {
        id: [0x22; 16],
        epoch: 2,
        key: [0x42; 32],
        operations: Arc::clone(&new_ops),
        acknowledgements: Arc::clone(&new_acks),
    });

    let relation = RelationId::new(9_400);
    let equivalence = EquivalenceId::new(9_401);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("schema");
    let path = temp_directory().with_extension("cfmd");
    let database = Database::builder(&path)
        .schema(schema)
        .encryption(Encryption::aes256_gcm_siv_with_provider(Arc::clone(
            &old_provider,
        )))
        .create()
        .expect("wrapped-DMK create");
    let mut plan = database.plan().expect("plan");
    plan.insert(relation, vec![Value::I64(314)]);
    database
        .commit_plan(&plan, TransactionId::new(9_402))
        .expect("encrypted commit");

    let before = fs::read(&path).expect("read before rewrap");
    let data_offset = 4096 * 3;
    let data_before = before[data_offset..].to_vec();
    let key_epoch = database
        .rewrap_encryption(&Encryption::aes256_gcm_siv_with_provider(Arc::clone(
            &new_provider,
        )))
        .expect("rewrap DMK");
    assert_eq!(key_epoch, 2);
    let acknowledgements = new_acks.lock().expect("new provider acknowledgements");
    assert_eq!(acknowledgements.len(), 1);
    assert_eq!(
        acknowledgements[0].provider_key_id(),
        EncryptionKeyId::from_bytes([0x22; 16])
    );
    assert_eq!(acknowledgements[0].provider_key_epoch(), 2);
    assert_eq!(acknowledgements[0].database_key_epoch(), 2);
    drop(acknowledgements);
    let after = fs::read(&path).expect("read after rewrap");
    assert_eq!(&after[data_offset..], data_before.as_slice());
    drop(database);

    assert!(
        Database::builder(&path)
            .encryption(Encryption::aes256_gcm_siv_with_provider(old_provider))
            .open()
            .is_err()
    );
    let reopened = Database::builder(&path)
        .encryption(Encryption::aes256_gcm_siv_with_provider(new_provider))
        .open()
        .expect("open with rotated provider key");
    let snapshot = reopened.snapshot().expect("snapshot");
    let prepared = snapshot
        .prepare(&Query::scan(relation))
        .expect("prepare scan after rewrap");
    assert_eq!(
        prepared.execute(&snapshot).expect("scan after rewrap"),
        RelationResult::Bag(vec![vec![Value::I64(314)]])
    );
    assert_eq!(
        *old_ops.lock().expect("old provider operations"),
        vec![EncryptionKeyOperation::Create, EncryptionKeyOperation::Open]
    );
    assert_eq!(
        *new_ops.lock().expect("new provider operations"),
        vec![EncryptionKeyOperation::Rewrap, EncryptionKeyOperation::Open]
    );
    drop(snapshot);
    drop(reopened);
    fs::remove_file(path).expect("remove rewrap fixture");
}

#[test]
fn granular_authorization_uses_semantic_query_and_field_coordinates() {
    use cfmd_runtime::{
        ErrorKind, Id, Object, Permission, PermissionSet, PrincipalId, Query,
        RowCodec, Session,
    };

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .build()
        .expect("country schema");
    let database = Database::create(&directory, schema).expect("create database");

    let seed_view = database.snapshot().expect("seed snapshot");
    let seed = seed_view
        .objects::<Country>()
        .expect("countries")
        .insert(Country {
            id: Id::new(1),
            code: "NL".to_owned(),
        })
        .expect("seed plan");
    database
        .commit_plan(&seed, TransactionId::new(9_201))
        .expect("seed commit");
    drop(seed_view);

    let schema_view = database
        .snapshot()
        .expect("schema snapshot")
        .schema()
        .expect("unrestricted model read");
    let relation = schema_view
        .relation(Country::relation_id())
        .expect("country relation");
    let code_field = relation.column_ids()[1];

    let projected_reader = database.session(Session::new(
        PrincipalId::new(301),
        PermissionSet::from([Permission::ReadField {
            relation: Country::relation_id(),
            field: code_field,
        }]),
    ));
    let read_view = projected_reader.snapshot().expect("granular read snapshot");
    let projected = read_view
        .execute(&Query::scan(Country::relation_id()).project([1]))
        .expect("authorized projected field");
    assert_eq!(projected.rows().len(), 1);
    assert_eq!(
        read_view
            .execute(&Query::scan(Country::relation_id()))
            .expect_err("full row must require every observed field")
            .kind(),
        ErrorKind::PermissionDenied
    );

    let field_writer = database.session(Session::new(
        PrincipalId::new(302),
        PermissionSet::from([Permission::WriteField {
            relation: Country::relation_id(),
            field: code_field,
        }]),
    ));
    assert_eq!(
        field_writer
            .snapshot()
            .expect_err("write-only field authority must not expose a readable snapshot")
            .kind(),
        ErrorKind::PermissionDenied
    );
    let countries = field_writer.objects::<Country>().expect("write-only countries handle");
    let mut tx = field_writer
        .transaction_with_id(TransactionId::new(9_202))
        .expect("field transaction");
    countries
        .set(&mut tx, Id::new(1), |country| country.code(), "DE".to_owned())
        .expect("authorized semantic field patch");
    field_writer.commit(&tx).expect("field-only commit");

    let mut raw_plan = field_writer.plan().expect("granular writer can form a plan");
    raw_plan.insert(
        Country::relation_id(),
        Country {
            id: Id::new(2),
            code: "FR".to_owned(),
        }
        .into_row(),
    );
    assert_eq!(
        field_writer
            .commit_plan(&raw_plan, TransactionId::new(9_203))
            .expect_err("field grant must not authorize whole-relation mutation")
            .kind(),
        ErrorKind::PermissionDenied
    );

    let creator = database.session(Session::new(
        PrincipalId::new(303),
        PermissionSet::from([Permission::CreateObject(Country::relation_id())]),
    ));
    let creator_countries = creator.objects::<Country>().expect("create-only countries handle");
    let mut create_tx = creator
        .transaction_with_id(TransactionId::new(9_204))
        .expect("create-only transaction");
    creator_countries
        .add(
            &mut create_tx,
            Country {
                id: Id::new(2),
                code: "FR".to_owned(),
            },
        )
        .expect("semantic create planning");
    creator.commit(&create_tx).expect("create-only commit");

    let mut raw_create = creator.plan().expect("create authority can form low-level plan");
    raw_create.insert(
        Country::relation_id(),
        Country {
            id: Id::new(3),
            code: "BE".to_owned(),
        }
        .into_row(),
    );
    assert_eq!(
        creator
            .commit_plan(&raw_create, TransactionId::new(9_205))
            .expect_err("semantic create authority must not authorize raw relation insertion")
            .kind(),
        ErrorKind::PermissionDenied
    );

    let deleter = database.session(Session::new(
        PrincipalId::new(304),
        PermissionSet::from([Permission::DeleteObject(Country::relation_id())]),
    ));
    let delete_countries = deleter.objects::<Country>().expect("delete-only countries handle");
    let mut delete_tx = deleter
        .transaction_with_id(TransactionId::new(9_206))
        .expect("delete-only transaction");
    delete_countries
        .remove(
            &mut delete_tx,
            Country {
                id: Id::new(2),
                code: "FR".to_owned(),
            },
        )
        .expect("semantic delete planning");
    deleter.commit(&delete_tx).expect("delete-only commit");

    let unrestricted = database.snapshot().expect("final snapshot");
    assert_eq!(
        unrestricted
            .objects::<Country>()
            .expect("countries")
            .require(Id::new(1))
            .expect("country")
            .code,
        "DE"
    );

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn history_inverse_preserves_object_action_authority_instead_of_raw_relation_write() {
    use cfmd_runtime::{Id, Object, Permission, PermissionSet, PrincipalId, Session};

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .build()
        .expect("country schema");
    let database = Database::create(&directory, schema).expect("create database");

    let seed_view = database.snapshot().expect("seed snapshot");
    let seed = seed_view
        .objects::<Country>()
        .expect("countries")
        .insert(Country {
            id: Id::new(441_001),
            code: "NL".to_owned(),
        })
        .expect("semantic create plan");
    database
        .commit_plan(&seed, TransactionId::new(9_441_001))
        .expect("semantic create commit");
    drop(seed_view);

    let delete_session = database.session(Session::new(
        PrincipalId::new(441_001),
        PermissionSet::from([
            Permission::Read,
            Permission::HistoryRead,
            Permission::DeleteObject(Country::relation_id()),
        ]),
    ));
    let mut undo_create = delete_session
        .transaction_with_id(TransactionId::new(9_441_002))
        .expect("undo transaction");
    delete_session
        .undo_latest(&mut undo_create)
        .expect("derive create inverse under delete authority");
    delete_session
        .commit(&undo_create)
        .expect("undo create without WriteRelation");

    let create_session = database.session(Session::new(
        PrincipalId::new(441_002),
        PermissionSet::from([
            Permission::Read,
            Permission::HistoryRead,
            Permission::CreateObject(Country::relation_id()),
        ]),
    ));
    let mut undo_delete = create_session
        .transaction_with_id(TransactionId::new(9_441_003))
        .expect("redo transaction");
    create_session
        .undo_latest(&mut undo_delete)
        .expect("derive delete inverse under create authority");
    create_session
        .commit(&undo_delete)
        .expect("undo delete without WriteRelation");

    let country = database
        .snapshot()
        .expect("final snapshot")
        .objects::<Country>()
        .expect("countries")
        .require(Id::new(441_001))
        .expect("country restored");
    assert_eq!(country.code, "NL");

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn roles_flatten_into_exact_permissions_and_model_metadata_is_separate_authority() {
    use cfmd_runtime::{ErrorKind, Object, Permission, PermissionSet, PrincipalId, Role, Session};

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let schema = Schema::builder()
        .object::<Country>()
        .build()
        .expect("country schema");
    let database = Database::create(&directory, schema).expect("create database");
    let full_schema = database
        .snapshot()
        .expect("unrestricted snapshot")
        .schema()
        .expect("unrestricted schema");
    let code_field = full_schema
        .relation(Country::relation_id())
        .expect("country relation")
        .column_ids()[1];

    let reader = Role::new("country-code-reader").grant(Permission::ReadField {
        relation: Country::relation_id(),
        field: code_field,
    });
    let observer = database.session(Session::from_roles(
        PrincipalId::new(442_001),
        [&reader],
    ));
    let snapshot = observer.snapshot().expect("field reader snapshot");
    assert_eq!(snapshot.schema_revision(), 1);
    assert_eq!(
        snapshot.schema().expect_err("model metadata must stay hidden").kind(),
        ErrorKind::PermissionDenied
    );

    let model_reader = Role::new("model-reader").grant(Permission::ModelRead);
    let observer = database.session(Session::from_roles(
        PrincipalId::new(442_002),
        [&reader, &model_reader],
    ));
    let snapshot = observer.snapshot().expect("model reader snapshot");
    assert_eq!(
        snapshot.schema().expect("model metadata authority").revision(),
        snapshot.schema_revision()
    );

    assert_eq!(reader.name(), "country-code-reader");
    assert!(PermissionSet::from_roles([&reader]).contains(Permission::ReadField {
        relation: Country::relation_id(),
        field: code_field,
    }));

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn schema_migration_requires_dedicated_authority_not_generic_write() {
    use cfmd_runtime::{
        ErrorKind, MigrationHistoryPolicy, MigrationModel, Permission, PermissionSet, PrincipalId,
        Role, Session,
    };

    let directory = temp_directory();
    fs::create_dir_all(&directory).expect("create fixture directory");
    let source = Schema::builder().revisions(442, 1).build().expect("source schema");
    let database = Database::create(&directory, source).expect("create database");
    let target = Schema::builder().revisions(443, 1).build().expect("target schema");
    let migration = MigrationModel::new(442_443, target);

    let generic_writer = database.session(Session::new(
        PrincipalId::new(442_003),
        PermissionSet::from([Permission::Write]),
    ));
    assert_eq!(
        generic_writer
            .migrate(
                &migration,
                TransactionId::new(9_442_001),
                MigrationHistoryPolicy::Forget,
            )
            .expect_err("generic data write must not grant schema migration")
            .kind(),
        ErrorKind::PermissionDenied
    );
    assert_eq!(database.snapshot().expect("head").schema_revision(), 442);

    let migrator = Role::new("schema-migrator").grant(Permission::SchemaMigrate);
    let migration_session = database.session(Session::from_roles(
        PrincipalId::new(442_004),
        [&migrator],
    ));
    migration_session
        .migrate(
            &migration,
            TransactionId::new(9_442_002),
            MigrationHistoryPolicy::Forget,
        )
        .expect("dedicated migration authority");
    assert_eq!(database.snapshot().expect("migrated head").schema_revision(), 443);

    drop(database);
    fs::remove_dir_all(directory).expect("remove fixture directory");
}
