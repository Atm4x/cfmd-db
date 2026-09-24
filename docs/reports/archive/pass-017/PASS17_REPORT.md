# CFMD Pass 17 — stable physical row handles for persisted Join indexes

Date: 2026-09-19
Baseline: Pass16 verified (193 declared tests)
Final: Pass17 verified (194 declared tests)
Wall-clock target: 20 minutes
Start: 13:34:17 UTC
Code/audit wall-clock boundary: 13:54:21 UTC (20m04s). Code was frozen before the boundary; the final interval contained verification only.

## 1. Problem

Pass16 made the I64 Join index persistent and delta-maintained, but its buckets stored cloned logical `Vec<Value>` rows. This was semantically correct but duplicated relation payload and measured about 1.32x a hand-written prebuilt `BTreeMap` baseline. It also meant the index was drifting toward a second mini-store rather than a reconstructible physical artifact.

## 2. Stable row-handle representation

Implemented `PhysicalRowId` plus `InstalledRelation` ownership inside `PhysicalStore`.

A persisted `MaterializedI64IndexState` now stores:

`BTreeMap<i64, Vec<PhysicalRowId>>`

rather than logical payload rows. The physical relation remains the sole payload owner. At probe time the row handle resolves to the current physical position and values are materialized only at the query/output boundary.

The installed relation maintains:
- stable monotonically allocated row handles;
- current dense row order;
- a row-handle -> current-position slot map;
- a fast identity-position mode before the first position-shifting deletion.

This permits physical positions to change without changing index identity.

## 3. Split-brain maintenance API retired

The standalone `maintain_i64_index_delta()` path was removed. Once an index contains row handles, an index delta cannot be applied correctly in isolation from the physical relation transition that allocates/removes those handles.

The only maintained path is now `PhysicalStore::apply_relation_delta(...)`:

1. clone the installed relation and all indexes bound to the relation/layout;
2. apply the semantic `RelationDelta` to the cloned relation;
3. produce physical edits `(PhysicalRowId, Row)` for removals/insertions;
4. update every bound index from those physical edits;
5. publish relation + indexes together only after every step succeeds.

Thus an inconsistent delta cannot publish a stale relation/index pair.

## 4. Hostile falsification

Added a two-column Bag test with duplicate join keys and distinct payloads. It performs multiple sequential deletions that shift dense physical positions, then inserts a new duplicate-key row. After every transition:

- persisted Join is used (`persisted_index_hits=1`);
- no ephemeral rebuild is used;
- physical result exactly equals logical recomputation;
- the surviving stable handle never aliases the payload of a neighboring row after compaction.

Existing sequential Bag-delta tests and atomic relation/index update tests remain green.

## 5. Performance

The first row-handle implementation used a `BTreeMap<RowId, position>` lookup and regressed to about 1.82x the prebuilt baseline. This was rejected.

The handle-position resolver was changed to a dense slot map, and I64 row materialization dispatch was hoisted outside the Join inner loop. Output capacity is also reserved from the left cardinality.

Three independent final process runs on the 20,000 unique-key self-join:

- 1.087x prebuilt baseline; persisted / ephemeral = 0.483x;
- 1.110x prebuilt baseline; persisted / ephemeral = 0.494x;
- 1.099x prebuilt baseline; persisted / ephemeral = 0.497x.

Median process ratio: ~1.099x prebuilt baseline.

A separate gated run measured 3.090 ms persisted vs 2.707 ms baseline = 1.141x and persisted ≈0.509x ephemeral. Scheduler/cache noise is visible, so the defensible conclusion is roughly 1.10–1.14x on this microbenchmark, not a universal constant-factor claim.

The Pass16 logical-payload defect is therefore closed. Remaining overhead is narrower: generic plan/output materialization, row-handle position resolution after compaction, and dense relation update mechanics.

## 6. Specification correction

`CFMD_IDEAL_DB_SPEC.md` was updated.

The normative physical rule is now:

- persisted indexes are revision-derived artifacts with first-class bindings;
- index payload is a typed/stable physical handle or slot, not a copied logical row;
- relation + dependent index updates form one atomic physical transition;
- standalone index maintenance is invalid unless it is part of that transition;
- stable row identity must survive physical position changes;
- logical `Value` materialization belongs at semantic/output boundaries.

This changes no logical kernel primitive.

## 7. Verification gate

Rust 1.98.1 standalone toolchain.

- 194 declared tests;
- `cargo fmt --all -- --check` PASS;
- `cargo test --workspace` PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` PASS;
- `cargo test --workspace --release` PASS;
- `cargo build --workspace --release` PASS;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` PASS;
- 19 crates;
- 20,032 Rust LOC;
- zero external Cargo registry/git sources;
- zero `unsafe`;
- zero TODO/FIXME;
- zero `panic!`/`todo!`/`unimplemented!` macros in Rust source.

Raw benchmark evidence:
- `PASS17_PERSISTED_INDEX_BENCH.txt`
- `PASS17_BENCH_REPEATS.txt`

## 8. Remaining problems, ordered

1. General typed batch-to-batch pipeline. Join output still becomes `Vec<Value>` rows instead of remaining a typed batch/row-handle stream.
2. Stable-slot/chunk physical relation storage. Dense `Vec::remove` still creates O(n) position repair after deletions even though index identities are stable.
3. Close the remaining ~10–15% persisted prebuilt-index gap on the measured I64 Join without benchmark-specific semantics.
4. Persisted/index-maintained joins for Bool, F64Bitwise, Text/collation and nominal entity IDs, plus planner costing.
5. Maintained TopK order-statistics.
6. Maintained Group state, especially deletion-capable exact/reproducible F64 state/provenance.
7. Long-lived derivative ownership for Join/TopK/Group integrated with their physical structures.
8. Runtime for KeyValue/Adjacency/CSR/DenseArray/Inverted/Custom layouts.
9. OrderedView/pagination.
10. WAL/recovery, durable materializations, compaction/crash tests; then concurrency scheduler and distribution/formal closure.
