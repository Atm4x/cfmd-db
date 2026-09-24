# AFTER PASS81 — HANDOFF / NEXT WORK

Pass81 goal: finish production integration of the closed write-R&D branch. Do not reopen deferred generalizations merely because they exist mathematically.

## Authoritative starting point

Use the final Pass81 workspace ZIP produced by this run. Read, in order:

1. `PASS81_REPORT.md`
2. `PASS81_INTEGRATION_LEDGER.md`
3. `PASS81_INTEGRATION_CLOSEOUT.md`
4. `CFMD_IDEAL_DB_SPEC.md` Pass81 addendum
5. `PASS81_NEXT_HANDOFF.md`

The final Pass81 source is authoritative over all intermediate A..AY checkpoints.

## What Pass81 intentionally does NOT try to close

These are post-integration / historical-open work, not reasons to reopen Pass81 write convergence:

- the historical **22 compound OPEN** ledger inherited from Pass80; attack these in later passes one real problem at a time;
- independent durable branch-head ingestion/retention and a fully general durable revision DAG / branch+merge ancestry;
- arbitrary finite concurrent-antichain normalization above the currently consumed bounded width; only generalize when a concrete runtime consumer needs it;
- writable `Group` / aggregate action policies while Group remains read-only;
- minimum-cardinality complement optimization;
- arbitrary executable LensSpec/plugin deployment beyond exact registered implementations;
- richer hidden-column constructors beyond the currently explicit fixed-hidden authority;
- transition-level erasure dependency enforcement before such erasure transitions are actually exposed;
- physical/lifecycle/performance work already tracked by the historical ledger (SAMF Ordered/Annotation, generic DTC/VMF/OFC, etc.).

## Suggested next-pass order

1. Re-open the authoritative 22-item historical ledger and choose the highest-value whole problem, not “one test = one closed problem”.
2. Prefer correctness/architecture gaps before constant-factor performance debt.
3. Keep `Revision=(S,Γ,M)` as semantic authority; physical artifacts and catalogs remain derivative/reconstructible.
4. Preserve Pass81 write contracts: Rewrite identity is not endpoint identity; Γ addresses semantic classes; VMF/freshness/WAL remain mandatory publication boundaries; sequence concurrent intent uses stable occurrence/gap identities, never snapshot indices.
5. When touching concurrency/durability, reuse Γ-REIC effect ideals/residual-family/cube machinery rather than inventing a second merge calculus.

## If provenance/R&D bundles are missing

The user previously supplied or the session contained these important bundles. Ask for them only if a future task genuinely needs historical provenance; the final Pass81 ZIP is sufficient for normal continuation:

- `CFMD_WRITE_INTEGRATION_AND_ALL_POST_RND_BUNDLE_2026-09-22.zip` — primary write-integration + post-R&D bundle used during Pass81 convergence.
- `CFMD_HISTORICAL_SOLUTIONS_PORTABLE_10_CLOSED_2026-09-22.zip` — portable historical closed solutions/reference bundle.
- `CFMD_RND_UNIFIES_ALL_CLOSED_CALCULI_INTEGRATION_SPINE_AND_CLOSES_ADVISOR_RECOVERY_SCHEDULING_2026-09-21(4).zip` — unified closed-calculi integration spine/advisor-recovery R&D source.
- `CFMD_RND_CLOSES_STREAMING_CHUNKED_CHECKPOINTS_AND_RESTART_POISON_POLICY_2026-09-21(1).zip` — streaming/restart-poison R&D source.
- `SYMBOL_MAP.csv` if symbol/provenance mapping is needed.

Older `CFMD_RND_PROGRAM*` ZIPs are reference history, not the first thing to request for ordinary post-Pass81 work.

## Build/test discipline

Rust toolchain used: 1.98.1. This environment has a ~45-second tool-call ceiling; do not run one monolithic workspace build from cold state. Run per-package/batched `check`, `clippy`, and tests with internal timeouts, and isolate `kernel-durability` / `kernel-plan` when necessary. Use an external target directory if checkpoint source cleanliness matters.
