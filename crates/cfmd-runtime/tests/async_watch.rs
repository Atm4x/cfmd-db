use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use cfmd_runtime::{
    Database, EquivalenceId, InProcessPublicationNotifier, PrimitiveEquivalence,
    PublicationNotifier, Query, RelationId, RelationSchema, Schema, TransactionId, Type, Value,
};

static NEXT_DIRECTORY_ID: AtomicU64 = AtomicU64::new(1);

fn temp_directory() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "cfmd-runtime-async-{}-{}",
        std::process::id(),
        NEXT_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

fn fixture() -> (std::path::PathBuf, Database, RelationId) {
    let directory = temp_directory();
    std::fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(90_100);
    let equivalence = EquivalenceId::new(90_101);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("async watch schema");
    let database = Database::create(&directory, schema).expect("create database");
    (directory, database, relation)
}

fn insert(database: &Database, relation: RelationId, value: i64, transaction: u128) {
    let mut plan = database.plan().expect("writer plan");
    plan.insert(relation, vec![Value::I64(value)]);
    database
        .commit(&plan, TransactionId::new(transaction))
        .expect("writer commit");
}

#[derive(Debug, Default)]
struct FlagWake(AtomicBool);

impl Wake for FlagWake {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::Release);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.store(true, Ordering::Release);
    }
}

#[derive(Debug, Default)]
struct CountWake(AtomicUsize);

impl Wake for CountWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

#[test]
fn executor_neutral_future_wakes_on_publication_and_emits_exact_event() {
    let (directory, database, relation) = fixture();
    let snapshot = database.snapshot().expect("watch snapshot");
    let watch = snapshot.watch(&Query::scan(relation)).expect("exact watch");
    let mut watch = watch;

    let flag = Arc::new(FlagWake::default());
    let waker = Waker::from(Arc::clone(&flag));
    let mut context = Context::from_waker(&waker);
    let mut next = Box::pin(watch.next());
    assert!(matches!(next.as_mut().poll(&mut context), Poll::Pending));

    insert(&database, relation, 41, 90_110);
    assert!(flag.0.load(Ordering::Acquire));

    let Poll::Ready(Ok(event)) = next.as_mut().poll(&mut context) else {
        panic!("publication wake must make the exact event ready");
    };
    assert_eq!(event.inserted(), &[vec![Value::I64(41)]]);
    assert!(event.removed().is_empty());

    drop(snapshot);
    drop(database);
    std::fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn dropping_pending_future_unregisters_waker_without_losing_durable_event() {
    let (directory, database, relation) = fixture();
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot.watch(&Query::scan(relation)).expect("exact watch");

    let abandoned = Arc::new(FlagWake::default());
    let abandoned_waker = Waker::from(Arc::clone(&abandoned));
    let mut context = Context::from_waker(&abandoned_waker);
    let mut pending = Box::pin(watch.next());
    assert!(matches!(pending.as_mut().poll(&mut context), Poll::Pending));
    drop(pending);

    insert(&database, relation, 42, 90_120);
    assert!(!abandoned.0.load(Ordering::Acquire));

    let replacement = Arc::new(FlagWake::default());
    let replacement_waker = Waker::from(replacement);
    let mut replacement_context = Context::from_waker(&replacement_waker);
    let mut next = Box::pin(watch.next());
    let Poll::Ready(Ok(event)) = next.as_mut().poll(&mut replacement_context) else {
        panic!("durable event must survive future cancellation");
    };
    assert_eq!(event.inserted(), &[vec![Value::I64(42)]]);

    drop(next);
    drop(snapshot);
    drop(database);
    std::fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn executor_task_migration_replaces_the_registered_waker() {
    let (directory, database, relation) = fixture();
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot.watch(&Query::scan(relation)).expect("exact watch");

    let old = Arc::new(FlagWake::default());
    let new = Arc::new(FlagWake::default());
    let old_waker = Waker::from(Arc::clone(&old));
    let new_waker = Waker::from(Arc::clone(&new));
    let mut old_context = Context::from_waker(&old_waker);
    let mut new_context = Context::from_waker(&new_waker);
    let mut pending = Box::pin(watch.next());
    assert!(matches!(
        pending.as_mut().poll(&mut old_context),
        Poll::Pending
    ));
    assert!(matches!(
        pending.as_mut().poll(&mut new_context),
        Poll::Pending
    ));

    insert(&database, relation, 44, 90_130);
    assert!(!old.0.load(Ordering::Acquire));
    assert!(new.0.load(Ordering::Acquire));
    let Poll::Ready(Ok(event)) = pending.as_mut().poll(&mut new_context) else {
        panic!("replacement task waker must receive the publication");
    };
    assert_eq!(event.inserted(), &[vec![Value::I64(44)]]);

    drop(pending);
    drop(snapshot);
    drop(database);
    std::fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn spurious_wake_storm_never_fabricates_an_event() {
    let directory = temp_directory();
    std::fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(90_140);
    let equivalence = EquivalenceId::new(90_141);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("async watch schema");
    let notifier = Arc::new(InProcessPublicationNotifier::default());
    let database = Database::create_with_publication_notifier(
        &directory,
        schema,
        Arc::clone(&notifier) as Arc<dyn PublicationNotifier>,
    )
    .expect("create database");
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot.watch(&Query::scan(relation)).expect("exact watch");
    let counter = Arc::new(CountWake::default());
    let waker = Waker::from(Arc::clone(&counter));
    let mut context = Context::from_waker(&waker);
    let mut pending = Box::pin(watch.next());

    for expected in 1..=128 {
        assert!(matches!(pending.as_mut().poll(&mut context), Poll::Pending));
        notifier.notify_waiters();
        assert_eq!(counter.0.load(Ordering::Acquire), expected);
        assert!(matches!(pending.as_mut().poll(&mut context), Poll::Pending));
    }

    insert(&database, relation, 45, 90_150);
    let Poll::Ready(Ok(event)) = pending.as_mut().poll(&mut context) else {
        panic!("real publication after spurious wakes must still be received");
    };
    assert_eq!(event.inserted(), &[vec![Value::I64(45)]]);

    drop(pending);
    drop(snapshot);
    drop(database);
    std::fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn cancellation_wakes_a_pending_future_and_fails_closed() {
    let (directory, database, relation) = fixture();
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot.watch(&Query::scan(relation)).expect("exact watch");
    let cancellation = watch.cancellation();
    let flag = Arc::new(FlagWake::default());
    let waker = Waker::from(Arc::clone(&flag));
    let mut context = Context::from_waker(&waker);
    let mut pending = Box::pin(watch.next());
    assert!(matches!(pending.as_mut().poll(&mut context), Poll::Pending));

    cancellation.cancel();
    assert!(flag.0.load(Ordering::Acquire));
    let Poll::Ready(Err(error)) = pending.as_mut().poll(&mut context) else {
        panic!("cancelled watch must fail a pending receive");
    };
    assert!(error.to_string().contains("cancel"));

    drop(pending);
    drop(snapshot);
    drop(database);
    std::fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn bounded_drain_yields_backlog_in_explicit_fairness_slices() {
    let (directory, database, relation) = fixture();
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot.watch(&Query::scan(relation)).expect("exact watch");

    for (offset, value) in [51_i64, 52, 53, 54, 55].into_iter().enumerate() {
        insert(&database, relation, value, 90_160 + offset as u128);
    }

    let first = watch.drain_ready(2).expect("first bounded drain");
    assert_eq!(first.events().len(), 2);
    assert!(first.has_more());
    let second = watch.drain_ready(2).expect("second bounded drain");
    assert_eq!(second.events().len(), 2);
    assert!(second.has_more());
    let final_slice = watch.drain_ready(2).expect("final bounded drain");
    assert_eq!(final_slice.events().len(), 1);
    assert!(!final_slice.has_more());
    assert!(watch.try_recv().expect("drained watch").is_none());

    drop(snapshot);
    drop(database);
    std::fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn two_thousand_pending_watches_share_one_source_without_lost_wakes() {
    const WATCHES: usize = 2_000;

    let (directory, database, relation) = fixture();
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watches = (0..WATCHES)
        .map(|_| snapshot.watch(&Query::scan(relation)).expect("exact watch"))
        .collect::<Vec<_>>();
    let source = watches[0].readiness().source_id();
    assert!(
        watches
            .iter()
            .all(|watch| watch.readiness().source_id() == source)
    );

    let counter = Arc::new(CountWake::default());
    let waker = Waker::from(Arc::clone(&counter));
    let mut context = Context::from_waker(&waker);
    let mut pending = watches
        .iter_mut()
        .map(|watch| Box::pin(watch.next()))
        .collect::<Vec<_>>();
    for future in &mut pending {
        assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    }

    insert(&database, relation, 99, 90_190);
    assert_eq!(counter.0.load(Ordering::Acquire), WATCHES);
    for future in &mut pending {
        let Poll::Ready(Ok(event)) = future.as_mut().poll(&mut context) else {
            panic!("every pending watch must recover the durable publication");
        };
        assert_eq!(event.inserted(), &[vec![Value::I64(99)]]);
    }

    drop(pending);
    drop(watches);
    drop(snapshot);
    drop(database);
    std::fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[test]
fn unrelated_relation_publication_does_not_wake_async_subscription() {
    let directory = temp_directory();
    std::fs::create_dir_all(&directory).expect("create fixture directory");
    let left = RelationId::new(90_220);
    let right = RelationId::new(90_221);
    let equivalence = EquivalenceId::new(90_222);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(left, [Type::i64()], [equivalence]))
        .relation(RelationSchema::bag(right, [Type::i64()], [equivalence]))
        .build()
        .expect("dependency-frontier watch schema");
    let database = Database::create(&directory, schema).expect("create database");
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut left_watch = snapshot
        .watch(&Query::scan(left))
        .expect("left exact watch");
    let mut right_watch = snapshot
        .watch(&Query::scan(right))
        .expect("right exact watch");

    let left_counter = Arc::new(CountWake::default());
    let right_counter = Arc::new(CountWake::default());
    let left_waker = Waker::from(Arc::clone(&left_counter));
    let right_waker = Waker::from(Arc::clone(&right_counter));
    let mut left_context = Context::from_waker(&left_waker);
    let mut right_context = Context::from_waker(&right_waker);
    let mut left_next = Box::pin(left_watch.next());
    let mut right_next = Box::pin(right_watch.next());
    assert!(matches!(
        left_next.as_mut().poll(&mut left_context),
        Poll::Pending
    ));
    assert!(matches!(
        right_next.as_mut().poll(&mut right_context),
        Poll::Pending
    ));

    insert(&database, left, 71, 90_223);
    assert_eq!(left_counter.0.load(Ordering::Acquire), 1);
    assert_eq!(right_counter.0.load(Ordering::Acquire), 0);
    let Poll::Ready(Ok(left_event)) = left_next.as_mut().poll(&mut left_context) else {
        panic!("left dependency publication must wake left watch");
    };
    assert_eq!(left_event.inserted(), &[vec![Value::I64(71)]]);
    assert!(matches!(
        right_next.as_mut().poll(&mut right_context),
        Poll::Pending
    ));

    insert(&database, right, 72, 90_224);
    assert_eq!(right_counter.0.load(Ordering::Acquire), 1);
    let Poll::Ready(Ok(right_event)) = right_next.as_mut().poll(&mut right_context) else {
        panic!("right dependency publication must wake right watch");
    };
    assert_eq!(right_event.inserted(), &[vec![Value::I64(72)]]);

    drop(left_next);
    drop(right_next);
    drop(snapshot);
    drop(database);
    std::fs::remove_dir_all(directory).expect("remove fixture directory");
}

#[derive(Debug, Default)]
struct TokioProbeNotifier {
    inner: InProcessPublicationNotifier,
    armed: AtomicBool,
    future_polled: AtomicBool,
}

impl PublicationNotifier for TokioProbeNotifier {
    fn generation(&self) -> u64 {
        if self.armed.load(Ordering::Acquire) {
            self.future_polled.store(true, Ordering::Release);
        }
        self.inner.generation()
    }

    fn wait_after(&self, observed: u64) -> u64 {
        self.inner.wait_after(observed)
    }

    fn register_waker_after(&self, waiter_id: u64, observed: u64, waker: &Waker) -> u64 {
        self.inner.register_waker_after(waiter_id, observed, waker)
    }

    fn unregister_waker(&self, waiter_id: u64) {
        self.inner.unregister_waker(waiter_id);
    }

    fn notify_waiters(&self) {
        self.inner.notify_waiters();
    }
}

#[test]
fn tokio_current_thread_runtime_drives_same_future_without_adapter_dependency() {
    let directory = temp_directory();
    std::fs::create_dir_all(&directory).expect("create fixture directory");
    let relation = RelationId::new(90_200);
    let equivalence = EquivalenceId::new(90_201);
    let schema = Schema::builder()
        .equivalence(equivalence, PrimitiveEquivalence::I64Exact)
        .relation(RelationSchema::bag(relation, [Type::i64()], [equivalence]))
        .build()
        .expect("async watch schema");
    let notifier = Arc::new(TokioProbeNotifier::default());
    let database = Database::create_with_publication_notifier(
        &directory,
        schema,
        Arc::clone(&notifier) as Arc<dyn PublicationNotifier>,
    )
    .expect("create database");
    let snapshot = database.snapshot().expect("watch snapshot");
    let mut watch = snapshot.watch(&Query::scan(relation)).expect("exact watch");

    notifier.armed.store(true, Ordering::Release);
    let writer_database = database.clone();
    let writer_notifier = Arc::clone(&notifier);
    let writer = std::thread::spawn(move || {
        while !writer_notifier.future_polled.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        insert(&writer_database, relation, 43, 90_210);
    });

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("Tokio current-thread runtime");
    let event = runtime
        .block_on(watch.next())
        .expect("Tokio drives executor-neutral watch future");
    assert_eq!(event.inserted(), &[vec![Value::I64(43)]]);
    writer.join().expect("writer thread");

    drop(snapshot);
    drop(database);
    std::fs::remove_dir_all(directory).expect("remove fixture directory");
}
