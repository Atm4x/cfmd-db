# IMPLEMENTATION REPORT — Pass46

## problem

Pass45 had a deliberate statistics gap: multiway planning would not credit an unbuilt transient index because its distinctness was unknown, while direct execution could build first and only then discover duplicate-heavy data made the candidate unprofitable. There was no retained Γ-bound cardinality artifact independent of a full persisted index.

## hypotheses

1. Exact key multiplicity statistics can use the same canonical Γ binding as semantic indexes without becoming semantic authority.
2. Maintaining `canonical_key -> multiplicity` atomically with relation deltas is sufficient to expose exact row/distinct counts for current primitive single/composite keys.
3. The shared Pass45 `JoinAccessDecision` can consume this measured distinctness without changing correctness because execution already revalidates/re-costs derivative candidates.
4. Arbitrary multiway leaf permutation must not be implemented until physical row-order restoration is explicit.

## implementation

- added `MaterializedSemanticStatisticsState` and public `SemanticKeyStatistics`;
- unified semantic-index/statistics binding resolution through `resolve_semantic_index_binding`;
- statistics support current primitive single and composite canonical keys;
- integrated validation/maintenance into the atomic relation physical transition;
- relation reinstall invalidates dependent statistics;
- Γ incompatibility makes statistics unavailable and blocks stale-law delta maintenance;
- added `PhysicalStore::install_semantic_statistics` / `semantic_statistics`;
- added immutable `RuntimeRevisionCell::install_semantic_statistics` publication;
- inconsistent retained row counts are ignored as non-authoritative physical state;
- `right_scan_join_access_decision` uses retained distinctness for transient work/output estimates;
- multiway transient costing is enabled only when compatible retained statistics exist.

## hostile falsification

- sequential duplicate birth/death updates preserve exact distinct counts;
- mixed TextAsciiCI+I64 composite statistics match persisted index distinctness;
- same SemanticId with changed Γ module invalidates statistics;
- stale Γ statistics cannot be maintained through a relation delta;
- deliberately corrupted statistics do not become planner authority;
- duplicate-heavy retained statistics prevent an unnecessary transient build;
- retained unique-key statistics permit safe transient multiway costing;
- runtime statistics publication changes only physical root version and stales a previously prepared transition;
- release and overflow-check suites remain green;
- no new Clippy suppression or debug-assert side-effect pattern was introduced.

## verification/result

Full Rust 1.98.1 fmt/check/debug/release/strict-Clippy/release-build/strict-rustdoc/overflow-release gate: PASS.

Metrics: 347 declared tests, 119 `kernel-plan`, 21 crates, 46,869 Rust LOC, 0 external Cargo sources, 0 `unsafe`.

## rejected routes

- no arbitrary leaf permutation before order restoration/provenance exists;
- no optimizer-generated fake distinctness represented as statistics;
- no host equality/hash keying;
- no statistics-as-authority fallback;
- no automatic statistics retention policy before memory/lifecycle accounting is unified.

## recommended next step

Introduce a physical multiway order-restoration/provenance boundary, then implement subset/general-bushy join enumeration while preserving the current exact output contract. Separately widen the physical advisor to own statistics + specialized I64 + generic semantic/future layouts under one byte-level benefit/budget model.

## remaining risks

The 22-item historical OPEN ledger remains active. In particular: arbitrary-permutation/bushy planning, autonomous/correlated statistics, multi-family lifecycle, byte-level budgeting, structural/custom canonicalization, remaining layouts, durability/distribution work and formal closure are still OPEN.
