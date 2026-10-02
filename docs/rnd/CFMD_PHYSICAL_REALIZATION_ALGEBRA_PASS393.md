# PASS393-R&D REPORT — CFMD Physical Realization Algebra

Start: 2026-10-01 14:08:25 UTC
Mode: hostile architecture/R&D pass; no production Rust integration in this pass.
Baseline: PASS392-R&D/PASS391 source tree.

## 0. Decision

The refined user proposal is **accepted as the strongest current migration architecture**, but not in the literal form “a chunk has schema/encoding A, then its tag is replaced by B”. The CFMD-native formulation is more general:

> **One authoritative semantic schema, independent durable physical atoms, and one certified realization program mapping those atoms to the current finite model.**

A physical atom/slot does not belong to a schema revision. It has only a physical codec/shape and immutable/COW payload. The current semantic revision owns a `RealizationProgram` which explains how the current schema/model is realized from those atoms.

For a current realization

```text
ρ_A : PhysicalAtoms P -> logical Model A
```

and verified migration

```text
M : A -> B
```

the semantic cutover is the composition

```text
ρ_B = M ∘ ρ_A
```

published atomically together with semantic context B.

The payload P may remain byte-for-byte unchanged at cutover. There is **no current semantic A after the cutover**: only B exists. A survives only as historical authority if retained.

Materialization later rewrites the *representation*, not the database semantics:

```text
(P, ρ_B)  -- representation rewrite -->  (P', ρ'_B)

with the proof obligation

ρ'_B(P') = ρ_B(P)     under Γ_B.
```

That transition does not create a database revision. It is an operational physical-root publication under the same semantic revision.

This architecture supersedes both extremes considered earlier:

1. mandatory full shadow-B/homogeneous physical cutover is too conservative as the default;
2. `MixedMigrationRevisionView` / semantic A/B relation routing is too invasive.

A shadow/preparation epoch remains useful **only as a certification/materialization technique** for migrations whose target invariants or expensive global realizations must be prepared before cutover.

The recommended name is **Physical Realization Algebra**, not “mixed schemas” and preferably not “encoding epochs”.

---

## 1. What the current repository actually stores

### 1.1 Logical authority is currently fully materialized

`kernel-revision::Revision` currently owns:

```text
SemanticContext
+ DatabaseState
```

and `Revision::build(...)` normalizes and validates a concrete full `DatabaseState`.

`kernel-durability/src/checkpoint/revision_codec.rs` serializes exactly:

```text
revision id
semantic context
full DatabaseState
```

`state_codec.rs` serializes lifecycle, carriers, every `(field, entity) -> Value`, and every relation row.

Therefore today the durable checkpoint is fundamentally **logical-state-authoritative**.

This means a superficial migration feature that leaves old physical slots in `PhysicalStore` does not actually achieve durable lazy representation: the next normal checkpoint still encodes the fully materialized logical B state.

This is the largest architectural seam exposed by the R&D.

### 1.2 Physical relation layouts already are schema-light

`kernel-plan::PhysicalStore` stores relations under:

```text
(SemanticId, LayoutId) -> InstalledRelation
```

and `NativeRelation` supports:

```text
RowStore
Columnar
I64Columnar
TypedColumnar
```

`LayoutBinding` separately identifies `LayoutId + LayoutFamily`.

This is already much closer to

```text
semantic coordinate + physical representation
```

than to

```text
schema revision + blob.
```

The user intuition was therefore directionally correct.

### 1.3 Physical layout persistence is currently reconstructible, not data authority

`kernel-durability::DurablePhysicalArtifactSpec::RelationLayout` persists only:

```text
relation
layout_id
layout kind
```

and its own documentation explicitly calls this a reconstruction recipe, not serialized physical payload. Recovery rebuilds physical layouts from the recovered authoritative `Revision`.

Therefore a real lazy physical realization architecture must promote selected physical data atoms/root metadata from “reconstructible acceleration” to **certified durable model representation**.

Indexes/statistics can remain reconstructible artifacts.

### 1.4 Runtime already supports physical publication under one unchanged revision

`RuntimeRevisionCell::install_*` publishes a new whole `RuntimeRevisionBundle` root-version while retaining the same logical `Revision`.

This is exactly the correct publication semantics for representation maintenance:

```text
same semantic revision B
old physical root -> new physical root
```

No new logical database revision is required.

This is an important positive fit: the runtime authority model does not need to be invented from scratch.

---

## 2. Hostile correction: do not attach “schema A/B” to a chunk

A whole chunk should not carry `schema_epoch = A` or `schema_epoch = B` as its semantic identity.

That is too coarse and produces false work.

Example:

```text
A.User:
    id: u64
    name: Text
    age: i64

B.User:
    id: u64
    name: Text
    age: f64
    active: bool = true
```

A physical realization can remain:

```text
p1 = id:u64
p2 = name:text
p3 = age:i64
```

while the current B realization is:

```text
B.id     <- Direct(p1)
B.name   <- Direct(p2)
B.age    <- I64ToF64(p3)
B.active <- Constant(true)
```

Nothing about p1/p2/p3 needs to say “I belong to schema A”.

Later physical materialization may create:

```text
p4 = age:f64
p5 = active:bool
```

and publish an extensionally equal B realization:

```text
B.id     <- Direct(p1)
B.name   <- Direct(p2)
B.age    <- Direct(p4)
B.active <- Direct(p5)
```

At that point p3 is unreachable from the current realization and can be GC'd if no retained historical root references it.

This is strictly more general than A/B chunk tagging.

---

## 3. Core algebra

### 3.1 Physical atoms

Introduce a physical unit whose identity is independent of schema names/revisions.

Conceptually:

```text
PhysicalAtom {
    id: PhysicalAtomId,
    codec: PhysicalCodec,
    shape/cardinality metadata,
    immutable/COW payload,
    integrity/authentication identity,
}
```

A `PhysicalCodec` is enough to decode bytes/columns (`i64`, `f64-bits`, text, entity-id, structural container, etc.) but does not assert current semantic field meaning.

Do not silently reinterpret raw bytes. Every semantic change of interpretation must pass through an explicit certified realization expression.

### 3.2 Realization program

A revision owns a certified mapping:

```text
RealizationProgram_B : PhysicalAtoms -> logical finite Model B
```

This should be a DAG, not one version tag.

Its leaves are physical atoms/constants. Internal nodes are exact deterministic expressions already close to CFMD's math:

```text
Direct(atom)
Constant(value)
ExactValueExpr(...)
RelationExpr(...)
Product/projection/composition
SemanticDeltaOverlay(...)
```

The exact concrete IR needs a later implementation pass; the architectural requirement is that it be serializable, deterministic, dependency-explicit and kernel-verified.

### 3.3 Migration is composition

The current P388 `SchemaMigrationProgram` remains the correct semantic migration object:

```text
M : A -> B
```

If current physical realization is `ρ_A`, compile the target realization by composition:

```text
ρ_B := normalize(M ∘ ρ_A)
```

This is the key improvement over retaining an A semantic read path after migration.

After cutover, B reads do **not** execute:

```text
read A according to schema A
then convert A -> B.
```

Instead they execute one current B realization program whose leaves happen to include older physical atoms.

Old schema A therefore does not leak into steady runtime semantics.

### 3.4 Materialization is representation normalization

For target coordinate q:

```text
q <- expression E(P)
```

the materializer may create a new atom:

```text
p_new := E(P)
```

and then publish:

```text
q <- Direct(p_new)
```

with a certificate of Γ_B-extensional equality.

The semantic revision does not change.

### 3.5 Compaction/GC is graph reachability

A physical atom is live iff referenced by at least one of:

1. current realization root;
2. retained historical realization root;
3. required durability/recovery authority;
4. live derived artifact which cannot yet be reconstructed without it.

Migration progress therefore needs no bitmap such as `field X migrated = true`.

The realization DAG itself is the progress state.

When the last current/historical dependency disappears, GC may reclaim the atom.

---

## 4. The neural-encoder analogy: useful but must be tightened

The user analogy “the same data under different encoders means different things” is useful if interpreted as:

```text
physical carrier P
+ certified decoder/realization ρ
= semantic model
```

It is unsafe if interpreted as “change a tag and pretend the same bytes have a different type”.

CFMD requires:

```text
new meaning = explicit deterministic transform / semantic morphism
```

and target validity must be certified before semantic publication.

The current semantic registry/Γ model is exactly the reason this distinction matters.

---

## 5. Rename, add, drop, split, merge under the realization model

### 5.1 Rename

If semantic identity is retained and only presentation spelling changes:

```text
B.clinical_note <- Direct(existing_atom)
```

Physical rewrite is zero.

If the migration deliberately creates a new semantic coordinate, B may still map that coordinate directly to the same physical atom through a certified identity transform.

### 5.2 Add/default

A new field need not allocate a column immediately:

```text
B.active <- Constant(true)
```

A later materialization may allocate a bool atom/column.

### 5.3 Drop

The dropped B coordinate simply has no realization output.

The old atom may remain physically present until current+history dependency GC proves it dead.

### 5.4 Type conversion

```text
B.age <- I64ToF64(old_age_atom)
```

No immediate rewrite is required.

### 5.5 Split

```text
B.first_name <- First(old_name_atom)
B.last_name  <- Last(old_name_atom)
```

Both coordinates may share one old source atom until materialized.

### 5.6 Merge

```text
B.full_name <- Join(first_atom, last_atom)
```

Again, physical inputs remain independent from schema versioning.

---

## 6. Hostile finding: relation columns are still too positional

The current schema/query stack has a mismatch with the desired architecture.

`RelationDef` currently stores:

```rust
columns: Vec<TypeExpr>
```

and migration rows store:

```rust
MigrationColumnRewrite {
    source_columns: Vec<usize>,
    target_column: usize,
    ...
}
```

This is precisely the positional coupling that previous Context work already identified as dangerous at the product layer.

Recommended kernel refactor before physical-realization integration:

```text
RelationColumnDef {
    id: SemanticId / RelationColumnId,
    type: TypeExpr,
    ...
}
```

A relation has stable semantic column coordinates. Query preparation may freely lower them to dense ordinals (`usize`) for hot execution.

Thus:

```text
semantic IR / migration / realization -> stable column identities
prepared/execution IR                 -> dense ordinals
physical layout                       -> PhysicalSlotId / atom coordinates
```

This would also eliminate `migration_column_input_id(column_index)` as a fundamental identity hack.

The current heavy use of `usize` throughout `kernel-query` is acceptable **after preparation**; it should not be the persistent semantic identity used by migrations/encoding lineage.

---

## 7. Hostile finding: physical realization must cover the whole finite model

Today `PhysicalStore` is relation-centric.

`FiniteModel` is:

```text
carriers: SemanticId -> Set<EntityId>
fields:   (FieldId, EntityId) -> Value
relations
```

The public object facade lowers most object payload into entity relations, but kernel lifecycle and mirrored live-reference fields still use carriers/fields directly.

If physical realization applies only to relations, migration remains split into two engines:

1. relation encoding lineage;
2. special eager carrier/field migration.

That would be architectural regression.

Recommended target is a **Physical Model Realization**, not merely relation realization.

Possible implementation direction:

- keep the logical `FiniteModel` API;
- define physical atoms/views for carrier sets, field maps and relations;
- or normalize these into a common internal physical relation substrate while retaining their distinct logical semantics.

The exact data structure is an implementation question. The invariant is not: all logical coordinates must be realizable through one physical-root calculus.

---

## 8. Writes: do not invert migration transforms

A critical hostile case is a non-invertible realization:

```text
B.flag <- (A.number > 0)
```

A B-write to `flag` cannot be pushed back uniquely into the old integer atom.

Therefore the system must never require a general inverse encoding.

CFMD already has the right mathematical ingredients.

### 8.1 Writable realization/lens when available

`kernel-lens` already models get/put laws and lifting rewrites through a lens.

If a realization coordinate has a certified writable lens, a semantic B rewrite may be lowered directly into the underlying atom while preserving rewrite intent/laws.

### 8.2 B-native delta overlay when no inverse exists

For non-invertible realization, create target-semantic physical authority instead of inventing an inverse:

```text
base = derived B view from old physical atoms
overlay = exact B semantic delta/new B-native slot
current B = apply(base, overlay)
```

The overlay is later folded into native atoms by materialization/compaction.

This reuses `kernel-change`/exact relation delta semantics rather than introducing a migration-specific write engine.

The physical lowering law becomes:

```text
ρ'(P') = apply_B(ρ(P), δ_B)
```

with the same stable intent/effect identity principles already used by normal CFMD changes.

---

## 9. Rich semantic-core reuse

The idea fits the mathematical core better if developed as general realization theory.

### 9.1 `SchemaMigrationProgram` — keep strongly

P388's compact deterministic program is exactly `M : A -> B` and is required for realization composition.

### 9.2 P389 forward slices — keep, but relocate

`MigrationRelationSlice`, row-local transforms and exact source dependency sets are useful as a **compiler/materializer analysis**.

They should not define current Revision semantics.

Long-term they should likely become generic realization dependency/output slices after `M ∘ ρ_A` composition.

### 9.3 Γ-canonical `RelationBaseWitness`

`RelationBaseWitness` already proves exact Γ-canonical relation support and advances incrementally with exact deltas.

It is close to the witness needed to prove that two physical realizations denote the same logical relation.

It should be used/extended for relation realization certificates instead of comparing raw bytes/layouts.

### 9.4 Revision observables / certified semantic morphisms

`RevisionObservableCatalog`, `CertifiedSemanticMorphism`, determinant closure and Anchor Pullback Normal Form already describe semantic dependence/reconstruction among observables.

They are not currently a physical-atom calculus, so this pass does **not** claim they can be reused unchanged.

However, the natural R&D extension is:

```text
physical grounded atoms
   -> revision observables
   -> determinant/anchor closure
```

A materialized derived coordinate becomes a new grounded physical atom. APNF/determinant analysis may then prove which old atom basis is redundant.

This is much more CFMD-native than a hand-written “migration progress table”.

### 9.5 Exact differential/query machinery

Global realization expressions can compile from existing `ExactQuery`/`RelExpr` and their exact differential programs. This matters for optional background/on-access materialization and keeping prepared global structures current.

The same no-fallback law should hold: if a production realization requires an exact capability not yet supported, fail certification explicitly rather than silently invoking a semantically different generic path.

---

## 10. Global transforms and the remaining role of Shadow Epoch preparation

The physical-realization architecture does **not** imply that every possible transform should be executed lazily on every read.

For example:

```text
B.customer_total = SUM(A.orders.amount)
```

is a valid realization expression, but repeatedly evaluating the entire aggregate on access may be unacceptable.

Similarly, target B may introduce a global invariant such as uniqueness which must be known valid before B becomes authoritative.

The common architecture remains one:

```text
TransformPlan M
+ current realization ρ_A
-> candidate realization ρ_B = M ∘ ρ_A
-> certify B
-> atomic cutover
```

Preparation may pre-materialize selected expensive/required realization nodes.

If A remains writable during long preparation, the previous PASS392 exact-effect-transport idea is still useful:

```text
A_i --δ--> A_{i+1}
 |           |
 M           M
 v           v
prepared B_i --M*δ--> prepared B_{i+1}
```

But the prepared target is not required to be a complete duplicate B database. It may be only the target witnesses/atoms that correctness or service policy requires before cutover.

Thus Shadow Epoch is demoted from **default storage architecture** to **optional online preparation mechanism**.

---

## 11. Cutover law

Before cutover, current world is A.

Kernel compiles:

```text
ρ_B = normalize(M ∘ ρ_A)
```

and proves/certifies:

1. the realization is total for every target semantic coordinate;
2. required physical codecs/modules exist;
3. logical B satisfies target typing/lifecycle/rules/invariants;
4. every pre-cutover mandatory service capability is available;
5. realization dependencies are durable/pinned.

Then one Database-owned publication atomically swaps:

```text
SemanticContext A -> B
RealizationRoot ρ_A -> ρ_B
SemanticChangeEvent M
```

The physical atom set may be unchanged.

After the root swap there is only B at the semantic/query/change/auth/watch layers.

There is no `if epoch == A` branch in those layers.

---

## 12. Read path law

Ordinary query code is compiled against schema B.

Physical planning resolves required B semantic coordinates through `ρ_B`.

Example:

```text
B.age
   -> I64ToF64(atom p3)
```

The execution planner may inline/fuse this expression with the query plan. It does not reopen schema A.

A read should **not semantically require a write**.

An on-access policy may schedule opportunistic physical materialization after/alongside the read, but the read result itself is produced from the certified realization.

This avoids turning all reads into unpredictable durability operations.

---

## 13. Materialization scheduler law

There should be one kernel operation/concept:

```text
materialize(realization_coordinate or subgraph)
```

Possible initiators:

- explicit maintenance;
- compaction;
- hot-read policy;
- write path;
- optional background worker;
- preparation before another migration.

These are scheduling policies, not semantic variants.

No background thread is required for correctness.

---

## 14. Multiple migrations: never stack schema interpreters

A major hostile risk of lazy migration systems is a chain:

```text
physical A -> B transform -> C transform -> D transform
```

on every read.

Physical Realization Algebra avoids this by composition/normalization.

If current realization is:

```text
ρ_B = M_AB ∘ ρ_A
```

and migration `M_BC` occurs, publish:

```text
ρ_C = normalize(M_BC ∘ ρ_B)
```

not a runtime “read B then migrate B to C” chain.

The optimizer/materializer may fuse scalar transforms, eliminate dead coordinates, share common subexpressions and choose strategic native atoms.

The current schema remains C only.

---

## 15. Durability redesign required for the full benefit

This is the largest required kernel change.

### Current

```text
checkpoint = full logical Revision/DatabaseState
physical layouts = reconstructible recipes
```

### Target

Conceptually:

```text
DurableRevisionRoot {
    revision id,
    SemanticContext,
    PhysicalModelRoot / atom root,
    RealizationProgram root,
    semantic/validation certificates or reconstructible witness roots,
    history/causal authority,
}
```

Physical data atoms become authenticated durable sections/segments.

The existing single-file section/generation machinery is a plausible owner for those atoms: it already supports immutable authenticated sections, root publication and historical epoch closure.

Derived indexes/statistics remain reconstructible recipes unless deliberately promoted.

Recovery should open B by validating root+program+atoms, **without materializing an O(data) B `DatabaseState` merely to become readable**.

Until this refactor occurs, lazy physical realization remains only a runtime optimization and does not deliver its full durable benefit.

---

## 16. Revision/kernel-model refactor

The current `Revision` structurally equates “logical finite model” with “fully materialized `DatabaseState` in memory”.

For a serious physical-realization architecture this should be reconsidered before release.

Recommended direction:

```text
CertifiedRevisionRoot
    = RevisionId
    + SemanticContext
    + certified extensional Model authority
```

where one implementation can be a fully materialized `DatabaseState` (excellent reference/oracle/test representation) and production can be a certified physical realization.

Do **not** repeat the P390 mistake of introducing a weak “semantic head” which no longer owns enough data authority. The production root must still certify one complete finite model; the model simply need not be eagerly materialized as Rust maps/vectors.

Validation/query/change APIs should gradually consume exact model/relation access interfaces or certified iterators rather than require the concrete `DatabaseState` container everywhere.

This is a significant refactor, but the repository is unreleased and this is the correct time if large-data productization is a goal.

---

## 17. Current constructs: keep / supersede

### Keep strongly

- authoritative Entity is legacy-free;
- consumer `bind` is local naming only;
- `MigrationModel` / `SchemaMigrationProgram` deterministic serialized transform;
- P388 removal of O(data) target Revision from migration WAL;
- P381-P386 historical semantic boundary/materializability/retention work;
- P389 dependency analysis and row-local streaming primitives, relocated under realization compilation/materialization;
- P392 exact effect transport as optional online preparation machinery.

### Keep conceptually but generalize

- physical root publication under same semantic Revision;
- relation layout identities;
- Γ `RelationBaseWitness`;
- semantic observables/APNF/determinant closure;
- writable lens laws;
- exact semantic delta overlays.

### Supersede/remove from the intended mainline

- `MixedMigrationRevisionView`;
- `MixedMigrationPhysicalAuthority`;
- migration-specific current-world A/B routing;
- `MixedMigrationCutoverCertificate` in its P391 role;
- schema-version tag as the primary identity of a physical chunk;
- `WalForwardCutover` vs `NativeCheckpoint` as a notion of whether current logical B is “physically done” — representation normalization can remain partial indefinitely;
- positional `MigrationColumnRewrite` as long-term migration identity.

`RevisionSemanticHead` should be re-evaluated. A small immutable header may remain useful, but it must not become a substitute for complete model authority.

---

## 18. DX fit

This architecture preserves the earlier DX law: description is not authority, and `Database` owns publication.

The frontend remains simple:

```text
Rust/Python/CLI sugar
      -> TransformPlan / MigrationModel
      -> runtime
      -> verified kernel migration M
```

No physical slot/atom/realization detail leaks to ordinary developers.

The old P354 authority rule remains correct: passive plans describe, Database publishes. The P355 principle also remains correct: exact semantic changes/laws from the math core should be transported rather than replaced by application-layer conflict/retry tables.

---

## 19. Hostile failure modes and answers

### “Just change encoding A tag to B”

Rejected. A tag alone cannot prove bytes have B meaning. Publish a certified realization program.

### In-place append of new columns

Logically acceptable, physically risky. Prefer immutable/COW atoms plus new root publication. A logical chunk may refer to both old and newly created column atoms.

### Read causes mandatory write

Rejected. Reads produce B via the current realization. Materialization is optional maintenance.

### Non-invertible transform then B write

Do not invert. Use writable lens only when certified; otherwise publish B-native exact delta/overlay/new atom.

### Global transform repeatedly recomputed

Correct but potentially expensive. Cost/capability planning may require/prefer pre-materialization. This is policy over one realization algebra, not a second migration semantics.

### Target invariant cannot be certified lazily

Preparation barrier. Optional shadow + exact effect transport keeps required target witnesses/materializations current until atomic cutover.

### Multiple migrations accumulate interpreter chains

Compose and normalize realization programs at each cutover.

### Old atoms deleted too early

GC uses dependency reachability over current realization + retained historical realization roots + recovery authorities.

### Historical `db.at(A_revision)` accidentally sees B

Historical revision owns/pins its A semantic context and historical realization root. Current B root may share physical atoms with it, but the realizations are distinct authorities.

---

## 20. Recommended implementation sequence

### PASS394 — semantic-column identity cleanup

Before new durable storage architecture:

1. introduce stable relation-column semantic IDs;
2. compile prepared/hot query IR to ordinals;
3. convert migration column rewrites to semantic coordinates;
4. prove no public/runtime migration identity depends on field ordinal.

This is useful even if the broader realization architecture changes later.

### PASS395 — Physical Realization core (in-memory/reference)

Create minimal generic concepts:

```text
PhysicalAtomId
PhysicalCodec
RealizationExpr / RealizationProgram
RealizationDependencyGraph
RealizationCertificate
```

Initially wrap existing `PhysicalStore` data and prove direct/constant/scalar-transform cases against materialized `DatabaseState` as the oracle.

Do not touch durability yet.

### PASS396 — migration composition

Compile:

```text
ρ_B = normalize(M ∘ ρ_A)
```

for rename/default/drop/type transform/split/merge.

Delete/supersede mixed current-world migration view.

### PASS397 — representation rewrites/materialization

Prove:

```text
ρ'(P') = ρ(P)
```

for materializing derived coordinates into native atoms.

Connect same-revision runtime physical-root publication.

### PASS398 — write lowering

Use writable lens where available and exact B-native delta overlays otherwise.

Prove:

```text
ρ'(P') = apply(ρ(P), δ_B)
```

while preserving client intent/effect identity.

### PASS399+ — durability root refactor

Only after algebra is stable:

- durable physical atoms;
- durable realization root;
- recovery without eager full `DatabaseState` reconstruction;
- historical root pinning/GC;
- single-file authenticated sections;
- crash matrix.

### Later — APNF/determinant R&D

Investigate whether physical atom bases can become grounded observables so existing determinant/anchor closure certifies minimal retained physical bases and compaction redundancy.

---

## 21. Final verdict

The user's refined proposal survives hostile review and becomes **stronger after being generalized**.

The best current architecture is not:

```text
chunk encoded under schema A
-> change chunk schema tag to B
```

and not:

```text
live Revision contains a mixture of A and B semantic relations.
```

It is:

```text
ONE current semantic world B

independent physical atoms P
        +
certified current realization ρ_B : P -> B
```

Migration is algebraic composition of semantic transform with the old realization. Materialization is an extensional representation rewrite under the same revision. Writes are semantic deltas lowered through a writable lens or B-native overlay. GC is dependency reachability. Historical revisions retain their own realization roots. Optional shadow/effect transport is only a preparation mechanism when target certification needs it.

This direction aligns unusually well with CFMD's existing separation of semantics Γ, exact rewrite laws, persistent/COW physical roots, relation-base witnesses, observables/morphisms and whole-root physical publication.

The largest mismatch is also clear and concrete: durability/revision ownership currently treats a fully materialized `DatabaseState` as the model authority and treats physical layouts as reconstructible acceleration. To realize this architecture fully, that boundary must be refactored before release rather than patched around with migration-specific mixed-state objects.

---

## 22. Late hostile check — validation does not invalidate the architecture

`kernel-validation` currently consumes concrete `DatabaseState`, so the first implementation cannot simply delete materialized logical-state access.

However, `RuntimeViolationState` is explicitly documented as a **reconstructible exact Γ-VMF state** whose authority is the logical revision, with the intent that full recomputation may later be replaced by Γ-DTC maintenance without changing publication semantics.

That is a strong fit, not a contradiction.

Recommended evolution:

1. validation functions accept semantic model iterators/views rather than concrete BTreeMap/Vec containers where possible;
2. build the exact violation measure from the new realization at migration certification;
3. retain the normal publication law `V = 0`;
4. maintain VMF incrementally from exact B deltas after cutover;
5. materialization-only physical rewrites must prove extensional equality and therefore leave VMF unchanged.

This means physical representation rewrites do not need to re-run global semantic validation. They need a representation-equivalence certificate. Semantic writes still go through ordinary validation/change closure.

The existing separation

```text
Revision semantic authority
RuntimeViolationState reconstructible exact measure
```

should be generalized to

```text
CertifiedRevisionRoot semantic authority
RealizationRoot physical model authority
RuntimeViolationState exact semantic validity witness
```

without allowing physical layout to become an alternate source of semantics.

---

## 23. Revised verdict after all hostile checks

No inspected kernel produced a counterexample forcing physical data to be schema-version-owned.

Instead the repository shows four unusually favorable foundations:

1. `PhysicalStore` already separates `SemanticId` from `LayoutId`;
2. whole-root physical publication already exists under an unchanged semantic Revision;
3. migration is already a deterministic serializable transport program;
4. Γ witnesses, exact rewrites, VMF and lens laws already provide most of the proof vocabulary needed for extensional representation changes.

The two real blockers are structural and should be treated as deliberate pre-release refactors:

1. durable checkpoints still own a fully materialized `DatabaseState` while physical layouts are reconstructible;
2. relation columns/migration column rewrites still use positional identity too deeply at the semantic migration boundary.

Therefore the architecture is not rejected. It is promoted from a migration optimization to the recommended long-term storage abstraction, with the warning that implementing it halfway would be worse than the current system.

---

## 24. Late hostile check — history/GC naturally becomes root reachability

P381-P386 currently retain historical materializability with `HistoricalEpochAnchor { generation }`; compaction therefore pins whole durable generations. That was the correct conservative authority under the existing checkpoint model.

Physical Realization Algebra allows a cleaner eventual generalization:

```text
HistoricalRevisionRoot
    -> historical RealizationRoot
    -> exact PhysicalAtomId dependency closure
```

Current and historical revisions may share immutable atoms while owning different semantic contexts/realization programs.

For example, the same old `age:i64` atom can simultaneously serve:

```text
historical A:  A.age <- Direct(p_age_i64)
current B:     B.age <- I64ToF64(p_age_i64)
```

This is safe because authority is in the two distinct realization roots, not in the atom itself.

Future GC can therefore move from generation-level pinning to atom/root reachability without creating a second retention engine. Generation-level historical anchors can remain a conservative backend representation until the finer durable atom root exists.

The P387 `WalForwardCutover` / `NativeCheckpoint` dichotomy becomes unnecessary as a semantic migration-progress concept once checkpoints can persist `(B, ρ_B, atoms)` directly: B may be fully authoritative while ρ_B still references old-origin atoms forever. “Fully native B” is then an optimization state, not a correctness state.
