# CFMD Pass 12 — checked PlanIR boundary, native-layout preservation, and materialized Set support state

Date: 2026-09-19
Wall-clock cycle start: 2026-09-19T11:11:31Z
Wall-clock target: 20 minutes
Wall-clock end: 2026-09-19T11:31:33Z
Measured elapsed: 20m 02s
Baseline: pass11 verified (159 tests)
Toolchain: Rust 1.98.1 standalone, local/offline
Status: **VERIFIED PASS**

## 1. Minimal PlanIR is now a separate compiler layer

Problem -> pass11 had a strong logical/semantic kernel but no executable representation of the logical-to-physical boundary. Adding physical choices directly to `kernel-query` would mix semantics, optimization and runtime policy, while a broad planner API could quietly become a second hidden DBMS.

Hypothesis -> the first physical IR should be deliberately small, closed and mechanically auditable. Each current `RelExpr` node should lower to exactly one PlanIR node, with every baseline algorithm choice explicit and no opaque host callback or generic "execute arbitrary code" node.

Implementation -> added the dependency-free `kernel-plan` workspace crate. Its closed `Plan` mirrors the current relational operator set while making baseline physical choices explicit: `FullScan`, `NestedLoop`, `LinearReplay` grouping and `FullSort` TopK. `PlanShape` counts every physical node/operator. `logical_node_count` independently counts logical nodes.

Falsification -> a composite expression containing every current relational operator lowers and exact-round-trips back to the identical `RelExpr`. The checked lowering rule additionally requires `physical.shape().nodes == logical_node_count(logical)`, so the admitted baseline cannot hide auxiliary plan expansion behind lowering.

Result -> the minimal PlanIR boundary exists without moving logical meaning into the physical layer. This proves only the shape/meaning contract, not physical execution performance.

## 2. Lowering is admitted through the generic checked-certificate boundary

Problem -> a planner-produced plan must not become trusted merely because it came from planner code.

Implementation -> `LoweringChecker`/`LoweringCertificate::ExactLogicalRoundTrip` use the existing `kernel-proof::CheckedCertificate` admission boundary. Admission rejects a physical plan whose logical round-trip differs from the source query. `PreparedPlan` first runs the normal query `prepare/typecheck`, then obtains a checked lowering certificate and pins the full `SemanticContext`.

Falsification -> an intentionally altered projection column is rejected as `LogicalMeaningChanged`; an out-of-bounds logical projection is rejected by query typechecking before a `PreparedPlan` exists; reference execution rejects semantic-context drift.

Result -> logical typechecking, semantic revision pinning and physical lowering now form one explicit compile boundary. The reference executor remains intentionally labeled as reference-only and reuses logical semantics; it is not claimed as the physical runtime.

## 3. Physical catalog preserves a chosen native layout without mandatory conversion

Problem -> a universal kernel would incur a structural abstraction tax if physical lowering forced every specialized layout through one canonical row representation before planning.

Hypothesis -> layout identity is non-semantic physical metadata and can be carried directly by scan nodes while the checked logical round-trip ignores it.

Implementation -> `PhysicalCatalog` binds relations to explicit `LayoutBinding { id, family }`. Current families are logical-model rows, row store, columnar, key-value, adjacency list, dense array, inverted and custom. `lower_with_catalog` copies the selected binding into `Plan::Scan`; there is no mandatory conversion node in the baseline PlanIR.

Falsification -> a columnar binding survives checked lowering under a projection with exactly two plan nodes; a key-value binding also survives the full typed `PreparedPlan` path. A vertical `kernel-integration` slice creates schema/Γ/model data, prepares a catalog-bound relational plan, checks the physical scan binding, and verifies reference execution against direct logical execution.

Result -> the PlanIR itself does not require canonical-layout conversion. This is necessary evidence for the "no mandatory abstraction tax" target, but it is not sufficient: real native-layout executors and runtime benchmarks are still missing.

## 4. Set support counts now have explicit materialized state

Problem -> pass10/11 had correct support-count semantics for Set projection and Distinct, but the optimized derivative path rebuilt support counts from the old intermediate on every call. The state was therefore implicit and scan-backed rather than materializable.

Hypothesis -> the transition law can be separated into a state object pinned to both `RelType` and `SemanticContext`, initialized once and then updated only from inserted/removed rows.

Implementation -> added `MaterializedSetSupportState`. Construction validates Set semantics, column/equivalence arity, semantic-module domains and row value shapes, then materializes semantic support counts. `apply_rows_delta` is atomic on failure, context-bound, and emits a Set delta only when support crosses `0 <-> positive`. Public support lookup validates row shape. The old stateless Set-projection/Distinct helper now delegates to this same state transition implementation instead of maintaining a duplicate algorithm.

Falsification -> duplicate ASCII-CI representatives survive one removal and disappear only on the final support removal; insertion from zero emits exactly one Set row; malformed rows, over-removal and semantic-context drift are rejected without mutating state. Two independent exhaustive differential tests cover all 64 old->new transitions for `Distinct(Project(Scan))` and all 64 for duplicate-collapsing Set projection, comparing maintained-state deltas to `rel_delta_by_recompute`.

Result -> the first persisted IVM state boundary is explicit and correctness-tested. The public `rel_delta_optimized` API still reconstructs this state per call for compatibility; wiring long-lived state ownership into a materialized-view/runtime layer remains open.

## 5. First PlanIR compiler-overhead diagnostic

A no-dependency release example (`kernel-plan/examples/lowering_bench.rs`) measures compiler/PlanIR overhead only. It is deliberately not a query-runtime benchmark.

Five final runs on this container:

- 9-node `lower_with_catalog + exact logical round-trip`: 590, 1010, 453, 462, 519 ns/iteration; median **519 ns**.
- typed 3-node `prepare/typecheck + checked catalog lowering`: 2989, 1324, 1269, 1276, 1319 ns/iteration; median **1319 ns**.

The spread shows ordinary microbenchmark noise/outliers. These numbers are diagnostic evidence that the compiler boundary itself is small; they do **not** establish absence of runtime abstraction tax.

## 6. Final verification gate

Final source passes:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo test --workspace --release`
- `cargo build --workspace --release`
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
- targeted hostile gate: `RUSTFLAGS="-C overflow-checks=yes" cargo test -p kernel-query -p kernel-plan -p kernel-integration --release`

Final audit:

- 171 `#[test]` declarations (159 -> 171);
- 19 workspace crates (new `kernel-plan`);
- 15,718 Rust LOC under crate `src/`;
- no external Cargo lockfile sources;
- zero `unsafe` in Rust sources;
- zero TODO/FIXME;
- zero `panic!`, `todo!`, `unimplemented!` macros.

## 7. Remaining problems, ordered by importance

1. **A real physical executor is still absent.** `PreparedPlan` and layout bindings are checked compiler artifacts; `reference_execute` intentionally round-trips through logical semantics. Native row/columnar/KV/etc. executors are the next system-level blocker.
2. **Runtime abstraction-tax benchmarks remain unproven.** The pass12 benchmark measures compiler/lowering overhead only. The important experiment is native specialized layout -> physical executor versus a direct specialist baseline on scan/filter/project/join/group/top-k workloads.
3. **Materialized state ownership is not yet wired into the derivative/runtime API.** `MaterializedSetSupportState` is persistent-capable and exhaustively checked, but `rel_delta_optimized` still constructs it from old intermediates per invocation.
4. **Join still lacks a true semantic-key index.** Current Join correctness uses local replay; a real `ΔJoin` needs an admitted canonical/hash/index key contract for equivalence classes rather than silently hashing representatives.
5. **TopK still lacks maintained order statistics.** Exact threshold/tie semantics are correct, but update work still replays sorting at the node.
6. **Group still lacks maintained aggregate state.** Count can be maintained directly; `ExactF64Sum` needs the exact accumulator state, not only the rounded visible result.
7. **OrderedView/pagination is absent.** Stable logical order/cursor semantics remain distinct from `TopKWithTies` Set/Bag output.
8. **Durability/engine work remains outside the kernel:** WAL/recovery, persistent indexes/materializations, compaction, concurrency/distribution and crash testing.

## Bottom line

Pass12 closes the first half of the physical-lowering blocker: there is now a separate, closed, typed and certificate-checked PlanIR that preserves chosen native layout metadata without mandatory conversion, plus a vertical integration test. It also converts Set support counts from an implicit recomputation detail into explicit materialized state with exhaustive differential evidence. What remains is no longer "define a physical boundary" but "execute that boundary natively and prove its runtime cost against specialist baselines."
