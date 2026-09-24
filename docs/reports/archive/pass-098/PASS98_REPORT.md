# PASS98 REPORT

**Status:** FINAL / V5 STAGE 4.3a SCALAR-I64 ORDEREDBOUNDARY INTEGRATED

## Baseline and wall-clock boundary

Baseline: frozen Pass97 (`cfmd_workspace_pass97_group_annotation_dense.zip`). Source work began at **2026-09-23 01:39:51 UTC**. The nominal 20-minute source boundary was **01:59:51 UTC**. Production source was frozen early at **01:55:20 UTC**. After freeze no `.rs` file or Cargo manifest was changed.

Before compilation, old Pass90–Pass97 build-target directories and the obsolete 194 MB Rust installer tar were removed because the filesystem had reached 100% usage. The installed Rust 1.98.1 toolchain, source workspaces, reports and ZIP artifacts were retained. This housekeeping changed no production source.

## Result

- **V5 Stage 4.3a scalar-I64 OrderedBoundary / TopK — COMPLETE.**
- Whole Stage 4.3 remains **PARTIAL** until `I64Rows` and `SemanticOrdered` are migrated to universal plan/commit.
- Historical production closure remains **14 / 22**. #21/#22 remain integration-in-progress, not closed.
- Source delta vs Pass97 is exactly two Rust files:
  - `crates/kernel-query/src/lib.rs`
  - `crates/kernel-query/src/topk_i64.rs` — new
- Frozen path-stable source fingerprint: `a9db7920683bacfe6c910d09a95e4325cf1295d18252075bf40af3b811de1b76`.
- Final gate: fmt/check/strict Clippy/full workspace tests PASS; **693 declared tests / 0 failed / 8 ignored**.

## Scalar-I64 physical lowering

The old scalar `BTreeMap<i64, usize>` TopK payload is replaced by an explicit physical state with three exact tiers:

1. `DenseUnit` for bounded windows with physical multiplicity invariant `{0,1}`;
2. `DenseCounted` after a duplicate appears inside the admitted dense window;
3. `PagedRadix` for sparse/extreme/window-escaping I64 keys.

Dense admission uses `margin=64` and `max_slots=4096`. Violating a physical premise causes pre-commit promotion/fallback rather than semantic rejection.

The scalar barrier now consumes a signed `DeltaView<Row>` and produces `AdaptiveDelta<Row,4>`. Planning operates on an unpublished candidate physical state; commit swaps in that state only after the complete plan succeeds. Unit replacements maintain threshold metadata and repair the kth-with-ties boundary by at most one adjacent live bucket. More general signed batches use the same exact planner with full boundary recomputation inside the candidate state.

## Hostile falsification

- dense unit source remains unchanged when a duplicate triggers counted promotion;
- dense unit source remains unchanged when an out-of-window key triggers paged-radix fallback;
- 2,000 sequential replacements for both ascending and descending order match a full TopK oracle and observe the adjacent-threshold bound;
- all pre-existing TopK differential/tie/Set/semantic-ordering/atomicity tests remain green;
- full workspace regression remains clean.

## Deliberate non-claims

- `I64Rows` and `SemanticOrdered` TopK branches are not yet universal plan/commit kernels;
- therefore Stage 4.3 as a whole is not complete;
- Stage 4.4 Join and Stage 4.5 Blocker are not started;
- Stage 5 root-only `RelationDelta` materialization is not started;
- no Stage 6 performance/allocation closure or PROD CLOSED claim is made for #21/#22.

## Next

Pass99 should finish Stage 4.3 by migrating non-scalar I64-row and generic Γ-ordered TopK to one read-only OrderedBoundary plan/commit contract with universal output. After a differential/full gate, Stage 4.4 Join may begin if time remains.
