# Kernel Hostile / Freeze Ledger

## Policy

This is the current repository-facing inventory after Pass280. It complements historical pass reports; it is not a promise that undiscovered bugs are impossible.

`FROZEN` means reopen only on evidence: correctness counterexample, proof/authority seam, measured complexity/performance regression, new R&D requirement, or public API/DX requirement.

| crate | closeout evidence | current status |
|---|---|---|
| `kernel-aggregate` | grouped small-kernel hostile sweep + final P280 global revalidation | FROZEN |
| `kernel-auth` | dedicated P277 security/resource hostile pass; P278 ownership split | FROZEN |
| `kernel-change` | P256–P267 closure; final hostile inventory; P280 revalidation | COMPLETE / FROZEN |
| `kernel-deployment` | dedicated P277 bounded/auth-first audit; P278 ownership split | FROZEN |
| `kernel-durability` | dedicated durability/store closeout line; P280 heavy-kernel revalidation; P306–P312 single-file backend R&D/parity hostile line | FROZEN / R&D REVALIDATED |
| `kernel-exact` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-fixpoint` | P276 incidence/checker/ownership closure | COMPLETE / FROZEN |
| `kernel-grounded-closure` | P275 unified witness calculus; P279 structural closeout | COMPLETE / FROZEN |
| `kernel-identity` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-integration` | P268–P273 prepared writable/integration closure; P280 revalidation | COMPLETE / FROZEN |
| `kernel-lens` | P274 hostile correctness pass; P275 ownership split | FROZEN |
| `kernel-lifecycle` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-model` | P279 dedicated hostile audit; P280 ownership split | COMPLETE / FROZEN |
| `kernel-persistent` | P276/P279 copy-amplification and whole-crate sweep | COMPLETE / FROZEN |
| `kernel-plan` | historical crate-wide hostile closeout (P194 line) + P280 heavy-kernel revalidation | FROZEN |
| `kernel-proof` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-query` | dedicated query/TopK/ExecGraph hostile line + P280 heavy-kernel revalidation | FROZEN |
| `kernel-retention` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-revision` | P274 dedicated inspection/regression pass | FROZEN |
| `kernel-schema` | P278 subtype authority; P280 bidirectional closure + ownership split | COMPLETE / FROZEN |
| `kernel-semantic-index` | P276 dedicated audit, clean/no-change | FROZEN |
| `kernel-semantics` | earlier dedicated semantics work + P280 heavy-kernel revalidation | FROZEN |
| `kernel-transport` | P274 hostile correctness/performance pass; P275 ownership split | FROZEN |
| `kernel-types` | grouped small-kernel hostile sweep | FROZEN |
| `kernel-validation` | P275/P277 correctness fixes; P278 ownership split | FROZEN |
| `kernel-violation` | grouped small-kernel hostile sweep | FROZEN |
| `storage-memory` | P274 LCA performance reopen; P279 revision-authority closeout | COMPLETE / FROZEN |

## Complexity inventory policy

The hostile campaign specifically searched for avoidable repeated full scans, nested traversals, repeated BFS/DFS, repeated typecheck/prepare/materialization, duplicated solver/authority state, copy amplification and error-driven generic fallbacks.

The closeout does **not** assert that every operation is sub-quadratic for every input. Some relations, closures, joins, products and result-producing operations have output-sensitive or mathematically inherent costs. Reopen when a measured/derived extra factor can be removed by an index, maintained authority, batching, structural sharing or stronger CFMD semantics.

## Marker policy

Production source is intentionally not spammed with `// CLEAR` markers, which become stale after edits. Use markers only for active, exceptional debt:

- `PAYER` — known hot-path/complexity debt requiring a ledger item;
- `RESIDUE` — structural/ownership debt requiring a ledger item;
- `BOUNDARY` — intentionally expensive semantic/I/O boundary when the cost is non-obvious;
- `INHERENT` — non-obvious asymptotic cost justified by result size/mathematical semantics;
- `SPECIALIZE` — specialized implementation of a shared semantic principle.

Any future `PAYER`/`RESIDUE` must be reflected in this ledger before a kernel can remain marked frozen.

## P324 — replication authority copy amplification

**OPEN — architectural replacement identified.** P323 removed whole-archive RAM duplication but each checkpoint still physically recopies the retained replication-authority byte history. P324 proved that simply replacing the log with one semantic snapshot does not close the worst case because replicated effects/frontiers are history-bearing causal semantics. The accepted direction is a persistent content-addressed authority-segment chain/tree with bounded delta publication and relocatable physical indexing. No generic "compact frames and fall back to old journal" path is accepted. See `docs/architecture/REPLICATION_AUTHORITY_COMPACTION.md`.

## P325 — executable replication authority segment foundation

**OPEN — identity/index/replay law executable; physical authority integration remains.** `CFAS` segments now bind exact existing replication frames to a parent `ReplicationAuthoritySegmentId`; `CFAI` provides a canonical relocatable index and rejects missing/unreachable parents, cycles, overlapping extents and conflicting duplicate IDs. Segment-chain replay is frame-bounded and feeds the maintained `ReplicationAuthorityJournal` replay semantics. Executable tests establish flat-history semantic equivalence, relocation invariance, exact-byte tamper rejection and parent-sensitive identity. The P323 product checkpoint path still recopies the historical authority section because SingleFile root/generation authority does not yet publish immutable external segment extents. P326 must integrate that physical authority with persistent O(new-delta) locator metadata and compaction closure before this payer can close. Rewriting the complete relocatable index each checkpoint is explicitly rejected because that would itself accumulate O(N²) index-entry writes.

## P327 — authenticated immutable replication-authority objects

**OPEN — P326 security blocker closed at the object layer; physical activation remains.** The rejected P326 candidate demonstrated O(new-delta) `CFAS` + linked locator publication but would have placed external authority payloads outside AE v1. P327 adds a dedicated HKDF-separated `ImmutableObject` AE domain and bounded 64 KiB `CFAO` object stream. AAD binds object kind, plaintext `SegmentId`, parent `SegmentId`, plaintext length and chunk identity while excluding physical location, so compaction can relocate exact authenticated bytes without decrypt/re-encrypt. Encrypted/plaintext object modes are fail-closed rather than fallback routes. `ReplicationAuthoritySegmentId` remains nonce/relocation invariant and object replay still feeds the maintained two-pass segment verifier/evaluator. The P323 archive-copy product path remains authoritative until P328 restores linked locator/root publication and segment-aware compaction on top of this boundary.

## P455 — live-ref exact-delta revalidation

Evidence reopened `kernel-model` / `kernel-revision` under the P454 live-ref optimization. One correctness seam was found and closed: append-only Bag revisions must advance compact live-ref row-coordinate authority even for non-ref appended rows, otherwise a later ref-bearing delta can receive a stale logical ordinal. Regression coverage now fixes this law. Sparse/dense release diagnostics show no O(total relation rows) payer in ordinary exact-delta maintenance. `kernel-plan --lib` returns to 297 passed / 7 ignored. The live-ref optimization line is FROZEN again unless new correctness or measured scaling evidence appears.

## PASS471 — pre-release compatibility hostile sweep
- CLOSED: Rust `DatabaseContext` alias removed; `Context<M>` is canonical concrete product type.
- CLOSED: obsolete Many backlink declaration/metadata (`via_field`) removed.
- CLOSED: retirement-only semantic-statistics advisor removed; manual statistics capability remains independent.
- FINDING: `MaterializedSemanticIndexState` / `LegacyIndex` is not a harmless compatibility adapter. It is a complete superseded physical execution family with advisor, persistence recipe, maintenance and runtime capability fallback. Partial removal would create dead/mixed architecture. Remove end-to-end in P472; do not add new consumers.


## PASS472 — legacy semantic-index family removed
- CLOSED: `MaterializedSemanticIndexState` / `LegacyIndex` was removed end-to-end rather than retained behind routing.
- CLOSED: old semantic-index advisor, durable artifact recipe, delta maintenance and capability fallback removed.
- VERIFIED: SAMF/`ObservableAtom` is the only persisted semantic-fiber execution family; exact active-crate symbol sweep finds no `LegacyIndex`, `UnifiedArtifactId::SemanticIndex`, `DurablePhysicalArtifactSpec::SemanticIndex`, `install_semantic_index` or `semantic_index(...)` execution path.
- HOSTILE TEST FIXTURE LAW: ObservableAtom has a durable canonical-key core and therefore rehydrates; deferred/rebuild policy tests must use rebuild-only surviving artifacts when they intend to measure rebuild work.
- NEXT PAYER: P465 source-epoch proof remains O(old-epoch history depth) across schema boundaries because P462 structural support/action roots are current-epoch only.

## P473 — schema-epoch stale-intent hostile closure
- CLOSED: O(old-epoch history depth) field-intent proof via `revision_transition_records_back_to` + linear `certify_old_epoch_field_segment`.
- CLOSED: old-schema `revision_at()` reconstruction on the field-intent commit hot path.
- SELECTED: one retained immutable P462 epoch root (support/action timelines + structurally shared field-value root) under the existing historical owner.
- REJECTED: mutable witness cache, serialized compatibility cache, old-schema query fallback/router, duplicated merge/conflict engine.
- OPEN: row-local relation exact effects and then general query/auth provenance.
