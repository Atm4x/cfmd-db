# Kernel Hostile / Freeze Ledger

## Current hostile frontier — PASS589 reconciliation

- P324/P325/P327/P328 replication-authority checkpoint payer is CLOSED by PASS588.
- No generation-local monolithic replication archive or fallback replay family remains active.
- PASS589 proved cut-only external freshness insufficient for total Durable -> Volatile and selected one no-fallback extension: signed `VolatileFence` persistence-lineage authority. Public DX remains absent until PASS590 makes the theorem executable.
- Semantic-fiber retention synthesis remains paused after PASS575/PASS581 unless concrete performance evidence reopens it.

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

**CLOSED by PASS588 — historical discovery.** P323 removed whole-archive RAM duplication but each checkpoint still physically recopies the retained replication-authority byte history. P324 proved that simply replacing the log with one semantic snapshot does not close the worst case because replicated effects/frontiers are history-bearing causal semantics. The accepted direction is a persistent content-addressed authority-segment chain/tree with bounded delta publication and relocatable physical indexing. No generic "compact frames and fall back to old journal" path is accepted. See `docs/architecture/REPLICATION_AUTHORITY_COMPACTION.md`.

## P325 — executable replication authority segment foundation

**CLOSED by PASS588 — foundation retained in the selected path.** `CFAS` segments now bind exact existing replication frames to a parent `ReplicationAuthoritySegmentId`; `CFAI` provides a canonical relocatable index and rejects missing/unreachable parents, cycles, overlapping extents and conflicting duplicate IDs. Segment-chain replay is frame-bounded and feeds the maintained `ReplicationAuthorityJournal` replay semantics. Executable tests establish flat-history semantic equivalence, relocation invariance, exact-byte tamper rejection and parent-sensitive identity. At P325 time the P323 product checkpoint path still recopied the historical authority section because SingleFile root/generation authority did not yet publish immutable external segment extents. That historical activation debt is now closed by PASS588. Rewriting the complete relocatable index each checkpoint is explicitly rejected because that would itself accumulate O(N²) index-entry writes.

## P327 — authenticated immutable replication-authority objects

**CLOSED by PASS588 — authenticated object layer is physically active.** The rejected P326 candidate demonstrated O(new-delta) `CFAS` + linked locator publication but would have placed external authority payloads outside AE v1. P327 adds a dedicated HKDF-separated `ImmutableObject` AE domain and bounded 64 KiB `CFAO` object stream. AAD binds object kind, plaintext `SegmentId`, parent `SegmentId`, plaintext length and chunk identity while excluding physical location, so compaction can relocate exact authenticated bytes without decrypt/re-encrypt. Encrypted/plaintext object modes are fail-closed rather than fallback routes. `ReplicationAuthoritySegmentId` remains nonce/relocation invariant and object replay still feeds the maintained two-pass segment verifier/evaluator. At P327 time the P323 archive-copy product path remained authoritative pending physical activation. PASS588 verifies that linked locator/root publication and segment-aware compaction are now the sole active path.

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

## PASS512 hostile closure — relational formation global-history payer
- REJECTED: per-intent `revision_transition_records_back_to` replay for formation relational capsules; exact but scales with unrelated retained history and clones durable effect records.
- SELECTED: persistent relation-delta lineage inside the existing historical/retained-epoch derived index. One exact delta is indexed once at commit/recovery; formation capsules select only source-relation timelines.
- REJECTED: memoization/cache/read-set shortcuts. The selected authority remains exact Gamma-DTC and reconstructible from durable effects.
- OPEN: tie lineage reclamation exactly to historical/epoch reachability so retention release drops no live proof and leaks no dead lineage.

## PASS567 — kernel version/compatibility re-inventory

- Added authoritative `docs/status/KERNEL_VERSION_COMPAT_INVENTORY.md`; future `vN`/legacy/compat findings must be classified there immediately.
- CLOSED: full-dependency `kernel-durability` Clippy debt on sealed external-freshness checkpoint publication. One logical checkpoint authority cut now travels through `SingleFileCheckpointAuthority`; no suppression added.
- CLOSED PASS568: no historical checkpoint/realization decoder stack remains. Because CFMD is still pre-release, the sole current checkpoint and realization discriminators were normalized from pass-era `7`/`4` to `1`; obsolete snapshots intentionally fail closed.
- VERIFIED: query exact-to-“legacy” adapters and BFC legacy mask are `cfg(test)` oracles only; no production fallback route.
- VERIFIED: PASS472 `LegacyIndex` family remains absent from active source.
- OPEN / ACTIVE COMPATIBILITY DEBT: `RuntimeRevisionCell::commit_revision_durable_full_exact` is still production-reachable through generic `DurableRuntime::commit_revision`; hostile-classify/remove only after all legitimate strict-full-revision callers are mapped to current intent-specific APIs.
- OPEN / CURRENT ALTERNATE CAPABILITY: advisor-owned `SemanticStatistics` / `SemanticQuotientFactor` are called legacy during ObservableAtom convergence but remain production inputs to cyclic optimizer/admission. Do not delete until ObservableAtom/SAMF can own those semantics directly.
- CLOSED (PASS567 full-Clippy follow-up): `kernel-plan` volatile-owner probe used `Result::map(...).unwrap_or(false)`; replaced with `is_ok_and`, preserving fail-closed lock-error behavior.

## PASS569 — semantic artifact capability convergence

- CLOSED: cyclic optimizer/admission artifact-name dependency on `SemanticStatistics`; planning consumes representation-neutral exact cardinality.
- CLOSED: quotient projection is recognized as an exact-cardinality provider; retaining quotient state no longer requires a duplicate statistics state for costing.
- CLOSED: SAMF/ObservableAtom capability declaration now includes quotient-fiber + exact-cardinality semantics already implemented by its fiber state.
- OPEN: family-specific advisor/durable/install vocabulary remains physical-profile debt. Converge it under one semantic-fiber identity/profile only after benchmarked resource equivalence; do not create a full-SAMF-only memory regression.

## PASS570 — semantic-fiber profile hostile closure
- CLOSED: separate `UnifiedArtifactId` variants for statistics/quotient/SAMF; one semantic-fiber identity now carries a physical profile.
- CLOSED: quotient advisor under-declared its exact-cardinality capability; all profile capabilities come from one lattice owner.
- CLOSED: inherited PASS568 retry regressions; they now test exact derived durable identity directly.
- HOSTILE MEASUREMENT: retained memory is non-monotone with capability strength (39,456 / 848,368 / 230,192 B for Cardinality/Quotient/Observable in the measured fixture). Profile selection must use real resource/work estimates, not family rank.
- OPEN: durable spec/install structs remain profile-specific physical implementations; do not delete until build/update frontier is measured.


## PASS571 — semantic-fiber physical lowering hostile result

- CLOSED: per-row owned canonical tuple duplication is not necessary for exact quotient routing; R&D carrier shares interned joint-class signatures.
- CLOSED: projected profile must not duplicate row membership per coordinate; projection can retain incidence to joint fibers and derive row unions exactly.
- OPEN: ProjectedFibers still retains ~1.74x current SAMF bytes on the 4096x257 probe despite faster build; SAMF removal is blocked on broader frontier evidence.
- OPEN: Measure carrier currently loses the compact specialized statistics point; do not collapse that production layout merely for structural uniformity.

## PASS572 hostile closure — semantic-fiber over-retention
- REJECTED: preserving quotient row buckets that are not observed by its production consumers.
- SELECTED: one exact `RowKeyMassRetention` law sharing each live canonical joint-key allocation between row routing and joint mass.
- FOUND/FIXED: SAMF retained-byte accounting omitted `RevisionObservableCatalog` heap.
- REJECTED: treating named profile rank as the observation lattice; exact capabilities are a partial observation algebra.
- MEASURED: ProjectedFibers is ~1.63x SAMF bytes at `D<<N`, but ~0.55x at `D~=N` and ~0.50x on the Cartesian/large-key probe. No fixed universal lowering is justified.
- NEXT: retention-plan compiler over exact resource atoms and witnesses; no semantic fallback branch.

## PASS573 — semantic-fiber retention compiler hostile closure

- CLOSED: observation-global read cost assumption. Read work is now owned by an exact realizer witness.
- CLOSED: ambiguous `ExactCardinality` demand. `GlobalCardinality` is distinct from keyed `JointMass`.
- CLOSED: unindexed aggregate slot demand. `SlotMass(slot)` / `SlotRows(slot)` preserve coordinate identity.
- CLOSED: synthesis may not erase an existing fast specialization merely because another exact layout is smaller. Protected references plus conservative interval guards fail closed under uncertainty.
- OPEN: calibrate real statistics / RowKeyMass / SAMF / ProjectedFibers read envelopes and path-copy/transition peaks before runtime advisor publication.
- OPEN: decompose coarse physical states into shared atoms; do not replace them with another fixed profile enum.

## PASS574 — semantic-fiber shared-key hostile calibration

### LEDGER — GOAL
Test whether the shared canonical row-route point can be admitted without hiding maintenance prerequisites or erasing a protected latency region.

### LEDGER — SELECTED IMPLEMENTATION
Use explicit maintenance-closed resource atoms and conservative multi-run release envelopes. Treat overlap as non-dominance.

### LEDGER — IMPLEMENTED
- `CanonicalJointKeyPool` owns canonical payload + exact joint mass; `SharedRowKeyRoute` owns only row routing.
- Compiler resource dependencies are transitively closed and cyclic dependency graphs fail closed.
- Four optimized harness invocations show shared routing materially below SAMF, but owned/shared envelopes still overlap for I64 and Text256; no protected route is deleted.

### OPEN
Calibrate N:D/key-size/churn/snapshot/transition axes and independent shared joint-row ownership before runtime plan publication.

## PASS575 — semantic-fiber crossover correction + arbitrary-target visibility closure

- HOSTILE: fair shared-key accounting shows a real crossover, not universal dominance. Low-reuse/small-key shared routing can regress build/update/RSS; large/high-reuse keys can dominate. A sole shared production quotient engine is therefore rejected for now.
- SELECTED: production quotient returns to an explicit direct owned row-key engine plus exact joint mass; shared retention remains a separate candidate engine. The obsolete quotient joint-row bucket stays deleted.
- CLOSED: `RevisionTransitionRequest` has no active external crate consumer and is no longer publicly re-exported. It remains crate-private for internal arbitrary-target preparation/repair and tests; durable relation publication remains derived source+delta authority.
- POLICY: freeze further semantic-fiber production expansion until R&D stabilizes. Prefer temporary physical-engine multiplicity over premature optimizer/routing complexity.
- NEXT: leave the semantic-fiber line and resume an independent productization ledger item.

## PASS576 — recovery diagnostics are projections, not recovery policy

- HOSTILE finding: product reopen/verify/restore collapsed typed kernel failures into prose `ErrorKind::Recovery` messages.
- CLOSED: public `RecoveryDiagnostic` now projects operation + authority + stable reason + exact offset/version when present; no debug-string parsing is used.
- CLOSED: diagnostic projection changes no FORMAT V1 bytes and no recovery/restore acceptance rule.
- REJECTED: salvage, guessed rollback, frontend-specific recovery classifiers, or a fake public DR operation before the certified freshness-transfer kernel path has a real product owner.
- OPEN: expose the sealed external-freshness transfer through product authority/DX, then attach the same diagnostic projection to its actual failure surface.

## PASS577 — product authority-transfer boundary
- CLOSED: sealed external-freshness transfer now has a real product owner; PASS576 no longer needs a fake DR diagnostic vocabulary.
- CLOSED: source retirement is enforced structurally by exclusive runtime admission plus in-place owner/path transfer. Any live Database/Context/Snapshot/Plan alias rejects before staging.
- CLOSED: restricted transfer requires `DatabaseAdministration::AuthorityTransfer` before encryption/provider resolution or target I/O.
- CLOSED: freshness-aware product reopen reuses the ordinary runtime recovery projection; no second recovery engine was introduced.
- OPEN: productize `PersistenceTransition` and later `ProtectionReconfigure` under the same pre-side-effect administration law.

## PASS578 — Memory -> Durable product boundary
- CLOSED: the proposed PASS577-style exclusive-owner rule was hostile-rejected for persistence promotion. Memory -> Durable is representation-only and must preserve live Context/read/watch/runtime identity; unlike freshness transfer it retires no source trust authority.
- CLOSED: `cfmd-runtime::Database` no longer caches a persistence path per clone. Location is projected from `DurableRuntime`'s authoritative persistence owner, removing stale facade state after promotion/transfer.
- CLOSED: restricted `PersistenceTransition` authorization is checked before target encryption-provider resolution or target staging.
- CLOSED: public memory creation uses the existing volatile durability owner; no fake temporary file/backend and no second semantic engine were introduced.
- DEFERRED: Durable -> Volatile remains forbidden until protection-floor + external-freshness import authority is proved.
- INHERITED TEST DEBT: `cfmd-runtime --all-targets` has one pre-existing unrelated failure, `object_first_schema_query_and_plan_results_need_no_relation_plumbing` (`example.user has no identity field`), reproduced unchanged on PASS577.


## PASS579 — protection reconfiguration product boundary
- CLOSED: restricted protection change now requires exact `DatabaseAdministration::ProtectionReconfigure` before provider interaction; generic Read/Write and `SchemaMigrate` do not imply it.
- CLOSED: product naming is operation-centric (`reconfigure_protection`) rather than implementation-centric (`rewrap_encryption`).
- HOSTILE FIX: expected caller-invalid rewrap requests (non-wrapped source/target or AEAD mismatch) are rejected before the durability poison-on-error boundary. A bad administration request no longer marks healthy committed storage as poisoned.
- VERIFIED: valid provider-backed rewrap still uses the existing dual-slot DMK handoff, external acknowledgement and predecessor retirement theorem; generation/WAL ciphertext remains untouched.
- OPEN: frontend/protocol projections must preserve these exact authority/diagnostic distinctions rather than flattening them into strings or generic admin.

## PASS580 — value-object/entity mutation boundary hostile closure
- CLOSED: identity-free `cfmd_object!` deletion no longer requires semantic identity; exact full-shape value objects delete by selected persisted row value.
- PRESERVED: identity-bearing entities still delete through stable semantic identity and lifecycle-aware canonical row resolution.
- REJECTED: inferring identity from a field named `id`; only explicit entity metadata owns identity semantics.
- REJECTED: `save_as = backup + open`; canonical persistence images carry live retry/prepared/replication authority and are not a concurrent-fork theorem.
- CLOSED: the inherited PASS578/PASS579 `cfmd-runtime --all-targets` failure is gone.

## PASS581 — catalog-free SAMF / shared atomic semantic-class authority
- CLOSED: production SAMF no longer owns a second `RevisionObservableCatalog` / product `EqClassId` namespace; exact joint/projection support is reconstructed from one shared revision/store atomic semantic-class authority plus local non-semantic fabric atom IDs.
- CLOSED: JoinEq, Group, Distinct and equality-filter execution consume the same encoded semantic lanes; no new planner semantic family or SQL/generic equality fallback exists.
- HOSTILE FIX: `SemanticRevision` equality alone was insufficient for encoded-lane reuse. A same-number revision with changed Gamma/equivalence semantics could otherwise consume stale physical classes. Every lane now carries the compiled-equivalence witness and mismatches fail/rebuild instead of aliasing semantics.
- HOSTILE FIX: relation/layout replacement now invalidates encoded lanes and releases catalog references in the same candidate-store transition; stale physical owners cannot pin class liveness.
- CLOSED: shared semantic catalog + encoded lanes are counted once as `SemanticClassSubstrate`; SAMF memory accounting no longer duplicates shared authority per retained consumer.
- REJECTED: wholesale overlay of the cumulative R&D workspace after it was shown to regress newer production fixes such as PASS572 memory accounting. Integration is selective over PASS580 mainline.
- PRESERVED: durable ObservableAtom payload remains canonical-key tuples; physical `EqClassId` remains reconstructible/non-durable.
- NEXT HOSTILE LINE: live copy/fork must not duplicate retry/prepared/replication/external-freshness authority under a convenience `save_as` API.

## PASS582 — security root-of-trust split
- CRITICAL CLOSED: `SchemaMigrate` and database administration are no longer derivable from mutable `Schema.Access`; client roles cannot promote themselves into control authority.
- CRITICAL CLOSED: migration programs/models no longer carry self-authored access approvals. Kernel transport reports access-policy change; control-plane authority decides whether it may proceed.
- CRITICAL CLOSED: policy widening requires independent `AccessPolicyAdmin` + `Declassify`; publication alone is insufficient.
- HIGH CLOSED: data-sensitive prepare requires `MigrationDataInspect`; static validate remains data-blind.
- HIGH CLOSED: role-bound sessions retain external RoleIds and automatically re-resolve on authoritative schema revision change; narrowing no longer depends on best-effort host refresh.
- FORMAT: no version bump. Pre-release FORMAT V1 simply retires the obsolete SchemaMigrate permission tag and migration approval payload.
- OPEN: exact `MigrationSecurityImpact`/noninterference proof and external control-credential authenticator lifecycle.

## PASS583 — migration noninterference / impact sealing
- CLOSED: declassification is now derived from exact per-role observation factorization, not coarse access-change kind.
- CLOSED: unchanged `ReadRelation` over a value-changing relation migration is detected as a declassification edge.
- CLOSED: write-only policy widening remains policy administration and does not falsely require declassification authority.
- CLOSED: data-inspection authority is conditioned on exact transform/integrity source-data dependencies rather than every prepare call.
- CLOSED: security-impact identity uses the durable canonical migration-program codec plus source revision and migration id.
- OPEN: external exact-impact approval token and hosted control-credential authenticator lifecycle.


## PASS584 — exact approval provenance / control credential lifecycle
- CRITICAL CLOSED: database-control operation sessions no longer own claim-rotation authority. Mutable lifecycle authority is isolated in `DatabaseControlCredential`; derived `DatabaseControlSession` is use-only.
- CRITICAL CLOSED: migration security approval is no longer an implicit re-check of the publisher's permissions. Policy/declassification approval is a separate sealed `MigrationSecurityApproval` and may originate from another control principal.
- CLOSED: approval binds exact complete `MigrationSecurityImpact` + digest + source revision + migration id + live database runtime identity + approver generation.
- CLOSED: rotation or revocation after prepare invalidates the embedded approval at execute before publication.
- CLOSED: identical source/program digest on a second live database cannot replay the approval.
- PRESERVED: `Schema.Access` remains client/data-plane only; no Schema role/capability can issue or mutate database-control claims.
- DEFERRED: wire-level credential authentication/signing format. Any future transport must resolve into the same credential/session/generation law rather than inventing frontend authority semantics.
- NEXT HOSTILE LINE: live copy/fork operational authority must be re-founded rather than cloned.

## PASS587 — kernel-plan post-catalog-free owner revalidation

**CLOSED — inherited verification debt, no semantic regression.** The eight failures carried by PASS585/PASS586 reproduced on untouched PASS586 and all traced to assertions that encoded the pre-PASS581 physical winner. Revision semantic encoded lanes now legitimately satisfy ordinary Join/Filter/multiway execution before older SAMF/QCN/ephemeral paths. QCN retained state itself remained delta-exact: direct QCN execution after the same mutations matched rebuilt state and consumed the maintained support. Tests now distinguish semantic correctness/capability consumability from optimizer owner choice. Full `kernel-plan --lib` returns green at 316 passed / 0 failed / 11 ignored. Strict `kernel-plan --all-targets --no-deps` Clippy is also clean after structural ownership/API hygiene fixes with no suppression.

**HISTORICAL PASS587 NEXT (CLOSED by PASS588):** P327/P328 replication-authority physical activation was the next payer at PASS587 freeze. PASS588 closed it; do not use this sentence as current work authority.


## PASS588 / PASS589 ledger reconciliation — replication-authority payer CLOSED

- CLOSED: P324 copy amplification. Ordinary checkpoint publication is O(new replication-authority delta), not O(retained history).
- CLOSED: P325/P327 physical-activation debt. Single-file publication/recovery/compaction use root-reachable authenticated immutable `CFAS` segments in `CFAO` objects with `CFLN` locator roots.
- CLOSED: unreleased generation-contained `ReplicationAuthority` section discriminator removed from active grammar; there is no monolithic archive fallback route.
- VERIFIED law: an empty replication-authority delta preserves the existing immutable authority root; rotation does not synthesize another object.
- COMPACTION boundary: work is O(reachable retained authority) because exact authenticated bytes of the live closure must be relocated before old extents can be reclaimed. This is a semantic/I/O boundary, not the former checkpoint payer.
- PASS589 RESULT: the current cut-only authority grammar cannot preserve external freshness across a pure volatile interval. Selected successor is `DurableCut -> VolatileFence -> DurableCut`, with protection floor carried in the existing volatile backend and no second evaluator. PASS590 is the current durability R&D implementation line.


## PASS590 HARD-STOP CHECKPOINT — 2026-10-07

PASS590 is in progress. Authority grammar is now `DurableCut | VolatileFence`; in-place Durable -> Volatile demotion and old-source rollback exclusion are implemented and focused-tested. Do not treat PASS590 as complete. Next is exact fenced repersistence, then runtime projection. PASS591 is the mandatory hostile cleanup/consolidation checkpoint before any new major feature line. See `docs/reports/current/PASS590_REPORT.md`.

## PASS590 — persistence-lineage authority executable

- CLOSED: signed external freshness is one `DurableCut | VolatileFence` state machine; no cut-only fallback grammar remains.
- CLOSED: externally anchored Durable -> Volatile fences the old durable predecessor before in-place backend demotion; volatile commits do not contact freshness authority.
- CLOSED: Volatile -> Durable stages/reopens/verifies bytes first, then exact-CAS consumes the fence, then swaps the sole live persistence owner.
- CLOSED: CAS response loss is reconciled by exact signed-state reread; stale/concurrent successor ambiguity poisons the source fail-closed.
- HOSTILE FIX: removed `RuntimePersistenceAuthority::{Volatile, Durable}` because it duplicated backend state and could disagree after in-place demotion.
- PRODUCT: `Database::make_volatile()` is guarded by `PersistenceTransition`; `persist()` repersists the same fenced lineage.
- NEXT CHECKPOINT: PASS591 is mandatory hostile cleanup/consolidation. No new feature line before stale cut-only vocabulary, duplicate persistence abstractions, generic fallbacks and persistence-image cost are audited.


## PASS591 — persistence-transition hostile consolidation

- CLOSED: removed the now-empty `RuntimePersistenceAuthority` tuple wrapper. `DurableRuntime` owns `Mutex<DurableRevisionStore>` directly; backend is the only persistence-state authority.
- CRITICAL CLOSED: directory/single-file demotion could retire the physical backend while retained historical epoch anchors still depended on that backend. Demotion now materializes every disk-only retained historical closure into `portable_historical_epochs` before fence publication/backend retirement.
- VERIFIED: directory migration history survives exact Durable -> Volatile -> single-file Durable repersistence.
- CORRECTED COST LAW: demotion is O(1) only without disk-only retained history; otherwise O(bytes of retained historical closure). No current-row rebuild, logical export/import, or fallback path is introduced.
- CLASSIFIED: `compare_and_rebind_signed` remains a distinct live authority-transfer operation across lineage/store identities; it is not the persistence-lineage `VolatileFence` CAS and is not stale compatibility routing.
- OPEN NEXT PAYER: `CanonicalPersistenceImage` still carries replication authority by portable journal frames and target staging replays them. The exact `ReplicationAuthoritySemanticSnapshot` already proves physical-frame history is not semantic authority. PASS592 should replace transfer-time frame-history payment with one canonical semantic authority carrier, targeting O(retained semantic authority) rather than O(physical journal history).
- CHECKPOINT: PASS591 closes the mandatory cleanup freeze. New feature work may resume only from the reconciled PASS592 frontier.

## PASS592 HARD-STOP CHECKPOINT

- CLOSED architecturally: canonical persistence transfer no longer carries/replays physical replication append history.
- SELECTED: exhaustive semantic snapshot -> deterministic bounded semantic-base carrier -> existing replication evaluator -> existing authenticated `CFAS/CFAO` objects.
- VERIFIED: 256 superseding term promises yield a semantic carrier < 1/8 of physical append-history bytes; full kernel-durability 280/0/2.
- NOT COMPLETE: split carrier encode/decode structurally until strict Clippy is green; no suppression/fallback.
- NEXT CLEANUP: PASS593 audits peak-memory/large-carrier bounds and residual physical-history payers after PASS592 completion.

## PASS592 COMPLETE — semantic-carrier branch frontier

- CLOSED: canonical persistence transfer no longer repays portable replication append history; exact current `ReplicationAuthoritySemanticSnapshot` is the transfer authority.
- CLOSED: semantic-base restore uses the same replication evaluator and authenticated immutable `CFAS/CFAO` path; no second evaluator/history fallback exists.
- CLOSED: carrier codec strict-Clippy debt; encode/decode are split by causal, membership/ordering, membership-vote, election/decision, security, recovery/quorum and publication authority families.
- VERIFIED: kernel-durability 280/0/2, kernel-plan 316/0/11, cfmd-runtime 30/0, workspace check and strict Clippy PASS.
- OPEN MEASURED PAYER: encoded semantic carrier is still temporarily owned alongside decoded semantic authority. Next R&D may pursue bounded-memory streaming verification only if verify-before-publish semantics remain exact and no second evaluator appears.
- ROADMAP CORRECTION: PASS593 is not automatically a cleanup PASS. Cleanup/Git checkpoints are selected at architectural branch boundaries when accumulated work warrants consolidation. PASS591 is the last clean/Git-ready checkpoint for the persistence-lineage branch.

## PASS596 CLEAN checkpoint
- CLOSED: buffered `SegmentPlan::from_frames/write_to` peer API; production publication is frame-source-only.
- CLOSED: test-only CFAO chain collection back into `Vec<Vec<u8>>`; authoritative tests bind/replay the physical root instead.
- CLOSED: blanket non-test dead-code suppression on replication segments. Raw CFAI codec/relocation and raw non-CFAO chain replay are explicitly `#[cfg(test)]`.
- VERIFIED: no CFAS-v1 compatibility path, no semantic-base full-buffer staging fallback, no second replication evaluator.
- RETAINED BY LAW: pending/live frame vectors represent actual incremental live replication deltas and streaming checkpoint cuts, not canonical semantic-base staging.
- CLEAN / GIT-READY: PASS596 closes the PASS592–PASS595 replication semantic-carrier branch.
- NEXT CANDIDATE BRANCH: PASS597 Γ-factorized aggregate productionization R&D, beginning with JointMass capability bridging and hostile admission frontiers.
