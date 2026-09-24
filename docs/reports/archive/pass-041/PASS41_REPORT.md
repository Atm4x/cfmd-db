# PASS41 REPORT — compositional typed stateful producers + I64 stateful hot-path reduction

**Baseline:** verified Pass40.  
**Status:** VERIFIED on Rust 1.98.1.  
**Scope:** data-plane only; no new semantic authority and no durability-format change.

Pass41 attacks the remaining stateful-operator pipeline debt after Pass40. CLOSED items below are eliminated functional/architectural problems. Benchmarks and regression cases are evidence, not separate CLOSED entries.

## 1. Problem

Pass40 still had three closely related data-plane issues:

1. `Group` and `TopKWithTies` could consume typed columnar batches, but their results normally crossed back into `Vec<Row>` before downstream operators. The typed DAG therefore stopped at a stateful operator instead of making Group/TopK genuine typed-batch producers.
2. Exact single-column I64 maintained TopK still stored full row buckets and used the generic semantic before/after diff machinery. Its historical constant-factor gap to a hand-written count-only baseline was roughly 20–23x.
3. Maintained exact-I64 Group still had a residual constant-factor gap after Pass40 (~2.3–2.4x).

During implementation a separate correctness defect was found in the existing typed physical descending-I64 TopK threshold selection.

## 2. Typed stateful producer architecture

Pass41 adds an internal owned typed result:

```text
OwnedTypedBatch
  columns: Vec<NativeColumn>
  positions: Vec<usize>
  stats: ExecutionStats
```

It is derivative execution state only. It is not a logical relation type, semantic authority, storage identity or durable representation.

The typed producer pipeline can now stay columnar across current builtin primitive stateful/unary compositions such as:

```text
Scan/Filter/Project
      ↓
Group
      ↓
TopK
      ↓
Filter/Project
      ↓
final Row materialization
```

and in the reverse stateful composition:

```text
Scan
 ↓
TopK
 ↓
Group
 ↓
Project
```

### Group producer

For the current builtin primitive equivalence fragment:

- exact I64 single-key Group keeps its specialized typed implementation;
- other primitive group keys are encoded through pinned-Γ `CanonicalEqKey` values;
- composite primitive group keys use `Vec<CanonicalEqKey>`;
- Count and ExactF64Sum output directly as `NativeColumn` values;
- structural/custom equivalence modules still fall back to the exact generic path and are **not** claimed as canonical typed producers.

### TopK producer

TopK can consume either:

- a raw typed selection; or
- an `OwnedTypedBatch` produced by another stateful operator.

The raw path does **not** clone entire source columns. It computes retained positions against the borrowed physical columns, then compacts only the selected logical columns.

I64 uses the specialized positional kernel; other current builtin orderings retain the pinned-Γ semantic comparator.

## 3. Correctness defect found and fixed

The old descending I64 positional TopK used the `k-1` order statistic, which is the k-th **smallest** key, then retained keys `>= threshold`. For descending queries this could retain far more than the true top-k-with-ties set.

Pass41 uses:

```text
ascending threshold index  = k - 1
descending threshold index = len - k
```

A hostile tie case `[1, 9, 8, 8, 2]`, `k=2`, descending now yields exactly `[9, 8, 8]`.

## 4. Maintained I64 TopK specialization

For a one-column exact-I64 input, maintained TopK no longer needs `key -> Vec<Row>` buckets. Pass41 adds:

```text
I64Scalar: BTreeMap<i64, multiplicity>
```

The common single-remove/single-insert delta path:

- validates multiplicities directly;
- updates scalar counts atomically;
- computes the small top-k-with-ties before/after difference over compact `(key,count)` slices;
- avoids the generic semantic row-diff path;
- avoids temporary tree/set allocations in the common replacement case.

The multi-column I64 path remains `I64Rows`, and non-I64 primitive ordering remains on the semantic ordered-key implementation.

This materially reduces the historical TopK constant factor, but does not eliminate it; see §6.

## 5. Maintained I64 Group hot path

`ExactCount` now exposes the exact representation-independent predicate `is_one()`.

For the common exact-I64 `remove old singleton key + insert previously absent key` transition, Group can remove the old lookup entry, verify singleton multiplicity, reuse the existing group slot and install the new key without the previous redundant lookup/finish sequence.

The general checked path remains unchanged for non-singleton/conflicting cases.

## 6. Performance evidence — still OPEN gaps

Raw evidence: `evidence/pass41/STATEFUL_OPERATOR_BENCH.log`.

### I64 Group, 50k groups

Seven release runs after Pass41:

```text
1.940x
2.000x
1.958x
1.976x
1.987x
1.969x
1.993x
```

Historical progression:

```text
pre-Pass40: ~4.8x
Pass40:     ~2.3–2.4x
Pass41:     ~1.94–2.00x
```

This is meaningful progress, but the constant-factor problem remains OPEN.

### I64 TopK, 50k rows, k=10

Seven release runs after Pass41:

```text
6.687x
6.734x
6.715x
6.686x
6.318x
6.614x
7.276x
```

Historical gap was roughly 20–23x. The specialized count-only state removes most of that tax, but ~6–7x remains; therefore the historical I64 TopK gap is still OPEN.

The maintained implementation is nevertheless ~400x faster than the measured full-replay path in this microbenchmark; that does not excuse the residual constant factor against the hand-maintained baseline.

## 7. Hostile / regression evidence

Pass41 verifies at least the following adversarial boundaries:

- `Group → TopK → Filter → Project` remains typed until the final output boundary;
- `TopK → Group → Project` also remains typed, so Group and TopK are compositional producers rather than only terminal producers;
- TextAsciiCaseInsensitive Group producer uses pinned primitive canonical keys and matches logical evaluation;
- F64Total TopK producer matches logical evaluation and stays typed;
- descending I64 TopK selects the correct high threshold and retains exact ties;
- raw TopK producer compacts selected positions instead of cloning the complete physical source;
- structural/custom semantic domains do not get silently treated as canonical primitive keys.

A static audit found no mutation hidden inside `debug_assert!`; the Pass39 release-only class of bug was not reintroduced.

## 8. Verification

Final Rust 1.98.1 gate after the last producer-composition change:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test ... --release PASS
```

Workspace metrics at source freeze:

- 316 declared Rust tests;
- 88 `kernel-plan` tests;
- 21 crates;
- 41,710 Rust LOC;
- 0 external Cargo sources;
- 0 `unsafe` in `crates/`.

## 9. Problem ledger

### CLOSED exactly in Pass41

1. ✅ **Group/TopK stopped the typed columnar DAG because they were consumers but not compositional typed-batch producers.** For the current builtin primitive typed fragment, Group and TopK now produce `OwnedTypedBatch` results consumable by downstream Group/TopK/Filter/Project without intermediate `Vec<Row>` materialization. Structural/custom semantic domains remain explicit fallback and are not included in this closure.
2. ✅ **Descending typed I64 TopK used the wrong order-statistic threshold and could return too many rows.** The direction-specific threshold is corrected and tie behavior is regression-tested against logical evaluation.

### Partially advanced, still OPEN

1. 🟨 **I64 TopK constant-factor gap.** Reduced from roughly 20–23x historical hand baseline to roughly 6.3–7.3x, but not closed.
2. 🟨 **Maintained I64 Group constant-factor gap.** Reduced from ~2.3–2.4x in Pass40 to ~1.94–2.00x, but not closed.

### Historical Pass26 OPEN backlog still active

1. ⬜ I64 TopK constant-factor gap — materially reduced in Pass41, still OPEN.
2. ✅ Group/TopK as typed-batch producers for the current builtin primitive typed fragment — **CLOSED in Pass41**.
3. ⬜ Maintained I64 Group constant-factor gap — materially reduced in Pass41, still OPEN.
4. ⬜ Structural/custom-equivalence Group canonical indexing/typed production. Primitive builtin portion is implemented; structural/custom laws remain unresolved.
5. ✅ Persisted Text/F64/Bool/entity primitive indexes + direct physical Filter/Join consumption — CLOSED in Pass40.
6. ⬜ Nested/multiway/mixed-key joins and broader join planning.
7. ⬜ Remaining physical layouts + OrderedView/pagination.
8. ⬜ Remaining durability/history assurance: revision DAG/merge ancestry, universal format migration, arbitrary semantic artifacts, power-loss/cross-filesystem assurance, scalability/distributed/authenticated durability.
9. ⬜ Transaction repair runtime, distribution and formal mechanization.

### Additional active OPEN after Pass41

1. ⬜ Cost/selectivity model for scan vs existing index vs index creation/retention. No magic threshold was added from one microbenchmark.
2. ⬜ Multi-column/mixed-key persisted semantic indexes and multi-key join planning.
3. ⬜ Structural/custom canonical-key laws for maintained/persisted indexes.
4. ⬜ Shared semantic-index reuse across multiple plans/operators.
5. ⬜ Physical-index memory budgeting, eviction and automatic rebuild scheduling.
6. ⬜ Canonical-key encoding migration/version compatibility for long-lived physical caches.
7. ⬜ I64 TopK and residual I64 Group constant-factor tuning.
8. ⬜ Clone-heavy runtime transaction candidates → COW/persistent roots.
9. ⬜ Durable/history/distribution items retained from Pass38/40: DAG ancestry, general historical migration, arbitrary semantic artifacts, intent GC, chunked checkpoints, power-loss/cross-FS assurance, group commit/replication/authentication/formal crash proof.

## 10. Result / next direction

Pass41 removes a structural typed-execution boundary rather than adding another isolated fast path: current builtin primitive Group and TopK can now participate as compositional typed producers.

The next high-value planner cluster is:

1. composite/mixed-key semantic physical indexes;
2. multi-key/multiway Join lowering;
3. an explicit cost/selectivity model deciding scan vs installed index vs maintained/created index.

The residual I64 Group/TopK constant factors should continue to be measured and reduced, but neither is allowed to masquerade as closed until the measured gap is actually small enough to justify that claim.
