# PASS81 UNIFIED INTEGRATION SPINE LEDGER

Status: **FINAL — WRITE-R&D INTEGRATION CONVERGED**.
Baseline: verified Pass80 (`210d37f49afab42bfed730c0508439773fe4c0044e6e6f86496ac84da11a4d63`).

This ledger tracks the unified R&D integration spine against production. Status values:

- **PROD CLOSED** — production path exists and is verification-covered;
- **PROD PARTIAL** — production substrate/path exists with named remaining migration;
- **R&D CLOSED / PROD OPEN** — architecture is settled but implementation is absent;
- **SYSTEMS OPEN** — systems/performance/assurance implementation remains.

## Baseline inherited from Pass80

- APNF finite nonrecursive general multiway executor — PROD CLOSED for current fragment.
- SAMF finite fibers + direct planner consumption + durable ObservableAtom recipe — PROD PARTIAL.
- DTC compiled maintenance contract — PROD PARTIAL.
- GCC/BFC QCN insertion/deletion/resurrection — PROD CLOSED for current QCN maintenance.
- VMF candidate publication boundary — PROD PARTIAL.
- OFC runtime observation guard — PROD PARTIAL.
- Capability required-field obligations — PROD CLOSED.

## Pass81 active convergence targets

1. Unified cross-family physical candidate/advisor/recovery scheduling contract.
2. Shared resource-footprint and read/write/build work model.
3. SAMF Annotation/Ordered capability prerequisites via StructuralFold/StructuralOrdering/CertifiedFn where dependency-safe.
4. Architecture authority/dependency checks from the unified integration spine.
5. Streaming checkpoint/restart-poison policy only if it can be integrated orthogonally without destabilizing the semantic spine.

## R&D bundles checked

- Unified integration spine prototype: 8/8 release tests PASS; clippy/fmt PASS.
- Streaming checkpoint/restart-poison prototype: 9/9 release tests PASS; clippy/fmt PASS.


## Checkpoint A — unified advisor/recovery ontology — DEBUG VERIFIED

Integrated:

- family-neutral `PhysicalCapability` vocabulary;
- `PhysicalWorkEstimate { read_work_saved, maintenance_work, build_work }`;
- generic shared `ResourceFootprint<R>` with weighted-union marginal cost;
- unified hard-budget selector with explicit build/retain hysteresis thresholds;
- legacy semantic-index, I64-index, semantic-statistics and quotient-factor advisors now adapt to the same selector;
- physical recovery recipes use the same capability/work/footprint candidate shape while preserving manual-pin-first startup priority and existing independent budgets;
- legacy public advisor methods remain compatibility adapters; no duplicate semantic authority was added.

Verification at checkpoint A:

- `cargo fmt --all -- --check` PASS;
- `cargo check --workspace --all-targets` PASS;
- `cargo test --workspace --all-targets`: **516 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings` PASS.

Still partial:

- legacy adapters currently supply `maintenance_work = 0`; workload write telemetry is not yet wired;
- current legacy footprints use unique resource atoms; SAMF overlays have not yet exposed shared backing atoms to the selector;
- one top-level cross-family workload API is still absent, although family admission algorithms now share one kernel;
- exact hard-budget global optimum remains intentionally outside semantic correctness.

## Checkpoint B — read-gap closure — DEBUG VERIFIED

### Read gap 1: finite non-monotone exact reads — PROD CLOSED for current nonrecursive fragment

Integrated:

- first-class `RelExpr::Difference` and `RelExpr::AntiJoin`;
- Set Difference uses Γ-semantic support subtraction;
- Bag Difference uses exact monus per Γ-canonical row class: `max(L-R, 0)`;
- AntiJoin uses zero/nonzero right-side blocker semantics and preserves the complete left multiplicity when unblocked;
- first-class physical `Plan::Difference` / `Plan::AntiJoin` lowerings use the same exact `kernel-query` semantic helpers as the logical evaluator;
- Γ-DTC classifies both operators as `BlockerZeroCrossing` with `BlockerMass` state requirement;
- maintained-plan state has an explicit blocker node and correctness-first exact output recomputation after child deltas;
- transport and durable logical metadata know both operators.

Hostile/parity coverage includes:

- ASCII-CI Set Difference: semantic `"A" - "a" = absent`;
- Bag monus: `A×3 - a×2 = A×1` under ASCII-CI Γ;
- AntiJoin: duplicate right rows remain a boolean blocker while left multiplicity is preserved;
- physical execution == logical reference.

Named residuals, not silently closed:

- optimized SAMF blocker-mass/zero-cross overlay remains production OPEN;
- general recursive negation/stratification remains outside this finite nonrecursive closure;
- complement relative to arbitrary/infinite domains is not implied by Difference.

### Read gap 2: structural semantic ordering — PROD CLOSED for current structural value algebra

Integrated:

- `StructuralOrderingDef` for guarded `Mu/Var`, Product, Sum, Option, Seq, Set, Bag and Map;
- Product field order and Sum variant rank are explicit semantic inputs rather than host/BTree/SemanticId iteration order;
- primitive Unit/Bool/entity-id ordering leaves are available alongside existing I64/F64/Text orderings;
- structural ordering compiles to canonical semantic order-class keys distinct from physical tie-break keys;
- Γ equality/order congruence is checked on the revision-local support boundary;
- structural TopKWithTies consumes semantic order classes and preserves semantic ties;
- checkpoint format v2 persists structural ordering definitions while retaining legacy v1 decode.

Hostile/parity coverage includes:

- explicit Sum variant rank overrides `SemanticId` ordering;
- Product/Text ASCII-CI structural TopKWithTies returns both `"a"` and `"A"` in the k=1 tie class;
- checkpoint structural-order roundtrip;
- physical TopK result == logical reference.

Named residuals, not silently closed:

- SAMF Ordered overlay/range-index/pagination consumer migration remains production OPEN;
- global structural range-join optimization is not part of this checkpoint.

### Checkpoint B verification

- `cargo fmt --all -- --check` — PASS;
- `cargo check --workspace --all-targets` — PASS;
- `cargo test --workspace --all-targets` — **521 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS;
- no new lint suppressions were added;
- Cargo build output is external to the checkpoint workspace.

### Next branch after checkpoint B

Return to the write-side convergence frontier:

1. typed `FineChange` / first-class `Rewrite` calculus;
2. dependent writable Lens + complement calculus;
3. only then confluence/I-confluence and write-through view synthesis consumers.

Read-side follow-up such as SAMF Ordered/Annotation optimization remains tracked, but does not block beginning the write branch.

## Checkpoint C — typed FineChange / first-class Rewrite foundation

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

- `Change<T>` now has `Fine(FineChange<T>)` in addition to `NoChange/Replace`.
- `FineChange<T>` is an extensional typed effect with an authoritative absolute endpoint plus a non-authoritative structural `FineChangeKind`; the tag never substitutes for pinned Γ semantics.
- Existing exact-Rust `SetChange` and checked `SeqSplice` have compatibility adapters into universal `FineChange`.
- `Change::apply`, extensional composition and exact-representation normalization cover Fine changes.
- `RewriteSpecId`, `RewriteLawSetId`, `RewriteEffect<T>` and generic `PreparedRewrite<T,I>` establish first-class intent/law identity separate from endpoint effect.
- Γ transport preserves Fine kind + transported endpoint for `Value` and `FiniteModel`; Rewrite intent persistence is still OPEN.

Verification before checkpoint:
- `cargo fmt --all -- --check` PASS;
- `cargo check --workspace --all-targets` PASS;
- `cargo test --workspace --all-targets` PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` PASS;
- no lint suppressions added.

Still OPEN after C: structural Γ-aware Set/Bag/Map/Relation fine payloads beyond endpoint adapters; durable RewriteSpec/law identity; dependent Lens/complement; writable-view synthesis; law inference/confluence.

## Checkpoint D — dependent Lens/complement + Rewrite lift

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

- Existing structural `LensExpr` now exposes `split(source) -> (view, complement)` and exact `restore(view, complement)`.
- `LensComplement` is explicit logical authority for Identity, ProductField remainder, and composed dependent complements; it contains no physical handles.
- ProductField and Compose satisfy complement round-trip plus GetPut/PutGet/PutPut regression laws.
- Complement/lens shape mismatch fails closed.
- `LensExpr::lift_rewrite` lifts an intent-bearing `PreparedRewrite<Value,I>` from view to source, preserving `RewriteSpecId`, explicit inputs and `RewriteLawSetId` while deriving a Fine source effect through the complement.
- Legacy `get/put` remain adapters over split/restore.

Gate: workspace fmt/check/tests/clippy `-D warnings` PASS; no lint suppressions.

Still OPEN after D: type-level DependentLensSpec/CertifiedFn integration, query-level WritableViewPlan synthesis, migration complement capsules/durability, law inference/confluence and REIC durable Rewrite identity.

## Checkpoint E — structural WritableViewPlan compiler

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

- `compile_writable_query` compiles the exact scalar-query structural fragment `Input` / chained `ProductField` into `WritableViewPlan` backed by the dependent `LensExpr` from checkpoint D.
- Each writable plan pins one allowed `RewriteSpecId`; runtime lifting rejects another rewrite family before deriving a source effect.
- Constants fail with `ConstantHasNoSourceOwner`; derived SeqLength/SeqSum/Add/If fail with `DerivedOperatorHasNoCertifiedLift` rather than selecting an arbitrary preimage.
- The compiled plan lifts a PreparedRewrite through its complement and the resulting source state re-evaluates to the requested view endpoint.

Gate: workspace fmt/check/tests/clippy `-D warnings` PASS; no lint suppressions.

Still OPEN after E: relational Project/Filter/Join writable synthesis; APNF determinant lift proofs; VMF/DTC obligations on view write publication; rewrite footprints/law inference; complement capsule durability/GC; REIC persistence of intent identity.

## Checkpoint F — semantic Rewrite footprint / conservative law inference

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

- Added semantic write coordinates for product fields and Γ-addressed set/bag/map classes plus relation identity and sequence anchor coordinates; no physical row IDs are accepted.
- `RewriteFootprint` separates guard/read sensitivity, writes/action laws and invariant obligations.
- Conservative `infer_pair_rewrite_law` derives StrongCommute for separated writes only when cross-read guards are stable and no VMF obligation remains.
- Identical idempotent assignments / same Ensure actions are recognized; opposing assignments or EnsurePresent/EnsureAbsent are definite intent conflicts.
- Same certified commutative-add algebra is accepted on overlapping coordinates.
- Opaque overlap, cross-read sensitivity or undischarge invariant obligations remain `Unknown`; endpoint equality is never used for law inference.

Gate: workspace fmt/check/tests/clippy `-D warnings` PASS; no suppressions.

OPEN: DTC-backed dynamic guard certificates, VMF invariant-closure certificates, cube/coherence family certificates, residual rules, REIC consumption/persistence.

## Checkpoint G — RewriteSpec authority + migration complement capsule contract

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

- Added first-class `RewriteSpec { id, law_set, footprint }`; preparing an effect through the spec is now the canonical constructor for `PreparedRewrite`, so intent identity, law-set identity and structural footprint live in one semantic object.
- Pair-law inference is directly available between Rewrite specs and remains footprint/action-law based, never endpoint-sampling based.
- Added logical `ComplementCapsule` pinned by source/target schema revisions, LensSpec ID, semantic-manifest ID and encoding version with a logical `Value` complement only.
- Added explicit retention policy: Forever / UntilRevision / UntilEpoch / ExternalArchive / Forget.
- Added ordered `MigrationComplementChain` with fail-closed schema-continuity checking.
- No durability/WAL encoding of capsules is claimed yet; this checkpoint fixes the authority IR before persistence is added.

Final verification before source freeze for this 20-minute cycle:
- `cargo fmt --all -- --check` PASS;
- `cargo check --workspace --all-targets` PASS;
- `cargo test --workspace --all-targets` PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` PASS;
- no lint suppressions added.

### Write frontier after G

Integrated now: FineChange universal endpoint semantics; first-class RewriteSpec/PreparedRewrite identity; dependent Lens complements; Rewrite lift through Lens; first structural WritableViewPlan compiler; conservative semantic footprint/law inference; migration complement capsule/retention IR.

Still production OPEN: Γ-aware structural Set/Bag/Map/Relation fine payloads replacing endpoint adapters; relational writable Project/Filter/Join synthesis and APNF determinant obligations; DTC/VMF validation wired into write publication; durable persistence of Rewrite intent/law IDs and complement capsules; REIC consumption of inferred certificates; residual/cube coherence; historical schema restore API and complement GC enforcement.

## Checkpoint H — Γ-aware collection deltas + Rewrite publication boundary

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

- Added revision-local `SemanticClassCoordinate { observable, class }`; semantic class identity is never inferred from Rust equality/hash.
- Added Γ-addressed `SemanticSetChange`, `SemanticBagChange` and `SemanticMapChange` structural change algebras.
- Set removal/insertion is per semantic class; duplicate/mismatched source classes fail closed.
- Bag changes update multiplicity per Γ class with checked underflow/overflow and preserve an existing representative rather than treating representation as semantic identity.
- Map writes address semantic key classes; duplicate semantic keys and remove+upsert ambiguity fail closed.
- `RelationDelta::prepare_relation_rewrite` derives a Fine Relation endpoint through the existing pinned Γ relation-delta semantics and binds it to one `RewriteSpec`/law set.
- Added `RevisionRewriteTransitionRequest`: rewrite effect is independently re-derived from the authoritative source + embedded delta before candidate creation. Forged endpoint/effect fails with `RewriteEffectMismatch`.
- Successful relation rewrites reuse the existing `prepare_revision` pipeline, therefore maintained Γ-DTC updates, candidate VMF `V=0`, physical candidate construction and freshness seal are not bypassed.
- `RewriteSpecId` / `RewriteLawSetId` survive prepared -> sealed runtime publication as explicit `RuntimeRewriteIntent` metadata.

Verification:
- `cargo fmt --all -- --check` PASS;
- `cargo check --workspace --all-targets` PASS;
- `cargo test --workspace --all-targets`: **541 passed / 0 failed / 8 ignored**;
- `cargo clippy --workspace --all-targets -- -D warnings` PASS;
- no lint suppressions added.

Still OPEN after H: durable WAL/checkpoint persistence of Rewrite intent/law IDs; durable complement capsules/retention enforcement; Γ classifier adapters that construct semantic-class Set/Bag/Map patches directly from `Value`; relational Project/Filter/Join writable synthesis; dynamic DTC guard/invariant certificates; REIC consumption of Rewrite laws.

## Checkpoint I — durable Rewrite intent / exact idempotency

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

- `RevisionCommitDescriptor` now owns Rewrite intent metadata directly; prepared/sealed transition objects do not keep a second authority copy.
- Added durable `RelationRewriteExact` transaction identity containing canonical relation deltas plus `(relation, RewriteSpecId, RewriteLawSetId)`.
- Mutation codec bumped to v6 with a dedicated relation-Rewrite tag; v5 RelationDataExact remains backward-decodable.
- Durable metadata codec bumped to v6 and persists Rewrite intent identities in the committed transaction ledger; v5 metadata remains accepted.
- `commit_derived_relation_rewrites` derives target state from authoritative source+delta and then uses the existing H Rewrite publication path, so DTC/VMF/freshness checks remain mandatory.
- Durable idempotency no longer identifies endpoint-equivalent rewrites: same transaction ID + same delta but different RewriteSpec/law set is a conflict even after checkpoint, compaction and reopen.

Verification: workspace fmt/check/tests/clippy PASS; **544 passed / 0 failed / 8 ignored**, 552 declared tests; no lint suppressions.

Still OPEN after I: durable complement-capsule/retention authority; Γ classifiers that build collection FineChange payloads from Value; relational writable Project/Filter/Join synthesis; dynamic guard/invariant certificates; REIC/cube-coherence consumers.

## Checkpoint J — durable migration-complement ledger / retention enforcement

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

- Added durable migration-complement records that retain schema/lens/semantic-pin/encoding metadata even after local payload release.
- Local authority is explicit: Forever always retains payload; UntilRevision/UntilEpoch release only on an explicit boundary; ExternalArchive/Forget carry no local complement payload.
- Payload release is irreversible and persisted by checkpoint generation; chain metadata remains as a tombstone instead of disappearing.
- Durable metadata codec v7 persists the complement ledger; v6 remains backward-readable.
- Store staging requires current durable-head source-schema match and migration-chain continuity.
- Restart hostiles confirm local payload survival and later released tombstone persistence.

Verification: workspace fmt/check/tests/clippy PASS; **547 passed / 0 failed / 8 ignored**, 555 declared tests.

Still OPEN after J: atomically bind/stage complement with full schema migration transaction; historical restore consumer; Γ classifiers for collection FineChange; relational writable Project/Filter/Join; dynamic guard/invariant certificates; REIC/cube-coherence consumers.

## Checkpoint K — atomic schema-migration WAL + complement authority

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

- Added `DurableTransactionIntent::SchemaMigrationExact`: exact target revision bytes and one `DurableMigrationComplement` now share the same PREPARE/COMMIT identity.
- Mutation codec bumped to v7; v5/v6 remain readable. Durable metadata codec bumped to v8; v7 remains readable.
- Prepare fails closed unless the complement target schema equals the exact decoded target revision and its source schema continues the durable migration chain.
- COMMIT installs the complement idempotently into the in-memory durable ledger; WAL recovery replays committed descriptors in commit order and reconstructs the same ledger after a COMMIT-before-checkpoint crash.
- Existing staged-complement API remains as an explicit administrative primitive but is no longer required for an atomic schema migration transaction.
- Hostile restart test commits a schema migration, performs no checkpoint rotation, reopens the store, and observes both the committed target head and the exact complement authority.

Gate: workspace fmt/check/tests/clippy `-D warnings` PASS; no lint suppressions added.

Still write-side OPEN after K: runtime `kernel-plan` schema-migration commit entry point carrying the capsule; relational writable Project/Filter/Join synthesis; dynamic guard/invariant certificates; REIC consumption/cube coherence; historical restore/GC enforcement.

## Checkpoint L — runtime schema-migration publication bridge

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

- Added `RuntimeRevisionCell::commit_schema_migration_durable`: full revision candidate validation/seal stays unchanged, but durable PREPARE is built with checkpoint K `SchemaMigrationExact` and the supplied complement authority.
- Added public `DurableRuntime::migrate_schema`, with exact transaction-idempotency over source revision + target revision bytes + complement identity/payload/retention.
- Runtime publication remains `Rebuilt`; commit durability uncertainty still forces recovery before further publication.
- The request registry is re-authorized against the runtime-owned registry exactly like the existing full-revision path.
- Production dependency graph remains unchanged; `kernel-lens` is only a `kernel-plan` dev-dependency for the hostile integration test.
- Hostile integration test commits a real schema/Γ revision change through `DurableRuntime`, drops the runtime without checkpoint rotation, opens `DurableRevisionStore`, and observes both target durable head and exact complement ledger.

Gate: full workspace fmt/check/tests/clippy `-D warnings` PASS; no lint suppressions.

Still write-side OPEN after L: relational writable Project/Filter/Join synthesis and determinant obligations; DTC-backed dynamic guard certificates; VMF invariant-closure certificates; REIC consumption/cube coherence; historical restore and complement GC/Forget enforcement at query/API level.

## Checkpoint M — relational writable compiler foundation

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD PARTIAL**.

- Added a relational `WritableViewPlan` compiler rather than a global `RelExpr::is_writable` predicate.
- Revision-local observable bindings are validated against schema column Γ-equivalence before they may participate in writability/determinant reasoning.
- `Scan(owner)` is unconditional; `Project`, `Filter`, and one-owner-side `Join` preserve semantic provenance and surface explicit complement/APNF/DTC/VMF obligations.
- Project duplicate columns, owner-on-both-sides joins, mismatched observable bindings, and operators without an explicit action policy fail closed.
- APNF determinant closure is a discharge hook; absence of a certificate leaves an obligation rather than guessing a preimage.
- Distributed workspace checks and Clippy pass; distributed debug tests total **554 passed / 0 failed / 8 ignored**.

Still write-side OPEN after M: executable `view FineChange -> unique owner RelationDelta/Rewrite` synthesis; planner/APNF binding handoff; DTC dynamic guard certificates; VMF invariant-closure certificates; durable publication of the lifted relational Rewrite; REIC residual/cube coherence; historical restore and complement GC/Forget API enforcement.

## Checkpoint N — exact finite relational Rewrite-lift classification

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD PARTIAL**.

Integrated:

- `RelWritableViewPlan::classify_source_rewrite_candidates` executes the R&D 0/1/>1 lift rule against the authoritative current `FiniteModel` and pinned Γ.
- Every candidate must belong to the plan's allowed source `RewriteSpecId`, carry the exact owner `RelType`, and have an effect independently equal to applying its explicit `RelationDelta`; forged delta/effect pairs fail closed.
- Candidate owner endpoints are installed only into a detached model clone and the original `RelExpr` is re-evaluated; a candidate is accepted only when its resulting view is Γ-semantically equal to the requested view Rewrite endpoint.
- Current endpoint equality is not used to collapse intent: distinct `PreparedRelationRewrite` identities remain ambiguous even when they produce the same state. Two witnesses are surfaced explicitly.
- Duplicate represented candidates are removed only by exact represented Rewrite identity. A future certified Rewrite-equivalence quotient may weaken this conservatively; no host iteration/tie-break policy is used.
- Added hostile tests for unique lift, same-endpoint/different-intent ambiguity, and forged Rewrite effect rejection.

Verification:

- `cargo fmt --all` PASS;
- `kernel-lens` 17/17 tests PASS; Clippy `-D warnings` PASS;
- distributed `cargo check` across all 23 crates PASS;
- distributed Clippy `-D warnings` across all 23 crates PASS;
- distributed debug tests: **557 passed / 0 failed / 8 ignored**.

Still write-side OPEN after N:

1. source-candidate generation from the structural lift stages/complements instead of caller-supplied finite candidates;
2. planner/APNF observable-binding handoff into the writable compiler;
3. DTC-backed dynamic guard certificates and automatic obligation discharge;
4. VMF invariant-closure certificates and automatic obligation discharge;
5. durable runtime publication API that accepts a uniquely lifted relational Rewrite without bypassing H/I validation;
6. REIC residual/cube-coherence consumption;
7. historical restore plus complement GC/Forget enforcement at query/API level.

## Checkpoint O — obligation-safe writable execution + durable publication bridge

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD PARTIAL**.

Integrated:

- `RelWritableViewPlan` now retains its own `required_obligations`; `Conditional` obligations are no longer separable metadata that a caller can accidentally discard.
- `classify_source_rewrite_candidates` fails closed with `UnresolvedObligations` whenever APNF/complement/DTC/VMF obligations remain unresolved.
- Added `kernel-integration` production dependency on `kernel-lens` and `commit_unique_relational_view_rewrite` as the cross-layer publication bridge.
- The bridge classifies against one immutable runtime snapshot and publishes only the unique case through the existing `DurableRuntime::commit_derived_relation_rewrites` / `RelationRewriteExact` path.
- Impossible and ambiguous lift classes are returned without publication; ambiguity witnesses remain intent-bearing.
- The bridge does not create a second validation path: target derivation, Rewrite-effect re-derivation, Γ-DTC maintained-state updates, VMF zero check, freshness seal, WAL identity and restart idempotency remain owned by the existing H/I runtime boundary.
- A concurrent/stale runtime transition cannot silently publish the previously classified source lift because the durable request is pinned to the captured `source_revision` and existing runtime transition validation fails stale requests closed.

Verification:

- targeted `kernel-lens` and `kernel-integration` fmt/check/tests/Clippy PASS;
- distributed `cargo check` across all 23 crates PASS;
- distributed Clippy `-D warnings` across all 23 crates PASS;
- distributed debug tests: **557 passed / 0 failed / 8 ignored**.

Still write-side OPEN after O:

1. certified discharge objects for projection complement/constructor/no-collapse, predicate admissibility, DTC guard and VMF invariant closure;
2. source-candidate generation from structural lift stages once those certificates make the section unique;
3. planner/APNF observable-binding handoff instead of caller-assembled `RelWritableColumnBindings`/`DeterminantTheory`;
4. REIC residual/cube-coherence consumption for concurrent Rewrite families;
5. historical restore plus complement GC/Forget enforcement at query/API level.

## Checkpoint P — sound complete-lift publication for identity relation views

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for `Scan(owner)` write-through**.

This checkpoint tightens checkpoint O. A caller-provided finite candidate list is not a proof that the global Rewrite-lift fiber was completely enumerated, so it is no longer accepted by the production durable publication bridge.

Integrated:

- `RelWritableViewPlan::synthesize_identity_source_rewrite` constructs the complete unique source Rewrite for the lossless identity section `RelExpr::Scan(owner)` directly from authoritative source state + requested view endpoint.
- The source `RelationDelta` is derived with Γ-semantic `RelationDelta::between_values` and then recompiled through the pinned source `RewriteSpec`; caller-provided endpoint/delta pairing is not trusted.
- Source `RewriteSpecId` mismatch fails closed, and any non-empty lift stage or unresolved obligation fails with `CandidateGenerationUnsupported` / `UnresolvedObligations` rather than pretending a candidate set is complete.
- The production `kernel-integration::commit_unique_relational_view_rewrite` now accepts a source `RewriteSpec` and requested view Rewrite only; it internally synthesizes the source Rewrite from the immutable runtime snapshot and then delegates to the existing durable Rewrite path.
- The earlier finite-candidate classifier remains a diagnostic/exhaustive-set utility, but it is no longer sufficient authority for production publication.
- Added tests proving identity synthesis derives the exact remove/insert delta and preserves source spec/law identity, plus a hostile test proving conditional projection cannot synthesize before obligations are discharged.

Verification:

- distributed `cargo check` across all 23 crates PASS;
- distributed Clippy `-D warnings` across all 23 crates PASS;
- distributed debug tests: **559 passed / 0 failed / 8 ignored**.

Still write-side OPEN after P:

1. certified complete lift-fiber generators for Project/Filter/one-owner Join using semantic complements, APNF determinant sections and explicit insertion constructors where required;
2. certified discharge of `ProjectionNoSemanticCollapse`, `PredicateAdmissibility`, `DtcGuardNoImpact`, `VmfInvariantClosure`, hidden-column complement/constructor and Join lookup determinant obligations;
3. planner/APNF observable-binding handoff instead of caller-assembled relation-column bindings;
4. REIC residual/cube-coherence consumption for concurrent Rewrite families;
5. historical restore plus complement GC/Forget enforcement at query/API level.

## Checkpoint Q — complete full-row Filter section + guarded durable write-through

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for lossless full-row Filter write-through**.

Integrated:

- The production write bridge now recognizes plans whose only lift stages are full-row `FilterEqConst` / `FilterEqColumns` stages and whose remaining obligations are exactly predicate admissibility + DTC guard + VMF closure.
- For that fragment the bridge constructs the dependent complement exactly as `Scan(owner) - FilterView` under the pinned relation Set/Bag Γ semantics, preserves the rejected complement unchanged, replaces only the accepted view section, and derives the authoritative owner `RelationDelta` from the reconstructed endpoint.
- Predicate admissibility is checked by re-evaluating the original `RelExpr` against a detached reconstructed model and requiring Γ-semantic equality with the requested view endpoint. A requested visible row that does not satisfy the filter fails `RequestedViewInadmissible`.
- `DtcGuardNoImpact` for this fragment is discharged structurally by preserving the rejected complement plus the exact query-effect check; no caller-provided dependency claim is trusted.
- `VmfInvariantClosure` is not converted into a stand-alone caller certificate: the filter synthesis helper is private to `kernel-integration`, and the resulting source Rewrite can only leave the helper through the existing durable commit path, which re-runs candidate VMF zero validation before publication.
- Source Rewrite family/law identity remains supplied by the pinned source `RewriteSpec`; view endpoint equality never substitutes for intent identity.
- Added hostile coverage showing preserved rejected complement for an admissible filter write and rejection of a requested visible row outside the predicate.

Verification:

- targeted `kernel-lens` / `kernel-integration` check, tests and Clippy `-D warnings` PASS;
- distributed `cargo check` across all 23 crates PASS;
- distributed Clippy `-D warnings` across all 23 crates PASS;
- distributed debug tests: **560 passed / 0 failed / 8 ignored**.

Write-side frontier after Q:

1. Project: certified no-collapse proof, hidden-column dependent complement, and insertion constructor/determinant section; full-column bijective projections can be handled as the next lossless subfragment.
2. One-owner Join: complete lookup-side lift fiber from APNF determinant/key uniqueness plus explicit key-move policy where needed.
3. Planner/APNF handoff: construct relation-column observable bindings and determinant theory from the same compiled planner coordinate system instead of caller assembly.
4. REIC: consume Rewrite pair/residual/cube-coherence certificates at the transaction/coordination boundary.
5. Historical write semantics: restore through migration complement chains and enforce complement GC/Forget at query/API level.

No claim is made that arbitrary Project/Join write synthesis is closed at Q; those remain fail-closed and cannot reach the production commit bridge.

## Checkpoint R — bijective full-column Project write-through

Status: **INTEGRATED / TARGETED DEBUG+CLIPPY VERIFIED / PROD CLOSED for duplicate-free full-column projection permutations**.

Integrated:

- `analyze_rel_project` now recognizes a duplicate-free projection containing every input column as a pure coordinate permutation and does not emit `ProjectionNoSemanticCollapse`; hidden-column obligations are already empty for this case.
- Added complete source-Rewrite synthesis for project-only plans whose stages are full-column permutations. The lift inverts every projection stage from the requested view endpoint back into owner-column order, derives the authoritative Γ-semantic `RelationDelta`, and re-evaluates the original query before returning the source Rewrite.
- The durable writable-view bridge tries this complete bijective Project section before the guarded Filter section. It still publishes only through the existing `RelationRewriteExact` runtime boundary, so VMF/DTC/freshness/WAL authority is unchanged.
- Lossy Project remains conditional: omitted columns still require semantic dependent complement/insertion-constructor obligations, and non-bijective projection retains `ProjectionNoSemanticCollapse`.
- Added compiler and integration hostile coverage for a nontrivial column permutation and its exact inverse source endpoint.

Verification:

- `cargo fmt --all -- --check` PASS;
- `cargo check -p kernel-lens --all-targets` PASS;
- `cargo check -p kernel-integration --all-targets` PASS;
- Clippy `-D warnings` for `kernel-lens` + `kernel-integration` PASS;
- `kernel-lens`: **20 passed / 0 failed**; `kernel-integration`: **3 passed / 0 failed**.

Write-side frontier after R:

1. lossy Project: hidden-column semantic complement plus certified insertion constructor/determinant section;
2. one-owner Join: complete lookup-side section from APNF determinant/key uniqueness and join-key preservation policy;
3. planner/APNF handoff for relation-column observable bindings and determinant theory;
4. REIC Rewrite residual/cube/coherence consumption;
5. historical restore through migration complements plus GC/Forget enforcement.

## Checkpoint S — direct one-owner Join dependent section

Status: **INTEGRATED / TARGETED DEBUG+CLIPPY VERIFIED / PROD CLOSED for direct `Scan(owner) ⋈ Scan(lookup)` with runtime-unique Set lookup keys**.

Integrated:

- Added a complete dependent section for direct one-owner joins. The mutable owner side is reconstructed from the requested joined rows; owner rows that were previously unmatched are retained as the dependent complement.
- The section is admitted only when the opposite lookup side is a direct `Scan`, has Set semantics, and every present lookup join-key fiber contains exactly one row under the pinned join equivalence. This runtime check is independent of caller-supplied determinant metadata and rejects duplicate-key lookup fibers with `JoinLookupKeyNotUnique`.
- Requested rows cannot mutate lookup-owned columns: after owner reconstruction the original Join is re-evaluated and must equal the requested view endpoint under Γ relation semantics. Invalid lookup payloads/key combinations fail `RequestedViewInadmissible`.
- `JoinLookupDeterminant` may remain on the compiled plan, but production publication does not trust it as authority for this subfragment; exact lookup-key uniqueness is re-established from the immutable runtime snapshot. APNF determinant handoff remains useful for static discharge/generalization but is not silently claimed here.
- `DtcGuardNoImpact` is discharged for this direct section by preserving the unmatched owner complement plus exact query-effect replay; VMF remains mandatory at the existing durable publication boundary.
- The same durable write bridge now tries identity, bijective Project, this direct Join section, then guarded full-row Filter; every successful lift still enters the existing `RelationRewriteExact` H/I path.
- Added hostile coverage that preserves an unmatched owner row while modifying a matched row and rejects a Set lookup containing two distinct rows in the same join-key class.

Verification:

- `cargo fmt --all -- --check` PASS;
- `cargo check -p kernel-integration --all-targets` PASS;
- Clippy `-D warnings` for `kernel-lens` + `kernel-integration` PASS;
- `kernel-lens`: **20 passed / 0 failed**; `kernel-integration`: **4 passed / 0 failed**.

Write-side frontier after S:

1. lossy Project: hidden-column semantic complement plus certified insertion constructor/determinant section;
2. generalized one-owner Join beyond direct Scan/Set runtime-unique lookup, preferably through planner/APNF determinant certificates;
3. planner/APNF handoff: build relation-column observable bindings and determinant theory from compiled planner coordinates rather than caller assembly;
4. REIC Rewrite residual/cube/coherence consumption;
5. historical restore through migration complement chains plus complement GC/Forget enforcement.

### Checkpoint S verification addendum — distributed workspace gate

After the final hostile lookup-payload test, the complete workspace was re-gated with per-package commands to stay below the tool-call compilation limit:

- distributed `cargo check --all-targets` across all **23 crates** — PASS;
- distributed `cargo clippy --all-targets -- -D warnings` across all **23 crates** — PASS;
- distributed debug tests across all **23 crates** — **563 passed / 0 failed / 8 ignored**;
- final Join hostile additionally proves requested writes cannot forge lookup-owned output columns: exact Join replay returns `RequestedViewInadmissible`.

No source work beyond Checkpoint S was started in this cycle. Next owner remains the planner/APNF writable-coordinate handoff, followed by generalized determinant-backed Join / lossy Project sections.

## Checkpoint T — planner-owned writable coordinate handoff

Status: **INTEGRATED / TARGETED DEBUG+CLIPPY VERIFIED / PROD CLOSED for relation-column coordinate ownership**.

Integrated:

- `RelExpr::scan_relations()` exposes the exact logical source-relation set without duplicating query-tree traversal in downstream crates.
- `PreparedPlan::writable_coordinates()` now builds one revision-local observable catalog from the same pinned semantic context as the prepared executable plan.
- Every scanned relation column receives a distinct `PinnedEquivalenceCoordinate`, even when multiple columns share one Γ-equivalence; the handoff therefore cannot alias semantic coordinates merely because equality modules are shared.
- Added `PreparedRelWritableCoordinates`, which owns both the observable catalog and the relation→column coordinate map and can construct `DeterminantTheory` over exactly those coordinates.
- `kernel-integration::compile_prepared_relational_writable_query` now adapts this planner-owned handoff into `RelWritableColumnBindings` and invokes the writable compiler. Callers no longer manufacture observable IDs or relation-column bindings manually.
- No `kernel-plan -> kernel-lens` production dependency was introduced. `kernel-plan` exports only semantics/planner-owned coordinate state; `kernel-integration` remains the adapter into the write compiler.
- The initial determinant theory contains no invented functional dependencies. Static/runtime determinant morphisms still have to be certified before lossy Project or generalized Join obligations can be discharged.
- Hostile coverage uses two columns with the same pinned Text equality and proves the prepared-plan handoff allocates distinct revision observables while still compiling the owner Scan as writable.

Verification:

- `cargo fmt --all -- --check` PASS after formatting;
- targeted `cargo check` for `kernel-query`, `kernel-plan`, `kernel-integration` PASS;
- targeted `kernel-integration` hostile test PASS;
- targeted Clippy `-D warnings` for `kernel-query`, `kernel-plan`, `kernel-integration` PASS.

Write-side frontier after T:

1. generalized one-owner Join: consume the planner-owned coordinates plus a certified/runtime-revalidated determinant section for nontrivial lookup expressions;
2. lossy Project: hidden-column complement plus insertion constructor/determinant section;
3. DTC/VMF dynamic certificate objects rather than implicit local discharge where useful to consumers;
4. REIC residual/cube/coherence consumption;
5. historical restore through migration complements plus complement GC/Forget enforcement.

## Checkpoint U — owner-free lookup-query Join section

Status: **INTEGRATED / TARGETED DEBUG+CLIPPY VERIFIED / PROD CLOSED for direct owner Scan joined to an owner-free runtime-unique lookup expression**.

Integrated:

- The one-owner Join dependent section no longer requires the lookup side itself to be a direct `Scan`. The mutable side must still be the direct owner `Scan`, but the opposite side may be an owner-free relational expression accepted by the writable analyzer (for example Filter/Project chains).
- The lookup expression is rejected if its logical source set contains the owner relation, preventing hidden self-dependence while reconstructing the owner.
- Runtime uniqueness is checked on the exact evaluated lookup expression, not on its base relation. Bag lookup outputs are therefore admitted only when every present join-key fiber has multiplicity exactly one; a duplicate active key fails `JoinLookupKeyNotUnique`.
- Lookup-side writable obligations inherited from Filter/Project analysis are treated as read-only lookup analysis facts for this section; they do not authorize writes to the lookup. Exact query replay still proves lookup-owned output columns were not forged.
- Unmatched owner rows remain the dependent complement via AntiJoin against the exact lookup expression, and successful reconstruction still enters the existing durable RelationRewrite path.
- Hostile coverage uses a Bag lookup with two base rows sharing a key but only one surviving a filter; the write succeeds while the visible key fiber is unique, then fails after a second surviving row is introduced.

Verification:

- targeted `kernel-integration` test PASS;
- `cargo check -p kernel-integration --all-targets` PASS;
- `cargo clippy -p kernel-integration --all-targets -- -D warnings` PASS;
- formatting PASS.

Write-side frontier after U:

1. lossy Project: deletion/update complement section and certified insertion constructor for genuinely new projected values;
2. generalized owner-side pipelines before Join (Filter/Project over owner) still require compositional inverse sections;
3. certified determinant morphisms should reuse the Checkpoint T planner-owned catalog rather than caller coordinate assembly;
4. REIC residual/cube/coherence consumption;
5. historical restore through migration complements plus GC/Forget enforcement.

## Checkpoint V — lossy Project deletion dependent section

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for deletion-only lossy Project with unique Bag preimages or exact Set removal**.

Integrated:

- Added a source-Rewrite section for Project-only owner plans that actually lose columns. It is intentionally deletion-only: if the requested projected endpoint introduces any new projected row class, synthesis returns `CandidateGenerationUnsupported` and does not invent hidden values.
- Hidden source columns are reconstructed from the authoritative old owner relation. For Bag views, each removed projected occurrence must have a unique semantic full-row preimage up to full-row Γ-equivalence; otherwise the lift fails with new explicit `ProjectionPreimageAmbiguous` rather than selecting an arbitrary hidden row.
- For Set views, deleting one projected class removes every source row projecting to that class, which is the unique source endpoint that makes the set-valued projected class absent.
- Existing retained source rows, including all hidden columns, remain the dependent complement. The reconstructed owner relation is replayed through the original Project query and must equal the requested endpoint exactly under pinned Γ before a source Rewrite is returned.
- The section is wired into `commit_unique_relational_view_rewrite` after the bijective Project section and before Join/Filter fallbacks. Successful lifts still use the existing durable RelationRewrite publication path; insertion remains fail-closed.
- Hostile coverage proves a unique hidden complement is preserved for deletion and that two semantically different Bag preimages of the same projected row are rejected as ambiguous.

Verification:

- `cargo fmt --all -- --check` PASS;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed debug tests across all 23 crates, executed in three bounded batches: **566 passed / 0 failed / 8 ignored**.

Write-side frontier after V:

1. lossy Project insertion/update: certified hidden-column constructor/determinant section for genuinely new projected values; no synthesis currently guesses hidden data;
2. compositional owner-side pipelines before Join (Project/Filter over the mutable side);
3. determinant morphism production/revalidation using the Checkpoint T planner-owned catalog, rather than caller-authored coordinate identities;
4. REIC Rewrite residual/cube/coherence consumption at coordination/transaction boundaries;
5. historical restore through migration complement chains plus complement GC/Forget enforcement at query/API level.

### Checkpoint V hostile boundary note

The write-view R&D explicitly permits relational Project updates only when stable row identity survives projection or is carried as semantic complement, and requires a certified constructor/default for hidden fields on insertion. Current logical `RelationValue` rows do not expose a stable semantic row identity; `PhysicalRowId`/stable storage handles are reconstructible physical state and are therefore forbidden as complement authority. Checkpoint V intentionally does not use physical handles to manufacture update pairing or insertion defaults. The remaining Project frontier must be closed by semantic constructor/complement policy or Γ/APNF determinant data, not by storage identity leakage.

## Checkpoint W — runtime-determined Bag Project multiplicity growth

Status: **INTEGRATED / TARGETED DEBUG+CLIPPY VERIFIED / PROD CLOSED for Bag multiplicity growth of an existing projected Γ-class with a unique full-row preimage class**.

Integrated:

- The lossy Project section no longer rejects every insertion-shaped Bag delta. An added occurrence of an already-observed projected Γ-class may be lifted when the authoritative source relation proves one unique full-row Γ-preimage class for that visible class.
- Hidden columns are never guessed. The source row used for the added Bag occurrence is copied only from the exact old owner fiber after Γ-semantic equality checks over both the projected view type and the complete owner relation type.
- If two semantically distinct full-row preimages share the projected class, lift fails with `ProjectionPreimageAmbiguous`.
- If the projected class is unseen in the old source fiber, lift remains `CandidateGenerationUnsupported`; Checkpoint W therefore does not pretend to provide an insertion constructor/default for new visible values.
- Set Project insertion remains unsupported by this section. The existing deletion semantics and final exact query replay remain mandatory.

Verification:

- `cargo fmt --all -- --check` PASS after formatting;
- targeted hostile test for unique/ambiguous/unseen Bag fibers PASS;
- `cargo check -p kernel-integration --all-targets` PASS;
- `cargo clippy -p kernel-integration --all-targets -- -D warnings` PASS.

Write-side frontier after W:

1. certified/explicit semantic constructor for genuinely unseen lossy-Project visible classes;
2. compositional mutable owner pipelines before Join (Filter/Project over owner);
3. first-class determinant morphism production/revalidation over Checkpoint T planner-owned coordinates, replacing ad-hoc local finite-fiber tests where the APNF object is the right authority;
4. REIC residual/cube/coherence consumption;
5. historical restore through migration complements plus complement GC/Forget enforcement.

## Checkpoint X — compositional filtered-owner Join section

Status: **INTEGRATED / TARGETED DEBUG+CLIPPY VERIFIED / PROD CLOSED for full-row owner Filter chains feeding a runtime-unique read-only Join lookup**.

Integrated:

- The one-owner Join section now accepts the mutable owner through any full-row `FilterEqConst` / `FilterEqColumns` chain rooted at `Scan(owner)`; Project/other lossy owner operators remain rejected by this section.
- Join reconstruction operates on the accepted owner section, while rows rejected by the owner Filter chain are recovered independently as the exact old `Scan(owner) - owner_filter_query` dependent complement.
- The accepted section still preserves unmatched owner rows through AntiJoin against the exact owner-free lookup query, and lookup-key uniqueness is revalidated on the pinned runtime snapshot.
- Accepted reconstruction and rejected complement are recombined under the owner relation's Set/Bag semantics, then the original full query is replayed exactly before a source Rewrite is emitted.
- Hostile coverage proves a filtered-out owner row survives unchanged while an accepted joined row is rewritten.

Verification:

- targeted filtered-owner Join hostile PASS;
- `cargo check -p kernel-integration --all-targets` PASS;
- `cargo clippy -p kernel-integration --all-targets -- -D warnings` PASS;
- formatting PASS.

Write-side frontier after X:

1. Project-over-owner before Join still needs compositional projection complement/constructor semantics;
2. genuinely unseen lossy-Project values still require a certified/explicit semantic hidden-column constructor;
3. first-class APNF determinant morphism production/revalidation over planner-owned coordinates;
4. REIC residual/cube/coherence consumption;
5. historical restore through migration complement chains plus complement GC/Forget enforcement.

## Checkpoint Y — planner-owned finite semantic measure handoff

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for constructing APNF determinant evidence from exact relation values on planner-owned coordinates**.

Integrated:

- `PreparedRelWritableCoordinates::relation_measure` now classifies an exact `RelationValue` through the same revision-local observable catalog created by the prepared plan and constructs a `RevisionFiniteMeasure` over the exact relation-column coordinates.
- Relation/row arity drift fails explicitly; observable and anchor-pullback errors remain typed rather than being flattened into caller assertions.
- The resulting measure can directly derive existing `CertifiedSemanticMorphism` determinant witnesses through `RevisionFiniteMeasure::determinant_morphism`; no caller-authored coordinate IDs or host equality are introduced.
- The integration hostile extends the shared-equivalence-column coordinate test and proves a finite owner relation yields a certified first-column -> second-column determinant on the planner-owned catalog.
- This checkpoint provides the first-class APNF evidence production primitive. It does not yet claim that a finite-snapshot determinant is a revision-global invariant or a constructor for unseen keys; consumers must revalidate/domain-check the witness at the rewrite boundary.

## Checkpoint Z — conservative pair coordination decision boundary

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for consuming the existing pair Rewrite-law classifier into a fail-closed coordination decision**.

Integrated:

- Added `PairCoordinationDecision::{CoordinationFree, IntentConflict, RequiresCoordination}`.
- `StrongCommute` and `SameIdempotentIntent` are admitted as coordination-free pair cases.
- `DefiniteIntentConflict` maps to an explicit intent conflict.
- `Unknown` maps only to `RequiresCoordination`; it is never interpreted as probable commutation.
- Existing footprint hostiles now also assert the coordination decision for disjoint safe writes, guarded/unknown writes, and conflicting assignments.
- This is the conservative pairwise admission boundary required before REIC can consume Rewrite-law evidence. Residual transformation, diamond certificates and cube coherence remain OPEN and are not implied by this checkpoint.

### Checkpoint Z distributed workspace gate

After the final source freeze:

- `cargo fmt --all -- --check` PASS (source was formatted before the distributed gate);
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed `cargo clippy --all-targets -- -D warnings` across all **23 crates** PASS;
- distributed debug tests across all **23 crates**: **568 passed / 0 failed / 8 ignored**.

Current write-side frontier after Z:

1. genuinely unseen lossy-Project values: certified/explicit semantic hidden-column constructor/default; physical row identity remains forbidden as semantic complement authority;
2. Project-over-owner before Join: compositional projection complement/constructor section;
3. consume Checkpoint Y APNF determinant morphisms at exact rewrite boundaries with explicit domain/revalidation rules instead of upgrading finite-snapshot evidence to a global invariant;
4. REIC branch-exclusive Rewrite consumption with first-class residual/rebase transformation, diamond certificate and cube/coherence family certificate; pairwise `Unknown` already fails closed to coordination;
5. historical restore through migration complement chains plus complement GC/Forget enforcement at query/API level.

## Checkpoint AA — rewrite-boundary APNF determinant revalidation

Status: **INTEGRATED / TARGETED DEBUG+CLIPPY VERIFIED / PROD CLOSED for finite-snapshot Project determinant consumption on the existing certified domain**.

Integrated:

- `PreparedRelWritableCoordinates` now derives relation determinants directly from exact `RelationValue` snapshots and can revalidate the same source/target column determinant across before/after states.
- `RevalidatedRelationDeterminant` exposes two distinct rewrite-boundary facts: whether the after-domain is covered by the before certified domain, and whether determinant images are stable on the shared domain. Finite support is not promoted to a revision-global invariant.
- Lossy Bag Project multiplicity growth now consumes this APNF determinant evidence instead of proving hidden-preimage uniqueness by pairwise full-row comparison.
- Existing projected classes may still grow only when the old visible->hidden determinant exists; after reconstruction the determinant is re-derived and must introduce no new source key and no remapping of surviving keys.
- Genuinely unseen projected classes remain fail-closed because no old determinant image exists. Physical row identity is still not semantic authority.
- Hostile coverage distinguishes stable, remapped and extended determinant domains and preserves the prior ambiguous/unseen Project rejection tests.

Verification:

- `cargo fmt --all` clean;
- `cargo check -p kernel-plan --all-targets` PASS;
- `cargo clippy -p kernel-plan --all-targets -- -D warnings` PASS;
- `cargo check -p kernel-integration --all-targets` PASS;
- `cargo clippy -p kernel-integration --all-targets -- -D warnings` PASS;
- targeted APNF revalidation + lossy Project hostiles PASS.

Write-side frontier after AA:

1. genuinely unseen lossy-Project values still require an explicit/certified semantic hidden-column constructor rather than finite-support extrapolation;
2. Project-over-owner before Join remains compositionally open;
3. REIC causal effect identity/cut + residual/diamond/cube coherence consumption remains production OPEN;
4. historical restore through migration complement chains plus complement GC/Forget enforcement remains OPEN.

## Checkpoint AB — REIC causal core + residual/coherence certificate substrate

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for in-memory causal effect-ideal algebra and fail-closed branch-exclusive Rewrite admission substrate**.

Integrated:

- Added first-class `RevisionEffectId`, exact `RevisionEffect<E>` records with prerequisite IDs, and validated `RevisionEffectIdeal<E>` causal cuts.
- Effect ideals reject duplicate IDs, missing prerequisites and cycles; event identity is exact, so the same `RevisionEffectId` carrying a different payload is an explicit `EffectIdentityConflict`.
- `common_ideal` computes the canonical causal intersection directly. `exclusive_from` returns branch-exclusive events, and `union` preserves exact identity/down-closure. The criss-cross hostile yields common `{root,a,b}` without choosing either snapshot LCA.
- Added exact `RewriteResidualDiamond` certification: caller-supplied residual rewrites are accepted only when both residual execution paths reach the same semantic endpoint.
- Added strong `RewriteCubeCoherence` certification: the two residualization paths must produce the same intent-bearing `PreparedRewrite`, not merely endpoint-equivalent effects. Same endpoint with a different RewriteSpec is rejected.
- Added `RevisionEffectMergeRequirements`: branch-exclusive event pairs consume the Checkpoint Z `PairCoordinationDecision`. `CoordinationFree` needs no residual; `RequiresCoordination` is retained explicitly as a required residual/coordination pair; `IntentConflict` is retained as an explicit conflicting pair. Unknown is never silently admitted.
- This checkpoint intentionally does not claim durable REIC persistence, automatic residual synthesis, arbitrary cube-family inference, or snapshot-DAG replacement. Those consumers remain separate production work.

Final distributed gate after source freeze:

- `cargo fmt --all -- --check` PASS;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed `cargo clippy --all-targets -- -D warnings` across all **23 crates** PASS;
- distributed debug tests across all **23 crates**: **572 passed / 0 failed / 8 ignored**.

Current write-side frontier after AB:

1. durable/revision ownership of REIC effect records and causal cuts; existing snapshot `unique_merge_base` remains compatibility behavior, not yet replaced by effect-ideal authority;
2. residual-transform implementation/family registry that can satisfy `requires_residual` pairs, plus multi-event cube/coherence and VMF invariant-closure consumption;
3. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors; finite APNF support is not extrapolated;
4. Project-over-owner before Join remains compositionally open;
5. historical restore through migration complement chains plus complement GC/Forget enforcement at query/API level remains OPEN.

## Checkpoint AC — durable REIC effect ledger + revision causal frontiers

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for durable single-head causal effect ownership and restart-safe exact intent linkage**.

Integrated:

- `DurableRevisionEffectRecord` gives every newly committed durable transition a stable Γ-REIC effect identity derived exactly from its `ClientTransactionId`, prerequisite effect frontier, source revision and target revision.
- Effect payload bytes are not duplicated: each causal record references the already-authoritative exact `DurableTransactionIntent` retained by the transaction ledger. Reconstructing a `RevisionEffectIdeal<DurableTransactionIntent>` therefore preserves exact Rewrite/transaction identity without a second intent authority.
- `DurableRevisionStore` now owns a causal coverage root, immutable effect records and per-revision causal frontiers. A new store starts with an explicit empty frontier at its base revision.
- COMMIT atomically advances the in-memory causal frontier after the WAL commit. Reopen reconstructs WAL-tail effects in commit order, and checkpoint rotation persists the effect ledger/frontiers in durable metadata codec v9.
- Metadata v1-v8 remain readable. Histories opened from pre-v9 metadata do **not** fabricate missing ancestry: causal coverage starts explicitly at the recovered durable head with an empty frontier, leaving the older prefix opaque/compatibility-only.
- Open validates effect identity, exact transaction linkage, source-frontier prerequisites, canonical target frontier and absence of dangling effect references. Corrupt causal metadata fails closed.
- Hostile/restart coverage proves a COMMIT recovered directly from WAL has the same exact causal effect and that checkpoint rotation preserves the frontier with an empty WAL tail.

Verification after source freeze:

- `cargo fmt --all -- --check` PASS;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed top-level package test summaries: **563 passed / 0 failed / 8 ignored** (nested subprocess helper output is not double-counted).

Current write-side frontier after AC:

1. runtime residual-family registry that can satisfy Γ-REIC `requires_residual` pairs and bind residual identity to exact RewriteSpec/law identity;
2. multi-event diamond/cube coherence consumption plus VMF invariant-closure validation before union/publication;
3. general multi-parent durable revision cuts remain OPEN; current `DurableRevisionStore` is single-head and AC intentionally does not pretend otherwise;
4. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
5. Project-over-owner before Join remains compositionally OPEN;
6. historical restore through migration complement chains plus complement GC/Forget enforcement remains OPEN.

## Checkpoint AD — residual-family identity registry + runtime causal-ideal exposure

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for exact residual-family identity admission and DurableRuntime access to covered causal ideals**.

Integrated:

- Added `RewriteFamilyIdentity { spec, law_set }`, `RewriteResidualFamilyId`, pair keys and `RewriteResidualFamilySpec`.
- `RewriteResidualFamilyRegistry` admits at most one exact residual family per ordered Rewrite-family pair and rejects family-ID rebinding or a second family for the same pair.
- Residual certification now binds both input RewriteSpec/law-set identity and the exact expected residual Rewrite-family identities before running the existing residual-diamond endpoint proof. Endpoint-equivalent residuals with a different RewriteSpec/law identity are rejected.
- `DurableRuntime` now exposes the durable causal coverage root and reconstructible `RevisionEffectIdeal<DurableTransactionIntent>` through its synchronized store boundary; REIC consumers no longer need direct access to durability internals.
- The durable RelationRewrite hostile now verifies that checkpoint+compaction+reopen preserves a one-event causal ideal whose payload is the exact retained `RelationRewriteExact` identity, including RewriteSpec/law-set metadata.
- This checkpoint does **not** synthesize residual transforms automatically. The registry is an exact admission/certification boundary for implementations supplied by later certified/runtime families.

Verification after final source freeze:

- `cargo fmt --all -- --check` PASS;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed top-level package summaries: **564 passed / 0 failed / 8 ignored**. The test pass was split only because one tool call reached the 45-second execution cap after `kernel-query`; the remaining crates were completed in a second bounded batch.

Current write-side frontier after AD:

1. consume registered residual families across branch-exclusive Γ-REIC effects, producing multi-event diamond/cube certificates rather than pair metadata only;
2. bind VMF invariant-closure discharge to residual/cube publication before union/merge becomes admissible;
3. general multi-parent durable revision cuts and explicit resolution effects remain OPEN; AC/AD durable store support is intentionally single-head coverage only;
4. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
5. Project-over-owner before Join remains compositionally OPEN;
6. historical restore through migration complement chains plus complement GC/Forget enforcement remains OPEN.

## Checkpoint AE — registered residual frontier consumption + TP2 cube + VMF closure certificate

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for one branch-exclusive concurrent frontier layer, registered residual cube checking, and exact revision-bound VMF zero certification**.

Integrated:

- Added `RevisionEffectResidualLayerCertificate`: a Γ-REIC pair of ideals can now consume every required registered residual family for a complete concurrent frontier layer instead of merely reporting `requires_residual` metadata.
- The frontier consumer fails closed on explicit intent conflicts, missing residual witnesses, unregistered/mismatched residual families, and any exclusive event whose prerequisites escape the common ideal. Pairwise diamonds are therefore not silently reused for deeper causal suffixes.
- Added `RewriteResidualCubeWitness` / `RewriteResidualCubeCertificate` and registry-backed `certify_cube`. It checks the three base residual diamonds, both second-order residual faces, and exact TP2/cube residual intent equality. Same endpoint with a different RewriteSpec/law identity is rejected.
- Added opaque `RuntimeInvariantClosureCertificate`, obtainable from `RuntimeRevisionBundle` only when the exact maintained Γ-VMF violation measure is zero and bound to that exact logical/semantic revision. Manually injecting a violation invalidates certificate issuance.
- This checkpoint intentionally does not claim a multi-parent merge publisher. A residual/cube proof and VMF closure token now exist as separate sound prerequisites; a later publication consumer must bind them to one exact merged candidate before union/merge is admitted.

Verification after source freeze:

- `cargo fmt --all -- --check` PASS;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **577 passed / 0 failed / 8 ignored**. The first test batch reached the 45-second tool-call boundary after `kernel-plan`; all remaining crates completed in the second bounded batch.

Current write-side frontier after AE:

1. bind residual/cube certificates and the exact VMF closure certificate to one concrete merge/resolution publication candidate; no certificate may float independently of the candidate it validates;
2. general multi-parent durable revision cuts plus explicit durable resolution effects remain OPEN; current durable store remains intentionally single-head;
3. deeper branch-exclusive causal suffixes need iterative residualization layers rather than reusing frontier pair certificates;
4. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
5. Project-over-owner before Join remains compositionally OPEN;
6. historical restore through migration complement chains plus complement GC/Forget enforcement remains OPEN.

## Checkpoint AF — local historical restore authority + retention-policy enforcement

Status: **INTEGRATED / DISTRIBUTED CHECK+CLIPPY VERIFIED / TARGETED TEST VERIFIED / PROD CLOSED for local historical complement-chain availability and explicit retention failure modes**.

Integrated:

- Added durability-owned `LocalHistoricalComplementChain`; `kernel-plan` does not acquire a production dependency on `kernel-lens`. An attempted direct `kernel-plan -> kernel-lens` return type was rejected during hostile integration and replaced before checkpoint freeze.
- `DurableRevisionStore::local_historical_complement_chain(source_schema, target_schema)` now resolves a contiguous locally-authoritative migration-complement chain and returns only steps whose complement payload still exists locally.
- Retention policy is enforced at the API boundary with typed failures: `ExternalArchiveRequired(proof)`, `ExplicitlyForgotten`, `LocalPayloadReleased`, and `PathNotFound`. Tombstone metadata is never treated as reversible payload authority.
- `DurableRuntime` exposes the same resolver through its synchronized durability owner, so query/runtime consumers do not bypass the store boundary.
- Hostiles cover a two-step local chain, irreversible `UntilEpoch` release, external archive authority, explicit Forget, and missing path. The real durable schema-migration runtime test consumes the local chain after commit.

Verification at freeze:

- `cargo fmt --all -- --check` PASS before the final test-placement correction; final source was run through `cargo fmt --all` again.
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS; `kernel-plan` Clippy was rerun after the final correction and PASS.
- distributed tests before AF API exposure: **578 passed / 0 failed / 8 ignored**.
- post-AF distributed test run: all packages before `kernel-plan` PASS; one `kernel-plan` test initially failed because the new API assertion was accidentally inserted into the neighboring full-revision test rather than the schema-migration test. The assertion was moved; both affected tests PASS and `kernel-plan` Clippy PASS. The 20-minute source-edit boundary was then reached, so a second full 23-crate test sweep was intentionally not started.

Current write-side frontier after AF:

1. bind residual/cube certificates and exact VMF closure to one concrete merge/resolution publication candidate;
2. general multi-parent durable revision cuts plus explicit durable resolution effects remain OPEN;
3. deeper branch-exclusive causal suffixes need iterative residualization layers;
4. historical restore *availability/authority* is now enforced, but actual value/state reconstruction through registered migration LensSpec implementations remains OPEN;
5. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
6. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AG — complete residual cube + concrete resolution publication candidate

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for complete three-upper-face cube coherence and single-relation candidate binding to cube endpoint + exact VMF closure**.

Integrated:

- Strengthened `RewriteResidualCubeCertificate`: the registered cube now checks all three upper residual diamonds (`after_a`, `after_b`, `after_c`) rather than only two.
- Cube certification now requires all three upper faces to converge to one exact final endpoint. A new `CubeEndpointMismatch` failure distinguishes endpoint disagreement from residual-family/intent mismatch.
- `RewriteResidualCubeCertificate::common_endpoint()` exposes the certified final state without allowing callers to construct the certificate directly.
- Added hostile coverage where all registered residual family identities remain correct but the third upper face reaches a different endpoint; certification fails closed.
- Added `PreparedCoherentResolutionTransition`: one concrete prepared single-relation Rewrite candidate can be bound to a complete residual cube only when its exact candidate relation value equals the cube common endpoint.
- The same binding obtains and retains an exact revision-bound `RuntimeInvariantClosureCertificate`; a candidate with nonzero Γ-VMF state cannot become a coherent resolution publication candidate.
- Binding requires exactly one relation delta and one intent-bearing Rewrite for the same relation. Multi-relation or non-Rewrite prepared transitions are not silently treated as resolution candidates.
- The wrapper retains the original `PreparedRuntimeRevisionTransition`; it adds proof/evidence only and does not create a second state authority. Freshness sealing still uses the existing runtime publication boundary.

Verification after source freeze:

- `cargo fmt --all -- --check` PASS;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed top-level package test summaries: **580 passed / 0 failed / 8 ignored**.

Current write-side frontier after AG:

1. carry `PreparedCoherentResolutionTransition` through the durable commit/publication API so a committed resolution retains explicit resolution identity together with its causal prerequisites;
2. general multi-parent durable revision cuts plus explicit durable resolution effects remain OPEN;
3. deeper branch-exclusive causal suffixes need iterative residualization layers;
4. historical restore availability is enforced, but actual value/state reconstruction through registered migration LensSpec implementations remains OPEN;
5. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
6. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AH — cube/VMF-gated durable resolution publication

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for single-head single-relation durable publication gated by complete cube coherence + exact candidate VMF closure**.

Integrated:

- Added `RuntimeRevisionCell::commit_coherent_resolution_durable`, routing an already-bound `PreparedCoherentResolutionTransition` through the existing WAL PREPARE -> freshness seal -> durable COMMIT -> immutable runtime publication sequence.
- Added public `DurableRuntime::commit_derived_coherent_resolution` for the compact delta-authoritative single-relation resolution path.
- The runtime independently derives the target from authoritative source + embedded `RelationDelta`, prepares the intent-bearing Rewrite, then requires `bind_coherent_resolution` before emitting durable PREPARE.
- A wrong cube final endpoint therefore fails before durable publication and leaves the live revision unchanged.
- Successful publication retains the existing exact `RelationRewriteExact` transaction identity, including `RewriteSpecId` and `RewriteLawSetId`; the causal effect ledger therefore preserves the exact committed resolution Rewrite intent rather than endpoint identity only.
- Retry/idempotency continues to use exact durable rewrite identity. This checkpoint intentionally does **not** encode cube proof material in WAL/metadata: cube/VMF are publication proofs, while logical transaction authority remains the exact Rewrite intent and resulting Revision.
- Multi-parent causal prerequisites / explicit durable resolution-event kind are not claimed by this checkpoint; current durable store remains single-head.

Verification after final source freeze:

- `cargo fmt --all -- --check` PASS;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed top-level package test summaries: **581 passed / 0 failed / 8 ignored**.
- hostile durable test proves an endpoint-mismatched cube is rejected with the source revision still live, while the correct cube commits and appears in the exact causal transaction payload.

Current write-side frontier after AH:

1. general multi-parent durable revision cuts plus explicit durable resolution effects/prerequisite frontiers remain OPEN;
2. deeper branch-exclusive causal suffixes need iterative residualization layers;
3. historical restore availability is enforced, but actual value/state reconstruction through registered migration LensSpec implementations remains OPEN;
4. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
5. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AI — explicit durable multi-parent resolution cut

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD PARTIAL for explicit multi-parent causal resolution over already-covered durable revision frontiers**.

Integrated:

- Added exact `DurableTransactionIntent::RelationResolutionExact`. A durable resolution now preserves the same relation mutations and exact `(RewriteSpecId, RewriteLawSetId)` identities as `RelationRewriteExact`, plus a canonical sorted list of two-or-more causal parent `RevisionId`s.
- Callers never supply raw `RevisionEffectId` prerequisites. `DurableRevisionStore` independently derives the resolution prerequisite cut by taking the union of the already-authoritative causal frontiers of every persisted parent revision at PREPARE, COMMIT replay, recovery, and metadata validation.
- The source revision must be one of the causal parents; duplicate/unsorted/missing/out-of-coverage parent revisions fail closed. Transaction retry identity includes the exact parent revision list, so the same transaction id cannot be rebound to a different causal cut merely because its endpoint Rewrite is identical.
- Mutation codec advanced to v8 with backward read support for v5/v6/v7. Durable metadata codec advanced to v10 with v9 and older readable. `RelationResolutionExact` survives WAL-tail recovery and checkpoint metadata encoding without duplicating raw effect prerequisites as transaction authority.
- Added `DurableRelationResolution` as the typed constructor payload so resolution mutation, Rewrite intent and causal-parent identity are carried together without expanding legacy APIs.
- Added `RevisionCommitDescriptor::durable_resolution_descriptor` and `DurableRuntime::commit_derived_multi_parent_coherent_resolution`. The latter still requires the AH complete cube + exact VMF closure gate before WAL PREPARE; only the durable causal identity/publication descriptor changes.
- Hostiles cover codec roundtrip, store-level commit/reopen and runtime publication. A resolution over parent revisions `r31,r32` recovers with prerequisite cut `{effect(r31), effect(r32)}`; the runtime path likewise persists exact parent revisions and their derived effect cut after reopen.
- Scope boundary: the physical durable store still has one mutable publication head. AI supports explicit multi-parent causal cuts among revision frontiers already present in that store; importing/retaining independently advancing durable branch heads is **not** claimed closed.

Verification after source freeze:

- `cargo fmt --all` / formatting clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **583 passed / 0 failed / 8 ignored**.

Current write-side frontier after AI:

1. independent durable branch-head ingestion/retention is still OPEN; AI only joins already-covered revision frontiers;
2. deeper branch-exclusive causal suffixes still need iterative residualization layers rather than one frontier certificate;
3. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
4. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
5. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AJ — canonical causal-layer schedule for deeper branch suffixes

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD PARTIAL substrate for iterative residualization of deeper branch-exclusive causal suffixes**.

Integrated:

- Added `RevisionEffectCausalLayerSchedule { common, left_layers, right_layers }` over validated Γ-REIC effect ideals.
- `RevisionEffectIdeal::causal_layer_schedule` computes the canonical common ideal and then peels each branch-exclusive DAG in dependency order: layer 0 contains effects whose prerequisites lie entirely in the common ideal; every later layer contains exactly effects whose prerequisites are contained in common + all prior layers.
- The schedule is derived from causal prerequisites only; it never uses timestamp/LCA selection or physical publication order. A hostile two-deep branch on each side produces deterministic layers `{left-a},{left-b}` and `{right-a},{right-b}`.
- This closes the ordering substrate that AE deliberately lacked: deeper suffixes no longer need to be flattened into one frontier or rejected merely because their causal depth is greater than one.
- Scope boundary remains explicit: AJ schedules the sequence of residualization frontiers but does **not** synthesize transformed `PreparedRewrite` payloads for layer > 0. Registered residual families must still be consumed iteratively by a later runtime/compiler consumer before a multi-layer merge can be certified.

Verification after source freeze:

- `cargo fmt --all` / formatting clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **584 passed / 0 failed / 8 ignored**.

Current write-side frontier after AJ:

1. consume `RevisionEffectCausalLayerSchedule` iteratively, constructing/rebinding residual Rewrite events between successive layers and producing a final multi-layer coherence certificate;
2. independent durable branch-head ingestion/retention remains OPEN; AI joins only already-covered durable revision frontiers;
3. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
4. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
5. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AK — registered two-layer iterative residualization

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for exact two-layer singleton branch-chain residualization; deeper chains remain fail-closed pending first-class composite residual families**.

Integrated:

- Added `RevisionEffectTwoLayerResidualWitness` and `RevisionEffectTwoLayerResidualCertificate` as the first consumer that actually executes the AJ causal-layer schedule rather than only storing it.
- `RevisionEffectIdeal::certify_registered_two_layer_chain` requires exactly two singleton causal layers on each branch. Layer 0 is certified through the existing exact residual-family registry.
- Layer 1 is not certified directly against stale branch payloads: the left second-layer Rewrite is independently transported across the exact right residual produced by layer 0, and the right second-layer Rewrite is independently transported across the exact left residual produced by layer 0.
- The two transported second-layer Rewrites are then certified against one another at the first diamond's common endpoint, producing a second exact residual diamond and final common endpoint.
- Every transport and second-layer crossing consumes a separately registered exact residual-family identity; endpoint equality alone never authorizes a residual transformation.
- Chains deeper than two layers or non-singleton layer shapes fail closed with `UnsupportedLayerShape`. This is intentional: depth >= 3 requires an explicit family identity for the cumulative opposite-prefix residual rather than silently composing residual endpoints.
- Hostile coverage proves a two-deep branch pair reaches the certified second-layer endpoint only through four registered residual families, while a three-deep suffix is rejected before any incomplete residual reasoning is attempted.

Verification after source freeze:

- `cargo fmt --all` / formatting clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **586 passed / 0 failed / 8 ignored**.

Current write-side frontier after AK:

1. introduce first-class sequential/composite residual-family identity so accumulated opposite-prefix residuals can be transported soundly beyond depth 2;
2. consume that composition registry to generalize iterative residualization to arbitrary causal-layer depth and emit one final multi-layer coherence certificate;
3. independent durable branch-head ingestion/retention remains OPEN; AI joins only already-covered durable revision frontiers;
4. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
5. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
6. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AL — first-class sequential/composite Rewrite family registry

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED substrate for exact identity-bearing composition of accumulated residual prefixes**.

Integrated:

- Added `RewriteSequentialFamilyId`, `RewriteSequentialFamilyKey`, `RewriteSequentialFamilySpec` and `RewriteSequentialFamilyRegistry`.
- Sequential composition is no longer represented by endpoint equality alone. A registered family binds the exact ordered pair of input `(RewriteSpecId, RewriteLawSetId)` identities to one exact composite family identity.
- `RewriteSequentialFamilyRegistry::certify` independently checks the two input family identities, the composite family identity, and the extensional sequential endpoint produced by applying `first` then `second`.
- Added `RewriteSequentialComposition` as the opaque successful composition certificate; callers cannot turn an endpoint-equivalent Rewrite with another intent family into the accumulated causal prefix.
- Registry IDs and ordered input-family pairs are one-to-one: family-ID rebinding and pair rebinding fail closed, matching the existing residual-family authority discipline.
- Hostile coverage rejects both a composite with the wrong exact Rewrite family and a composite carrying the registered family identity but the wrong endpoint.
- This closes the missing authority primitive identified by AK. It does not by itself claim arbitrary-depth residualization; the next consumer must use these certified composites when carrying cumulative opposite-prefix residuals between causal layers.

Verification after source freeze:

- `cargo fmt --all` / formatting clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **587 passed / 0 failed / 8 ignored**.

Current write-side frontier after AL:

1. generalize AK from exactly two singleton layers to arbitrary-depth singleton causal chains, certifying every accumulated opposite-prefix through `RewriteSequentialFamilyRegistry`;
2. non-singleton concurrent layers still need a higher-dimensional residual/cube consumer rather than arbitrary ordering;
3. independent durable branch-head ingestion/retention remains OPEN;
4. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
5. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
6. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AM — arbitrary-depth singleton causal-chain residualization

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for arbitrary-depth singleton-per-side causal chains with exact residual + sequential-family certification at every layer**.

Integrated:

- Added `RevisionEffectResidualChainWitness`, per-step witnesses/certificates, and `RevisionEffectResidualChainCertificate` as the general consumer of AJ causal-layer schedules for singleton branch chains.
- `certify_registered_residual_chain` accepts any positive causal depth when both branches expose exactly one event per causal layer and the witness count matches the schedule exactly.
- Layer 0 is certified by the exact residual-family registry. Every later layer independently transports the original branch Rewrite across the accumulated opposite-prefix Rewrite, then certifies the two transformed layer Rewrites against one another at the previous merged endpoint.
- For every non-final layer, the accumulated opposite prefix for the next iteration is itself rebuilt from two exact residual Rewrites and must pass `RewriteSequentialFamilyRegistry::certify`. A cumulative prefix therefore carries a registered composite Rewrite family identity; endpoint equality alone cannot advance the chain.
- Added internal `ResidualChainProgress` and a single-step certification helper so the production algorithm remains SRP-oriented and Clippy-clean rather than hiding complexity behind lint suppressions.
- A depth-three hostile exercises the complete mechanism: first cross residual, second-layer bilateral transport, registered sequential composition of both accumulated prefixes, third-layer bilateral transport, and final cross residual. The final endpoint is accepted only after all seven residual families and both sequential families are consumed exactly.
- Scope boundary: this closes arbitrary **singleton-per-side causal depth**. A causal layer containing two or more mutually concurrent effects still requires a higher-dimensional layer cube/normalization consumer rather than choosing an arbitrary intra-layer order.

Verification after final source freeze:

- `cargo fmt --all` / formatting clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **588 passed / 0 failed / 8 ignored**.
- Heavy package tests (`kernel-durability`, `kernel-plan`) were intentionally executed in separate tool-call batches after the combined batch hit the 45-second transport ceiling; both passed independently.

Current write-side frontier after AM:

1. generalize residualization from singleton causal layers to non-singleton concurrent layers by consuming registered cube/higher-dimensional coherence without imposing arbitrary event order;
2. bind a completed multi-layer chain certificate to the AH/AI coherent durable-resolution publication API rather than only exposing its final endpoint in `kernel-change`;
3. independent durable branch-head ingestion/retention remains OPEN; AI joins only already-covered durable revision frontiers;
4. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
5. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
6. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AN — non-singleton frontier cube + residual-chain durable publication

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for one 2×1 or 1×2 concurrent causal frontier cube and for publishing an already-certified arbitrary-depth singleton residual chain through the existing multi-parent durable resolution path**.

Integrated:

- Added `RevisionEffectResidualCubeLayerCertificate` and `RevisionEffectIdeal::certify_registered_residual_cube_frontier` as the first non-singleton causal-layer consumer.
- The admitted shape is exactly one concurrent frontier layer on each branch with three total exclusive events: `(2 left, 1 right)` or `(1 left, 2 right)`. No intra-layer execution order is used as proof authority: the three events are certified by the existing registered residual cube, which checks all three lower pairwise diamonds, all three upper residual faces, exact residual intent identity, and one common final endpoint.
- The deterministic event ordering used to name cube inputs is only a canonical argument mapping; certification still consumes the complete symmetric three-event cube rather than serializing same-branch concurrent effects.
- Shapes outside the certified three-event frontier remain fail-closed with `UnsupportedLayerShape`; arbitrary 2×2 or larger concurrent layers are not claimed closed.
- Refactored the multi-parent durable resolution publication boundary so the exact candidate endpoint/VMF check is shared rather than being tied specifically to a cube wrapper.
- Added `DurableRuntime::commit_derived_multi_parent_residual_chain_resolution`. An opaque `RevisionEffectResidualChainCertificate<RelationValue, I>` from checkpoint AM can now gate the same `RelationResolutionExact` WAL PREPARE/freshness/COMMIT/publication path as the earlier cube certificate.
- The runtime independently evaluates the concrete prepared relation candidate and requires equality with the chain certificate's final common endpoint, then requires the candidate-bound Γ-VMF zero certificate before durable PREPARE/publication. The chain proof therefore cannot authorize a different candidate merely by sharing transaction metadata.
- Existing cube-based multi-parent publication remains supported and now reuses the same endpoint-gated internal path. No new durable transaction format or duplicate proof authority was introduced.
- Hostile/regression coverage exercises the 2×1 non-singleton frontier cube and switches the existing multi-parent restart/causal-cut publication test to the residual-chain proof path, preserving exact parent-cut recovery and `RelationResolutionExact` identity.

Verification after final source freeze:

- `cargo fmt --all` / formatting clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **589 passed / 0 failed / 8 ignored**;
- heavy `kernel-durability` and `kernel-plan` tests were run in isolated tool-call batches to stay below the 45-second transport ceiling.

Current write-side frontier after AN:

1. generalize non-singleton residualization beyond a single three-event frontier: 2×2 and larger concurrent layers need registered higher-dimensional normalization/coherence rather than pairwise ordering;
2. compose non-singleton frontier certificates with AM multi-layer scheduling so a deeper causal chain may contain concurrent layers, not only singleton layers;
3. independent durable branch-head ingestion/retention remains OPEN; AI/AN publication still joins already-covered durable revision frontiers;
4. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
5. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
6. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AO — direct durable publication of certified non-singleton frontier cubes

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for carrying AN's certified 2×1 or 1×2 causal-frontier proof intact through the AI durable multi-parent resolution publication boundary**.

Integrated:

- Added `DurableRuntime::commit_derived_multi_parent_frontier_cube_resolution` taking `RevisionEffectResidualCubeLayerCertificate<RelationValue, I>` directly.
- The caller no longer needs to strip AN's causal-frontier membership wrapper and submit only its raw `RewriteResidualCubeCertificate`. The exact fact that the cube was certified over one admissible non-singleton causal frontier therefore remains part of the typed proof passed into publication.
- Publication reuses the same shared resolution endpoint boundary introduced in AN: the runtime independently evaluates the concrete prepared relation candidate, requires equality with the frontier certificate's common endpoint, requires exact candidate-bound Γ-VMF zero closure, and only then enters the existing `RelationResolutionExact` WAL PREPARE/freshness/COMMIT path.
- No new durable format or second proof authority is introduced; cube, residual-chain, and frontier-cube proof objects all converge on the same concrete endpoint + VMF publication gate while retaining their stronger typed construction boundaries before that gate.

Verification after final source freeze:

- `cargo fmt --all` / formatting clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **589 passed / 0 failed / 8 ignored**;
- `kernel-durability` and `kernel-plan` test suites were executed in separate tool-call batches under the 45-second transport ceiling.

Current write-side frontier after AO:

1. compose AN non-singleton frontier normalization with AM causal-layer iteration so deeper branches may contain concurrent layers instead of only singleton layers;
2. generalize one-layer non-singleton normalization beyond three total events: 2×2 and larger layers still require registered higher-dimensional coherence without arbitrary serialization;
3. independent durable branch-head ingestion/retention remains OPEN; publication still joins already-covered durable revision frontiers;
4. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
5. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
6. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AP — order-independent 2×2 concurrent frontier normalization + durable publication

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for one exact 2×2 concurrent causal frontier and direct AI durable publication of its typed proof**.

Integrated:

- Added `RewriteConcurrentPairWitness` / `RewriteConcurrentPairCertificate`. A two-event concurrent branch is accepted only after its registered residual diamond is certified and **both** sequential paths (`a -> b_after_a` and `b -> a_after_b`) independently certify to one exact intent-bearing composite Rewrite through `RewriteSequentialFamilyRegistry`.
- Same-branch event ID ordering is therefore only canonical argument naming; it is not semantic execution authority. If the two legal concurrent orders normalize to different exact composite Rewrite identities, certification fails closed with `ConcurrentCompositeIntentMismatch`.
- Added `RevisionEffectResidualSquareLayerWitness` / `RevisionEffectResidualSquareLayerCertificate` and `certify_registered_residual_square_frontier` for exactly one `(2 left, 2 right)` causal frontier layer.
- After each branch pair is order-independently normalized, the two exact composite Rewrite families must themselves pass a registered residual diamond at the original base. This yields one certified common endpoint for all four branch events without selecting an intra-branch serial order.
- Added `DurableRuntime::commit_derived_multi_parent_frontier_square_resolution`; the intact 2×2 frontier certificate gates the same concrete endpoint + candidate Γ-VMF + `RelationResolutionExact` WAL/freshness/COMMIT boundary used by prior coherent resolution proofs. No new durable format or second authority was introduced.
- Scope remains explicit: arbitrary `m×n` concurrent layers are not inferred from this 2×2 certificate. Larger same-branch antichains still require a higher-dimensional/associative normalization proof rather than repeated arbitrary pairing.

Verification after source freeze:

- `cargo fmt --all` clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **590 passed / 0 failed / 8 ignored**;
- heavy `kernel-durability` and `kernel-plan` tests executed in isolated batches under the 45-second tool-call ceiling.

Current write-side frontier after AP:

1. compose the 2×2 normalized frontier with AM causal-layer iteration so deeper causal chains may start from a concurrent square and continue with residualized later layers;
2. generalize branch normalization beyond two same-layer events without arbitrary grouping/order (associative/higher-dimensional normalization);
3. independent durable branch-head ingestion/retention remains OPEN;
4. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
5. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
6. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AQ — 2×2 concurrent first layer integrated into iterative residual-chain publication

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for a 2×2 normalized first causal layer followed by arbitrary-depth singleton-per-side residual layers, including direct AI durable publication of the completed proof**.

Integrated:

- Added `RevisionEffectResidualSquareChainWitness` / `RevisionEffectResidualSquareChainCertificate`.
- Refactored AP's 2×2 normalization into an internal typed `ResidualSquareLayer` helper so one-layer certification and deeper-chain certification consume the exact same residual/sequential authority rather than duplicating logic.
- `certify_registered_residual_square_chain` accepts a causal schedule whose first branch-exclusive layer is exactly `(2 left, 2 right)` and every later layer is exactly `(1 left, 1 right)`.
- The first square independently normalizes both same-branch concurrent orders to exact composite Rewrite families and certifies their cross-branch residual diamond. Its `right_after_left` / `left_after_right` residuals then become the accumulated opposite prefixes used by the existing AM `ResidualChainProgress` machinery.
- Every later singleton layer is therefore transported through the exact accumulated prefix, cross-certified, and (when another layer remains) re-composed only through `RewriteSequentialFamilyRegistry`, preserving AM's no-endpoint-only-composition rule after a non-singleton first layer.
- Added `DurableRuntime::commit_derived_multi_parent_square_chain_resolution`; the completed square-chain proof gates the same concrete endpoint + candidate Γ-VMF + `RelationResolutionExact` durable publication boundary used by AH/AI/AN/AO.
- Hostile/regression coverage exercises a 2×2 first frontier followed by one dependent singleton layer per branch and reaches the final endpoint only after three additional registered residual families consume the AP cross-prefixes.
- Scope remains explicit: a non-singleton layer appearing **after** layer 0 is not yet consumed by this chain, and branch antichains larger than two events remain OPEN.

Verification after source freeze:

- `cargo fmt --all` clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **591 passed / 0 failed / 8 ignored**;
- heavy `kernel-durability` and `kernel-plan` tests executed in isolated batches under the 45-second tool-call ceiling.

Current write-side frontier after AQ:

1. admit a 2×2 normalized concurrent layer at arbitrary positions inside a deeper causal schedule, not only as layer 0;
2. generalize same-layer branch normalization beyond two events without arbitrary grouping/order (associative/higher-dimensional normalization);
3. independent durable branch-head ingestion/retention remains OPEN;
4. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
5. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
6. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AR — arbitrary-position 2×2 causal layers in mixed residual chains

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for mixed causal chains whose paired branch layers are each either 1×1 singleton or 2×2 order-independently normalized concurrent squares, including direct durable publication**.

Integrated:

- Added `RevisionEffectResidualMixedChainWitness` / `RevisionEffectResidualMixedChainCertificate` with typed first-layer and subsequent-layer proof variants.
- A mixed chain may start with either a registered singleton residual diamond or AP/AQ's exact 2×2 square normalization. Every later paired causal layer may independently be either `(1 left, 1 right)` or `(2 left, 2 right)`.
- A non-initial 2×2 layer is normalized on the **current branch bases**, not the original revision: both same-branch concurrent orders must certify to one exact composite Rewrite family through `RewriteSequentialFamilyRegistry` before cross-branch processing.
- The two branch composites are then transported through the exact accumulated opposite residual prefixes using the same AM residual-chain step machinery. Their transported forms must pass a registered cross residual diamond at the current merged base.
- If another causal layer follows, accumulated prefixes are rebuilt only through registered sequential-family composition exactly as in AM. Endpoint equality alone still cannot advance the chain.
- This removes AQ's positional restriction: 2×2 squares may appear after singleton prefixes and may be repeated at later positions, provided each layer carries the exact residual/sequential witnesses required by the schedule.
- Added `DurableRuntime::commit_derived_multi_parent_mixed_chain_resolution`; the intact mixed-chain proof gates the existing concrete candidate endpoint + Γ-VMF + `RelationResolutionExact` WAL/freshness/COMMIT path without adding a second durable proof authority.
- Hostile/regression coverage includes a `1×1 -> 2×2` schedule, proving that a square after an accumulated residual prefix is normalized and transported without choosing one same-branch execution order as semantic authority.

Verification after source freeze:

- `cargo fmt --all` clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **592 passed / 0 failed / 8 ignored**.

Current write-side frontier after AR:

1. generalize same-layer branch normalization beyond two concurrent events without arbitrary grouping/order; antichains of size >2 still require associative/higher-dimensional normalization authority;
2. independent durable branch-head ingestion/retention remains OPEN;
3. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
4. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
5. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AS — exact three-event concurrent branch normalization substrate

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / R&D-to-production substrate CLOSED for order-independent normalization of exactly three concurrent Rewrite events; 3×N causal-layer consumption remains OPEN**.

Integrated:

- Added `RewriteConcurrentTripleWitness` / `RewriteConcurrentTripleCertificate` and `certify_registered_concurrent_triple`.
- The certificate consumes the existing registered three-event residual cube, so all three lower pairwise diamonds, all three upper residual faces, exact residual-family identities, and one common cube endpoint remain mandatory.
- Each unordered event pair is additionally normalized through `RewriteSequentialFamilyRegistry`: both legal orders of `(a,b)`, `(a,c)`, and `(b,c)` must certify to one exact intent-bearing pair composite family.
- The six complete event permutations are then checked as registered sequential compositions from those three pair composites through the appropriate cube residuals to **one exact final composite Rewrite family**. Endpoint equality alone is insufficient.
- No permutation is selected as semantic execution authority. Canonical naming of `a/b/c` only maps witnesses to the symmetric cube; acceptance requires all six permutation paths to certify the same final intent-bearing composite.
- Registry one-to-one authority remains enforced. A test-construction attempt to register the same exact `(pair composite, residual family)` twice under two sequential-family IDs was correctly rejected as `PairAlreadyRegistered`; the fixture now shares that canonical family instead of duplicating authority.
- Scope boundary: this checkpoint establishes exact size-3 same-branch normalization. It does **not** yet plug a three-event antichain into `RevisionEffectResidualMixedChainCertificate`, nor does it claim arbitrary-size antichain associativity/coherence.

Verification after source freeze:

- `cargo fmt --all` clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **593 passed / 0 failed / 8 ignored**.

Current write-side frontier after AS:

1. consume the new exact three-event branch composite inside causal layers (3×1, 3×2, 3×3) and the mixed-layer scheduler without selecting grouping/order;
2. generalize same-layer normalization from exactly three events to arbitrary finite antichains with an explicit higher-dimensional/associative coherence authority rather than inductive arbitrary grouping;
3. independent durable branch-head ingestion/retention remains OPEN;
4. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
5. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
6. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AT — normalized concurrent frontier widths 1..=3 + durable publication

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for a single paired causal frontier whose left and right branch antichains each have width 1..=3 and admit exact registered order-independent normalization**.

Integrated:

- Added `RewriteConcurrentBranchWitness` / `RewriteConcurrentBranchCertificate` as the branch-neutral normalization authority for one concurrent causal frontier.
- Width 1 consumes the exact singleton Rewrite; width 2 consumes the existing AP order-independent pair normalization; width 3 consumes AS's full six-permutation triple normalization. Unsupported widths fail closed with `UnsupportedLayerShape`.
- Added `RevisionEffectResidualNormalizedLayerWitness` / `RevisionEffectResidualNormalizedLayerCertificate` and `RevisionEffectIdeal::certify_registered_residual_normalized_frontier`.
- Each branch is normalized independently to one exact intent-bearing composite Rewrite family before any cross-branch merge reasoning. The two composites must then pass a registered residual diamond at the common base; endpoint equality alone is never sufficient.
- No intra-branch execution order becomes semantic authority: pair normalization still requires both orders, triple normalization still requires all six permutations, and the frontier certificate retains the corresponding proof object.
- Added `DurableRuntime::commit_derived_multi_parent_normalized_frontier_resolution`; the intact normalized-frontier proof gates the existing concrete candidate endpoint + revision-bound Γ-VMF `V=0` + `RelationResolutionExact` WAL/freshness/COMMIT path.
- Recovered interrupted-work hostile coverage for 3×1 was preserved and independently revalidated. Added explicit 3×2 hostile coverage proving that triple normalization on one branch and pair normalization on the other are each certified before the cross residual family is consumed.
- The generic production path is symmetric in the two branches and accepts any supported width pair in `1..=3`; this checkpoint does not claim normalization for width >3.

Verification after source freeze:

- `cargo fmt --all -- --check` PASS;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **595 passed / 0 failed / 8 ignored**;
- explicit recovered/new hostiles: 3×1 PASS; 3×2 PASS;
- heavy `kernel-durability` and `kernel-plan` tests executed as separate batches under the 45-second tool-call ceiling.

Current write-side frontier after AT:

1. generalize exact same-layer normalization beyond width 3 to arbitrary finite antichains with explicit higher-dimensional/associative coherence rather than arbitrary grouping/order;
2. consume normalized width-3 layers inside the deeper mixed causal-layer scheduler, not only as a single frontier proof;
3. independent durable branch-head ingestion/retention remains OPEN; current multi-parent resolution can cite already covered revisions but does not own independent branch heads;
4. historical restore availability is enforced, but actual state reconstruction through registered migration LensSpec implementations remains OPEN;
5. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
6. Project-over-owner before Join remains compositionally OPEN.

## Checkpoint AU — bijective Project-over-owner Join reconstruction

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for bijective owner Project permutations over a full-row owner filter chain before one-owner Join**.

Integrated:

- Extended the one-owner Join reconstruction boundary from `Scan(owner)` / full-row owner filter chains to `Project(permutation, Filter*(Scan(owner)))` pipelines.
- The accepted Join owner section is reconstructed in the projected coordinate space and every bijective Project stage is then inverted in reverse order before deriving the authoritative owner `RelationDelta`.
- Rejected owner rows are no longer computed as `Difference(Scan, projected_owner_query)`: the compiler/runtime keeps the pre-Project full-row filter query and derives the rejected complement against that exact full-row section, avoiding arity/coordinate corruption.
- Filter-after-Project remains fail-closed; lossy Project remains outside this path and continues to require its determinant/complement/constructor authority.
- Stage identity is checked exactly against the compiled owner pipeline; a manually drifted Project/filter stage sequence cannot enter the Join reconstruction path.
- Hostile/regression coverage uses `Project([1,0], FilterEqConst(Scan(owner))) ⋈ lookup`, rewrites the projected owner payload, and verifies both inverse-coordinate reconstruction (`[a,new]`) and preservation of the rejected full-row owner complement (`[z,hidden]`).

Verification after source freeze:

- `cargo fmt --all` clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **596 passed / 0 failed / 8 ignored**.

Current write-side frontier after AU:

1. actual historical state/value reconstruction through registered migration `LensSpec` implementations remains OPEN (availability/tombstone authority is already closed);
2. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
3. lossy Project-over-owner before Join remains OPEN; only bijective Project permutations are closed here;
4. arbitrary finite concurrent branch antichains beyond width 3 still require higher-dimensional normalization authority;
5. independent durable branch-head ingestion/retention remains OPEN.

## Checkpoint AU — bijective Project-over-owner Join reconstruction

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for owner-side pipelines consisting of full-row Filters followed by one or more bijective full-column Projects before a one-owner Join**.

Integrated:

- Recovered and completed the interrupted `OwnerJoinPipeline` path in `kernel-integration`; it is now verification-covered rather than an untracked source fragment.
- Owner-side Join synthesis no longer assumes the owner subquery is already in authoritative owner-column order. A supported owner pipeline may contain full-row `FilterEqConst` / `FilterEqColumns` stages followed by duplicate-free full-column `Project` permutations.
- Each Project is validated against the exact input arity as a bijection. Lossy/duplicate/out-of-range projections remain fail-closed with `CandidateGenerationUnsupported`; no hidden-column constructor is guessed.
- Join reconstruction first rebuilds the owner-side **visible projected section** from the requested Join endpoint plus unmatched owner-query rows, then applies the Project permutations in reverse to recover authoritative owner coordinates.
- Rejected Filter rows are preserved from the last full-row owner query, so the implementation never computes the invalid cross-schema `Difference(Scan(owner), projected_owner_query)`.
- The recovered owner state is independently re-evaluated through the original Join query; a forged lookup-side payload or otherwise inadmissible requested view still fails before Rewrite publication.
- Hostile/regression coverage includes `Filter(owner) -> Project([1,0]) -> Join`: the matched owner row is updated through projected coordinates while the rejected owner row remains unchanged.

Verification after source freeze:

- `cargo-fmt fmt --all -- --check` PASS;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **596 passed / 0 failed / 8 ignored**;
- explicit Project-over-filtered-owner Join hostile PASS.

Current write-side frontier after AU:

1. genuinely unseen lossy-Project values still require an explicit/certified semantic hidden-column constructor;
2. owner pipelines with a Filter **after** a Project remain fail-closed until rejected-section reconstruction is defined in projected coordinates and transported back exactly;
3. exact same-layer concurrent normalization remains bounded to branch width `1..=3`; arbitrary finite antichains require higher-dimensional coherence authority;
4. normalized width-3 frontier certificates are not yet composed as arbitrary positions inside the deeper mixed causal-layer scheduler;
5. independent durable branch-head ingestion/retention remains OPEN;
6. historical restore availability is enforced, but actual value/state reconstruction through registered migration LensSpec implementations remains OPEN.

## Checkpoint AV — historical structural restore registry + recovered normalized mixed first layer

Status: **INTEGRATED / DISTRIBUTED CHECK+CLIPPY VERIFIED / TARGETED TEST VERIFIED / FULL DISTRIBUTED TEST SWEEP COMPLETED BUT SUMMARY CAPTURE LOST TO TOOL TRANSPORT TIMEOUT**.

Integrated:

- Added runtime-only `HistoricalLensRegistry`, keyed by exact `(LensSpecId, SemanticManifestId, encoding_version)`; durable complement metadata remains the authority for which implementation identity is required, while executable implementation availability remains runtime capability.
- Added structural builtin historical lens implementations for `Identity` and `ProductField` and `LocalHistoricalComplementChain::restore_value`.
- Historical reconstruction consumes the already-retention-checked local complement chain strictly in reverse migration order. Missing exact implementation identity, manifest/version mismatch, released/missing complement payload, or malformed complement shape fail typed/closed instead of falling back to host semantics.
- Product-field restoration rebuilds logical `Value::Product` state from the target view plus the persisted logical complement and rejects complements that already contain the restored field.
- Added hostile/regression coverage for a two-step reverse reconstruction and for exact semantic-manifest binding failure.
- Recovered and completed the previously uncheckpointed post-AT normalized-first-layer mixed-chain substrate: `RevisionEffectResidualMixedFirstWitness::Normalized` can consume a width-1..=3 normalized concurrent frontier as the first mixed-chain layer through the existing residual/sequential registries rather than endpoint-only composition. Its internal helper is typed through `ResidualNormalizedMixedFirstLayer`, keeping Clippy-clean API shape.
- No new persistent executable/plugin bytes or second semantic authority were introduced.

Verification before the hard source freeze:

- `cargo fmt --all` clean;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- AU projected-owner Join hostile PASS;
- historical lens registry targeted tests: **2 passed / 0 failed**;
- a full distributed package-test sweep was launched and its cargo processes completed before the source-freeze boundary, but the enclosing tool call returned `TransportTimeoutError`; therefore this checkpoint intentionally does **not** claim a captured aggregate pass count from that sweep.

Current write-side frontier after AV:

1. expose the structural historical restore registry through the public runtime/query historical API and extend beyond Identity/ProductField only under certified LensSpec implementations;
2. genuinely unseen lossy-Project values still require explicit/certified semantic hidden-column constructors;
3. lossy Project-over-owner before Join remains OPEN; AU closes only bijective Project permutations;
4. arbitrary finite concurrent branch antichains beyond width 3 still require higher-dimensional normalization authority;
5. independent durable branch-head ingestion/retention remains OPEN;
6. obtain a freshly captured full distributed test aggregate in the next cycle before promoting AV verification status beyond the explicit statement above.

## Checkpoint AW — explicit fixed-hidden constructor for unseen lossy Project inserts

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for fixed hidden-column construction of genuinely-new projected Γ-classes**.

Integrated:

- Added first-class `FixedHiddenProjectInsertConstructor`, pinned to the exact owner relation and `RewriteSpecId`; it cannot act as an unscoped default policy for unrelated writable views.
- Lossy Project synthesis still prefers authoritative existing source representatives for already-known visible Γ-classes and still requires APNF determinant/revalidation there.
- A genuinely-new visible Γ-class may now be inserted only when every hidden owner column is supplied explicitly by the constructor. Visible columns always come from the requested projected row and cannot be overridden by constructor state.
- Constructor owner mismatch, RewriteSpec mismatch, out-of-range hidden columns, missing hidden columns, and visible-column override attempts fail typed/closed.
- Revalidation continues to require stable images on the shared before/after determinant domain. Domain extension is permitted only for rows that were actually built through the explicit constructor; the prior implicit `after_domain ⊆ before_domain` requirement remains in force when no new class was constructed.
- Added `commit_unique_relational_view_rewrite_with_project_constructor`; publication still flows through the existing authoritative derived relation Rewrite path and therefore re-runs DTC, VMF and freshness/WAL validation.
- No executable closure/plugin bytes, host-language default values, physical row IDs or second semantic authority were introduced.
- Hostile coverage inserts unseen projected class `b` with explicit hidden value and proves that an attempt to supply a constructor value for visible column 0 is rejected.

Verification after source freeze:

- freshly recaptured AV distributed package-test baseline: **598 passed / 0 failed / 8 ignored**;
- AW distributed package tests: **599 passed / 0 failed / 8 ignored**;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- `cargo fmt --all` clean;
- explicit unseen-class constructor hostile PASS.

Normalization priority decision:

- branch-antichain normalization remains intentionally bounded to width `1..=3` for now. General finite-width induction/folding is not required by the current production write consumers and is deferred until a concrete wider-antichain consumer requires it; no correctness claim depends on pretending width 3 is universal.

Current write-side frontier after AW:

1. expose historical structural restoration through the public runtime/query historical API while preserving the durability-owned complement authority and exact runtime LensSpec binding;
2. general executable/certified hidden constructors beyond the fixed-hidden fragment remain OPEN; AW closes only explicit fixed hidden assignments;
3. lossy Project-over-owner before Join remains OPEN; AU closes only bijective Project permutations;
4. independent durable branch-head ingestion/retention remains OPEN;
5. arbitrary finite concurrent branch antichains beyond width 3 remain deferred until they become a real runtime requirement.

## Checkpoint AX — public runtime historical structural restoration

Status: **INTEGRATED / TARGETED DEBUG+CLIPPY VERIFIED / PROD CLOSED for public runtime exposure of durability-owned structural historical restore**.

Integrated:

- Added `DurableRuntime::restore_historical_value(source_schema, target_schema, target_value, registry)` as the public runtime historical reconstruction entry point.
- The runtime does not introduce a second complement or Lens authority: it first resolves the durability-owned `LocalHistoricalComplementChain`, then consumes the exact runtime `HistoricalLensRegistry` binding already introduced in AV.
- Extended `DurableRuntimeHistoricalError` with typed `Restore(HistoricalRestoreError)` propagation; retention/tombstone failures and executable Lens implementation failures remain distinguishable.
- The production dependency boundary remains unchanged: `kernel-plan` consumes only `kernel-durability` historical types and does not gain a production dependency on `kernel-lens`.
- Extended the real schema-migration runtime hostile to persist a structural ProductField complement, bind its exact `(LensSpecId, SemanticManifestId, encoding_version)` implementation, and reconstruct the historical source-side structural value through the new runtime API.

Verification after source freeze:

- `kernel-plan` `cargo check --all-targets` PASS;
- `kernel-plan` Clippy `-D warnings` PASS;
- full `kernel-plan` package tests: **222 passed / 0 failed / 4 ignored**;
- targeted durable schema-migration + public historical restore hostile PASS;
- AW full-workspace captured baseline remains **599 passed / 0 failed / 8 ignored**; AX changes only the already-counted `kernel-plan` test body and add no new test case.

Current write-side frontier after AX:

1. extend historical reconstruction beyond builtin `Identity` / `ProductField` only through certified executable LensSpec implementations; no fallback to host closures;
2. general hidden-column constructors beyond AW's fixed-hidden fragment remain OPEN;
3. lossy Project-over-owner before Join remains OPEN; AU closes only bijective Project permutations;
4. independent durable branch-head ingestion/retention remains OPEN;
5. arbitrary finite antichain normalization beyond width 3 remains intentionally deferred until required by a concrete runtime consumer.

## Checkpoint AY — lossy Project-over-owner Join reconstruction

Status: **INTEGRATED / DISTRIBUTED DEBUG+CLIPPY VERIFIED / PROD CLOSED for lossy owner-side Project pipelines before one-owner Join when hidden reconstruction is APNF-certified or supplied by the explicit AW fixed-hidden constructor**.

Integrated:

- Removed AU's runtime-only bijectivity restriction from the owner-side Join pipeline. Duplicate-free lossy Project stages may now participate in `Filter*(Scan(owner)) -> Project* -> Join` while Filter-after-Project remains fail-closed.
- Join reconstruction first derives the complete requested projected-owner section from requested matched rows plus the old unmatched owner complement. It never attempts to infer hidden owner cells from Join output.
- Lossy inverse reconstruction reuses the existing planner-owned writable coordinates and APNF determinant machinery: deletions preserve hidden complements, existing Bag-class growth reuses only a determinant-stable authoritative source representative, and shared determinant images are revalidated after reconstruction.
- A genuinely-new projected Γ-class remains impossible without explicit authority. When supplied, AW's `FixedHiddenProjectInsertConstructor` is reused unchanged and remains pinned to the exact owner relation + `RewriteSpecId`; every hidden column must be explicit and visible columns cannot be overridden.
- Rejected pre-Project Filter rows remain a separate full-row complement and are merged back only after projected accepted-section reconstruction, preserving AU's no-cross-schema-Difference rule.
- The complete original Join is independently re-evaluated on the candidate model before preparing the owner `RelationDelta`, so lookup payload remains read-only and forged/inadmissible requested views still fail before publication.
- The public `commit_unique_relational_view_rewrite_with_project_constructor` path now forwards the exact constructor through Join synthesis as well as standalone lossy Project synthesis; no second publication path was added.
- Hostile coverage inserts unseen projected class `b` through a lossy owner Project before Join. The same request fails closed without constructor authority and succeeds with an explicit hidden value, producing authoritative owner row `[b, hidden-new]`.

Verification after source freeze:

- `cargo fmt --all -- --check` PASS;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- distributed package tests: **600 passed / 0 failed / 8 ignored**;
- explicit lossy Project-over-owner Join hostile PASS both for constructor-required rejection and constructor-authorized insertion.

Pass81 write-integration frontier after AY:

1. audit the original closed write-R&D bundles against production to distinguish any remaining **integration gaps** from post-integration feature work before declaring the write branch fully converged;
2. independent durable branch-head ingestion/retention remains OPEN, but must be classified by that audit before treating it as a Pass81 integration blocker rather than later systems work;
3. historical execution beyond builtin `Identity` / `ProductField` still requires an exact certified executable LensSpec implementation; no host-closure fallback is permitted;
4. general executable hidden-column constructors beyond AW's explicit fixed-hidden fragment remain OPEN and likewise require classification as an actual R&D integration obligation versus future extensibility;
5. arbitrary finite antichain normalization beyond width 3 remains intentionally deferred because no current production write consumer requires it.

### Post-AY canonical write-R&D convergence audit — classification only, no source change

The original `01_WRITE_INTEGRATION_CORE` bundles were re-audited against production after the lossy Project/Join gap closed.

**Already converged in production:** typed `FineChange` / first-class `RewriteSpec`; dependent Lens/complements; structural `WritableViewPlan`; Project/Filter/owner-Join synthesis with APNF/DTC/VMF obligations; conservative Rewrite-law inference; durable Rewrite identity; migration complement chains/retention and public structural historical reconstruction; Γ-REIC effect ideals/residual diamonds/cubes/multi-parent resolution publication for the currently supported bounded frontier; capability required-field correctness; non-monotone reads and structural ordering prerequisites.

**Actual remaining Pass81 write-R&D integration blocker:** stable Seq Rewrite intent. The canonical R&D explicitly distinguishes snapshot-local `SeqSplice(index)` from durable/concurrent intent addressed by stable semantic occurrence IDs/anchors, with typed outcomes such as missing occurrence, expired anchor history, same-gap ordering required and conflicting occurrence rewrites. Production currently has `SeqSplice` and a `SemanticWriteCoordinate::SeqAnchor` footprint coordinate but no first-class anchored Seq Rewrite intent/residual policy boundary. This is a genuine semantic-contract migration gap and should be the next Pass81 write checkpoint.

**Not Pass81 blockers under the canonical R&D scope:**

- `AggregateActionPolicy` is explicitly recommended only when writable `Group` is exposed; current `Group` remains conservative/read-only, so no write authority is missing today.
- Exact complement minimum-cardinality optimization is optional after a sufficient lens compiler; current correctness does not depend on minimum complements.
- arbitrary finite antichain normalization beyond width 3 remains deferred until a concrete wider runtime consumer exists; current consumers are not blocked.
- independent durable branch-head ingestion/retention is the mathematical-frontier/historical OPEN #8 (`durable revision DAG / branch+merge ancestry`). It is therefore deferred to the later historical-open-problem passes requested by the orchestrator rather than being used to hold Pass81 write integration open.
- transition-level erasure dependency enforcement remains required before exposing an erasure transition that can drop a dependency while retaining complements/anchors/artifacts. No such production erasure transition is currently used by the write publication path, so this is a future boundary requirement rather than an unsound active write path.

Accordingly, Pass81 should next integrate the stable anchored Seq Rewrite contract, then perform one final canonical write-bundle parity/hostile audit before declaring the write-R&D integration branch converged.

## Checkpoint AZ — stable anchored Seq Rewrite intent convergence

Status: **INTEGRATED / FINAL DIFFERENTIAL GATE VERIFIED / PASS81 WRITE-R&D BLOCKER CLOSED**.

Integrated:

- Added first-class `SeqOccurrenceId`, `SeqAnchorHistoryId`, `StableSeqGapAnchor`, `StableSeqOccurrence<T>`, `StableSeqSnapshot<T>` and `StableSeqRewriteIntent<T>` to separate durable/concurrent sequence intent from snapshot-local `SeqSplice(index)` execution.
- Stable intent resolves against semantic occurrence identities. Unrelated index drift does not change which occurrence/gap the intent addresses; the resolved `SeqSplice` is derived only at one authoritative snapshot boundary.
- Anchor interpretation is fail-closed. Missing occurrences, duplicate inserted occurrence IDs, expired anchor-history epochs and anchors whose retained endpoints no longer form a live gap are distinct typed failures.
- Stable inserts write the semantic gap plus new occurrence and read their left/right occurrence anchors; replace/delete write the exact occurrence coordinate. `SemanticWriteCoordinate::SeqOccurrence` complements the pre-existing `SeqAnchor` coordinate and keeps snapshot indices out of Rewrite footprints.
- Added conservative `StableSeqRewriteIntent::rewrite_footprint`: same-gap inserts and delete-of-anchor/insert cannot be admitted as `StrongCommute` by the generic Rewrite-law engine.
- Added `classify_stable_seq_pair` / `StableSeqPairDecision`: distinct inserts into one exact stable gap require explicit same-gap ordering; rewrites targeting the same occurrence conflict; deleting an occurrence used by a concurrent insertion anchor conflicts; independent stable occurrences remain coordination-free.
- `StableSeqPairDecision::coordination_decision` feeds the existing coordination boundary (`CoordinationFree` / `RequiresCoordination` / `IntentConflict`) rather than inventing a second scheduler.
- `RewriteSpec::prepare_stable_seq` preserves the anchored semantic intent in `PreparedRewrite.explicit_inputs` while producing the extensional `FineChangeKind::Seq` endpoint from the authoritative snapshot. Intent identity is therefore not reconstructed from the final sequence value.
- Snapshot-local `SeqSplice` remains the exact derivative/execution primitive; it is no longer the canonical durable/concurrent intent representation.
- Hostiles cover index drift, missing occurrence, expired anchor history, same-gap ordering, same-occurrence conflict, delete-anchor conflict, conservative generic footprint classification, and preservation of anchored intent through `PreparedRewrite`.

Verification:

- targeted stable-Seq tests: **5 passed / 0 failed**;
- `kernel-change` Clippy `-D warnings` PASS;
- `cargo fmt --all -- --check` PASS;
- distributed `cargo check --all-targets` across all **23 crates** PASS;
- distributed Clippy `-D warnings` across all **23 crates** PASS;
- full `kernel-change` tests: **39 passed / 0 failed / 0 ignored**;
- direct dependent runtime tests (`kernel-query`, `kernel-lens`, `kernel-integration`) PASS;
- AY frozen baseline before the single-file AZ source delta was **600 passed / 0 failed / 8 ignored**. AZ adds five passing `kernel-change` tests and changes no other production source file. A cold `kernel-plan` test-binary rebuild exceeded the bounded 30-second check slot and was not retried after the verification cutoff; its source is byte-identical to AY and its all-targets check/Clippy gate passed.

### Pass81 canonical write-R&D convergence result

The post-AY canonical audit identified stable anchored Seq Rewrite intent as the sole remaining integration blocker. Checkpoint AZ closes that blocker. The write-R&D integration spine is therefore **PROD CONVERGED for the currently exposed write surfaces and explicitly supported bounded concurrency fragments**.

Items intentionally deferred from Pass81 are not hidden integration gaps: writable Group/Aggregate action policy remains gated by the still-read-only Group surface; minimum-complement optimization is optional; antichain width >3 has no current production consumer; independent durable branch-head ingestion belongs to the historical durable revision-DAG OPEN; executable LensSpec/plugin expansion and richer hidden-column constructors are future extensibility; transition-level erasure dependency enforcement is required only before such erasure transitions are exposed.


## Pass81 FINAL CLOSEOUT

Pass81 is closed as the production convergence pass for the closed write-R&D branch. The sole post-AY canonical integration blocker — stable anchored sequence Rewrite intent — is now first-class and verification-covered. The authoritative historical problem ledger remains **22 compound OPEN**; those are deliberately handed to later passes rather than being mislabeled as Pass81 integration failures.
