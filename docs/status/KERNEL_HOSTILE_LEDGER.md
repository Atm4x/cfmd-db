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

## PASS474 — row-local relation transport / runtime-realization publication seam
- CLOSED: old-epoch relation certification no longer scans the complete retained causal suffix or reconstructs an old `Revision`; retained P462 support/action roots are the proof authority.
- CLOSED: exact row-local relation delta transport uses P464 migration transport and fails closed for general relational slices.
- CLOSED: relation residual retry identity now includes the client's formation semantic revision.
- HOSTILE FINDING / OPEN: after semantic A->B cutover with physical convergence lag, `prepare_revision_derived` still assumes the physical relation accepts the current-B delta directly. A type-changing stale write therefore reaches `PhysicalTypeMismatch` only at publication. Full materialization is rejected as an O(data) fallback. Integrate the existing factorized B-native relation overlay into runtime physical publication.

## PASS474 FINAL — RowStore shape authority correction
- CLOSED: retained-epoch row-local relation exact effects now survive type-changing schema migration and reopen end-to-end.
- CORRECTED FINDING: the checkpoint attribution to lagging A-shaped runtime physical authority was false. Instrumentation showed current B/F64 physical rows and a correct B/F64 transported delta before the failure.
- ROOT CAUSE: `NativeRelation::RowStore` derived arity from the first row. `remove(last)` therefore made arity zero and the immediately following exact replacement insert failed `PhysicalTypeMismatch`.
- SELECTED LAW: physical relation shape is representation authority independent of cardinality. RowStore stores `column_count`; empty cardinality does not erase arity.
- REJECTED: migration-specific overlay/router/materialize-before-write workaround for this defect. Existing realization-overlay algebra remains reserved for actual derived physical authority, not used to mask a local representation invariant failure.
- VERIFIED: active A(I64)->B(F64) reopen hostile regression passes; focused empty-RowStore shape regression passes; kernel-plan full gate passes.
- NEXT: general relational/query guard provenance across migration, exact/fail-closed before authorization transport.

## P479 hostile finding — schema boundary and retained source authority

- REJECTED: keeping a formation-world guard executable across B/C epochs via query/provenance/derivative transport.
- SELECTED: exact formation-world serialization seal followed by forward effect-only transport.
- FIXED: migration candidate construction started with an empty historical index; schema sealing now starts from the persistent source historical root before target-epoch reset.
- RULE: B-native changes after the formation seal cannot retroactively invalidate an A transaction guard. Conditions requiring current-head validity belong to a distinct publication authority.


## PASS480 — B-native causal observation authority for retroactive sealed-effect reorder

### CLOSED THIS PASS
- Hostile audit confirmed PASS479's formation seal alone was insufficient: a sealed old effect physically published after a later B transaction could otherwise invalidate an observation that B transaction causally relied on.
- Added durable causal-observation coordinates to committed relation/mixed intents, explicitly separate from `ClientIntentGuardDigest` retry identity.
- Added reconstructible persistent current/retained-epoch observation timelines over the same durable causal authority; no second history store.
- Schema-aware post-boundary braid now checks transported sealed writes against observations of logically later B/C transactions. Overlap is reported as coordination-required, not falsely claimed to be an exact semantic conflict.
- Hostile reopen regression proves: B reads password=OLD then writes generation; late sealed A password:=NEW cannot be retroactively inserted before B.
- Preserved PASS479 law: later B writes with no causal observation of the old effect remain reorderable when normal effect laws certify them.

### OPEN — IMMEDIATE
1. Lower product `Transaction::require` canonical requirement footprints automatically into durable causal-observation authority; current kernel path accepts explicit observation footprints.
2. Derive exact observation-preservation/commutation certificates so coordinate overlap need not always require coordination when the earlier write provably preserves the later predicate.
3. Introduce distinct current-world publication preconditions for semantics that must hold at publication HEAD; do not reuse formation guards.
4. Extend B-native causal observation authority to general relational/OFC requirements without reintroducing B->A or old-guard transport.

### SUPERSEDED / DO NOT EXTEND
- Treating write/write post-boundary rebase alone as sufficient proof for retroactively inserting a formation-sealed effect before later current-world transactions.
- Reinterpreting this B-native anti-dependency proof as transport/revalidation of the old A guard.

### PERFORMANCE BASELINES TO PRESERVE
- Causal observation lookup is coordinate-indexed persistent-tree authority; no linear history scan.
- Observation timelines are reconstructible from durable committed intent and retained epoch roots; no independently serialized witness cache/history.
- Ordinary same-schema stale transactions do not pay the retroactive-seal anti-dependency rule unless logical order has already been fixed before a crossed schema boundary.

### NEXT RECOMMENDED PASS
**PASS481 — product `Transaction::require` causal-observation lowering + exact observation-preservation/commutation law.** Read `PROJECT_RULES.md` first and carry that requirement into every successor PASS.


## PASS484 — Interned causal-group routing / general OFC hostile boundary

### CLOSED THIS PASS
- P483 reconstructible group routing no longer clones the full grouped predicate/vector under every member coordinate. One payload is interned by `(effect_id, group_id)`; coordinate timelines store only `u32` group ids.
- Retained schema epochs structurally share the interned group pool instead of multiplying payloads at cutover.
- Structural hostile scaling is exact: widths 8/64/512 retain 1 payload, N payload fields and 2N routing refs (`Field` + `ObjectField`), replacing the previous O(N²) payload duplication.
- Executable join hostile proves `query + observed OFC + source-relations` is insufficient for general relational causal preservation; hidden Γ-DTC state is a mandatory part of any exact certificate.

### OPEN — IMMEDIATE
1. Build a shared/interned persistent relational causal capsule lineage over exact `MaterializedRelPlanState`/Γ-DTC state, with one capsule payload shared by observations rather than one O(data) snapshot per transaction.
2. Make that capsule lineage reconstructible across reopen from existing durable causal authority without per-braid query replay or an independently serialized second history store.
3. Add relation/source-occurrence routing and hostile scaling for many relational observations sharing hot source relations.
4. Current-world publication preconditions remain a separate authority from formation guards.

### OPEN — DEFERRED / RETURN AFTER CURRENT LINE
- Context reference/relationship DX and field-granular certified changes.
- DB-owned granular authorization.
- General deterministic Semantic Rules/invariants.
- Migration frontend and final Python/.NET/Studio/CLI surfaces; backup/recovery UX; public perf/binary budgets; Windows secure memory.

### SUPERSEDED / DO NOT EXTEND
- Copying one full `RuntimeJointCausalObservationGroup` into every member-coordinate timeline.
- Treating relational source-relation envelopes or observed OFC keys as exact causal conflict authority.
- Per-transaction full maintained-query snapshots, global query replay, relation-wide coarse conflicts, or any P475-P478 old-guard transport architecture.

### PERFORMANCE BASELINES TO PRESERVE
- Group payload count is O(groups), not O(groups × width); member routing refs are O(total memberships).
- One touched group is evaluated once per braid.
- Future relational capsules must be persistent/shared; no O(source-data) duplication per committed observation.
- No global history scan or full query recomputation on the retroactive hot path.

### NEXT RECOMMENDED PASS — PASS485
Shared persistent relational/OFC causal capsule lineage and reopen reconstruction. Read root `PROJECT_RULES.md`, PASS484 report, active ledger tails, and relevant R&D before implementation; carry this instruction forward again.
