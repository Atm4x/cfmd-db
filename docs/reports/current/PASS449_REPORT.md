# PASS449 REPORT — COLUMN-SELECTIVE MODEL-RULE INVALIDATION + BENCHMARK GATE

Start: **2026-10-02 22:39:04 UTC**  
Functional freeze: **2026-10-02 22:53:32 UTC**  
Useful boundary: **22:59:04 UTC**  
Hard boundary: **23:03:04 UTC**

## CLOSED THIS PASS

- Connected P448 `ModelRuleDependency.columns` to P438/P439 stable semantic object-field coordinates through a kernel-owned `RelationMutationFootprint`.
- Certified field-only relation rewrites carry `membership_changed = false` plus the exact changed stable semantic columns.
- Generic/raw relation mutations and create/delete/relationship actions remain full invalidations. A narrow footprint is never inferred from row-count coincidence alone.
- `CompiledRulePlan::model_rules_for_mutation(...)` now filters model rules by exact relation + column dependency:
  - cardinality is unaffected by same-membership field rewrites;
  - `Exists` / `All` / exact sum run only when membership changes or one of their dependency columns changes.
- The selective law is consumed by both Candidate/revision validation and runtime VMF rule accounting.
- Pure scalar object-field Candidate construction now uses selective relation-update validation rather than full-state `Revision::build`.
- Hostile audit found and fixed a pre-existing append-only Bag fast-path bypass introduced once P447/P448 model-wide rules became authoritative. The old path validated only appended rows and transported `V=0`; it could miss full-target cardinality/quantifier/aggregate violations. Structural validation remains append-bounded, while model rules now evaluate against the complete persistent target and VMF recomputes only affected model-rule witnesses.

## SELECTED LAW

```text
DurableObjectFieldWrite
        -> exact stable changed columns
        -> RelationMutationFootprint {
               membership_changed = false,
               columns = exact fields
           }

other relation semantic mutation
        -> FULL footprint

ModelRuleDependency
        x
RelationMutationFootprint
        -> affected / unaffected
        -> Candidate validation + VMF
```

The footprint is derived from already-certified semantic mutation authority. It is not a user/frontend hint.

## HOSTILE / R&D BENCHMARK

Release benchmark for one compiled `RelationAll(TextLength)` rule whose predicate must inspect every row:

| Rows | Relevant rule scan | Column-selective skip |
|---:|---:|---:|
| 1,000 | 9,024 ns | 631 ns |
| 100,000 | 945,936 ns | 1,412 ns |
| 1,000,000 | 9,549,789 ns | 4,607 ns |

The relevant path scales linearly and reaches about **9.55 ms per rule at 1M rows**. Several hot quantified/aggregate invariants can therefore consume tens of milliseconds of commit latency. This is enough evidence to justify maintained-witness R&D in PASS450; the optimization is no longer speculative.

## HOSTILE / REJECTED

- REJECTED: externally trusted "changed columns" hints.
- REJECTED: narrowing a generic relation delta merely because removed/inserted cardinalities match.
- REJECTED: reevaluating cardinality for certified field-only same-membership writes.
- REJECTED: old append-only Bag `V=0` transport after model-wide rules became authoritative.
- REJECTED: building maintained witness state before measuring the actual O(N) payer.

## VERIFICATION ON FROZEN CODE

- `cargo check --workspace --all-targets`: **PASS**.
- `kernel-validation`: **25 passed / 2 ignored**.
- `kernel-revision`: **7/7**.
- `kernel-plan`: **297 passed / 5 ignored**.
- `cfmd-runtime`: **51/51** total (3 + 9 + 39).
- public `cfmd`: **53/53**.
- public API contract: **PASS**.
- release 1k/100k/1M benchmark: **PASS**.
- same-relation unrelated-field selective hostile: **PASS**.
- append-only Bag full-target model-rule hostile: **PASS**.
- repository verifier: **PASS**.
- public Rust facade verifier: **PASS**.
- repository manifest entries: **5145**.
- repository-local `target/`: **absent**.

## LEDGER — OPEN IMMEDIATE

1. PASS450: maintained reconstructible runtime witnesses for `Exists`, `All`, and exact-f64 sum, updated from exact relation deltas and benchmarked against the P449 scan baseline. Cardinality already has an O(1) persistent specialization.
2. Witness state must remain reconstructible optimization/proof state; authoritative semantics remain persisted `ModelRuleExpr` + current `Revision`. Do not introduce a second durable truth store.
3. Only extend grouped/multi-relation rules through existing exact query/aggregate algebra with explicit dependency closure.
4. Define durable canonical transaction-requirement digest before external/recoverable transaction IDs become ordinary cross-binding DX.
5. Final typed Rust invariant/`require` syntax and typed object-field grant DX remain open.

## LEDGER — REMOTE READER SCHEMA-EVOLUTION DX

- reader may contain no authoritative schema definition;
- no reader-specific metadata/contracts on the DB server;
- no dedicated compatibility handshake;
- numeric schema epoch/version arrives through ordinary metadata without `ModelRead`;
- compatibility resolves once at `Context<M>` / `Snapshot<M>` binding, never per query/row;
- no field-name/existence fallback; same name may carry different semantics across epochs;
- uncovered epochs fail closed;
- authoritative schema stays current-only and compatibility history remains consumer-side;
- final API/annotation syntax remains undecided; `bind/rebind` is not selected terminology.

## LEDGER — DEFERRED

- migration frontend Rust/Python/TMD/CLI + diagnostics;
- final Python/.NET/Studio surfaces;
- backup/restore/corruption UX and future admin permissions;
- public performance/binary-size budgets;
- native Windows secure-memory expansion;
- persistent-tree bulk-builder R&D remains performance-only.

## SUPERSEDED / DO NOT EXTEND

- public `ReadContext` / `ReaderContext`;
- `Snapshot::transaction()`; only `Transaction::new()` and `Transaction::from(snapshot)`;
- Context shape as security;
- callback/generic invariant engines;
- externally supplied validation-footprint hints;
- unconditional append-only Bag VMF-zero transport with model rules;
- global model-rule scans when exact dependency certificates exist;
- per-query reader schema-version routing.

## PERFORMANCE BASELINES TO PRESERVE

- same-relation unrelated field writes do not scan model-rule relations;
- cardinality stays O(1);
- P449 pre-witness relevant-scan baseline: ~9.55 ms / 1M rows / quantified rule;
- append-only Bag structural validation remains proportional to appended rows except for genuinely affected global model rules;
- preserve realization native-cost class, persistent sharing, non-backtracking `TextPattern`, exact sum semantics, and stale-`require` behavior.

## NEXT RECOMMENDED PASS

**PASS450 — maintained exact model-rule witnesses.** Derive reconstructible witness state for `Exists`, `All`, and exact-f64 sum from the existing compiled rule/query/aggregate laws, update it using exact relation deltas, and measure delta-update cost against the P449 1k/100k/1M baseline. No second durable semantic store and no fallback engine.
