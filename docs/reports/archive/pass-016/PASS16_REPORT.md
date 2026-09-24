# CFMD Pass 16 — normative ideal-DB specification + persisted/delta-maintained I64 index

Date: 2026-09-19
Baseline: Pass15 verified (189 declared tests)
Final: Pass16 verified (193 declared tests)
Implementation wall-clock target: 20 minutes
Code freeze: before the 20-minute boundary
Final frozen-state release/build/rustdoc/overflow verification: completed at approximately 20m11s; no source changes after the boundary

## 1. New normative handoff: `CFMD_IDEAL_DB_SPEC.md`

Problem -> the research journal contains the final database design, but it is mixed with rejected candidates, hostile attacks, literature notes and historical intermediate architectures. That makes it unsuitable as a compact source of truth for another implementation/research agent.

Implementation -> added `CFMD_IDEAL_DB_SPEC.md` as a normative snapshot of the current target architecture. It extracts the late consolidated journal result and overlays verified implementation findings through Pass16.

The document explicitly separates:

- normative mathematical semantics;
- verified executable implementation;
- partial/reference implementation;
- still-open engine/metatheory work;
- rejected claims/architectures;
- physical corrections learned from benchmarks.

The core target remains:

```text
Revision = (Schema S, SemanticEnv Γ, finite Model M)
```

with algebraic structural values, nominal identities, typed relations/maps, total declarative query/rewrite/lens/fixed-point semantics, universal exact change fallback, explicit lifecycle reachability, versioned semantic environment, proof-carrying lowering and reconstructible physical artifacts.

The spec also contains a standalone “advantages versus ordinary database families” block and a non-claims section, so it can be handed to another agent without the 13k+ line journal.

Theory change in Pass16 -> **none**. The implementation changed the status mapping and refined one physical requirement; no new logical primitive was introduced.

## 2. Persisted I64 Join index

Problem -> Pass15 removed the O(n^2) Join path for direct typed I64 scans, but rebuilt the right-side `BTreeMap` for every query. This was an indexed algorithm, not maintained physical state.

Implementation:

- added `I64IndexBinding { relation, layout, key_column, equivalence }` as a first-class physical index identity;
- `PhysicalStore` can own `MaterializedI64IndexState` objects;
- `install_i64_index` validates the pinned schema, I64 physical key and exact semantic equality before admitting the index;
- `execute_native`/indexed Join probes the materialized index first;
- `ExecutionStats` distinguishes `persisted_index_hits` from `ephemeral_index_builds`;
- replacing a physical relation/layout invalidates every persisted I64 index bound to that relation/layout, preventing silent stale reuse.

Result -> a persisted index hit executes without an ephemeral rebuild.

## 3. RelationDelta maintenance and Bag multiplicity

`MaterializedI64IndexState::apply_delta` consumes the same semantic `RelationDelta` used by relational IVM.

For removals it deletes exactly one semantic-equivalent row from the appropriate key bucket; for insertions it appends one row. Therefore Bag multiplicity is preserved rather than accidentally converting the index into Set semantics.

Falsification:

- sequential insert/remove transitions, including duplicate removal, are compared after every step with a fresh rebuild oracle;
- maintained and rebuilt bucket states agree at every step.

## 4. Atomic physical relation + index update

A correctness hole remained if callers could update an index but leave the underlying physical relation at the old revision.

Pass16 adds `PhysicalStore::apply_relation_delta`:

1. clone the installed physical relation;
2. apply the semantic RelationDelta to the clone using the relation's pinned semantic equivalences;
3. clone every persisted I64 index on the exact relation/layout and apply the same delta;
4. if any step fails, publish nothing;
5. only after all checks succeed, replace relation and indexes together.

A vertical self-Join test applies a delta to relation+index, executes the persisted-index physical Join, and compares the result with a freshly recomputed logical model. It also checks that the persisted index was hit and no ephemeral index was rebuilt.

This closes the most dangerous consistency gap in the first persisted-index implementation.

## 5. Performance falsifier

Workload: 20,000-row unique-key typed I64 self-Join.

Raw median evidence from `PASS16_PERSISTED_INDEX_BENCH.txt`:

```text
ephemeral index build each query : 7,600,191 ns
persisted index reuse            : 4,592,682 ns
hand-written prebuilt BTreeMap   : 3,478,438 ns
persisted / ephemeral            : 0.604x
persisted / hand-written prebuilt: 1.320x
```

Interpretation:

- persistence is materially useful: the maintained path uses about 60% of the old per-query indexed runtime;
- however the persisted CFMD index is still about 32% slower than a hand-written already-built map on this workload;
- the leading representation difference is now concrete: the materialized index stores logical `Vec<Value>` payload rows, while the specialist baseline retains compact/raw row locations and reads typed data directly.

This is recorded as a physical-spec correction, not hidden as benchmark noise. The ideal persisted index should store typed payloads or stable physical row/slot handles and materialize logical Values only at the semantic boundary.

## 6. Specification change caused by practice

The mathematical kernel did not change.

The physical target was sharpened:

```text
persisted derived index
  = revision-derived reconstructible artifact
  + first-class binding
  + semantic-delta maintenance
  + atomic consistency with physical source
  + typed/stable physical payload
```

A persisted `Vec<Value>` cache is acceptable as a reference correctness implementation but is no longer considered the ideal final representation.

## 7. Final verification gate

Rust 1.98.1 standalone toolchain.

- 19 crates;
- 23 Rust source files;
- 19,805 Rust LOC;
- **193 declared tests**;
- `cargo fmt --all -- --check` PASS;
- `cargo test --workspace` PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` PASS;
- `cargo test --workspace --release` PASS;
- `cargo build --workspace --release` PASS;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` PASS;
- release `kernel-plan + kernel-integration` with forced overflow checks PASS;
- 0 external registry/git Cargo sources;
- 0 `unsafe`;
- 0 TODO/FIXME;
- 0 `panic!` / `todo!` / `unimplemented!` macros.

The first combined release command hit the container timeout after the release tests had completed; the remaining build/rustdoc/overflow gates were immediately rerun separately on the same frozen source and passed. This is an infrastructure timeout, not a test failure.

## 8. Remaining problems, ordered

1. **Persisted-index payload representation.** Replace `Vec<Value>` payload rows with typed payloads or stable physical row/slot handles; current prebuilt-index gap is ~1.32x on the diagnostic workload.
2. **General typed batch pipeline.** Strong fusion remains narrow; downstream operators should consume typed batches without `Vec<Value>` boundaries.
3. **Persisted indexes for other semantic key classes.** Bool, F64Bitwise, Text/collation and nominal IDs, plus cost/admission policy.
4. **Batch-to-batch Join output.** Current Join eventually materializes logical rows even when a downstream physical operator could stay typed.
5. **Stateful TopK.** Maintained order-statistics with ties and versioned ordering semantics.
6. **Stateful Group.** Maintained Count and deletion-capable exact/reproducible F64 aggregation/provenance.
7. **Long-lived derivative state for Join/TopK/Group** integrated with their physical structures.
8. **Other layout runtimes:** KeyValue, Adjacency/CSR, DenseArray, Inverted, Custom.
9. **OrderedView/pagination** as explicit semantic/runtime type.
10. **Durability:** WAL/recovery, durable materializations/indexes, compaction and crash tests.
11. **Concurrency/distribution:** observation-repair scheduler, replication/consensus and merge coherence.
12. **Formal closure:** surface elaboration theorem, full Dq proof, merge cube, retention IFC and proof-checker mechanization.

Immediate next implementation frontier: typed/stable persisted index payload + broader maintained/batch state. The central logical database ontology is not currently missing a primitive.
