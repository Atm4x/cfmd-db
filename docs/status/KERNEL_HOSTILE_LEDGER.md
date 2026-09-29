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
