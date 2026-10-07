# CFMD — Selected `Database` / scoped `Context<M>` DX

Status: **selected DX direction / architectural contract for further implementation**

This document records the selected developer-facing model discussed after the schema-epoch transaction R&D. It is intentionally about the **runtime/frontend primitive model and DX**, not about preserving the current implementation shape. Pre-release CFMD may rewrite `cfmd-runtime`, codegen, or kernel-facing plumbing if needed to satisfy this model without semantic or performance regressions.

---

## 1. Core rule

The public application model should be built around two primary concepts:

```text
Database
    = long-lived schema-neutral database/runtime authority

Context<M>
    = scoped typed working context for consumer contract M
```

`Transaction` should **not remain a normal public primitive** if `Context<M>` owns the complete speculative unit-of-work state.

The old transaction machinery remains internally as an exact intent/change journal and proof input.

---

## 2. `Database` is the long-lived database object

`Database` represents the actual CFMD database/runtime authority.

It owns or controls:

- storage and durability;
- the authoritative current database schema;
- hosting / serving;
- migration execution;
- history and retention;
- revision publication;
- runtime/session authority;
- creation of typed consumer contexts.

Crucially, `Database` is **schema-neutral for the host application**.

Hosting an existing `.cfmd` database must not require codegen of its complete schema.

Example:

```rust
let db = Database::open("large.cfmd")?;

db.host(...)?;
```

This must remain valid even if the database contains an enormous schema and the hosting process itself does not need typed access to those relations.

A consumer schema appears only when some application code asks for typed access:

```rust
let ctx = db.context::<AuthContract>()?;
```

`AuthContract` may cover only the subset/semantic contract that this consumer needs.

Therefore:

```text
authoritative DB schema != Context<M> consumer contract
```

and `Database<M>` is not the target model.

---

## 3. `Context<M>` is scoped

The normal application lifetime of `Context<M>` is a **logical scope / unit of work**.

Conceptually:

```text
with db.context::<M>() as ctx {
    read
    application logic
    optional staged writes
    optional commit
}
```

Rust does not need literal `with`; RAII/block syntax or an equivalent ergonomic frontend API is sufficient.

The important semantic rule is:

> One `Context<M>` has one bounded lifetime in which its typed contract and formation world do not change.

This scope is the natural zero-downtime cutover boundary.

Example:

```text
scope #1:
    Context<A>
    ------------------
          A -> B migration
    ------------------
    Context remains A until scope ends

scope #2:
    may open Context<B>
```

A live `Context<A>` is never mutated into `Context<B>`.

---

## 4. Scoped does not mean artificially short-lived

`Context<M>` should have a **bounded semantic lifetime**, but CFMD must not impose an arbitrary request-duration rule.

For a server with online migrations, a natural scope may be:

- one HTTP/RPC request;
- one message;
- one background job;
- one logical operation;
- another explicitly bounded application unit.

For an embedded database where migrations are guaranteed to occur only **between process lifetimes**, it is perfectly valid to keep one context for the whole process:

```text
process starts
    Context<M> created
    Context<M> lives for process lifetime
process stops

migration happens offline / between processes

new process starts
    new Context<N>
```

So the rule is not:

> `Context` must be short-lived.

The rule is:

> `Context` must not outlive the boundary across which the application expects its typed consumer contract to change.

---

## 5. What `Context<M>` owns

A scoped `Context<M>` is the owner of the speculative working state for that scope.

Conceptually it contains:

```text
Context<M>
├── database/session capability
├── formation schema epoch
├── formation revision/root
├── Candidate<M>
├── semantic observations / guards
├── staged exact effects
└── internal IntentJournal
```

The internal `IntentJournal` is the descendant of the old public `Transaction` primitive.

It retains the important semantics previously associated with transaction intent:

- stable intent/retry identity when required;
- exact semantic effects;
- prerequisites / guards / observations;
- formation provenance;
- data needed by the kernel for exact rebase / transport / publication.

But the developer should not have to pass `&mut tx` through every typed operation.

---

## 6. Normal read → logic → optional write flow

The target application DX should allow normal host-language control flow:

```rust
let mut ctx = db.context::<AppContract>()?;

let x = ctx.users.x.get(id).await?;
let y = ctx.users.y.get(id).await?;

if x != y {
    return Ok(NoChange);
}

ctx.users.enabled.set(id, true)?;

ctx.commit().await?;

Ok(Changed)
```

A read does **not** imply that a commit must happen.

If application logic decides to return, the context is simply dropped and no staged mutation is published.

Likewise, if writes were staged but commit is never called, they remain speculative and are discarded with the context.

There must be **no auto-commit on drop**.

---

## 7. Reads inside a mutable Context are semantic observations

If application code reads data and then uses arbitrary Rust/Python/.NET logic to decide what to write, CFMD cannot inspect that host-language control flow.

Therefore the safe default is:

> Reads performed through a write-capable scoped Context become semantic observations of that unit of work.

Example:

```rust
let generation = ctx.security.generation.get(id).await?;

if generation != expected {
    return Ok(Rejected);
}

ctx.credentials.password.set(id, new_hash)?;
```

Internally the context records roughly:

```text
Observation:
    Security(id).generation

Effect:
    Credentials(id).password := new_hash
```

At commit, CFMD checks the exact semantic observation/effect laws required by the kernel.

This must remain semantic and coordinate/query-aware; it must not degrade into a global physical read set or table-wide invalidation.

---

## 8. Candidate and read-your-writes

The Context reads from its speculative Candidate, not blindly from the original formation snapshot.

Therefore:

```rust
ctx.users.name.set(id, "Artem")?;

let name = ctx.users.name.get(id).await?;

assert_eq!(name, "Artem");
```

must work naturally.

Conceptually:

```text
formation world
    +
staged exact intent
    =
Candidate<M>
```

Candidate maintenance must remain factorized/incremental. The DX must not introduce O(data) cloning or full-state materialization on ordinary reads or writes.

---

## 9. Explicit inspection before commit remains available

Removing the public `Transaction` must not remove the useful old capabilities of inspecting or discarding planned changes.

A Context should support an API of the following semantic shape:

```text
ctx.changes() / ctx.intent_view()
    read-only inspection of planned exact changes

ctx.preview()
    inspect/query the resulting Candidate

ctx.commit()
    explicitly publish the unit of work

drop(ctx)
    discard everything
```

An explicit `rollback()` may be offered for clarity, but it must not be required for correctness; dropping an uncommitted Context is sufficient.

The internal intent representation itself should not be exposed as arbitrary mutable state.

---

## 10. Public `Transaction` is no longer necessary

Under this model, keeping a normal public API such as:

```rust
let mut tx = Transaction::new();

ctx.users.set(&mut tx, ...);
ctx.orders.remove(&mut tx, ...);

db.commit(&tx)?;
```

duplicates ownership:

- Context knows the typed contract/world;
- Transaction accumulates the same operation's semantic intent;
- every mutation must redundantly mention both.

The selected direction is instead:

```rust
let mut ctx = db.context::<M>()?;

ctx.users.set(...)?;
ctx.orders.remove(...)?;

ctx.commit().await?;
```

The old `Transaction` implementation should become an internal concept such as:

```text
IntentJournal
TransactionIntent
ExactIntent
```

Exact naming is implementation-level and remains open.

The important law is that there is **one public unit-of-work owner**: `Context<M>`.

---

## 11. `Context<M>` and migration

A Context formed in schema epoch A remains an A context for its complete lifetime.

Example:

```text
Context<A>
    formation A@R
    reads A Candidate
    stages A intent

database migrates A -> B

same Context<A>
    still uses A formation semantics
```

At commit, CFMD does not reinterpret the Context as B.

The schema-epoch architecture remains:

```text
Candidate<A>
    -> exact A rebase to first migration source
    -> evaluate/certify A formation semantics
    -> FormationWorldSeal
    -> exact ΔA
    -> forward effect transport A -> B
    -> native B-side rebase/conflict logic
    -> current publication
```

After the formation-world seal, A-specific query/guard semantics end. The continuation is effect-oriented; no B-native state needs to be interpreted as an old A query/world.

This preserves the corrected post-P479 direction.

---

## 12. Migration-ready scoped Context selection

When a deployment temporarily supports both an old and a new consumer contract, the application needs one unavoidable typed control-flow branch.

A manual two-step pattern such as:

```rust
match db.current_schema() {
    A => db.context::<A>()?,
    B => db.context::<B>()?,
}
```

is not sufficient because migration can occur between observing the schema and admitting the Context.

Therefore CFMD runtime should expose one **atomic Context admission** primitive:

```text
begin_context()
    -> exact admitted schema/contract epoch
    -> formation revision/root
    -> ContextCore
```

The Rust frontend may present a thin typed match:

```rust
cfmd::context!(db, {
    A(ctx) => old_logic(ctx).await?,
    B(ctx) => new_logic(ctx).await?,
});
```

Exact syntax is not frozen.

The semantic requirements are:

1. admission and formation binding are atomic;
2. exactly one typed branch starts;
3. the chosen Context type never changes during the scope;
4. migration during the scope does not switch branches;
5. the next scope may choose the new contract.

---

## 13. This is not a migration router framework

The selected design does **not** introduce public permanent abstractions such as:

```text
CutoverContext
MigrationRouter
SchemaRouter
Context<A, B>
SwitchContext
ReaderContext
```

Nor does it require a client compatibility graph.

The A/B branch is temporary deployment code and should be removable after rollout.

Before rollout:

```rust
let ctx = db.context::<A>()?;
```

During rollout:

```rust
cfmd::context!(db, {
    A(ctx) => ...,
    B(ctx) => ...,
});
```

After rollout:

```rust
let ctx = db.context::<B>()?;
```

The runtime primitive is generic Context admission, not a special migration object.

---

## 14. Hosting and typed application access stay separate

A database server process may do only:

```rust
let db = Database::open("db.cfmd")?;
db.host(...)?;
```

and never create a typed Context at all.

Another process/module may create:

```rust
let ctx = db.context::<AuthContract>()?;
```

A different consumer may use:

```rust
let ctx = db.context::<BillingContract>()?;
```

No consumer contract becomes the authoritative schema of `Database`.

---

## 15. Snapshot / historical mode

Historical access remains a distinct concept because it is an exact immutable committed world.

The selected conceptual relation is:

```text
Database
    -> Context<M>      scoped current/admitted working world

Database/history
    -> Snapshot<M>     exact immutable historical world
```

If editing from an exact historical world is supported, the preferred direction is to create a scoped Context explicitly based on that Snapshot rather than reintroduce a second public transaction vocabulary.

Exact surface syntax remains open, for example:

```rust
let snapshot = db.at::<M>(revision)?;
let mut ctx = snapshot.edit()?;
```

The important invariant is that strict historical basis and ordinary current/admitted basis remain distinguishable internally.

---

## 16. Frontend/runtime architecture

The same semantic primitive should exist in `cfmd-runtime`; Rust/Python/.NET frontends must not each reconstruct their own transaction model.

Conceptually:

```text
DatabaseCore

ContextCore
├── database/session capability
├── formation epoch/revision
├── Candidate
├── ObservationJournal
└── IntentJournal

SnapshotCore
```

Rust:

```text
ContextCore + generated typed surface -> Context<M>
```

Python:

```text
ContextCore + Python schema binding -> Context[M]
```

.NET:

```text
ContextCore + generated typed surface -> Context<TModel>
```

This avoids creating a beautiful facade over a confused runtime API.

---

## 17. Performance / semantic non-regression requirements

The DX redesign is not allowed to weaken the kernel or hide expensive fallbacks.

Required invariants include:

- no full-state materialization for normal Context creation/read/write/commit;
- no O(history-depth) stale proof where indexed/persistent-root proof already exists;
- no old-schema current-world query routing;
- no inverse migration;
- no hidden replay of application callbacks;
- no last-write-wins fallback;
- no table-wide conflict fallback where exact semantic coordinates/observations exist;
- no per-operation mandatory global mutex/heap indirection merely to emulate Context ownership;
- Candidate maintenance proportional to touched/affected data where the underlying algebra permits;
- cross-schema continuation uses forward exact effect transport;
- publication remains revision-bound and durable;
- existing retry/idempotency separation between original client intent and realized residual must be preserved.

If the current code structure prevents these properties, the code structure should be rewritten rather than weakening this DX contract.

---

## 18. Selected mental model

The simplest mental model for developers is:

```text
Database
    = the database

Context<M>
    = my scoped typed working world over that database

Snapshot<M>
    = an immutable exact past world
```

Within a Context:

```text
read
-> think in normal host-language code
-> optionally stage changes
-> inspect/preview if desired
-> explicitly commit
   OR
-> leave scope and discard
```

During online migration:

```text
old scope  = Context<A> until it ends
new scope  = Context<B> when newly admitted
```

No running typed object changes its Rust/Python/.NET contract in place.

---

## 19. Open syntax questions

The following syntax choices are **not yet architectural laws** and may still be optimized experimentally:

- literal Rust macro spelling for migration-ready Context admission;
- `ctx.commit()` versus `db.commit(ctx)`; both can preserve authority if Context carries a non-forgeable database capability, but commit must remain explicit;
- exact names of `changes()`, `preview()`, `rollback()`;
- exact snapshot-to-edit spelling;
- whether read-only scopes get an optimized immutable Context facade;
- exact internal name replacing the old public `Transaction`.

These should be decided by compile probes / hostile DX examples without changing the selected ownership model above.

---

## 20. Selected result

The selected direction is:

```text
long-lived:
    Database

scoped:
    Context<M>

internal to Context:
    Candidate
    observations/guards
    exact intent / former Transaction machinery

explicit:
    commit or discard

migration:
    current Context keeps its formation contract
    next Context scope is the natural typed cutover point

hosting:
    never requires the host process to codegen the full database schema
```

This is the baseline DX model to use for subsequent runtime/frontend R&D unless a concrete semantic or performance counterexample invalidates it.

---

## PASS537 clarification — bounded lifecycle vs post-cutover old-client bridge

PASS537 executable coverage freezes the scoped lifecycle law:

- a `Context<A>` admitted before `A -> B` remains on its exact A formation/Candidate for the rest of that scope;
- it does not observe B-only HEAD changes by silently refreshing;
- its exact staged A intent may still commit after cutover through the existing formation seal + forward effect transport;
- the next admission observes B;
- typed binding incompatibility is exposed as stable `ContractNotRepresentable` rather than an accidental low-level schema/type error.

This closes the lifecycle of an **already-admitted** scope, but deliberately does not pretend that a **new A-only client arriving after B publication** is solved. Such a client requires a certified current-world `SchemaBridge<A,B>`-class read/write representation theorem. Historical-A reads, dual live schemas, name fallback and per-query migration routing remain forbidden.

---

## PASS538 clarification — current-world SchemaBridge kernel

PASS538 introduces the kernel bridge authority required by the post-cutover old-client case without changing the selected scoped Context ownership model.

```text
new A-only client arrives while B is authoritative
        |
        v
retained verified migration lineage
        |
        v
CurrentSchemaBridge<A,current>
        |
        +-- compile representable A read -> current-native read
        +-- transport exact relation intent/write coordinates -> current coordinates
        `-- fail closed when the law is not proved
```

The first exact read class is row-representation identity, including certified relation-coordinate retargeting. Query operators remain unchanged and the target expression must typecheck to the identical result type. Value-changing row transforms are intentionally not decoded back into A unless a separate read-factorization theorem exists.

The bridge is resolved from retained migration metadata and contains no historical source data. Authorization is always checked on the resulting current-schema coordinates.

Product `Context<A>` activation over this bridge remains the next step; PASS538 freezes the semantic compiler before wiring it into object/model-field DX.
