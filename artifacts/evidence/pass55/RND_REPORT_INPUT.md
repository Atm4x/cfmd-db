# CFMD Generic-Tax R&D — four-program checkpoint

Baseline used: verified Pass52 snapshot. Pass53 was not visible in the conversation files or `/CFMD` Library at checkpoint time, so no claim of Pass53 rebase is made.

Scope: independent non-Join R&D. The goal is to find places where implementation still pays conventional generic/SQL/runtime costs despite stronger CFMD semantics, and to falsify or develop replacements.

## 1. Γ Quotient Runtime — executable slice

### Problem
Exact relation equality/delta still had quadratic semantic row matching in fallback paths. CFMD already has pinned Γ equivalences and, after the earlier structural-canonical R&D, exact canonical keys for algebraic structural values.

### Implementation
`kernel-query` now attempts an exact canonical row key per row and constructs a semantic multiset. Equality becomes multiset equality; delta becomes canonical multiplicity subtraction while preserving original source-row representatives/order for emitted removals/inserts. If any equivalence cannot produce an exact canonical key, execution falls back to the old semantic matching algorithm.

This makes canonical quotient execution an optimization, not semantic authority.

### Verification
`kernel-query`: 70 passed, 0 failed, 1 ignored diagnostic benchmark. `cargo check` and strict Clippy pass.

### Performance falsifier
4000-row diagnostic, five release processes, old pairwise semantic matching versus canonical multiset path. Ratios: 179.459x, 110.661x, 112.067x, 106.449x, 136.577x. Median ratio: **112.067x**.

This is a hostile workload for the quadratic baseline, not a universal DB speed claim.

### Status
**Candidate / continue.** Next: move uniqueness validation and remaining semantic multiset consumers onto the same quotient representation; then replace heavyweight recursive keys with revision-local interned `EqClassId` where beneficial.

## 2. Persistent Revision Runtime — COW/path-copy prototype

### Problem
`PhysicalStore::clone()` deep-cloned installed relation payloads during correctness-first prepare paths. This contradicts the intended immutable-revision/path-copy architecture and can make tiny changes proportional to database size.

### Prototype
`PhysicalStore.relations` changed from `BTreeMap<..., InstalledRelation>` to `BTreeMap<..., Arc<InstalledRelation>>`; mutation uses `Arc::make_mut` only for the touched relation. Logical/publication semantics are unchanged.

### Verification
`kernel-plan`: 129 passed, 0 failed, 2 ignored; check passes.

### Performance falsifier
One installed typed I64 relation with 300k rows; clone whole `PhysicalStore` 20 times. Five process runs:
- baseline per-clone proxy: 4.696–4.913 ms, median **4.804 ms**;
- Arc-relation prototype: 0.688–1.777 µs, median **0.788 µs**;
- median ratio ≈ **6096x** for this deliberately relation-dominated store.

The ratio is diagnostic only. It proves relation payload deep-copy is a real tax. It does **not** close whole-root persistence because index/statistics/materialization maps and `source_state` still clone structurally.

### Status
**Prototype, not integration-ready.** Correct direction is an immutable `PhysicalRoot` whose large subroots are shared/persistent; transaction prepare should path-copy affected nodes and use root identity/version witnesses rather than retain a full `source_state` clone.

## 3. Dense Identity + Graph Runtime — executable projection

### Problem
Lifecycle traversal uses full `EntityId` keys through `BTreeSet/BTreeMap` even though a pinned revision can assign a dense internal identity space. External nominal identity therefore leaks into graph hot paths.

### Prototype
`DenseLifecycleProjection` compiles one lifecycle revision into:
- deterministic `EntityId -> ordinal` map;
- ordinal -> `EntityId` vector;
- dense root ordinals;
- adjacency `Vec<Vec<usize>>`.

Reachability runs on dense ordinals and returns the same logical `EntityId` live set. Hostile test includes live cycles and an unreachable self-supporting cycle; result matches the reference semantics.

### Verification
`kernel-lifecycle`: 6 passed, 0 failed, 1 ignored diagnostic benchmark.

### Performance falsifier
50k-node chain, ten repeated reachability traversals, five release processes. Dense/reference ratios: 28.471x, 38.231x, 32.889x, 28.166x, 25.577x. Median **28.471x**.

### Status
**Prototype / strong physical direction.** Next: make `LocalId` revision-local first-class physical metadata, use bitmap extents and CSR/chunked adjacency, and investigate incremental lifecycle LFP. Dynamic edge deletion must respect SCC semantics; naive support-counting is insufficient because unreachable cycles can self-support.

## 4. Algebraic Native Layout — representation falsifier

### Problem
`NativeColumn` is scalar-only while structural CFMD values remain represented by `Value` variants (`Product` uses `BTreeMap`; `Option` uses boxed payload). This makes one of CFMD's core algebraic advantages pay generic heap/dispatch costs.

### Prototype
Standalone benchmark against the actual `kernel_model::Value` representation:
- `Option<I64>` lowered to validity vector + aligned I64 payload;
- `Product{3 x I64}` lowered to three child arrays (struct-of-arrays).

### Performance falsifier
Five release processes:
- `Option<I64>` equality scan ratios: 2.525x, 3.923x, 5.126x, 4.420x, 2.715x; median **3.923x**;
- `Product{3×I64}` field filter ratios: 41.861x, 38.936x, 48.430x, 45.599x, 43.852x; median **43.852x**.

These are representation microbenchmarks, not integrated query benchmarks. The Product result in particular demonstrates how expensive `Value::Product(BTreeMap<...>)` is as a hot physical representation.

### Status
**Prototype / continue.** Next integrated slice should be `Product` and `Option` native columns with recursive child columns, late logical materialization, and exact structural Γ equality/order kernels. Set/Bag/Map/Seq need layout contracts preserving their distinct semantics rather than one generic nested container.

## Combined conclusion
All four R&D programs produced positive evidence in the first attack:
1. Γ quotienting can eliminate quadratic semantic matching.
2. Persistent/path-copy roots can eliminate full relation payload clones.
3. Dense revision-local identity can eliminate B-tree graph tax.
4. Algebraic native columns can eliminate structural `Value` representation tax.

None of programs 2–4 should be merged wholesale from this checkpoint. Program 1's relation-multiset slice is closest to an integration candidate because it has an exact fallback and preserves authority boundaries. Programs 2–4 establish physical architecture and measurable targets for the next passes.
