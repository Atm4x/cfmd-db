# CFMD — POST-PASS400 MASTER HANDOFF / ARCHITECTURE & PRODUCTIZATION LEDGER

**Authoritative handoff point:** PASS400 (`CFMD_PASS400_BOUNDED_FIELD_CARRIER_SEGMENTS`).  
**Purpose:** this file is the single continuation document for a new agent/session. It intentionally carries both the recent migration/Physical Realization line and older unfinished product/DX lines so they cannot disappear when the next 10–20 passes focus on one subsystem.

---

# 0. NON-NEGOTIABLE CONTINUATION PROTOCOL

## 0.1 Wall clock / execution discipline

Every implementation pass uses the same external UTC wall-clock discipline unless the user explicitly changes it:

- announce official UTC start **at the beginning** of the pass;
- useful wall clock: **20 minutes**;
- hard wall clock: **24 minutes**;
- at the hard boundary, stop processes even if work is mid-flight; do not invent a later start time;
- functional freeze should normally occur before useful boundary; after freeze only deterministic verification, reports, cleanup and packaging;
- compilation of this workspace can exceed a single tool-call timeout. Compile/test by crate/target in stages and reuse a warm `target`; do not launch one monolithic build and assume a 45-second tool call will survive;
- Unity projects: do not run dotnet builds (not directly relevant to CFMD, but this is a persistent execution rule).

Toolchain convention used in recent passes:

- Rust 1.98.1 x86_64-unknown-linux-gnu tar may be supplied by user;
- extract only required components (`rustc`, host `rust-std`, `cargo`, optionally `rustfmt`);
- compile a minimal probe if environment/toolchain needs validation;
- delete temporary Rust tar copies/toolchain/build output after pass packaging;
- final repository must have `target/ = 0`.

## 0.2 Required pass artifacts

For implementation passes, final user-facing deliverables remain:

1. **full repository ZIP**;
2. **PASSxxx_REPORT.md**;
3. concise overview of what changed, verification/perf numbers, and next target.

Do not return changed-files-only bundles unless the user explicitly asks. Do not silently edit unrelated Legacy/Obsolete code; obsolete code may be used as information sources but should not be modernized unless still on an active path.

## 0.3 Hostile / R&D law

Before adding a generic fallback, router, compatibility shim, SQL-shaped abstraction or O(N²)/O(data) workaround, ask whether CFMD's existing algebra can yield a more universal law.

Preferred order:

1. locate the semantic owner in `kernel-*`;
2. recover existing Γ/change/transport/query/lens/proof machinery;
3. formulate the exact law / certificate;
4. implement one universal principle;
5. lower/specialize for performance;
6. use explicit fail-closed gaps where the mathematics is not yet complete;
7. never hide a missing law behind "fallback to old path".

Architecture may be aggressively refactored; pre-1.0 compatibility is not a reason to keep an inferior model.

## 0.4 PERFORMANCE IS NOW A FIRST-CLASS GATE

Starting P396+, architectural beauty is insufficient. Every new physical realization path that can affect hot reads/scans/cutover cost must be benchmarked against native/direct baseline.

Do not accept:

- O(number of rows) metadata construction for semantic cutover when a column/segment law can be O(schema/layout);
- permanent generic transform-on-read with multi-x overhead on hot coordinates when specialization/materialization can remove it;
- per-scalar routing in sequential scans when chunk/range routing can be lowered once;
- short noisy microbench outliers as conclusions. Repeat warm runs / extend inner repetitions.

## 0.5 MASTER LEDGER CARRY RULE — MANDATORY

**This is the rule that must prevent another lost Context/DbContext line.**

Every future PASS report and every successor master handoff MUST carry forward all unresolved ledger items from this file. A line may disappear only if the report explicitly marks it:

- `CLOSED in Pxxx` with the actual solution;
- `SUPERSEDED in Pxxx` with the replacement architecture;
- `REJECTED` with a concrete hostile reason.

Each pass report should contain (or update `docs/status/PRODUCTIZATION_LEDGER.md` with) these buckets:

```text
CLOSED THIS PASS
OPEN — IMMEDIATE
OPEN — DEFERRED / RETURN AFTER CURRENT LINE
SUPERSEDED / DO NOT EXTEND
PERFORMANCE BASELINES TO PRESERVE
NEXT RECOMMENDED PASS
```

**Do not only carry the currently active subsystem.** Context/DX, authorization, Semantic Rules, bindings, history/retention, etc. stay visible while realization work continues.

Recommended repository continuation artifact after this handoff is eventually copied into mainline:

`docs/status/POST_PASS400_MASTER_LEDGER.md`

Until explicitly added, this handoff file itself is the continuation authority.

---

# 1. CURRENT REPOSITORY BASELINE AT PASS400

Baseline archive supplied by user:

- `CFMD_PASS400_BOUNDED_FIELD_CARRIER_SEGMENTS(1).zip`
- `PASS400_REPORT(1).md`

PASS400 report states:

- `kernel-realization`: 13 passed / 6 ignored perf benchmarks;
- `kernel-durability`: 230 passed / 2 ignored;
- runtime migration: 3/3;
- public surface: 44/44;
- public API contract: 1/1;
- workspace/repository/public verifiers PASS;
- durability/on-disk format not changed;
- `target/` removed.

Important mainline docs inside P400 repo:

- `docs/status/PRODUCTIZATION_LEDGER.md`
- `docs/status/PROJECT_STATUS.md`
- `docs/status/HISTORICAL_PROBLEMS_LEDGER.md`
- `docs/status/KERNEL_HOSTILE_LEDGER.md`
- `docs/rnd/CFMD_PHYSICAL_REALIZATION_ALGEBRA_PASS393.md`
- `docs/api/CFMD_DX_CONTROL_MODEL_RU.md`
- `docs/api/PRODUCT_ROADMAP.md`
- `docs/architecture/CFMD_RUNTIME_KERNEL_DX_P356_RU.md`
- `docs/spec/CFMD_CORE_SPEC.md`
- root `SPEC.md`

The main product crate remains `cfmd`; universal binding/runtime semantic owner remains `cfmd-runtime`; `kernel-*` are internal.

---

# 2. CORE PRODUCT / DX LAWS THAT MUST NOT BE LOST

These are not cosmetic preferences; they constrain future architecture.

## 2.1 Authority is visible in the callsite

From the P354 control-model decision:

- `Query` describes a read;
- `Plan` describes a low-level change;
- `Transaction` is a passive container of intent/effects;
- `Candidate` is a proposed future world;
- `Database` / `SessionDatabase` owns publication to the live database;
- `ReadContext` owns reads at a specific revision;
- `Watch` owns only subscription lifecycle, not database authority.

Good:

```rust
let preview = db.preview(&tx)?;
let outcome = db.commit(&tx)?;
```

Bad target DX:

```rust
tx.commit()?;      // hides which database owns publication
candidate.commit()?;
```

Ordinary CRUD should eventually read like:

```rust
let mut tx = Transaction::new(/* stable intent identity */);

db.users.add(&mut tx, user)?;
db.projects.remove(&mut tx, project_id)?;

db.preview(&tx)?;
db.commit(&tx)?;
```

`Plan`, `RelationId`, internal Γ, `QueryNode`, manual transaction IDs should not be required in ordinary application code.

## 2.2 Exact kernel semantics are the single source of truth

No frontend/binding may implement independent:

- query semantics;
- migration semantics;
- Candidate semantics;
- history/undo semantics;
- watch semantics;
- conflict/rebase semantics;
- authorization semantics.

Rust/Python/TMD/CLI/.NET compile/translate into the same runtime/kernel vocabulary.

## 2.3 No hidden ORM-style behavior

Preserve:

- immutable committed revisions;
- explicit current vs historical vs Candidate future worlds;
- no hidden field-load I/O;
- no implicit re-read of a newer HEAD while evaluating an already-bound value;
- explicit many-valued traversal;
- exact/delta watch rather than polling/recompute degradation;
- deterministic publication and durable idempotency.

## 2.4 Transactions are semantic intents, not frozen-head mutexes

Old P355 analysis plus P371–P373 implementation converged to:

- ordinary transaction carries stable client intent identity, exact operations/effects/preconditions/provenance;
- stale HEAD is not automatically an error;
- kernel-change certificates decide whether intent is transportable/rebasable/conflicting;
- `Ready`, `Rebasable`, `Conflict` derive from semantic coordinates/action laws, not raw revision-number equality;
- do not re-run user callbacks on a newer HEAD;
- do not implement last-write-wins;
- do not add application conflict tables over the kernel.

**Status:** the major P355 durable-intent payer is already CLOSED by P371–P373 for ordinary relation and mixed/object transaction classes. Do not redo it.

Strict snapshot-bound/full-revision/schema-migration operations remain exact by design.

---

# 3. DATABASE / CONTEXT / ENTITY MODEL — THE OLD LINE THAT MUST RETURN

The user may informally call this "DbContext". Current repository vocabulary is `Context<M>` / `Database::context::<S>()`.

## 3.1 Physical Database and typed Context are separate concepts

Target model established in P377–P380:

```text
Database
    = physical/runtime/durability authority
    = owns complete current persisted semantic model
    = can open/host without compiling the application's full Rust entity model

Context<M>
    = local typed consumer/application contract over an already-open Database
    = not a second persisted schema
    = not by itself an authorization boundary
```

`Database::context::<S>()` exists from P378.

## 3.2 Explicit membership; no global entity autodiscovery

`#[derive(CfmdSchema)]` / named `EntitySet<T>` roots were established for explicit model composition. No linker/global registry should silently decide DB membership.

P376 typed creation scaffolding exists, but the long-term separation is:

- authoritative DB can exist independent of consumer Rust structs;
- typed Context binds afterward.

Large-model sub-root/module composition remains an optional DX problem; it must preserve explicit membership and direct `db.<set>` ergonomics.

## 3.3 `CfmdEntity` authoritative vs consumer

Current intended law after P380:

```rust
#[derive(CfmdEntity)]
#[cfmd(key = "app.user", authoritative)]
struct User { ... }
```

means one complete canonical current definition eligible to create the persisted model.

Consumer:

```rust
#[derive(CfmdEntity)]
#[cfmd(key = "app.user")]
struct ReaderUser { ...subset... }
```

means only the fields/types this consumer knows.

There is deliberately **no `partial` marker**. Partiality is discovered at Context binding.

## 3.4 Local naming: `bind`, not authoritative legacy aliases

P379's `rename_from` direction was superseded by P380.

Target:

```rust
#[derive(CfmdEntity)]
#[cfmd(key = "app.user")]
struct ReaderUser {
    #[cfmd(bind = "clinical_note")]
    doctor_note: f64,
}
```

`bind` is consumer-local naming only. It is forbidden on `authoritative` entities.

Authoritative persisted evolution occurs only through Migration, never legacy alias history embedded in the current entity.

## 3.5 What partial Context already safely does (P378)

CLOSED:

- semantic local-field -> persisted-field projection, not ordinal binding;
- omitted fields are not decoded/materialized into the consumer object;
- full-row mutation from a partial contract fails closed;
- partial create remains unavailable when omitted required/reference/rule-constrained data would have to be invented;
- first safe scalar patch preserves hidden persisted columns exactly;
- persisted Semantic Rules still validate the authoritative future state even when the reader does not know those rules.

## 3.6 Context line still OPEN — DO NOT LOSE

After the current realization/migration line stabilizes, return here:

1. **reference-field patch calculus** for partial Context;
2. **relationship patch/mutation calculus** for partial Context;
3. **field-granular certified change coordinates** — current scalar patch may still lower to a full row delta, causing unrelated field patches to conflict because they share one relation row;
4. safe delete/create authority semantics independent of object shape;
5. traversal safety: a local `Ref<T>` must not bypass server-side authorization merely because it has access to a `ReadContext`;
6. final Context creation/open DX cleanup; P376 `SchemaDatabase<S>` typed-open/create scaffolding is transitional, not the desired conceptual model.

This was explicitly deferred after P380 and repeatedly carried as "return to Context" through P391. It is still open after P400.

---

# 4. AUTHORIZATION / ROLES — OLD OPEN LINE

**Context shape is NOT security.** Omitting `passport_secret` from a Rust struct only reduces typed projection; a raw/dynamic client might still name it.

Need DB-owned authorization enforced on query/change IR, including dynamic/binding clients.

Target dimensions include at least:

- entity/relation read;
- field read;
- traversal;
- scalar/reference field write;
- relationship mutation;
- create;
- delete;
- history;
- watch/subscription;
- hosted/session operation authority.

The authorization law must survive field rename/migration/local `bind`; permissions bind to semantic coordinates, not source-language spelling.

Recommended ordering remains:

1. finish Context reference/relationship + field-granular change coordinates;
2. then enforce granular DB-owned authorization on those coordinates.

Do not treat `Context<M>` as a role system.

---

# 5. SEMANTIC RULES / VALIDATION — OLD OPEN LINE

P374–P375 already established database-owned typed field rules:

- integer inclusive ranges;
- Unicode-scalar text length bounds;
- finite text membership;
- rules are persisted schema semantics and participate in authoritative validation / VMF;
- rules are not Rust callbacks.

OPEN:

1. deterministic `Matches` / regex/pattern semantics;
2. richer compositional/custom semantic rule expressions;
3. entity/model invariants;
4. transaction `require` / semantic preconditions where the same underlying expression law genuinely applies;
5. generated/schema-file/TMD ergonomics.

Hard rule: no host-language callback validator. A regex/pattern capability must be a serialized deterministic DB-owned semantic module/VM/law recoverable across bindings.

The VMF/zero-law remains the authoritative validity concept; physical realization rewrites that are extensionally equal to the same semantic world must not create new invariant meaning.

---

# 6. TRANSACTION / CHANGE / MERGE STATUS

## 6.1 CLOSED P371–P373

P370 identified the need to preserve kernel-change strength through product commit.

P371 found the durable retry-identity split: stable client intent must not be confused with its realized residual effect.

P372 closed relation-data residual publication with durable split:

```text
client intent identity
!=
realized residual effect
```

P373 extended this to mixed/object changes with exact residual/complement history.

Therefore ordinary adaptive transaction commits now preserve kernel rebase certificates across relation-only and mixed/object mutation classes.

Do not reopen this as a new merge engine.

## 6.2 Still relevant change-kernel debt

Context scalar patches currently need more granular semantic change coordinates so independent field writes do not conflict simply because the physical storage relation row is shared.

Future writable Physical Realization overlays/lenses must reuse `kernel-change` / `kernel-lens` semantics rather than invent migration-specific write conflict rules.

---

# 7. QUERY / WATCH / HOSTING FOUNDATIONS ALREADY CLOSED

Do not regress these while refactoring realization/durability.

Closed product foundations include:

- object-first `Ref<T>`, `Many<T>`, `OwnedMany<T>` semantics;
- relationship set mutations/moves/orphan policy;
- deep strong-reference traversal (P370) using the same relational algebra, not an ORM navigation engine;
- native predicate operators and broad Γ-native query work;
- exact watch with runtime-neutral async/Future/Waker semantics;
- dependency-frontier wake filtering; no second event queue;
- hosted protocol/session composition (`cfmd-protocol`, `cfmd-host`);
- canonical hosted wire framing;
- storage builder / single-file default lifecycle;
- hosting composed after open rather than `Database::open` secretly starting a listener.

OPEN later product surfaces:

- final Python facade/wheels (P349/P350 are validation harness, not final API);
- Python multi-handle/process lifecycle, transaction/conflict/error mapping, larger stress;
- .NET/WPF adapter over same runtime protocol;
- Studio/CLI polish;
- backup/restore/corruption-recovery UX;
- public benchmark/binary-size budgets;
- platform expansion, especially native Windows secure-memory support.

Rust-first remains the target: Python/.NET consume the stable Rust/runtime protocol, not kernels directly.

---

# 8. MIGRATION — CANONICAL SEMANTIC MODEL

## 8.1 Frontend is sugar; kernel/runtime own migration semantics

Original intended pipeline remains correct:

```text
Rust SDK / Python / CLI / TMD
           |
           v
     MigrationModel / TransformPlan
           |
           v
       cfmd-runtime
           |
           v
   kernel migration calculus
           |
           v
   durable semantic revision
```

No SDK implements a second migration engine.

No arbitrary Rust callback is permitted as migration semantics.

## 8.2 Authoritative entity is legacy-free

At every current semantic revision there is one canonical schema.

Migration is the only legal way to change persisted meaning/shape. Consumer Context may bind local names, but authoritative entity metadata must not accumulate aliases/rename history.

## 8.3 P380 migration calculus

P380 introduced the verified kernel migration object and runtime `MigrationModel` covering:

- old field -> new field;
- old field -> several new fields;
- several old fields -> one new field;
- drop;
- create/default;
- deterministic type transform such as `i64 -> f64`;
- relation/row migration primitives.

Target revision runs complete authoritative validation.

P388 later removed the O(data) full target snapshot from migration WAL: durable migration stores a compact deterministic `SchemaMigrationProgram` plus semantic target information, verified again during recovery.

This P388 result remains important and is NOT superseded.

---

# 9. HISTORY / TIME TRAVEL / RETENTION — P381–P386

These passes remain largely valid under Physical Realization Algebra.

## P381 — semantic migration boundary

Separated:

- ordinary Plan reversibility;
- historical materializability across migration.

Migration appears as one `SemanticChangeEvent`; it does not masquerade as an ordinary Plan inverse.

## P382 — `HistoricalEpochAnchor`

Compaction may not delete the last physical authority needed for retained old-world history.

Also fixed migration complement chain base: historical base must not be reinterpreted relative to current checkpoint schema.

## P383 — cross-epoch `revision_at()`

Historical read before migration selects the historical epoch/material, its own semantic registry and exact replay; it does **not** compute B -> inverse migration -> A.

## P384–P385 — single-file history authority

- active single-file generation can serve retained historical epoch directly;
- old generations can be archived as authenticated historical closure sections through checkpoint rotations;
- closures are carried only while pinned; no eternal archive leak.

## P386 — retention release / encryption / crash hardening

- historical materialization authority can be explicitly released without erasing the fact the migration occurred;
- encrypted historical closure path tested;
- torn-root/reopen authority tested.

## Realization-era evolution still OPEN

Current P382–P386 history retention often pins generation-level historical authority. Once durable Physical Realization becomes mainline, evolve toward:

```text
HistoricalRevisionRoot
    -> historical RealizationRoot
    -> reachable PhysicalAtoms
```

Then one old atom can simultaneously serve:

```text
historical A:  A.field <- Direct(atom)
current B:     B.field <- Transform(atom)
```

Authority lies in the root, not the atom.

Do not create a second history store.

---

# 10. P387–P391: WHAT TO KEEP, WHAT IS SUPERSEDED

## P387 — causal semantic cutover frontier

Useful discovery: semantic migration effect can be committed while physical checkpoint/materialization lags, and physical maintenance need not create semantic history events.

However the old `WalForwardCutover / NativeCheckpoint` framing should not become the permanent migration progress model once realization roots are durable. "Fully B-native" is an optimization/representation state, not semantic correctness.

## P388 — KEEP

Keep compact durable `SchemaMigrationProgram`; it removes O(data) target `Revision B` payload from migration WAL and verifies program recovery against causal source.

## P389 — KEEP SELECTIVELY

Useful primitives:

- forward slices;
- exact source dependency sets;
- row-local streaming transform;
- `required_source_relations(...)` style dependency reasoning.

These are useful realization compiler/materializer tools.

## P390–P391 — SUPERSEDED AS TARGET ARCHITECTURE

Do **not** continue steady-state current-world:

- `MixedMigrationRevisionView`;
- `MixedMigrationPhysicalAuthority`;
- semantic/runtime routing that knows "relation A vs relation B".

P392/P393 hostile review superseded this direction.

Useful proof ideas from P391 may be re-expressed as generic `RealizationCertificate`, but mixed schema/epoch must not leak into Query/Change/Watch/Auth.

---

# 11. SELECTED ARCHITECTURE AFTER P393: PHYSICAL REALIZATION ALGEBRA

This is the central architecture going forward.

## 11.1 One current semantic world

At any current revision:

```text
ONE current SemanticContext / schema B
+
PhysicalAtoms P
+
certified RealizationRoot rho : P -> current finite model B
```

Old physical atoms may remain, but old schema A is not a current read authority.

For migration `M : A -> B`:

```text
rho_B = normalize(M o rho_A)
```

Semantic cutover is A -> B once. Physical convergence afterward changes representation only.

## 11.2 Do not think "chunk belongs to schema A"

Preferred model:

```text
PhysicalAtom / segment / column
    has physical codec/layout

Realization rule
    maps physical atoms -> current semantic coordinate
```

Example:

```text
physical p_age_i64

current B.age <- I64ToF64Direct(p_age_i64)
```

Later:

```text
materialize p_age_f64
B.age <- Direct(p_age_f64)
```

Semantic revision does not change.

## 11.3 Physical materialization is extensional same-revision rewrite

Required law:

```text
evaluate(rho_before) == evaluate(rho_after)
```

Materialization/compaction may replace realization roots/atoms but may not change current database meaning.

GC is reachability from current + retained historical roots, not migration progress flags.

## 11.4 Shadow/preparation is optional, not default storage model

For global migrations/invariants that cannot be certified cheaply at cutover, a shadow/preparation phase with exact effect transport may construct required witnesses/materializations before publication.

But after cutover there is still one current schema B. Shadow epoch is a preparation technique, not a second live database model.

---

# 12. P394–P400: CURRENT PHYSICAL REALIZATION IMPLEMENTATION

## P394 — stable relation-column semantic identity

Found positional debt: migration/validation rules used relation column ordinals as identity.

Now stable semantic relation-column IDs exist separately from ordinal/layout positions.

Migration/checkpoint codecs preserve semantic column IDs. Runtime positional convenience lowers to IDs before kernel boundary.

**Law:** semantic identity != physical ordinal.

## P395 — `kernel-realization` reference calculus

Independent kernel owner introduced instead of putting semantics in `kernel-plan`.

Reference model includes:

- `PhysicalAtomId`;
- physical codecs/payloads;
- `PhysicalAtomStore`;
- `RealizationRoot`;
- field expressions `Direct`, `Constant`, deterministic transform via existing `ExactQuery`;
- exact evaluation back to `DatabaseState` as oracle;
- representation rewrite certification;
- dependency graph / GC reachability over current + historical roots.

Durability deliberately unchanged; full `Revision=(S,Gamma,M)` / `DatabaseState` remains oracle/authority while realization laws mature.

## P396 — migration composition + performance hostile

`SchemaMigrationProgram` composes into target realization without first materializing full target state.

Correctness law worked, but per-cell/per-row reference granularity was rejected for production:

- generic derived scalar read roughly 12–13x direct;
- composition for 100k field-values ~139.6 ms and O(N) metadata.

Therefore P395/P396 per-value forms are **correctness oracle/reference calculus only**.

## P397 — factorized entity-field columns

Production direction became factorized:

```text
semantic field -> one factorized realization rule -> many entity values
```

`normalize(M o rho)` recognizes:

- direct alias/copy;
- constant/default;
- specialized `i64 -> f64` direct column transform;
- general split/merge via Product + ExactQuery.

100k benchmark:

- composition metadata ~46–51 us;
- 3 physical atoms/dependencies;
- normalized lazy read roughly native baseline;
- whole-column materialization ~14–16.5 ms.

## P398 — factorized relation columns

Stable relation-column IDs now drive one rule per semantic column. Row-local migration no longer needs per-row metadata or `Vec<Row>` storage representation.

Also fixed transport passthrough law: same `RelationDef` is insufficient; stable column IDs/order must also match.

General relational query rewrite is still fail-closed.

100k benchmark:

- composition ~64–73 us;
- constant metadata/dependency count;
- derived scan native cost class;
- whole target column materialization ~3.7–4.1 ms.

## P399 — bounded relation-column chunks

One semantic relation-column rule may have sparse native physical chunks over a base realization.

No dense row bitmap / migration journal.

Range scan lowering resolves physical chunk once per range instead of per scalar.

Hot native chunk scan stabilized around direct cost class (~0.96–1.07x in reported runs).

## P400 — bounded entity-field carrier segments

Entity IDs are semantic identities and must not define physical chunk arithmetic.

Introduced immutable physical carrier order and:

```text
CarrierSegmentCoordinate {
    carrier_atom,
    start_ordinal,
    len
}
```

Field rule may have sparse native carrier segments over derived base.

100k sparse EntityId benchmark, 4096-entity segments:

- partial random point/direct: ~0.94–1.10x;
- hot native segment scan/direct: ~0.94–1.07x;
- materialize one segment: ~390–440 us.

Current law shared by fields and row-local relation columns:

```text
semantic coordinate
    -> one factorized realization rule
    -> zero or more sparse bounded native physical segments
```

Chunk completion is representation reachability, not semantic migration state.

---

# 13. IMMEDIATE OPEN LINE AFTER PASS400 — PASS401+

**Do not jump to durability yet.** Two semantic layers remain more dangerous than serialization.

## 13.1 PRIORITY 1 — hostile general relational-query migration + write semantics together

The P400 report recommends hostile-auditing these together because a global relation rewrite affects what local writes can safely mean.

### A. Factorized general relational-query migration/preparation

Current factorized compiler supports passthrough/reorder/row-local transforms. General relation transforms (union/global query/aggregate-like dependency) remain fail-closed.

Need exact non-fallback architecture with:

- explicit dependency closure from target semantic relation/column to source semantic coordinates;
- factorized preparation/materialization plan;
- no current-world schema-A query fallback;
- bounded/streaming execution where possible;
- if global target validity/index/invariant needs preparation, use certified preparation/shadow technique before cutover rather than dual live schema;
- performance scaling measured vs relation cardinality/dependency size.

### B. Writable current-schema B overlays/lenses

After migration, all writes are in B semantics even when realization leaves include old physical atoms.

For invertible/writable realizations, `kernel-lens` may lower B write through a certified lens.

For non-injective transform, **never require inverse A reconstruction**. Preferred law:

```text
current B = derived base B + exact B-native overlay/delta
```

A B write may create/publish native B segment/overlay; later compaction folds base+overlay into new native atom(s).

Must integrate with existing `kernel-change` exact effects/conflict/rebase, not create migration-specific conflict rules.

### C. Shared physical segment index only if structurally justified

P399 relation chunk and P400 field segment routing have similar sparse overlay ideas but different payload shapes. Unify behind a small shared physical segment index only if hostile review proves the duplication is structural, not merely superficial.

## 13.2 PRIORITY 2 — then runtime current-world integration

Once general relational realization + writes are proven:

- connect `kernel-realization` to actual runtime/current physical execution owner;
- Query/Change/Watch/Auth continue to see only current semantic B;
- execution lowers into realization/physical segments beneath semantic boundary;
- remove/supersede remaining P390/P391 mixed prototypes once their useful behavior is covered.

## 13.3 PRIORITY 3 — only then durable physical atoms + realization roots

Current durability still checkpoints full logical `Revision/DatabaseState` and considers many physical layouts reconstructible.

Target after semantic laws stabilize:

```text
DurableRevisionRoot
    current SemanticContext
    PhysicalAtom authority/root
    RealizationRoot
    validation/semantic certificates
    causal/history authority
```

Requirements:

- checkpoint/reopen can serve current semantic model directly from durable realization authority without eagerly materializing full B `DatabaseState`;
- realization-only publication changes physical root under same semantic revision;
- crash/root-switch matrix;
- single-file compaction carries reachable current + historical atoms;
- encryption/authentication of atoms/root metadata;
- no O(data) cutover regression;
- no loss of P381–P386 time-travel laws.

## 13.4 PRIORITY 4 — history retention refactor to roots/atoms

Evolve generation-level epoch pins into realization-root/atom reachability once durable realization exists.

Historical semantic A remains A. Current semantic B may reuse the same physical atom through another realization expression.

## 13.5 PRIORITY 5 — return to Context / Authorization / Semantic Rules

After current realization architecture is durable enough not to churn semantic coordinates:

1. Context reference patches;
2. Context relationship patches;
3. field-granular change coordinates;
4. granular DB-owned authorization;
5. deterministic regex/general Semantic Rules;
6. entity/model invariants + transaction `require` on the common semantic expression substrate.

This line is **not optional cleanup**; it is a deferred product goal and must remain visible in every ledger.

---

# 14. MIGRATION FRONTEND / DX STILL OPEN

Runtime/kernel migration vocabulary is intentionally lower-level than final application syntax.

Need eventually:

- Rust migration builder/DSL over typed old/new entity coordinates;
- Python/TMD/CLI compile into the same `MigrationModel` / `SchemaMigrationProgram`;
- migration diagnostics that explain unmapped required coordinates / invalid transforms / preparation requirements;
- no arbitrary callbacks;
- authoritative entity remains current-only and legacy-free.

The original user mental model remains valid:

> frontend gives sugar; runtime receives a general TransformPlan written by the developer for field/entity evolution; the kernel verifies that transform; at the migration revision the whole logical world becomes schema B; physical normalization can continue afterward without exposing A semantics.

---

# 15. CREATION / DATABASE BUILDER / HOSTING STATUS

Already established:

- primary Rust construction path: `Database::builder(path)`;
- new path defaults to single-file storage;
- directory storage explicit / detected on reopen;
- storage/encryption/notifier are DB/runtime concerns;
- hosting is composed after open via `cfmd-host`, not embedded in `Database::open`;
- external transports/providers are optional; runtime semantic authority stays central.

Keep this architecture while Context creation/open DX is refined.

---

# 16. ENCRYPTION / SECURE MEMORY STATUS (DEFERRED, DO NOT REDO WITHOUT NEW EVIDENCE)

Productization work already selected AES-256-GCM-SIV as practical default direction and integrated secure-memory work on Linux. Historical realization/closure paths have encrypted hostile coverage from P386.

Prior product decision: strict elimination of every third-party constructor transient is not a mainline blocker for general/medium business; high-assurance deployments may compile CFMD with patched/custom crypto vendor/backend. Do not derail realization/product DX back into this R&D unless requirements change.

Windows secure-memory expansion remains deferred to native Windows work.

---

# 17. PERFORMANCE BASELINES / REGRESSION NUMBERS TO CARRY

These are sandbox microbenchmarks, not universal hardware guarantees; preserve their qualitative class and remeasure after relevant refactors.

## Rejected reference granularity (P396)

- generic per-cell scalar derived read ~12–13x direct;
- 100k per-cell composition ~139.6 ms + O(N) metadata.

This is why per-cell realization is oracle-only.

## Factorized fields (P397)

100k values, direct `i64->f64` normalized rule:

- compose ~46–51 us;
- direct ~33–34 ns;
- derived ~28–31 ns (sub-1x is noise; native cost class);
- materialize full 100k field column ~14–16.5 ms;
- post materialization native ~32–37 ns.

## Factorized relation columns (P398)

100k rows:

- compose ~64–73 us;
- derived/native scan same cost class;
- whole-column materialization ~3.7–4.1 ms.

## Relation chunks (P399)

100k rows, 4096-row chunks:

- hot native chunk range scan ~0.96–1.07x direct after range lowering;
- one chunk roughly ~0.17–0.4 ms depending measurement setup/noise.

## Entity field carrier segments (P400)

100k sparse entities, 4096-entity segment:

- partial random point ~0.94–1.10x direct;
- hot segment scan ~0.94–1.07x direct;
- one segment ~390–440 us.

Performance law to preserve:

> normalized/simple lazy realization may stay derived if it remains native-cost-class; hot generic transforms must materialize/specialize; semantic cutover metadata must not scale with total row count for local/factorizable migrations.

---

# 18. SUPERSEDED / DO NOT EXTEND

Unless new hostile evidence overturns the decision, do not build further architecture on:

1. authoritative `rename_from`/legacy alias chains — superseded by Migration + consumer `bind`;
2. current-world `MixedMigrationRevisionView` / `MixedMigrationPhysicalAuthority` — superseded by one semantic world + Physical Realization Algebra;
3. routing Query/Change/Watch/Auth based on physical schema epoch A/B;
4. per-value/per-row realization as production storage — oracle only;
5. generic full-row map in live query engine for migration;
6. permanent generic transform-on-read on hot coordinates when normalization/materialization can make it native;
7. migration progress bitmap/journal when progress can be derived from realization reachability;
8. ordinal relation-column identity;
9. full target `Revision B` snapshot in migration WAL (P388 removed it);
10. inverse-migration time travel B->A as default history strategy;
11. Context omission as authorization;
12. host-language validation callbacks;
13. ORM-style hidden joins/includes/load behavior;
14. last-write-wins / application conflict tables / re-running user callbacks for stale transactions.

---

# 19. PASS370–PASS400 TIMELINE — COMPACT BUT COMPLETE CONTINUITY MAP

## P370

Symbolic deep strong-reference traversal + native predicate operators. Kept same query algebra; optional-reference traversal intentionally explicit. Opened certified change/commit and Semantic Rules audit.

## P371

Found real loss of kernel-change strength after certification; added Γ Set residual primitive. Discovered durable retry identity cannot equal realized residual effect; prototype integration reverted until dual authority is correct.

## P372

Durable stable client relation intent + realized residual effect split; relation-data certified residual commit, retry and history/reopen closed.

## P373

Extended client-vs-realized split to mixed/object residual commits; exact complement/history/recovery; ordinary adaptive transaction commit closure completed.

## P374

Database-owned typed Semantic Rules foundation: range/length/membership through schema validation/VMF/durability. Regex/general rules remain open.

## P375

Rules moved into `CfmdEntity` metadata. Explicit typed schema-root R&D chose named `EntitySet<T>` fields; global autodiscovery rejected.

## P376

`CfmdSchema`, live `EntitySet<T>`, typed schema database facade. Later declared transitional relative to physical `Database` + bound Context.

## P377

`authoritative` entity capability; hostile design for partial Context. Determined partial structs are compatibility contracts, not authorization; full-row partial writes unsafe.

## P378

Real `Database::context::<S>()`, semantic field projection, partial materialization, safe scalar patch; hidden-field preservation; Context auth/reference/field-granular issues remain open.

## P379

Stable semantic field identity across Rust rename via `rename_from`; demonstrated old reader compatibility. This particular authoritative alias mechanism is later superseded by P380 Migration + consumer `bind`.

## P380

Kernel-first schema migration calculus; strict authoritative current schema; deterministic runtime MigrationModel; consumer `bind`; `rename_from` removed from target design.

## P381

First-class semantic migration history boundary; Plan undo != historical materializability.

## P382

Historical epoch anchor and compaction retention; historical complement chain base fix.

## P383

`revision_at()` crosses migration by opening anchored historical epoch/registry, never inverse migration.

## P384

Single-file active generation can serve historical epoch; P382 blanket compaction failure narrowed.

## P385

Streaming immutable historical epoch closure carried across rotations/compaction; only live pinned closures retained.

## P386

Historical authority release, encrypted closure path, torn-root/crash hardening.

## P387

Formalized semantic cutover vs checkpoint physical frontier without semantic history churn. Later progress-state framing is partially superseded by realization architecture.

## P388

Durable deterministic migration program; removed O(data) full target revision from migration WAL. Keep.

## P389

Forward-slice calculus, exact dependencies, bounded row-local cursor. Keep as realization/materializer primitive.

## P390

Semantic revision head + mixed migration revision view prototype. Useful research, but target architecture later superseded.

## P391

Certified mixed physical authority prototype; global validation thought through. Superseded as steady-state by P393, though proof/certificate ideas survive generically.

## P392

No standalone final `PASS392_REPORT.md` in the recovered chain. This was the architecture-hostile discussion that rejected extending mixed-current-world direction and led into P393. Do not treat the absent file as a missing implementation patch.

## P393

Full hostile/R&D Physical Realization Algebra decision. One current semantic world; physical atoms + realization root; `rho_B = normalize(M o rho_A)`; shadow only optional preparation.

## P394

Stable semantic relation-column identity separate from ordinal; migration/codec/validation plumbing moved to semantic IDs.

## P395

Created independent `kernel-realization` reference calculus; physical atoms, roots, exact evaluation/certification, dependency/GC law. Durability intentionally untouched.

## P396

Composed migration program into realization. Correctness worked; perf proved per-cell model unacceptable. Established mandatory factorization/perf law.

## P397

Factorized entity-field column realization; normalization makes simple lazy transform native-speed class; O(schema/layout) cutover metadata.

## P398

Factorized relation-column realization; fixed stable-ID passthrough bug; general relational query remains fail-closed.

## P399

Sparse bounded relation-column chunks; reachability-based progress; range scan lowering removes per-row routing overhead.

## P400

Sparse bounded entity-field carrier segments over immutable physical carrier order; sparse EntityId-safe routing and native-cost-class perf.

---

# 20. SOURCE / LIBRARY INDEX FOR A NEW AGENT

The handoff is self-contained, but these sources are useful for exact detail. Search ChatGPT Library / conversation files by the exact names below.

## Old DX / continuation authorities

- `CFMD_DX_CONTROL_MODEL_P354_RU (2).md` — database-owned control surface, target ordinary DX, history/watch/application laws.
- `CFMD_TRANSACTION_MERGE_DX_P355_RU (3).md` — transaction intent vs snapshot, Ready/Rebasable/Conflict, old merge payer later closed by P371–P373.
- `CFMD_POST_PASS380_CONTINUATION_LEDGER.md` / `(1).md` — large post-P380 Context/Migration/History/Rules/Auth continuation document.

## P370–P380 reports (Library)

- `PASS370_REPORT.md`
- `PASS371_REPORT.md`
- `PASS372_REPORT.md`
- `PASS373_REPORT.md`
- `PASS374_REPORT.md`
- `PASS375_REPORT.md`
- `PASS376_REPORT.md`
- `PASS377_REPORT.md`
- `PASS378_REPORT.md`
- `PASS379_REPORT.md`
- `PASS380_REPORT.md`

## P381–P400 reports (current conversation/generated files)

- `PASS381_REPORT.md`
- `PASS382_REPORT.md`
- `PASS383_REPORT.md`
- `PASS384_REPORT.md`
- `PASS385_REPORT.md`
- `PASS386_REPORT.md`
- `PASS387_REPORT.md`
- `PASS388_REPORT.md`
- `PASS389_REPORT.md`
- `PASS390_REPORT.md`
- `PASS391_REPORT.md`
- P392: no independent finalized report recovered; architecture discussion is consolidated by P393.
- `PASS393_RND_PHYSICAL_REALIZATION_ALGEBRA_REPORT.md`
- `PASS394_REPORT.md`
- `PASS395_REPORT.md`
- `PASS396_REPORT.md`
- `PASS397_REPORT.md`
- `PASS398_REPORT.md`
- `PASS399_REPORT.md`
- `PASS400_REPORT(1).md`

## P400 repository docs to read before a large refactor

Inside the P400 repository:

- `docs/status/PRODUCTIZATION_LEDGER.md`
- `docs/rnd/CFMD_PHYSICAL_REALIZATION_ALGEBRA_PASS393.md`
- `docs/api/CFMD_DX_CONTROL_MODEL_RU.md`
- `docs/api/PRODUCT_ROADMAP.md`
- `docs/status/PROJECT_STATUS.md`
- `docs/spec/CFMD_CORE_SPEC.md`
- `SPEC.md`

## Mechanically combined recent reports

During creation of this handoff, P370–P400 reports were mechanically concatenated for local reading into:

`combined_reports_370_400.md`

It is a source appendix, not the authoritative continuation plan. This master handoff is the curated authority.

---

# 21. RECOMMENDED FIRST ACTIONS FOR THE NEXT AGENT

Before writing code:

1. open P400 repository;
2. read this master handoff;
3. read `docs/status/PRODUCTIZATION_LEDGER.md` tail from P392–P400;
4. inspect `kernel-realization`, `kernel-transport`, `kernel-lens`, `kernel-change`, relation query/dependency owners;
5. do a hostile pass on **general relational-query factorization + writable B-semantic overlays/lenses together**;
6. benchmark any proposed hot path before accepting it;
7. keep general/global rewrite fail-closed until exact dependency/preparation law exists;
8. do not touch durable format until these semantic write/read laws close;
9. update both PASS report and the ledger carry buckets so Context/Auth/Rules remain visible.

Likely PASS401 question:

> Can a general relational migration be represented as a certified factorized preparation/dependency plan, while current-schema B writes are expressed as exact B overlays/lenses, such that neither read nor write ever needs to revive schema A as a current semantic world or invert a non-injective migration?

If yes, implement the smallest universal primitive. If not, record the obstruction and redesign before durability.

---

# 22. FINAL ONE-PAGE MENTAL MODEL

```text
USER / SDK
    |
    | typed Query / Transaction / Migration TransformPlan
    v
cfmd / cfmd-runtime
    |
    v
CURRENT SEMANTIC WORLD (one schema only)
    |
    | semantic coordinates, validation, change laws, auth, watch
    v
kernel-realization
    |
    | RealizationRoot rho
    | stable semantic field / relation-column IDs
    v
PHYSICAL ATOMS / COLUMNS / SEGMENTS / CHUNKS

Migration M : A -> B

semantic publication:
    A ----atomic----> B

realization publication:
    rho_B = normalize(M o rho_A)

old physical atoms may remain leaves of rho_B
but schema A is NOT a current query/change authority.

materialization:
    derived segment -> native segment
    same semantic revision

GC:
    reachable(current roots + retained historical roots)

history before migration:
    opens historical semantic A root/authority
    NOT inverse B -> A.
```

Product work postponed but mandatory after this storage/realization line:

```text
Context reference/relationship patches
    -> field-granular certified changes
    -> DB-owned granular authorization
    -> deterministic regex/general Semantic Rules
    -> final Context/DX audit
    -> final Python/.NET/Studio product surfaces
```

**Carry this list forward in every pass.**


### PASS418 realization/runtime authority delta

- Bag duplicate occurrence order is now explicit: Γ-class occurrence buckets are persistent FIFO queues, so witness removal and logical Bag survivor-order choose the same oldest live occurrence.
- Witness-owned logical Scan evidence now covers Set **and Bag**; runtime publication no longer requires O(N) Bag survivor projection.
- Maintained Group composes correctly over Bag seed authority under 128-transition churn, but a fresh full Group output occurrence root remains rejected; full result evidence may only return if derivable incrementally from already-maintained group+aggregate authority.
- Next hostile target: retained snapshot/root sharing + remaining runtime/history ownership seams, then factorized/streaming preparation debt before durability.

Carry forward unchanged product obligations: Context references/relationships, field-granular changes, DB-owned auth, Semantic Rules, final bindings/product surfaces, durable PhysicalAtoms/RealizationRoot and historical root retention.


### PASS419 realization/runtime authority delta

- Retained runtime snapshots now hold exactly one Γ evidence authority: `RelationBaseWitness`. The duplicate bundle-level Scan-seed directory is removed; Query/Watch/materialization bootstrap derive persistent Scan views directly from the witness.
- Atomic-publication hostile explicitly proves old Bag snapshot evidence remains old and structurally owned by the retained witness while the new snapshot advances independently.
- Bootstrap/late runtime materializations consume witness semantic Scan seeds, then attach storage rows separately; no historical Watch/physical-handle authority conflation is reintroduced.
- Production no longer exposes standalone logical seed advancement as an alternate publication authority.
- Next hostile target: retained-root memory/reclamation under many pinned snapshots/watch anchors, then factorized/streaming global preparation + workload-weighted overlay compaction before durable physical roots.

Carry forward unchanged product obligations: Context references/relationships, field-granular changes, DB-owned auth, Semantic Rules, final bindings/product surfaces, durable PhysicalAtoms/RealizationRoot and historical root retention.

### PASS420 retention / compaction delta

- Relation witness retention is now measurable structurally rather than inferred from `Arc` roots. Set, Bag FIFO, and pinned runtime snapshot hostile tests show path-copy growth rather than O(snapshot_count * relation_size) duplication, and weak probes prove unique oldest nodes reclaim when the anchor is dropped.
- Factorized relation compaction reuses already-certified physical row canonical evidence. It no longer recanonicalizes all materialized rows merely to establish a new dense storage-handle witness.
- Important physical law: compaction rebind follows the **current physical Scan seed order**, not semantic stable-slot order, because relation overlays use `swap_remove`.
- Remaining immediate gates before durability: whole-root/watch/history memory census, workload-weighted compaction/global preparation, maintained position-index duplication audit.

Carry forward unchanged product obligations: Context references/relationships, field-granular changes, DB-owned auth, Semantic Rules, final bindings/product surfaces, durable PhysicalAtoms/RealizationRoot and historical physical-root retention.

## PASS421 CONTINUATION UPDATE — WHOLE-ROOT RETENTION / Γ-DELTA / COMPACTION POLICY

### CLOSED THIS PASS

- Whole-root census disproved the tempting extrapolation from P420 witness sharing to complete revision memory: 33 pinned 4,096-row Set snapshots own 33 distinct logical row buffers (135,168 row slots) while their witness union is 17,920 persistent structural nodes.
- `StorageResolvedRelationDelta` is now the single transition carrier for changed-row Γ keys. Canonicalization occurs once under the exact semantic context; maintained Scan and `RelationBaseWitness` reuse it.
- Workload-weighted compaction economics is now encoded generically; fixed depth/N routing remains rejected.
- Watch ownership audit found no direct historical runtime-root pin inside `QueryWatch`; `ReadContext`/historical revision roots remain the direct whole-root holders.

### OPEN — IMMEDIATE

1. Persistent logical relation state for immutable `Revision/model` roots; eliminate full changed-relation row-buffer duplication while preserving exact Set/Bag semantics, order, history and migration laws.
2. Factorized/streaming general `RelExpr` preparation with compositional exact occurrence evidence; no full-row generic fallback.
3. Structural ownership audit of `CanonicalRowPositionIndex` after duplicate Γ computation removal.
4. Maintained Group result evidence only if derivable incrementally from existing group/aggregate authority.
5. Durable PhysicalAtoms / durable `RealizationRoot` remains gated by these semantic/retention seams.

### OPEN — DEFERRED / MANDATORY CARRY

- durable physical roots/atoms + crash/root-switch/single-file/encryption matrix;
- historical `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` reachability;
- Context reference/relationship patches, safe create/delete, final Database/Context DX;
- field-granular certified changes;
- DB-owned granular authorization;
- deterministic regex/general Semantic Rules, entity/model invariants, transaction `require`;
- migration frontend DSL/diagnostics across Rust/Python/TMD/CLI;
- final Python/.NET/Studio, backup/restore/corruption UX, performance/binary budgets, Windows secure-memory work.

### SUPERSEDED / DO NOT EXTEND

- downstream re-canonicalization of already storage-resolved relation deltas;
- fixed depth/N compaction thresholds;
- any claim that P420 witness retention numbers characterize the entire immutable runtime root.

### NEXT RECOMMENDED PASS

PASS422 = persistent logical relation authority + whole-runtime-root reclamation proof. Only after that closure resume global preparation and then durable realization roots.

## PASS422 continuation update
P422 introduced factorized persistent logical relation roots (`immutable base - persistent sparse removals + persistent inserted tail`) for runtime-derived relation endpoints. A 33-root/4,096-row hostile reduces logical authority from 135,168 naïve row slots to an upper bound of 528 delta rows + 225 persistent delta structural nodes, while preserving exact Set/Bag endpoint semantics through the existing Γ delta law.

Hostile also found the next blocker: legacy vector-like `RelationStore::get` / `SharedRelationRows::Deref` can cache a full strong compatibility projection inside published roots; 32/33 roots in the lineage acquired one during later runtime work. Therefore whole-runtime reclamation remains OPEN. P423 must remove that borrowed-vector/cache authority rather than hide it behind eviction. Global streaming/factorized RelExpr preparation and durable PhysicalAtoms remain gated behind this closure. All Context/Auth/Semantic Rules/history/bindings/product deferred lines remain mandatory carry.

## PASS423 continuation update

P423 closes P422's compatibility-projection blocker. `SharedRelationRows` no longer has an immutable borrowed-`Vec`/`Deref` surface or a strong lazy full-row cache. Persistent successor roots are read structurally through iterator/point access; contiguous materialization is explicit, transient caller-owned work only. The whole-root hostile deliberately materializes every historical root and still observes 0/32 persistent successors retaining a projection. The 33×4,096 lineage remains bounded at 528 delta-row upper bound + 225 logical persistent nodes (plus the P420 witness union of 17,920), and a weak probe proves unique oldest logical-delta nodes reclaim after dropping the snapshot.

Immediate continuation order:
1. factorized/streaming global `RelExpr` preparation without whole-row staging or generic fallback;
2. maintained `CanonicalRowPositionIndex` structural/key ownership hostile;
3. maintained Group full evidence only if derivable from existing maintained group/aggregate authority;
4. then durable PhysicalAtoms / durable `RealizationRoot` + historical root reachability.

Mandatory deferred carry remains unchanged: Context reference/relationship patches, field-granular changes, DB-owned auth, deterministic/general Semantic Rules, migration frontend DX, bindings/product surfaces, backup/recovery UX, perf/binary budgets and Windows secure-memory expansion.

## PASS424 continuation update — global preparation hostile

PASS424 tested and rejected using `MaterializedRelPlanState` as the universal streaming substrate for one-shot global migration preparation. Correctness and retracting output deltas worked, but 100k Bag Union regressed from the accepted **311.676 ms** preparation to **3199.062 ms**; disabling the target sink still cost **1907.967 ms**. The payer is construction/update of long-lived maintained Scan row/position authority, which one-shot preparation does not need.

Selected continuation law:

```text
Prepared RelExpr
+ storage-neutral RelExecutionSource over factorized/segment storage
    -> one-shot operator DAG
    -> RelExecutionSink producing physical columns/segments
       + exact physical-order occurrence evidence
```

Do not revive the rejected maintained-streaming prototype, do not route to the old full-row evaluator as fallback, and do not treat a fixed batch size as an architectural fix. Operator-specific lowerings are implementations of one execution law; unsupported mathematics fail closed.

Immediate order now:
1. PASS425 one-shot source/sink execution boundary + Bag Union and retracting query hostile;
2. expand exact operator lowerings while reusing P408–P420 Γ evidence/certificates;
3. maintained position-index ownership cleanup;
4. only then durable PhysicalAtoms/RealizationRoot + historical reachability;
5. mandatory deferred Context/Auth/Semantic Rules/migration frontend/bindings/product UX lines remain unchanged and must continue to be carried.

## PASS425 continuation — accepted one-shot relational execution

PASS425 replaces the old complete-source/global-row preparation path for its supported operator class with a storage-neutral one-shot execution law. `RelExecutionSource` reads factorized relation authority directly; `RelExecutionSink` owns target columns; target Γ authority is built columnarly. Exact lowerings now cover Scan, Union, Difference and AntiJoin. Difference retains only canonical blocker multiplicities; AntiJoin retains only canonical blocker-key support. Direct Bag Union(Scan, Scan) lowers further to column-range transfer with no transient full `Row` assembly.

There is deliberately no compatibility fallback: an unsupported operator returns `UnsupportedOneShotRelExpr`. This keeps P424's hostile result binding—`MaterializedRelPlanState` remains a long-lived Query/Watch owner, not a one-shot migration executor.

100k Bag Union release measurements for the accepted P425 path were **284.239 / 265.027 / 245.619 ms**, versus P423 **311.676 ms** and rejected P424 maintained streaming **3199.062 ms**. The architectural payer removed is more important than any fixed timing: no complete source FiniteModel, no full target row staging, no maintained Scan position authority, and no target row rebuild solely for witness creation.

Immediate continuation:
1. exact one-shot Filter/Project/Set Union/Distinct lowerings;
2. Join under existing Γ/fiber certificates;
3. Group/TopK only with inherent annotation/order state;
4. carry exact occurrence evidence to the sink where derivable to avoid duplicate output canonicalization;
5. maintained `CanonicalRowPositionIndex` structural/key ownership audit;
6. only then durable PhysicalAtoms/RealizationRoot + historical physical-root reachability.

Mandatory deferred Context/Auth/Semantic Rules/history/migration-frontend/bindings/product lines remain unchanged and must continue to appear in every successor ledger.

## PASS426 continuation delta

PASS426 broadens the P425 one-shot execution substrate through deterministic filters, Bag/Set projection, Set quotient operations, exact equi-Join, and PromoteToBag. Join is not a generic SQL fallback: one side is indexed by the existing Γ equivalence key and the other streams, so retained state is exactly the join fiber authority required by the operator.

The next immediate seam is now narrower: Set Union/Project/Distinct/Difference already calculate full canonical row keys, but final columnar witness construction calculates them again. Do not solve this with a forgeable raw-key API. Add an opaque `kernel-query`-owned row-aligned Γ evidence carrier, adopt it at the physical sink, then audit `CanonicalRowPositionIndex` ownership. Group/TopK remain fail-closed. All deferred Context/Auth/Rules/history/durability/product lines from this ledger remain mandatory carry items.

## PASS427 — sealed Γ evidence / maintained position-key ownership

P427 closes the duplicate final Γ pass for one-shot Set Union/Project/Distinct and Set/Bag Difference without exposing a raw-key trust API. `kernel-query` now owns a compiled `RelationRowCanonicalizer`; it emits opaque `CertifiedCanonicalRowKey` tokens bound by pointer identity to one exact semantic authority. A dense token stream becomes a `RelationOccurrenceCertificate`, and the physical target witness adopts the same persistent occurrence root. Mixing tokens from separately compiled authorities fails closed.

The maintained `CanonicalRowPositionIndex` hostile also found real payload duplication: the class map owned a canonical key and `by_position` owned another complete key for every row. It now shares one `Arc<CanonicalRowKey>` payload per Γ class while position entries retain only Arc references. `PersistentOrdMap` supports borrowed lookup so no temporary canonical-key clone is paid. Complexity remains ordered-map lookup + swap-remove repair.

Immediate continuation: propagate certified evidence through unchanged-row one-shot operators; close exact Group/TopK lowerings; hostile memory/perf the shared-key position index. Durable PhysicalAtoms/RealizationRoot remains gated until these execution/evidence laws are stable. All deferred Context/Auth/Semantic Rules/history/product-surface goals remain mandatory carry.

## PASS428 CONTINUATION UPDATE — one-shot execution/evidence gate closed

P428 completes exact one-shot lowering for the entire current `RelExpr` vocabulary. Group owns only Γ group lookup plus exact aggregate state; TopKWithTies owns only `K + boundary ties`; unchanged-row Filters and AntiJoin preserve sealed Γ row evidence when available. The executor match is exhaustive and `UnsupportedOneShotRelExpr` was removed, so future relational syntax cannot silently route into the old full-row evaluator.

P427 shared canonical position keys also passed a frozen-P426 release hostile: unique 100k build/churn improved from ~1199/431 ms to ~524/98 ms, and 100k rows over 1024 classes from ~1143/417 ms to ~284/87 ms. Thus the memory ownership correction did not create a performance payer.

**Immediate continuation authority:** PASS429 may enter durable `PhysicalAtoms + RealizationRoot`. Required law remains one current semantic world, same-revision extensional physical rewrites, crash-safe root publication, and reachability from current + retained historical roots. No eager logical-B reconstruction may become the permanent reopen path; no second history store or migration/query engine may appear.

Deferred product lines remain mandatory: Context reference/relationship patches and field-granular changes; DB-owned authorization; deterministic/general Semantic Rules; migration frontend DSL/diagnostics; Python/.NET/Studio and backup/recovery UX; public perf/binary budgets; native Windows secure-memory expansion.

## PASS429 continuation update — durable realization authority begins

P429 introduces the first actual durable `PhysicalAtoms + FactorizedRealizationRoot` image instead of another recipe/progress flag. The independently versioned `CFPR` payload contains only reachable atoms and a direct root topology, validates physical codec/shape without eager logical materialization, and is carried by the existing authenticated/encrypted single-file `PhysicalArtifact` section. A same-semantic-revision root switch is proven: unpublished tail bytes are not authority; after generation/root publication reopen sees the new physical dependencies under the same `RevisionId`. Stable relation-column IDs remain distinct from ordinal order; a hostile sorted-ID decode prototype was rejected before freeze.

Immediate continuation authority for PASS430:
1. move this root/image under real `DurableRevisionStore` checkpoint/recovery ownership and bind its semantic revision to authoritative durable revision;
2. preserve direct physical reopen without eager `DatabaseState` reconstruction;
3. extend root-switch through checkpoint/WAL/compaction crash matrices and encrypted path;
4. then evolve historical retention to `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` reachability with shared atoms and no second history store;
5. carry all deferred Context/Auth/Semantic Rules/migration frontend/bindings/product goals unchanged.

## PASS430 continuation update — store-owned checkpoint realization authority

P430 moves CFPR from an isolated section capability into the actual `DurableRevisionStore` checkpoint model for single-file storage. `checkpoint_realization` is explicitly a root for the checkpoint cut, never a guessed representation of the current WAL head. Synchronous rotation publishes checkpoint + metadata + physical realization in one generation; streaming publication snapshots the same CFPR at the cut and then carries later WAL frames exactly. Reopen rejects revision mismatch and restores the physical authority without eager logical reconstruction. Active single-file compaction preserves it.

The remaining architecture seam is backend/publication, not semantics: directory storage currently has no equivalent physical-generation carrier, so explicit physical-root publication there fails closed. PASS431 should add that carrier and bounded streaming CFPR I/O, then begin `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` reachability with shared atoms and no second history store. Mandatory deferred Context/Auth/Semantic Rules/migration frontend/bindings/product UX lines remain unchanged.

## PASS431 UPDATE — BACKEND-NEUTRAL DURABLE REALIZATION CARRIER

P431 closes the remaining backend/first-buffer gate from P430. Directory generations now own optional checkpoint realization through a generation-local CFPR file whose exact revision/length/CRC binding is inside manifest-checksummed metadata; publication order is CFPR fsync -> metadata fsync -> prerequisite directory sync -> manifest rename/sync. Missing/corrupt bound CFPR fails closed on reopen. Directory streaming preserves the same immutable-cut physical root + carried-WAL head split as single-file. CFPR production write and reopen are now bounded/streaming on both backends, with no whole encoded-image `Vec` on the active publication/recovery path. External freshness and directory compaction include the CFPR artifact.

Immediate next line: `HistoricalRevisionRoot -> RealizationRoot -> PhysicalAtoms` reachability over the same physical atom graph, including compaction/release. Deferred Context/Auth/Semantic Rules/migration frontend/final bindings remain mandatory and unchanged.

## PASS432 CONTINUATION UPDATE — shared current + historical durable realization roots

P432 crosses the first historical-root reachability gate. `DurableFactorizedRealization` now carries one shared immutable physical atom graph, the current root, and causal retained historical root topologies keyed by `RevisionEffectId`. CFPR v2 serializes only `deps(current) U deps(retained history)` once; roots never duplicate atom payloads. Checkpoint replacement inherits exactly the roots named by live `HistoricalEpochAnchor`s, and release removes the root plus unreachable old-only atoms transactionally.

Hostile explicitly rejects physical-atom ID remapping as a hidden second identity law: a shared `PhysicalAtomId` must denote the same immutable payload across roots or merge fails closed.

Generation-level historical closure remains a conservative compatibility authority for full `HistoricalEpochMaterial` until the historical semantic registry/context and causal replay boundary are bound to the retained physical root. **PASS433 must close that semantic/recovery binding first**, then stop pinning whole generations for root-backed historical anchors and prove compaction + release over shared atoms. Do not create another history store. All deferred Context/Auth/Semantic Rules/migration frontend/bindings/product goals remain mandatory carry.

## PASS433 CONTINUATION UPDATE — historical semantic/context authority joins shared roots

P433 upgrades retained historical roots from physical topology alone to complete historical revision authority. CFPR v3 optionally binds each causal historical root to its exact source `SemanticContext`; the shared durable `SemanticRegistry` supplies semantic module implementations. `revision_at()` first reconstructs/validates the source `Revision` from shared atoms + historical root and only falls back to legacy generation checkpoint/WAL material when the root is context-incomplete. CFPR v1/v2 remain readable and therefore stay conservatively pinned.

Whole-generation history retention is no longer the primary law for complete roots. Directory compaction can remove the old generation and single-file publication no longer emits an outgoing historical archive when candidate-root completeness already removes that generation from the pin set. Release remains causal-effect keyed and prunes the corresponding historical root/old-only atoms.

Immediate continuation: crash/streaming hostile for context-complete root capture/publication and retained-history scaling/lazy-read audit. Deferred Context/Auth/Semantic Rules/migration frontend/bindings/product goals remain mandatory carry items.

## PASS434 continuation update

P434 hostile-closes streaming publication timing for complete historical roots and measures many-root sharing: 65 live root topologies use 67 unique physical atoms versus 195 naive per-root atom slots in the fixture. A new public `kernel-realization::evaluate_relation_expr_factorized` executes the complete current `RelExpr` IR directly over `RealizationRoot + PhysicalAtoms`, proving lazy historical queries do not need a second query engine or full `DatabaseState` materialization.

Do **not** wire this into long-lived `ReadContext` by cloning `PhysicalAtomStore`: it is currently a deep `BTreeMap` clone and would create O(all atoms) memory per historical read handle. PASS435 must first establish cheap immutable/shared atom authority, then refactor `ReadContext` to one representation-neutral read-authority model. All deferred Context/Auth/Semantic Rules/migration frontend/product surface items remain mandatory carry.

## PASS435 continuation delta

P435 closes the P434 deep-clone blocker. Physical atom ownership is now persistent-map topology plus Arc payloads, with exact merge/reclamation and cheap historical read handles. Complete retained historical roots bind directly into ordinary `ReadContext`; ordinary prepared relational queries execute from factorized physical authority without constructing a full historical logical revision. Whole-map Arc COW is rejected as the versioned authority because first-write under a pinned snapshot copies O(N) topology. Carry all Context/Auth/Semantic Rules/migration frontend/final binding goals unchanged.

Immediate continuation: direct field/entity/point reads under the same `ReadAuthority`; exact intermediate historical revisions without eager full-world materialization; bulk-build hostile for large atom ingestion.

## PASS436 UPDATE — intermediate history stays factorized across exact relation effects
P436 extends the P435 representation-neutral read authority beyond revisions that are themselves retained migration-source roots. Starting from a complete retained factorized root, exact reversible relation-only history effects now advance the physical/read authority by rewriting only the touched relation through `kernel-query::RelationDelta`; untouched PhysicalAtoms remain shared and no full historical `DatabaseState` is built. Mixed model/schema effects remain on the exact logical `revision_at()` path until an equally exact factorized lowering exists. Ordinary object/ref/point reads already compile through `PreparedQuery` and therefore inherit this authority without a second historical API.

Immediate continuation: factor exact `DurableModelDelta` coordinates into the same historical realization law; after mixed relation+model history closes, return to Context reference/relationship patches, field-granular changes, authorization and Semantic Rules. P435 transient/wide-tree bulk optimization remains a deferred performance R&D line rather than a correctness blocker.

## PASS437 CONTINUATION UPDATE — factorized model delta closure
P437 adds the missing exact representation-space action of `DurableModelDelta`: carriers, fields, lifecycle entities/roots and keeps-alive edges rewrite only their direct factorized authorities, validate the target topology and preserve sharing elsewhere. `DurableRuntime::factorized_read_snapshot_at` therefore covers exact reversible `MixedRevision` history by composing relation and model effects at one target revision. Schema/semantic-change/full boundaries still fail over to their pre-existing exact authority rather than being approximated.

**Mainline direction changes here:** storage/history architecture is sufficiently closed for productization. Next return to the mandatory deferred line from the post-P400 ledger: Context reference/optional-reference patches and relationship mutation calculus -> field-granular certified changes -> DB-owned granular authorization -> deterministic/general Semantic Rules and invariants/transaction require. Keep P435 transient/wide-tree bulk optimization deferred until these product blockers close.

## PASS438
CLOSED: Context reference/optional-reference patches, relationship partial-context calculus, field-granular durable coordinates and retry-safe stale-field reapply. Durable relation mutation codec v11 carries field writes; v2-v10 remain legacy-readable. NEXT: DB-owned granular authorization; then Semantic Rules/invariants/require. Deferred persistent-tree batch R&D remains performance-only.

## POST-PASS440 CONTINUATION UPDATE

P438 closed field/reference/partial-relationship semantic coordinates. P439 established one semantic read-footprint authorization law. P440 now classifies high-level writes before physical lowering as exact object/relationship actions (`CreateObject`, `DeleteObject`, `AttachRelationship`, `DetachRelationship`, `MoveRelationship`) while raw relation mutation remains `WriteRelation`. Write-only field planning uses internal non-observable state inspection and never grants public read authority. `OwnedMany(orphan=delete)` detach additionally requires target delete authority, closing the induced-lifecycle bypass.

Immediate authorization continuation is historical undo/redo action transport plus hosted/watch refresh/revocation hostile. Do not reconstruct historical inverse as generic relation authority when exact semantic coordinates are recoverable.

New mandatory Context/DX R&D item: zero-downtime remote-reader compatibility across schema semantic migrations. The reader may have no authoritative schema code; server DB must not carry reader-specific annotations or a dedicated handshake. Current schema version/epoch may be normal metadata and client compatibility may resolve once per Context/ReadContext epoch. Same-name fields may change meaning, therefore name/existence fallback is forbidden. Authoritative schema remains current-only/legacy-free. Exact reader annotation/API syntax is intentionally unresolved; do not freeze `bind/rebind` or another spelling until the DX is designed with the user.

## PASS455 continuation update — ref-heavy live-ref line closed

P455 measured sparse/dense `LiveRefSensitivityIndex` behavior and found no hidden relation-cardinality scan in exact delta maintenance. The hostile sweep did find one P454 correctness seam: append-only Bag publication advanced logical relation rows without advancing the compact live-ref tail-coordinate authority when the appended rows had no refs. A later ref-bearing exact delta could then receive a stale ordinal. The fast path now advances coordinate history for every exact append while reverse maps remain ref-only. Dense/sparse release diagnostics keep exact one-row delta work in single-digit microseconds through 500k rows; explicit dense consumer enumeration remains output-sensitive and is not an ordinary commit payer.

Productization priority after this closure is PASS456 durable canonical transaction-requirement intent identity before external/recoverable transaction IDs become normal DX. Typed field-grant API sugar and remote-reader schema-evolution DX remain open but separate.

## PASS456/PASS457 continuation update — guarded retry identity and public idempotency DX

P456 separates the durable retry authorities cleanly: `TransactionId` is the idempotency namespace, `ClientIntentGuardDigest` is canonical passive-requirement identity, and the exact client effect remains the requested semantic change. Same key/effect/guard is retry-equivalent; changing either effect or guard under the same key conflicts.

P457 removes the hidden database/session-bound `transaction_with_id` adapter and exposes external retry identity on the existing passive transaction only: `Transaction::new().with_idempotency_key(key)` (or strict `Transaction::from(snapshot).with_idempotency_key(key)` before intent formation). Key selection does not bind adaptive intent to a HEAD. The old P446 generated-ID rotation on `require()` is removed because it became both redundant and unsafe after P456: requirement changes are now represented exclusively by the guard digest, while the transaction key remains stable for the lifetime of the intent.

Immediate continuation is PASS458: hostile the hosted/binding boundary for raw transaction-u128/retry exposure and select one transport-neutral idempotency representation without a protocol-side transaction state machine. Remote-reader schema-evolution DX and typed field-grant sugar remain separate open product lines. All deferred Context/Auth/Semantic Rules/migration frontend/final bindings/backup/perf/Windows secure-memory items remain mandatory carry.

## PASS458 continuation update — hosted idempotency authority

P458 removes raw hosted `transaction: u128` from the source API in favor of `IdempotencyKey` while retaining the identical fixed-u128 wire encoding. Current-base hosted commits now lower into the same runtime `Transaction` publication path as local Rust instead of publishing directly through `commit_plan`.

A hostile retry case exposed a correctness gap: repeating a successfully committed hosted request necessarily carries its old `base_revision`, so the previous protocol returned `StaleRevision` before durable idempotency was consulted. P458 adds a read-only durable relation-intent retry probe over the existing committed-transaction authority. Identical key/effect returns `AlreadyCommitted`; same key with changed effect returns `TransactionConflict`; an unknown stale key remains `StaleRevision` and is never reinterpreted on current HEAD. This is not a protocol retry table and cannot publish.

Immediate continuation is P459: R&D whether genuinely new stale remote relation intents can use the same kernel-change transport law as local adaptive transactions directly from exact effect + formation revision, without historical physical-Plan reconstruction, client-code replay, or generic fallback. Remote-reader schema evolution and typed field-grant DX remain separate open lines; all earlier deferred Context/Auth/Rules/migration/frontend/bindings/backup/perf/Windows items remain carried.

## PASS459 continuation update — adaptive remote exact-effect transport

P459 closes the semantic remote-staleness gap left intentionally fail-closed by P458. A hosted relation mutation is now treated as an exact-effect carrier, not reconstructed as a current-head Plan. The sufficient authority is `formation revision + idempotency key + exact inserted/removed rows`; runtime derives formation typing and asks the existing Γ-aware `certify_transition_rebase` law to prove commutation across intervening effects. Certified disjoint stale intents publish through durable residual relation-data authority; overlapping, opaque or unavailable-history cases fail closed. Retry identity remains checked first and therefore does not depend on historical materialization availability.

The hostile performance result is also explicit: formation-world validity currently rebuilds a `RelationBaseWitness` from the touched historical relation because durable history exposes the historical Revision but not its retained Γ witness. This is exact and non-fallback, but O(touched relation). PASS460 should connect the already persistent P419/P420 witness lineage to historical intent validation so the proof is O(delta log N), then audit current Set residualization for the analogous materialization payer. Remote-reader schema evolution, typed field-grant DX and all deferred Context/Auth/Rules/migration/frontend/bindings/backup/perf/Windows items remain mandatory carry.

## PASS460 continuation update — retained Γ support, no historical/full-Set scans

P460 closes both explicit P459 data-size payers without introducing a second state authority. Formation validity uses a restricted `RelationSupportWitness` projected from the current P419/P420 persistent Γ root and rewound through exact durable deltas. The restricted type cannot emit Scan evidence or physical positions, so reversing value-only history cannot accidentally masquerade as reconstruction of old stable-handle order. Semantic/schema/opaque boundaries fail closed.

Transition footprint certification now reuses the shared semantic context only after proving the intervening path contains no semantic/opaque boundary, eliminating `revision_at()` from the ordinary stale exact-effect certificate path. Stale Set residualization also becomes witness-native: touched Γ classes resolve to logical positions and exact current representatives are point-read; the whole current relation is never materialized. Bag behavior is unchanged.

Next: PASS461 history-depth hostile/perf before deciding whether any revision->support-root memo is justified. If not, return to the remote-reader schema-evolution DX line. Typed field-grant sugar and all prior deferred Context/Rules/migration/frontend/bindings/backup/perf/Windows lines remain mandatory carry.

## P479 correction — schema-epoch transaction boundary

The post-P463 guard-transport direction is superseded for transaction semantics. A transaction's
formation-world reads/requirements terminate at an exact `FormationWorldSeal` immediately before the
first crossed schema migration. Migration transports the exact effect only. B/C native history is
handled by ordinary current-epoch change/rebase laws; B does not need to mean anything in A.

Current-world conditions that must remain true at physical publication time are a separate publication
precondition/authority problem. Every successor PASS must read `PROJECT_RULES.md` first and propagate
that requirement again.

## PASS485 UPDATE — persistent relational causal capsule primitive
PASS485 establishes the exact no-fallback Γ-DTC state carrier required by PASS484. `RelCausalCapsule` shares one revision-bound `MaterializedRelPlanState` payload, checks proposed relation deltas directly against hidden maintained state, and reconstructs predecessor capsule states from exact durable forward relation effects without physical handles or query replay. Unrelated revision churn preserves the same payload Arc; relevant transitions use existing persistent/COW maintained-plan nodes. Immediate continuation is durable relational observation->capsule identity plus compact runtime relation/source-occurrence routing and reopen interning; no serialized maintained-state history may be introduced.

## PASS486 UPDATE — relational causal identity / compact routing / reopen interning
PASS486 removes the need for any durable capsule namespace. Durable authority is observation-local identity plus observed state frontier and `RelExpr`; runtime capsule refs are derived only. Current/retained indexes intern shared `RelCausalCapsule` states, relation routes retain compact observation refs, reopen reconstructs one persistent lineage per unique query by rewinding existing exact durable relation effects, and snapshot certification evaluates proposed relation deltas directly through Γ-DTC state. Immediate continuation is product observation capture and canonical history-normalization braid integration with before/after-reopen equivalence tests. All deferred Context/Auth/Rules/frontend/bindings/backup/perf/Windows lines remain mandatory carry.

## PASS487 UPDATE — product relational capture / canonical braid / restart-stable effect identity
PASS487 moves relational causal authority into explicit transaction-bound application reads without a second query replay: one maintained Γ-DTC execution yields both the application result and the shared causal capsule. A hostile public-surface regression refined the boundary: mutation lowering is not an observation, so `set`/relationship/query mutation construction uses a passive clone of the same formation snapshot while `tx.objects()` reads retain capture authority. This preserves field-granular rebase and `require` semantics without whitelists. Captured observations persist through direct/residual commits and canonical current/retained braid certification invokes exact relational anti-dependency proof. A hostile restart regression also exposed that live history used client transaction ids while reopen used durable revision-effect ids; PREPARE now returns the assigned `RevisionEffectId` and runtime history binds only with that durable identity, making verdict/witness identity restart invariant. Immediate continuation is retained-epoch end-to-end restart proof plus distinct-query/hot-relation scaling. All deferred Context/Auth/Rules/frontend/bindings/history/backup/perf/Windows lines remain mandatory carry.

## PASS488 continuation update — retained relational braid restart + output-sensitive reopen
P488 proves the schema-boundary case end-to-end: retained source-epoch Γ-DTC join observations are reconstructed after reopen and issue the same causal conflict identity as live execution. Relational capsule recovery is now a single backward history sweep with `relation -> unique lineage` influence routing. It does not pay Q times for irrelevant history and constructs effect delta maps only when at least one exact compiled lineage depends on a touched relation. The preserved complexity law is O(H + Q + I + O), with I the true semantic effect-lineage influence count; no fallback is permitted to pretend fully-hot I=H*Q work does not exist. Next R&D should determine whether shared maintained sub-DAG interning can reduce the fully-hot distinct-query case exactly; if not, carry the lower bound and continue broader relational/object capture coverage before returning to Context/Auth/Rules/frontend/bindings/history/backup/perf/Windows lines.

## PASS489 continuation update — relational multi-root state-cell architecture

P489 proves that the remaining fully-hot relational payer is reducible exactly when observation roots share canonical semantic substructure. Identical `RelExpr` subtrees at the same semantic world/revision have the same Γ-DTC maintained state and transition, so one maintained state cell may fan out to many operator occurrences/observation roots. Relation-envelope sharing is explicitly rejected by a hostile regression because distinct filters over the same relation have different future causal behavior. Query-kernel structural `Eq/Hash` is the derived runtime key; no durable DAG identity or maintained-state snapshot is introduced.

Worst-case arbitrary distinct stateful semantics remain `H*Q`; the correct target is output-sensitive unique-node work, not a claim that all hot query sets become sublinear. PASS490 should implement a multi-root `RelObservationForest` with canonical state cells and separate occurrences, prove exact equivalence to independent Γ-DTC roots, and only then replace per-lineage reopen rewind. Parameter-family fusion is deferred until the structural forest is exact. Deferred Context/Auth/Rules/frontend/bindings/history/backup/perf/Windows lines remain mandatory carry.

## PASS490 continuation update — canonical multi-root relational forest

P490 closes the representation prerequisite from P489. `RelObservationForest` interns identical semantic subtrees into one maintained Γ-DTC state-cell arena while preserving operator/root occurrences as fanout edges. Exact multi-root transitions reuse the existing maintained node kernels and are executable-equivalent to independent roots across the current relational operator vocabulary. A self-join explicitly shares one Scan cell but preserves two input occurrences. Forest reconstruction has one sweep anchor and per-root causal dependency frontiers; no forest/node identity is durable. PASS491 should switch P488 reopen from per-lineage capsules to one forest transition lineage and measure unique-node/node-transition savings before any parameter-family fusion. Deferred Context/Auth/Rules/frontend/bindings/history/backup/perf/Windows lines remain mandatory carry.

## PASS491 continuation update — forest-backed reopen + shared-root causal batch

P491 closes the runtime integration half of the P489/P490 forest line. Reopen creates one `RelObservationForest` for all unique relational observation roots, rewinds it once per relevant historical effect, and emits root capsules sharing the same forest snapshot. Current and retained causal braid checking batches shared forest capsules, so one proposed effect no longer replans the same forest once per observation root. Durable observation identity and on-disk authority are unchanged.

The next immediate hostile payer is forest construction itself: canonical cells are retained once, but parent-cell initialization still transiently builds whole independent subtrees and can duplicate O(source-data) boundary work. PASS492 should make forest construction bottom-up/local-cell exact before parameter-family fusion. All deferred Context/Auth/Rules/frontend/bindings/history/backup/perf/Windows lines remain mandatory carry.

## PASS492 continuation update — bottom-up canonical forest construction

P492 closes the remaining initial-build payer from P491. `RelObservationForest` no longer constructs a complete independent maintained subtree for every new parent cell. Canonical children are initialized first and each parent local Γ-DTC state is built once directly from the already-built child output/type using existing node-local semantic builders. Hostile coverage proves 64 `Filter(Scan)` roots produce 65 local cell initializations and one source materialization, while self-join keeps two occurrence edges but materializes its shared Scan once. Transition/reopen/braid semantics remain unchanged.

Next: PASS493 may investigate exact parameter-family fusion (equality-value dispatch and ordered cuts) as a lowering of the same canonical forest. It must not weaken root identity/frontiers, add relation-envelope heuristics, or introduce a second query/history engine. Deferred Context/Auth/Rules/frontend/bindings/history/backup/perf/Windows lines remain mandatory carry.

## PASS493 continuation update — exact equality/order parameter-family fusion

P493 adds an exact lowering inside `RelObservationForest` for sibling parameterized stateless filters. Equality constants sharing one parent/column/Γ-equivalence are indexed by compiled canonical equivalence key; ordered constants sharing one parent/column/ordering share one canonical order key and sorted cut projections across `<`, `<=`, `>`, `>=`. Roots, observation identities/frontiers and downstream occurrence edges remain distinct; no family id is durable. Work is output-sensitive and preserves the true cost of emitted root memberships. The prior PASS488 miss-heavy reopen workload now needs only its four Scan transitions instead of transitioning 64 filter cells per hot effect. Next hostile question is whether richer conjunction/range families admit one general exact normalized decision/interval structure; if not, stop specializing and return to broader product observation/Context lines. Deferred Context/Auth/Rules/frontend/bindings/history/backup/perf/Windows work remains mandatory carry.

## PASS494 continuation update — generalized predicate-family boundary

P494 closes the current parameter-family specialization line. Constant order conjunctions over one canonical semantic coordinate `(parent, column, ordering)` have an exact interval normal form obtained by intersecting canonical lower/upper cuts. Executable coverage verifies the normal form against ordinary nested `RelExpr` evaluation. The hostile multi-column case is fundamentally different: conjunctions become product regions, and rows sharing the same canonical key on either individual axis can require different root routing because another independent coordinate matters. Therefore no existing one-dimensional scalar family key can be a universal exact conjunction router.

Do not grow recovery/forest code with conjunction-specific pattern routers or host tuple/hash shortcuts. A future multidimensional acceleration, if justified, must be a first-class Γ-aware index algebra. Immediate continuation returns to broader product observation coverage: audit object/reference/relationship/deep-traversal application reads and ensure they lower into the existing exact relational causal capture while internal mutation/lowering reads remain passive. Deferred Context/Auth/Semantic Rules/frontend/bindings/history/backup/perf/Windows lines remain mandatory carry.

## PASS495 handoff — product observation breadth / selected scoped Context authority

PASS495 hostile-audited application-visible object/reference/relationship/deep-traversal reads against the P487-P494 relational causal authority. `Ref`, `Many`/`OwnedMany`, deep typed paths, projections and aggregates all lower through the ordinary `RelExpr` / `ReadContext::execute` path and therefore reuse the same Γ-DTC causal observation machinery; internal mutation/effect lowering remains passive. New public-surface regressions prove chained reference reads, deep reference-path predicates and relationship cardinality observations reject retroactive effects that would change the committed observation.

The selected scoped Context DX contract has now been copied into mainline as `docs/api/CFMD_SELECTED_SCOPED_CONTEXT_DX_RU.md`. It supersedes treating current `ContextSource::Current` as final semantics. Target model: long-lived schema-neutral `Database`; bounded `Context<M>` owns one formation world, incremental Candidate, observation journal and internal exact intent journal; ordinary public `Transaction` becomes implementation plumbing rather than a second unit-of-work owner. No auto-commit on drop, no old-schema current-world routing, no second observation model.

Immediate continuation: PASS496 should replace fresh-snapshot-per-call `ContextSource::Current` with the selected scoped Context core while preserving P487-P495 causal capture, durable intent identity, rebase/FormationWorldSeal and touched-data performance laws. Then complete reference/relationship partial-Context writes, DB-owned authorization and Semantic Rules.

## PASS496 continuation update — scoped Context formation core

P496 replaces public `Context<M>`'s fresh-snapshot-per-call semantics with one admitted formation world and one Context-owned speculative unit of work. Basic create/delete/field-patch staging, preview and commit now use an internal adaptive exact-intent journal; ordinary Context reads run on the formation world before writes and on the exact Candidate after staged effects. Successful commit seals the scope; drop does not publish. Existing transaction/Plan/query/change machinery remains the single semantic owner.

Hostile R&D found the exact remaining RYW durability gap: a relational observation after a staged effect lives in a virtual Candidate world and cannot be represented by the current durable `(committed revision, RelExpr)` certificate. P496 therefore fails commit closed after such an observation rather than inventing a fake revision or dropping causal authority. PASS497 must add ordered intent-prefix/segment authority so recovery can reconstruct candidate observation worlds exactly. Relationship mutation DX, authorization, Semantic Rules, bindings, history/backup/perf/Windows lines remain mandatory carry.

## PASS497 continuation update — scoped Candidate prefix authority

P497 removes the P496 fail-closed boundary for reads after staged writes. Relational observations in a speculative Context are durably qualified by the committed formation revision plus the exact cumulative staged relation-effect prefix. Reopen rebuilds the ordinary shared observation forest at the formation revision and advances it through that prefix using the same Γ-DTC transition machinery; maintained query state is not serialized and application callbacks are never replayed. Equal prefixes are interned by the durability codec.

Immediate continuation is PASS498: replace cumulative prefix snapshots with a persistent/interned ordered IntentJournal segment chain so many alternating writes/reads cannot create O(S²) durable prefix payload; then prove retained-schema/migration restart equivalence for prefix-qualified Candidate observations. Context relationship/reference mutations, authorization, Semantic Rules, bindings, history/backup/perf/Windows remain mandatory carry.

## PASS498 continuation update — persistent scoped intent-prefix lineage

P498 removes the cumulative-prefix O(S^2) payer introduced by P497. Candidate observation authority now uses a persistent parent-linked `DurableIntentPrefix`: each Context stage appends one exact relation delta from the previous Candidate to the next, and observations retain only the current tail. The durable codec stores unique `(parent_ref, segment)` nodes once and observations store compact tail refs; reopen reconstructs shared `Arc` lineage and advances the existing `RelObservationForest` through segments in order. No fake Candidate revision, callback replay, maintained-query serialization or second query engine is introduced.

The retained-schema proof is now prefix-qualified: an A-native self-join observation formed after a staged A relation write survives A->B migration and reopen, and the same retroactive A effect yields the same coordination conflict before and after restart. Formation semantics still terminate at `FormationWorldSeal`; prefix reconstruction belongs entirely to the retained native formation epoch.

Immediate continuation: PASS499 moves reference-field and relationship mutations (`Ref`, `Many`, `OwnedMany`) under the scoped Context-owned internal intent journal/persistent prefix lineage so ordinary DX no longer requires external `&mut Transaction`. Atomic migration-ready Context admission follows. Authorization, Semantic Rules, bindings, history/backup/perf and Windows secure-memory lines remain mandatory carry.

## PASS499 continuation update — scoped relationship/reference mutation ownership

P499 moves ordinary reference and relationship writes under the scoped Context unit-of-work owner. Required/optional Ref fields use `Context::set`; `Many` and `OwnedMany` expose sealed semantic handles consumed by Context attach/detach/move APIs. Handles are rebound to the current Candidate by semantic owner/relation identity before lowering, so stale materialized handles cannot regress read-your-writes. Internal edge discovery remains passive, and every resulting exact Plan is staged through the same internal journal/persistent intent-prefix lineage. OwnedMany exclusivity/orphan laws remain exact and can fail during Candidate formation.

Immediate continuation: PASS500 should implement atomic migration-ready Context admission, eliminating the check-then-bind race between schema/contract selection and formation binding. Authorization, Semantic Rules, bindings, history/backup/perf and Windows secure-memory remain mandatory carry.

## PASS500 continuation update — atomic scoped Context admission

P500 closes the migration-ready Context check-then-bind race with one generic `ContextAdmission`. `Database::begin_context()` clones one coherent immutable runtime root and exposes the schema revision from that same admitted world; consuming the token with `context::<M>()` binds the scoped Context to that exact formation root. A migration may publish between admission and typed branch execution without switching the admitted scope, while the next admission sees the new schema. The implementation reuses existing atomic runtime-root publication and introduces no migration router, long-held cutover lock, old-schema current-world path or client compatibility graph. `Database::context::<M>()` now delegates through the same admission primitive.

Immediate continuation should hostile-audit the remaining ordinary public `Transaction` and transitional live `SchemaDatabase<S>` ownership surfaces. The selected target remains one public scoped unit-of-work owner (`Context<M>`), with separate transaction/snapshot vocabulary retained only where semantics are genuinely distinct. Authorization, Semantic Rules, bindings, history/backup/perf and Windows secure-memory lines remain mandatory carry.

## PASS501 continuation update — one public scoped unit-of-work owner

P501 removes the transitional live typed-database model. `SchemaDatabase<S>`, its builder, `CfmdSchema::database(...)`, and `ContextSource::Current` are gone. Typed application worlds are now structurally either bounded `Context<M>` scopes or immutable `Snapshot<M>` values. The `cfmd` facade no longer exports ordinary Transaction/TransactionId/TransactionReadiness ownership. Existing low-level transaction regressions import the hidden runtime journal explicitly rather than defining product DX.

Strict historical editing is preserved without a second public transaction vocabulary: `Snapshot<M>::edit()` starts a scoped Context whose internal journal is snapshot-bound and refuses silent transport if HEAD advances. Consumer subset Contexts are intentionally not full authoritative-schema identities; binding a valid subset is legal.

Immediate continuation: PASS502 should rename/internalize remaining runtime Transaction-shaped binding plumbing and finalize authoritative Database creation ergonomics without resurrecting `SchemaDatabase<S>`. Then return to DB-owned granular authorization, Semantic Rules, bindings, history/backup/perf and Windows secure-memory.

## PASS502 continuation update — internal IntentJournal boundary + authoritative creation

P502 closes the post-P501 mutable-owner naming leak. Active runtime/application plumbing no longer uses `Transaction` / `TransactionReadiness` as the carrier vocabulary; the hidden shared carrier is `IntentJournal` / `IntentReadiness`, while `TransactionId` remains the durable retry/protocol identity. `Context<M>` is still the only normal mutable unit-of-work owner and `Snapshot<M>::edit()` remains the strict snapshot-bound form. No Candidate/rebase/Γ-DTC/durability semantics were forked or replaced.

Authoritative typed creation now has a schema-neutral builder endpoint: `Database::builder(path).create_authoritative::<S>()` (plus direct convenience) consumes `S: DatabaseDefinition` only to define the initial persisted semantic model, then returns ordinary `Database`. Raw `Schema` creation remains for dynamic/generated bindings. `Database<S>` / `SchemaDatabase<S>` remain forbidden.

Immediate continuation is DB-owned granular authorization over stable Context semantic coordinates, with hosted/session Context admission audited in the same line so restricted clients do not revive hidden journal ownership. Semantic Rules, bindings, history/backup/perf and Windows secure-memory remain mandatory carry items.

## PASS503 CONTINUATION UPDATE — scoped semantic authorization

PASS503 closes the old deferred "DB-owned granular authorization" line for the current-schema scoped Context surface. Restricted typed work now enters through `SessionDatabase::context::<S>()`, with one live `RuntimeAuthority` carried inside the exact formation world and all derived Candidate/Snapshot worlds. `Context::database()` was removed because it was an unrestricted authority escape. Historical selection requires `HistoricalRead`; `undo_latest` requires restricted history authority; preview/readiness validate exact plan permissions; publication observes live session revocation.

Authorization remains bound to persisted semantic relation/column/action coordinates. A consumer-local `bind` rename does not create a new permission identity. Relationship actions continue to use semantic attach/detach/move authorities and induced lifecycle effects still require their own object authority.

The remaining authorization line is schema migration/current-world freshness, not ordinary CRUD: prove grant transport for identity-preserving coordinates, define fail-closed semantics for split/merge/non-representable migration mappings, and keep publication-time grant/lease freshness separate from formation-world transaction guards.

## PASS504 continuation update — migration authorization law / publication generation seal

PASS504 rejects ACL/grant migration as the wrong abstraction. Grants remain current-world facts over stable semantic IDs. Migration transports only the exact authority footprint required by a transported effect, using the same verified migration dependency graph as data transport. `SchemaMigrationTransport` now has bounded row-local relation/column authority-footprint transport: split/fan-out requires every dependent target column; general relational/query slices fail closed. Real migration E2E proves a preserved `RelationColumnId` retains grant identity while a new target ID inherits nothing.

Current-world freshness is now structurally stronger: runtime publication collects one exact `PublicationAuthorityFootprint` and holds the session-state read seal through durable publication. Permission refresh/revocation takes the write side of the same authority state, so it cannot race between authorization and publish. Preview/readiness remain ordinary checks and do not hold publication authority.

Immediate continuation is PASS505: fuse transported B/C publication footprints into the retained-epoch schema-aware stale-intent walkers and then into typed Context + hosted/dynamic paths. Semantic create/delete/relationship action transport must be certified by identity/action law; do not degrade actions to generic relation writes or revive old-schema permission routing. Semantic Rules, bindings, history/backup/perf and Windows secure-memory remain mandatory carry items.

## PASS505 continuation update — schema-aware current-world publication authority

P505 fuses current-world authorization into normal stale relation-intent publication across retained schema epochs. A crossed semantic-schema boundary no longer attempts source-world publication authorization first. `kernel-plan` derives `SchemaAwareRelationAuthorityFootprint` through the same verified migration dependency graph as the effect and returns the exact current HEAD used by that proof; publication must present that `authorized_head_revision`, so B authority cannot be reused if HEAD advances to C. Grants are still never transported.

Relation writes and relation-column writes follow exact target fan-out. Semantic create/delete/relationship action authority survives only exact relation passthrough identity; a row-local rewrite is fail-closed even when relation IDs happen to match. The schema-aware relation walker now preserves transported durable action/history authorization metadata in the realized residual. Runtime E2E proves an A-formed intent after A->B is denied with only A authority and succeeds with only B authority. Idempotent `AlreadyCommitted` remains valid independently of later HEAD movement because the head certificate constrains new publication only.

Immediate continuation is PASS506: remove the remaining duplicate migration traversal by introducing one kernel-owned prepared schema-aware publication object containing current effect + exact current authority footprint + authorized HEAD + client semantic identity. Reuse it for commit/readiness/preview, extend it to field/model/lifecycle effects, and upgrade the hosted/dynamic wire path so formation semantic identity is explicit/certified rather than guessed from `base_revision`. Semantic Rules, bindings, history/backup/perf and Windows secure-memory remain mandatory carry items.

## PASS506 continuation update — one prepared schema-aware publication

P506 removes PASS505's deliberate duplicate retained-migration traversal. `PreparedSchemaAwarePublication` is now the kernel-owned relation-effect artifact that derives transported current effect and exact current authority footprint in the same verified epoch walk, binds them to one authorized HEAD and formation semantic identity, and carries the certified intervening-effect count. New publication consumes that artifact without re-walking migration; durable idempotent retry remains checked independently because it is not a new publish.

The product runtime now uses the same prepared meaning for schema-aware relation `commit`, `preview` and `intent_readiness`. Current-world permission is therefore consistent across diagnostics and publication; source-world grants cannot preview/readiness-certify a B/C publication. Unsupported formation `require`, relational-observation, field/model/lifecycle and hosted dynamic cases remain explicit fail-closed/open work rather than fallback paths.

Immediate continuation is PASS507: extend prepared publication to exact field/model/lifecycle classes and give hosted/dynamic wire commits explicit/certified formation semantic identity before routing them through the same artifact. Do not infer formation semantics from `base_revision`, add compatibility ACL/schema routing, or widen unsupported actions to generic relation authority. Semantic Rules, bindings, history/backup/perf and Windows secure-memory remain mandatory carry items.

## PASS507 continuation update — explicit hosted formation identity + prepared field publication

P507 gives hosted/dynamic relation commits an explicit transport-neutral `SemanticRevision { schema, environment }`. `base_revision` is no longer used as an implicit schema identity. The exact formation revision verifies the client-declared identity, while the stale relation effect itself enters the same kernel `PreparedSchemaAwarePublication` and current-world publication-authority path as scoped local commits. Hosted E2E proves an A-only grant is denied after A->B, a B-only grant publishes the same A-formed intent, and a false formation identity fails closed.

The prepared artifact is also widened to the exact field-only model-delta class: it can carry source/current `DurableModelDelta`, exact authorized HEAD and the sealed publication guard. The P465 field walker now exposes preparation separately from publication and publishes the prepared artifact without a second migration traversal. Carrier/lifecycle and mixed object-field effects remain fail-closed; the existing lifecycle identity calculus is not yet a proven `DurableModelDelta` transport law.

Hostile perf debt remains visible: hosted source typing currently verifies the explicit identity by reconstructing the exact formation revision through `revision_at`. This is semantically valid proof material, not schema inference, but it is not the target cost model. PASS508 should move formation-context verification/source typecheck into the kernel preparation walk itself, then unify mixed relation+field prepared publication and derive exact carrier/lifecycle delta transport where the identity calculus proves it. Semantic Rules, bindings, history/backup/perf and Windows secure-memory remain mandatory carry items.


## GitHub checkpoint sync after PASS507

Repository checkpoint synchronized the current PASS507 mainline with later GitHub-only CI repairs without importing the older PASS472 product tree: Rust workflow prerequisites (`ripgrep`, Ubuntu user-namespace enablement), Git-aware repository manifest validation, and the current Lean durability/refinement source-closure checker are retained. Workspace formatting was normalized with pinned Rust 1.98.1. Strict Clippy cleanup was in progress when the external hard wall-clock boundary was reached; no new product semantics were introduced by the checkpoint. PASS508 remains the next architecture target exactly as recorded above.
