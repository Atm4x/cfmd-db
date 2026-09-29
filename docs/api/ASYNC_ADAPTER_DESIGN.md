# CFMD Async Watch Design

Status: P348 executor-neutral direct-watch async + dependency-frontier readiness implemented; P349/P350 Python asyncio hostile validation active.

## Law

Async is a consumption mode of the existing exact watch, not a second watch implementation and not a wrapper object. `cfmd-runtime` owns subscription identity, exact revision/delta semantics, durable catch-up, cancellation, dependency readiness and backpressure state. Durable causal history and maintained query state remain event authority.

The Rust surface is intentionally one object:

```rust
let mut watch = query.watch()?;

let event = watch.next().await?; // executor-neutral async
let ready = watch.try_recv()?;   // nonblocking
let batch = watch.drain_ready(64)?;
let event = watch.recv()?;       // blocking host/thread integration
```

There is no `into_async()`, async wrapper, Tokio-specific watch type, worker thread, polling loop, executor-owned cursor, or second event queue.

## Future / Waker contract

`watch.next()` returns an ordinary standard-library `Future`. A poll samples the readiness generation, consumes exact durable watch state, then atomically registers the task `Waker` against that observed generation. Publication between the watch read and registration is therefore detected and cannot be lost. Dropping the pending Future unregisters its waker.

Tokio is used only as dev compatibility coverage. Any executor capable of polling a Rust Future can drive the same watch.

## Dependency-frontier readiness

Each exact watch already owns a maintained query program whose `scan_relations()` set is an exact relation dependency frontier. P348 binds that frontier to the publication wait handle.

The in-process notifier maintains:

- a global publication generation;
- an opaque/broadcast generation;
- the latest generation for each relation;
- an inverted relation -> pending-waiter index.

For an exact relation-data publication touching relation set `R`, only wildcard waiters and waiters indexed under members of `R` are made runnable. If a commit is relevant to `K` subscriptions, async wake work is proportional to the touched dependency buckets plus those `K` subscriptions, not every `N` watch in the runtime. Full/schema/opaque/liveness notifications broadcast because exact relation irrelevance is not available there.

This is wake filtering only. No mutation or result data is stored in the notifier.

## Observable quotient

Global causal revisions and public watch events are no longer required to be one-to-one.

A watch keeps a causal cursor through durable history and a separate observable frontier. An exact effect that has zero action on the maintained query is consumed by the causal cursor but emits no empty public event. If the effect touches a dependency but the differential program proves an empty output delta (for example, another row of the same filtered relation changes), maintained state advances but the public observable frontier remains unchanged.

The next non-empty event therefore represents the exact quotient transition from the previous observable frontier to the new target revision. This is not recomputation or heuristic suppression: zero action is established by exact history metadata plus the maintained differential program.

## Blocking and host integration

`recv()` remains the blocking form for thread/FFI hosts. `try_recv()` and bounded `drain_ready(max_events)` remain available on the same object. There is no API mode switch.

Cross-process notification transport may optimize wake delivery later, but must preserve this same dependency/readiness law and may never become event authority.

## Language bindings

Python asyncio and .NET should project the same direct watch semantics. A binding may use Tokio internally if convenient, but that is a binding implementation detail. Cancellation must ultimately drop/cancel the same Rust wait registration; lag/catch-up still comes from exact durable history.

P349 showed that Python cancellation can race a Rust event becoming ready: the bridge must retain a replayable delivery reservation until Python actually accepts the result. P350 found a second lifecycle edge: `pyo3-async-runtimes::future_into_py` returns a bare `asyncio.Future`, and a bare Future that is abandoned is not one of the Tasks automatically cancelled by `asyncio.run()` during loop shutdown. A Python binding must therefore make each in-flight receive loop-owned/cancellable (the validation harness taskifies it) or provide an equivalent acknowledgement/lifetime law. Event-loop shutdown must never strand the mutable watch cursor or lose a durable event. These are FFI delivery laws, not new database-event semantics.

## Sequencing

1. P345 shared readiness + bounded durable drain. **Complete.**
2. P346 race-free executor-neutral Future/Waker prototype. **Complete.**
3. P347 hostile cancellation/task-migration/spurious-wake/fan-out closure. **Complete.**
4. P348 direct `watch.next().await`, removal of `cfmd-async`, dependency-frontier wake filtering, observable quotient. **Complete.**
5. P349 Python/PyO3 asyncio proof with cancellation-safe replay reservation. **Complete.**
6. P350 hostile Python lifecycle/product validation, including loop-shutdown task ownership and public-facade black-box database tests. **Complete foundation.**
7. Continue multi-handle/process/error/concurrency hostile coverage before freezing the final Python facade; add executor/OS-specific acceleration only where measurement proves a capability unavailable from the generic contract.
