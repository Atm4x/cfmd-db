use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    },
};

use cfmd::{
    CfmdEntity, CommitOutcome, Database, Id, ObjectWatch, ObjectWatchEvent, OwnedMany, Plan, Ref,
    Schema, Transaction, TransactionId,
    dynamic::{
        EquivalenceId, PrimitiveEquivalence, Query, RelationId, RelationSchema, Type, Value,
    },
};
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use tokio::sync::{Mutex, OwnedMutexGuard};

const LEFT_RELATION: RelationId = RelationId::new(99_100);
const RIGHT_RELATION: RelationId = RelationId::new(99_101);
const LEFT_EQUIVALENCE: EquivalenceId = EquivalenceId::new(99_110);
const RIGHT_EQUIVALENCE: EquivalenceId = EquivalenceId::new(99_111);
static NEXT_TRANSACTION: AtomicU64 = AtomicU64::new(100_000);

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "probe.todo")]
struct Todo {
    #[cfmd(id)]
    id: Id<Todo>,
    title: String,
    done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "probe.user")]
struct User {
    #[cfmd(id)]
    id: Id<User>,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "probe.task")]
struct Task {
    #[cfmd(id)]
    id: Id<Task>,
    title: String,
    owner: Ref<User>,
    reviewer: Option<Ref<User>>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "probe.owner")]
struct Owner {
    #[cfmd(id)]
    id: Id<Owner>,
    name: String,
    #[cfmd(orphan = "delete")]
    assets: OwnedMany<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, CfmdEntity)]
#[cfmd(key = "probe.asset")]
struct Asset {
    #[cfmd(id)]
    id: Id<Asset>,
    label: String,
}

fn runtime_error(error: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

fn relation(side: &str) -> PyResult<RelationId> {
    match side {
        "left" => Ok(LEFT_RELATION),
        "right" => Ok(RIGHT_RELATION),
        other => Err(PyRuntimeError::new_err(format!(
            "unknown probe relation {other:?}; expected 'left' or 'right'"
        ))),
    }
}

type EventTuple = (u64, u64, Vec<i64>, Vec<i64>);
type TodoTuple = (u128, String, bool);
type TodoEventTuple = (u64, u64, Vec<TodoTuple>, Vec<TodoTuple>);

fn event_tuple(event: &cfmd::dynamic::WatchEvent) -> PyResult<EventTuple> {
    fn rows(rows: &[Vec<Value>]) -> PyResult<Vec<i64>> {
        rows.iter()
            .map(|row| match row.as_slice() {
                [Value::I64(value)] => Ok(*value),
                _ => Err(PyRuntimeError::new_err(
                    "probe watch received a non-I64 row",
                )),
            })
            .collect()
    }

    Ok((
        event.source_revision().raw(),
        event.target_revision().raw(),
        rows(event.inserted())?,
        rows(event.removed())?,
    ))
}

fn todo_tuple(todo: &Todo) -> TodoTuple {
    (todo.id.raw(), todo.title.clone(), todo.done)
}

fn todo_event_tuple(event: &ObjectWatchEvent<Todo>) -> TodoEventTuple {
    (
        event.source_revision().raw(),
        event.target_revision().raw(),
        event.inserted().iter().map(todo_tuple).collect(),
        event.removed().iter().map(todo_tuple).collect(),
    )
}

#[pyclass]
struct TaskifiedAwaitable {
    inner: Py<PyAny>,
}

#[pymethods]
impl TaskifiedAwaitable {
    fn __await__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.inner.bind(py).call_method0("__await__")
    }
}

fn taskify<'py>(py: Python<'py>, future: Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let wrapper = Py::new(
        py,
        TaskifiedAwaitable {
            inner: future.unbind(),
        },
    )?;
    py.import("asyncio")?
        .getattr("ensure_future")?
        .call1((wrapper,))
}

struct PendingDelivery {
    event: Option<EventTuple>,
    replay: Arc<StdMutex<VecDeque<EventTuple>>>,
    guard: Option<OwnedMutexGuard<cfmd::dynamic::QueryWatch>>,
}

impl PendingDelivery {
    fn new(
        event: EventTuple,
        replay: Arc<StdMutex<VecDeque<EventTuple>>>,
        guard: OwnedMutexGuard<cfmd::dynamic::QueryWatch>,
    ) -> Self {
        Self {
            event: Some(event),
            replay,
            guard: Some(guard),
        }
    }
}

impl Drop for PendingDelivery {
    fn drop(&mut self) {
        if let Some(event) = self.event.take() {
            self.replay
                .lock()
                .expect("Python probe replay mutex poisoned")
                .push_front(event);
        }
        self.guard.take();
    }
}

impl<'py> IntoPyObject<'py> for PendingDelivery {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(mut self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        let event = self
            .event
            .take()
            .expect("pending Python watch delivery already consumed");
        let output = event.into_pyobject(py)?.into_any();
        self.guard.take();
        Ok(output)
    }
}

struct PendingTodoDelivery {
    event: Option<TodoEventTuple>,
    replay: Arc<StdMutex<VecDeque<TodoEventTuple>>>,
    guard: Option<OwnedMutexGuard<ObjectWatch<Todo>>>,
}

impl PendingTodoDelivery {
    fn new(
        event: TodoEventTuple,
        replay: Arc<StdMutex<VecDeque<TodoEventTuple>>>,
        guard: OwnedMutexGuard<ObjectWatch<Todo>>,
    ) -> Self {
        Self {
            event: Some(event),
            replay,
            guard: Some(guard),
        }
    }
}

impl Drop for PendingTodoDelivery {
    fn drop(&mut self) {
        if let Some(event) = self.event.take() {
            self.replay
                .lock()
                .expect("Python probe todo replay mutex poisoned")
                .push_front(event);
        }
        self.guard.take();
    }
}

impl<'py> IntoPyObject<'py> for PendingTodoDelivery {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(mut self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        let event = self
            .event
            .take()
            .expect("pending Python todo delivery already consumed");
        let output = event.into_pyobject(py)?.into_any();
        self.guard.take();
        Ok(output)
    }
}

#[pyclass(name = "Database")]
struct ProbeDatabase {
    database: Database,
}

impl ProbeDatabase {
    fn commit_transaction(&self, transaction: &Transaction) -> PyResult<u64> {
        let outcome = self.database.commit(transaction).map_err(runtime_error)?;
        let revision = match outcome {
            CommitOutcome::Committed { revision }
            | CommitOutcome::AlreadyCommitted { revision } => revision,
        };
        Ok(revision.raw())
    }

    fn commit_plan(&self, plan: &Plan) -> PyResult<u64> {
        let transaction = NEXT_TRANSACTION.fetch_add(1, Ordering::Relaxed);
        let outcome = self
            .database
            .commit_plan(plan, TransactionId::new(u128::from(transaction)))
            .map_err(runtime_error)?;
        let revision = match outcome {
            CommitOutcome::Committed { revision }
            | CommitOutcome::AlreadyCommitted { revision } => revision,
        };
        Ok(revision.raw())
    }
}

#[pymethods]
impl ProbeDatabase {
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let schema = Schema::builder()
            .equivalence(LEFT_EQUIVALENCE, PrimitiveEquivalence::I64Exact)
            .equivalence(RIGHT_EQUIVALENCE, PrimitiveEquivalence::I64Exact)
            .relation(RelationSchema::bag(
                LEFT_RELATION,
                [Type::i64()],
                [LEFT_EQUIVALENCE],
            ))
            .relation(RelationSchema::bag(
                RIGHT_RELATION,
                [Type::i64()],
                [RIGHT_EQUIVALENCE],
            ))
            .object::<Todo>()
            .object::<User>()
            .object::<Task>()
            .object::<Owner>()
            .object::<Asset>()
            .build()
            .map_err(runtime_error)?;
        let database = Database::builder(path)
            .schema(schema)
            .create()
            .map_err(runtime_error)?;
        Ok(Self { database })
    }

    fn insert(&self, side: &str, value: i64) -> PyResult<u64> {
        let mut plan = self.database.plan().map_err(runtime_error)?;
        plan.insert(relation(side)?, vec![Value::I64(value)]);
        self.commit_plan(&plan)
    }

    fn watch(&self, side: &str) -> PyResult<ProbeWatch> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let watch = snapshot
            .watch(&Query::scan(relation(side)?))
            .map_err(runtime_error)?;
        Ok(ProbeWatch {
            watch: Arc::new(Mutex::new(watch)),
            replay: Arc::new(StdMutex::new(VecDeque::new())),
            keepalive: self.database.clone(),
        })
    }

    fn todo_insert(&self, id: u128, title: String, done: bool) -> PyResult<u64> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let plan = snapshot
            .objects::<Todo>()
            .map_err(runtime_error)?
            .insert(Todo {
                id: Id::new(id),
                title,
                done,
            })
            .map_err(runtime_error)?;
        drop(snapshot);
        self.commit_plan(&plan)
    }

    fn todo_get(&self, id: u128) -> PyResult<Option<TodoTuple>> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        snapshot
            .objects::<Todo>()
            .map_err(runtime_error)?
            .get(Id::new(id))
            .map_err(runtime_error)
            .map(|value| value.as_ref().map(todo_tuple))
    }

    fn todo_query_done(&self, done: bool) -> PyResult<Vec<TodoTuple>> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let values = snapshot
            .objects::<Todo>()
            .map_err(runtime_error)?
            .where_(|todo| todo.done().eq(done))
            .all()
            .map_err(runtime_error)?;
        Ok(values.iter().map(todo_tuple).collect())
    }

    fn todo_update_title(&self, id: u128, title: &str) -> PyResult<u64> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let plan = snapshot
            .objects::<Todo>()
            .map_err(runtime_error)?
            .where_(|todo| todo.id().eq(Id::new(id)))
            .update_plan(|mut todo| {
                title.clone_into(&mut todo.title);
                todo
            })
            .map_err(runtime_error)?;
        drop(snapshot);
        self.commit_plan(&plan)
    }

    fn todo_set_done(&self, id: u128, done: bool) -> PyResult<u64> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let plan = snapshot
            .objects::<Todo>()
            .map_err(runtime_error)?
            .where_(|todo| todo.id().eq(Id::new(id)))
            .update_plan(|mut todo| {
                todo.done = done;
                todo
            })
            .map_err(runtime_error)?;
        drop(snapshot);
        self.commit_plan(&plan)
    }

    fn todo_delete(&self, id: u128) -> PyResult<u64> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let plan = snapshot
            .objects::<Todo>()
            .map_err(runtime_error)?
            .where_(|todo| todo.id().eq(Id::new(id)))
            .delete_plan()
            .map_err(runtime_error)?;
        drop(snapshot);
        self.commit_plan(&plan)
    }

    fn todo_watch(&self, done: Option<bool>) -> PyResult<ProbeTodoWatch> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let query = snapshot.objects::<Todo>().map_err(runtime_error)?.query();
        let query = if let Some(done) = done {
            query.where_(|todo| todo.done().eq(done))
        } else {
            query
        };
        let watch = query.watch().map_err(runtime_error)?;
        let initial = watch.initial().iter().map(todo_tuple).collect();
        Ok(ProbeTodoWatch {
            watch: Arc::new(Mutex::new(watch)),
            replay: Arc::new(StdMutex::new(VecDeque::new())),
            initial,
            keepalive: self.database.clone(),
        })
    }

    fn user_insert(&self, id: u128, name: String) -> PyResult<u64> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let plan = snapshot
            .objects::<User>()
            .map_err(runtime_error)?
            .insert(User {
                id: Id::new(id),
                name,
            })
            .map_err(runtime_error)?;
        drop(snapshot);
        self.commit_plan(&plan)
    }

    fn task_insert(&self, id: u128, title: String, owner_id: u128) -> PyResult<u64> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let plan = snapshot
            .objects::<Task>()
            .map_err(runtime_error)?
            .insert(Task {
                id: Id::new(id),
                title,
                owner: Ref::new(Id::new(owner_id)),
                reviewer: None,
            })
            .map_err(runtime_error)?;
        drop(snapshot);
        self.commit_plan(&plan)
    }

    fn task_titles_for_owner(&self, owner_name: &str) -> PyResult<Vec<String>> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let tasks = snapshot
            .objects::<Task>()
            .map_err(runtime_error)?
            .where_(|task| {
                task.owner()
                    .matches(|user| user.name().eq(owner_name.to_owned()))
            })
            .all()
            .map_err(runtime_error)?;
        Ok(tasks.into_iter().map(|task| task.title).collect())
    }

    fn owner_insert_with_asset(
        &self,
        owner_id: u128,
        owner_name: String,
        asset_id: u128,
        asset_label: String,
    ) -> PyResult<u64> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let plan = snapshot
            .objects::<Owner>()
            .map_err(runtime_error)?
            .insert(Owner::cfmd_new(
                Id::new(owner_id),
                owner_name,
                OwnedMany::new([Asset {
                    id: Id::new(asset_id),
                    label: asset_label,
                }]),
            ))
            .map_err(runtime_error)?;
        drop(snapshot);
        self.commit_plan(&plan)
    }

    fn owner_insert_empty(&self, owner_id: u128, owner_name: String) -> PyResult<u64> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let plan = snapshot
            .objects::<Owner>()
            .map_err(runtime_error)?
            .insert(Owner::cfmd_new(
                Id::new(owner_id),
                owner_name,
                OwnedMany::empty(),
            ))
            .map_err(runtime_error)?;
        drop(snapshot);
        self.commit_plan(&plan)
    }

    fn owner_move_asset(&self, source: u128, target: u128, asset: u128) -> PyResult<u64> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let owners = snapshot.objects::<Owner>().map_err(runtime_error)?;
        let source = owners.require(Id::new(source)).map_err(runtime_error)?;
        let target = owners.require(Id::new(target)).map_err(runtime_error)?;
        let mut transaction = Transaction::new();
        source
            .assets
            .move_to(&mut transaction, Id::new(asset), &target.assets)
            .map_err(runtime_error)?;
        drop(snapshot);
        self.commit_transaction(&transaction)
    }

    fn owner_asset_ids(&self, owner: u128) -> PyResult<Vec<u128>> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let owner = snapshot
            .objects::<Owner>()
            .map_err(runtime_error)?
            .require(Id::new(owner))
            .map_err(runtime_error)?;
        Ok(owner
            .assets
            .load()
            .map_err(runtime_error)?
            .iter()
            .map(|asset| asset.id.raw())
            .collect())
    }

    fn owner_detach_label_preview(
        &self,
        owner: u128,
        label: &str,
    ) -> PyResult<(usize, usize, u64)> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        let owner = snapshot
            .objects::<Owner>()
            .map_err(runtime_error)?
            .require(Id::new(owner))
            .map_err(runtime_error)?;
        let mut transaction = Transaction::new();
        owner
            .assets
            .where_(|asset| asset.label().eq(label.to_owned()))
            .map_err(runtime_error)?
            .detach_all(&mut transaction)
            .map_err(runtime_error)?;
        let preview = self.database.preview(&transaction).map_err(runtime_error)?;
        let orphaned = preview.derived().orphan_entities_deleted();
        let normalized = preview.derived().normalized_rows_removed();
        drop(snapshot);
        let revision = self.commit_transaction(&transaction)?;
        Ok((orphaned, normalized, revision))
    }

    fn asset_exists(&self, asset: u128) -> PyResult<bool> {
        let snapshot = self.database.snapshot().map_err(runtime_error)?;
        snapshot
            .objects::<Asset>()
            .map_err(runtime_error)?
            .get(Id::new(asset))
            .map_err(runtime_error)
            .map(|value| value.is_some())
    }

    fn history_len(&self) -> PyResult<usize> {
        Ok(self
            .database
            .history()
            .map_err(runtime_error)?
            .entries()
            .len())
    }

    fn undo_latest(&self) -> PyResult<u64> {
        let mut transaction = Transaction::new();
        self.database
            .undo_latest(&mut transaction)
            .map_err(runtime_error)?;
        self.commit_transaction(&transaction)
    }

    #[staticmethod]
    fn persisted_todo(path: &str, id: u128) -> PyResult<Option<TodoTuple>> {
        let database = Database::open(path).map_err(runtime_error)?;
        let snapshot = database.snapshot().map_err(runtime_error)?;
        snapshot
            .objects::<Todo>()
            .map_err(runtime_error)?
            .get(Id::new(id))
            .map_err(runtime_error)
            .map(|value| value.as_ref().map(todo_tuple))
    }
}

#[pyclass(name = "Watch")]
struct ProbeWatch {
    watch: Arc<Mutex<cfmd::dynamic::QueryWatch>>,
    replay: Arc<StdMutex<VecDeque<EventTuple>>>,
    keepalive: Database,
}

impl ProbeWatch {
    fn next_awaitable<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let watch = Arc::clone(&self.watch);
        let replay = Arc::clone(&self.replay);
        let keepalive = self.keepalive.clone();
        let future = pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let _keepalive = keepalive;
            let mut guard = watch.lock_owned().await;
            let event = if let Some(event) = replay
                .lock()
                .expect("Python probe replay mutex poisoned")
                .pop_front()
            {
                event
            } else {
                event_tuple(&guard.next().await.map_err(runtime_error)?)?
            };
            Ok(PendingDelivery::new(event, replay, guard))
        })?;
        taskify(py, future)
    }
}

#[pymethods]
impl ProbeWatch {
    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.next_awaitable(py)
    }

    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.next_awaitable(py)
    }
}

#[pyclass(name = "TodoWatch")]
struct ProbeTodoWatch {
    watch: Arc<Mutex<ObjectWatch<Todo>>>,
    replay: Arc<StdMutex<VecDeque<TodoEventTuple>>>,
    initial: Vec<TodoTuple>,
    keepalive: Database,
}

impl ProbeTodoWatch {
    fn next_awaitable<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let watch = Arc::clone(&self.watch);
        let replay = Arc::clone(&self.replay);
        let keepalive = self.keepalive.clone();
        let future = pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let _keepalive = keepalive;
            let mut guard = watch.lock_owned().await;
            let event = if let Some(event) = replay
                .lock()
                .expect("Python probe todo replay mutex poisoned")
                .pop_front()
            {
                event
            } else {
                todo_event_tuple(&guard.next().await.map_err(runtime_error)?)
            };
            Ok(PendingTodoDelivery::new(event, replay, guard))
        })?;
        taskify(py, future)
    }
}

#[pymethods]
impl ProbeTodoWatch {
    fn initial(&self) -> Vec<TodoTuple> {
        self.initial.clone()
    }

    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.next_awaitable(py)
    }

    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.next_awaitable(py)
    }
}

#[pymodule]
fn cfmd_async_probe(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<ProbeDatabase>()?;
    module.add_class::<ProbeWatch>()?;
    module.add_class::<ProbeTodoWatch>()?;
    Ok(())
}
