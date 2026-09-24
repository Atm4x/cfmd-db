# CFMD Pass 15 — mixed typed batches and adaptive indexed I64 Join

Date: 2026-09-19
Baseline: Pass14 verified (184 declared tests)
Final: Pass15 verified (189 declared tests)
Wall-clock target: 20 minutes
Start: 12:27:36 UTC
Code/audit wall-clock boundary: 12:47:50 UTC (20m14s)

## 1. Mixed typed physical columns replace the I64-only architecture

Problem -> Pass14's near-baseline physical path depended on a dedicated `I64Columnar` relation variant. Extending that design with one relation variant per scalar type would duplicate executor structure and make mixed physical rows awkward.

Hypothesis -> retain logical `Value` semantics at operator boundaries, but represent physical column batches as a heterogeneous vector of typed columns. Specialize once on the physical predicate column before entering the row loop; never dispatch on the semantic/value variant for every scanned row.

Implementation -> added `NativeColumn` and `NativeRelation::TypedColumnar`. Native columns cover every current scalar carrier:

- Unit;
- Bool;
- I64;
- F64 exact stored bits;
- Text;
- nominal LiveEntityRef IDs with pinned entity type;
- nominal HistoricalEntityId IDs with pinned entity type.

`validate_typed_columnar_schema` checks every physical column against the pinned relation type before execution, including nominal entity type. The fused `Scan -> FilterEqConst -> Project` path resolves the semantic predicate once, dispatches once on `(NativeColumn, BoundPrimitivePredicate)`, then scans raw slices. Only matching rows are materialized back into logical `Value`s.

Falsification -> a vertical mixed relation containing all seven scalar carrier forms is filtered with ASCII-CI Text equality and projected through the typed executor. Its result exactly matches the logical evaluator. Column-length mismatch and physical/logical scalar mismatch are rejected. Entity columns preserve nominal entity type rather than treating IDs as untyped integers.

Performance falsifier -> the first generic typed implementation put enum dispatch inside the row loop and regressed the I64 workload to 1.566x hand-written baseline. That implementation was retired. After hoisting dispatch outside the loop, three independent final mixed-batch benchmark processes produced ratios 0.906x, 0.966x and 0.929x versus the same hand-written I64 baseline (median process ratio 0.929x). The legacy dedicated-I64 path in those same processes produced 1.024x, 0.969x and 0.958x. Inter-process noise is visible, so this is evidence that the unified typed batch no longer imposes the earlier mandatory tax, not evidence that CFMD is intrinsically faster.

Result -> the typed physical representation is no longer I64-only. A single heterogeneous typed-batch contract can carry all current scalar types while retaining raw hot loops and exact logical semantics.

## 2. Columnar Join gains an adaptive indexed I64 path

Problem -> physical Join was still `NestedLoop`, so even typed storage left a quadratic execution algorithm on a core relational operator.

Hypothesis -> direct Columnar `Scan x Scan` can admit an I64 index specialization without changing logical Join semantics. Planner choice may be optimistic (`IndexedI64IfAvailable`) because runtime can validate the pinned equivalence and physical key columns, then fall back to semantic nested-loop when the specialization is unavailable.

Implementation -> added `JoinAlgorithm::IndexedI64IfAvailable`. Catalog lowering selects it for direct Columnar Scan pairs. Runtime:

1. confirms the pinned equality contract binds as `I64Exact`;
2. confirms both physical key columns expose raw I64 slices;
3. validates typed physical storage against schema;
4. builds a `BTreeMap<i64, Vec<row_index>>` on the right input;
5. probes in left-row order and preserves right-row order within each key bucket;
6. materializes one final joined row directly from physical columns, without intermediate left/right `Vec<Row>` allocations.

If any specialization condition fails, execution falls back to the existing semantic nested-loop path rather than changing result semantics.

Falsification -> duplicate-key Bag join `[1,1,2] x [1,1,2]` yields the same five rows as logical reference with the same multiplicity/order. A Text ASCII-CI Columnar Join is deliberately lowered to the adaptive operator but cannot take the I64 specialization; its fallback result exactly matches logical reference.

Performance -> on a 20,000-row unique-key self-join, the first indexed implementation measured 1.194x a hand-written BTreeMap baseline. Removing temporary left/right row allocations reduced a subsequent run to 1.093x. Three final process-level runs measured 1.029x, 1.056x and 1.104x (median ratio 1.056x). The algorithmic O(n^2) path is gone for admitted typed I64 direct scans; a small implementation gap remains and the index is rebuilt per query.

## 3. Final verification gate

Rust 1.98.1 standalone toolchain.

- 189 declared unit/integration tests;
- `cargo test --workspace` — PASS;
- `cargo test --workspace --release` — PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS;
- `cargo fmt --all -- --check` — PASS;
- `cargo build --workspace --release` — PASS;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` — PASS;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --release -p kernel-plan -p kernel-integration` — PASS;
- 19 workspace crates;
- 18930 Rust LOC including benchmark examples;
- no Cargo registry/git sources;
- 0 `unsafe`;
- 0 TODO/FIXME;
- 0 `panic!`/`todo!`/`unimplemented!` macros in workspace Rust source.

Raw benchmark evidence:

- `PASS15_TYPED_BATCH_BENCH_FINAL.txt`;
- `PASS15_INDEXED_JOIN_BENCH_FINAL.txt`;
- `PASS15_BENCH_REPEATS.txt`.

## 4. Remaining problems, ordered

1. **Typed batch fusion is still narrow.** Mixed typed storage exists for every scalar carrier, but the strongest non-materializing path is still fused `Scan -> FilterEqConst -> Project`. General vectorized/batch execution across arbitrary operator trees remains open.
2. **Indexed Join is rebuilt on every query.** It is a physical index algorithm, not yet a persisted/materialized index with maintenance under `RelationDelta`.
3. **Indexed Join specialization currently targets I64 exact keys only.** Bool/F64-bitwise/Text/nominal-ID indexed strategies and planner cost selection remain open.
4. **Join output still materializes logical `Value` rows.** A downstream batch-to-batch Join pipeline would avoid this boundary when another physical operator can consume typed batches directly.
5. **TopK remains correctness-first full/insertion sort.** Maintained order-statistics and physical incremental state remain open.
6. **Group remains correctness-first replay.** Maintained Count/ExactF64Sum state, especially deletion-capable exact-sum provenance, remains open.
7. **Long-lived derivative ownership covers Project(Set)/Distinct only.** Join/TopK/Group need compositional maintained state and connection to the physical index/aggregate structures.
8. **Other physical layout families remain contracts only:** KeyValue, AdjacencyList, DenseArray, Inverted and Custom.
9. **OrderedView/pagination** remains a distinct semantic/runtime type.
10. **Durability/engine layer** remains: WAL/recovery, persistent indexes/materializations, compaction, concurrency control, crash testing, then distribution.

The immediate frontier is now persisted/index-maintained execution state plus a general typed batch pipeline, not missing scalar physical representation or a quadratic baseline Join.
