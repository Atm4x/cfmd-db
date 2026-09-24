# IMPLEMENTATION REPORT — Pass89

## Scope

Pass89 rebases historical #12 onto the frozen Pass88 durability architecture without replacing Pass87 authority-uncertainty handling, Pass88 canonical recovered-state migration, or the existing authoritative WAL path.

## Changed production files

### `crates/kernel-durability/src/lib.rs`

Adds the WAL substrate required by exact cut/tail mirroring:

- `FileRevisionWal::create_at_lsn` for a shadow suffix beginning at the cut's exact next LSN;
- seeded WAL recovery for cross-cut unresolved PREPAREs;
- exact-frame append with contiguous-LSN validation;
- PREPARE/COMMIT append variants returning the encoded frame used by the authoritative WAL;
- public `PreparedCutCapsule` and `StreamingCheckpointProgress` exports.

The ordinary WAL APIs remain compatibility wrappers around the frame-returning path.

### `crates/kernel-durability/src/store.rs`

Adds the production streaming-checkpoint state machine:

- checkpoint format v2 chunk roots while retaining v1 monolithic decode;
- manifest v3 binding `H`, `E`, first tail LSN, certified tail LSN and object checksums;
- exact `PreparedCutCapsule` encoding/decoding with prepare LSN and payload checksum;
- unpublished `StreamingCheckpointJob` with pinned canonical checkpoint bytes, chunk progress, shadow WAL and mirrored/durable watermarks;
- `begin_streaming_checkpoint[_with_chunk_size]`;
- bounded `write_streaming_checkpoint_chunks`;
- `finalize_streaming_checkpoint` with full replay/certificate verification before manifest publication;
- exact-frame mirroring from all store PREPARE/COMMIT paths;
- shadow barriers aligned with authoritative durability barriers;
- reopen from chunked checkpoint + capsule + arbitrary-LSN WAL suffix;
- publication-prefix validation that permits legal later WAL growth;
- orphan chunk/capsule generation recognition for safe generation allocation/compaction;
- hostile tests for cross-cut transactions, interleaved writes, chunk corruption and unpublished shadow failure.

Unpublished shadow/chunk failure marks the job failed but does not poison or revoke the published generation. Authority uncertainty begins only at manifest publication, preserving Pass87 semantics.

## Verification

Frozen-tree gate:

- `cargo fmt --all -- --check` — PASS;
- `cargo check --workspace --all-targets` — PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS;
- `cargo test --workspace --all-targets` — PASS;
- declared tests: **664**;
- failed: **0**;
- ignored: **8**.

Targeted durability checks also passed: all streaming-checkpoint hostile tests and the complete `kernel-durability` suite (**63 tests** at the Pass89 checkpoint).

Frozen source fingerprint over Rust/Cargo production inputs: `852fc52eb7551e34916ac1342ee472ed08c43372c33a20ec51932f15d895c52c`.

## Problem → hypothesis → implementation → falsification → result

**Problem:** chunking checkpoint bytes alone is insufficient because a PREPARE can occur before cut and COMMIT after it, and writes can continue while chunks are emitted.

**Hypothesis:** one immutable cut H plus exact unresolved-PREPARE capsule and byte-identical WAL suffix mirroring is sufficient to reconstruct any publication endpoint E reached during the job.

**Implementation:** pin H, capture capsule, seed an unpublished shadow at the exact next LSN, mirror exact authoritative frames, track durable shadow watermark, encode one chunk-root checkpoint, and publish only after replay proves E.

**Falsification:** force a cross-cut transaction, interleave commits with chunk writes, corrupt a chunk, fail the unpublished shadow, and write again after publication before reopen.

**Result:** all hostile cases behave according to the protocol; old authority survives every pre-publication failure, and a published generation recovers its certified E prefix plus legal later WAL commits.

## Engineering follow-up outside historical #12 closure

The canonical checkpoint encoder currently materializes the logical stream into memory before chunk emission. A future implementation may stream encoding/decoding directly into chunk files to lower peak memory and overlap CPU/I/O. This must preserve exactly the same cut/capsule/shadow/publication contract; it must not create a second authority model.

# Pass90 implementation — OrderedView/pagination closure

## Production delta

Exactly one production source file differs from Pass89:

- `crates/kernel-plan/src/lib.rs`

Added `OrderedViewSpec`, `OrderedViewCursor`, `OrderedViewPage`, `OrderedViewError`, `PreparedOrderedView`, `PreparedPlan::ordered_view`, and native pinned page execution.

The cursor binds the exact physical revision, semantic revision, full pinned `SemanticContext`, logical relational expression and ordering specification. Entries are ordered by semantic order-class key, canonical semantic row key and duplicate occurrence ordinal. `PhysicalRowId` is deliberately excluded so continuation remains stable across physical rebuilds/layouts.

## Problem → hypothesis → implementation → falsification → result

**Problem:** existing structural ordering, Ordered SAMF and ordered TopK did not expose a resumable semantic pagination surface, and an opaque physical row-handle cursor would break across layout/rebuild boundaries.

**Hypothesis:** a cursor whose semantic boundary is the Γ order class and whose deterministic tie continuation is expressed only through Γ-canonical row identity plus duplicate occurrence ordinal is stable across native layouts while preserving semantic ties.

**Implementation:** compile `PreparedOrderedView` from a pinned prepared plan, validate order/equality congruence, execute against native revision-bound stores, derive canonical semantic order/row keys, group indistinguishable Bag occurrences, and bind continuation to the complete logical/Γ snapshot.

**Falsification:** paginate a tied Bag across RowStore, ValueColumnar, I64Columnar and TypedColumnar with one shared cursor; inject wrong physical revision, reversed ordering and same revision IDs with an altered pinned-module set; exercise unbound store and zero page size.

**Result:** all hostile cases pass and all four implemented backends return identical continuation semantics. Historical #6 is PROD CLOSED on its authoritative hostile gate.

## Verification

- `cargo fmt --all -- --check` — PASS
- `cargo check --workspace --all-targets` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace --all-targets` — PASS
- declared tests: **666**
- failed: **0**
- ignored: **8**
- production source delta: **1 file**
- source fingerprint: `a99ca7cf172e1a9f5c4f4d568b00028d7d328c339d7b06dab35f71ef8cc081c5`
- `kernel-plan/src/lib.rs`: `03fe5bd9221efdaffdb096f462e3e94652b7b954d6cd4db71094852d9722076d`

## Non-claim

Pass90 does not introduce specialized payload implementations for every historical `LayoutFamily` taxonomy label. It closes the current #6 correctness/semantic parity contract over all implemented row/value/typed native payload backends. Future specialized KeyValue/Adjacency/CSR/DenseArray/Inverted/Custom lowerings remain physical-performance engineering unless a new correctness contract requires them.

## Pass91 — semantic deployment boundary + REIC coordination classification

Pass91 intentionally left historical #21/#22 untouched because those rows are under external R&D ownership.

Implemented in `kernel-semantics`:
- explicit `SemanticContractIdentity` for defined versus opaque contracts;
- implementation artifact/runtime identities;
- semantic refinement certificates;
- independent artifact-authentication evidence;
- runtime allowlist/revocation policy;
- explicit `ExecutionAuthorization`;
- `SemanticDeploymentRegistry` and `SemanticExecutionCapability` over the operation-required contract subset;
- fail-closed executable installation.

Integrated in `kernel-durability`:
- durable builtin semantic-module reopening/replay now reconstructs a deployment registry and obtains authorization before installation;
- direct store-level installation loops were replaced by the package boundary;
- builtin authentication remains binary-provided trust evidence and is not presented as an external signature verifier.

Advanced historical #8 by adding stable `DurableEffectKind` and explicit `OpaqueNonConfluent` coordination classification to durable effects. This makes the replication boundary conservative: there is no implicit coordination-free REIC merge without a durable certificate.

Historical #10 and #8 remain production-partial rather than closed. #10 still lacks the external CAS/signature/trust-root/sandbox/ABI stack. #8 still lacks independent durable branch-head ingestion/retention and therefore meets #16 at the replication authority boundary.

---

## Pass92 — durable branch REIC authority

Added `kernel-durability::replication` and integrated it with store create/open. Non-head effects now persist independently without weakening the linear transaction WAL. Admission verifies exact causal cuts, semantic execution availability, effect namespace identity, sequencer slot uniqueness and sequencer epoch fencing. Durable branch heads/retirement and mixed local+replicated causal ideals survive restart. Historical #8 is closed; #16 is materially advanced but remains open for membership/quorum/election/network semantics.

## Pass93 — replication membership/quorum authority

Pass93 adds durable replication membership epochs, quorum certificates and explicit local/quorum/published stages to the Pass92 branch journal. Publication is branch-contiguous and cannot outrun quorum durability of replicated causal prerequisites. Reopen reconstructs membership, certificates and published branch heads. #16 remains partial because authenticated voter-side vote-once/election/network consensus is not yet implemented. Full frozen gate: 680 tests, 0 failures, 8 ignored; fmt/check/strict Clippy PASS.


# IMPLEMENTATION REPORT — PASS94

## Production source delta

Exactly five source files differ from frozen Pass93:

1. `crates/kernel-durability/src/lib.rs`
2. `crates/kernel-durability/src/replication.rs`
3. `crates/kernel-durability/src/store.rs`
4. `crates/kernel-query/src/lib.rs`
5. `crates/kernel-query/src/delta_abi.rs` — new

## Durability / consensus changes

- added `ReplicationEffectVote` and `ReplicationMembershipVote` public authority-side values;
- added fsync-backed effect-vote and membership-vote frames to the existing replication journal;
- effect vote-once key: membership epoch + global ordered position + voter;
- membership vote-once key: previous membership epoch + voter;
- quorum certification now requires matching durable effect votes for every acknowledgement;
- non-bootstrap membership installation now requires same-term durable votes for the exact successor membership;
- replay reconstructs votes before dependent quorum/membership decisions;
- new restart hostiles reject conflicting effect votes across leaders and conflicting membership successors.

The implementation deliberately does not authenticate peers itself. The caller/security transport must authenticate voter identity before recording a vote; numeric `ReplicaId` alone is not trust evidence.

## V5 Delta ABI Stage 1

New `kernel-query::delta_abi` contains:

- `Weighted<R>`;
- `DeltaView<R>`;
- `DeltaSink<R>`;
- `CompactDelta<R>`;
- `InlineDelta<R, N>`;
- `AdaptiveDelta<R, N>`;
- `RelationDeltaView<'a>`.

`RelationDelta::as_delta_view()` exposes the compatibility view without allocating or cloning rows. `AdaptiveDelta` starts in fixed array-backed inline storage, spills after capacity is exhausted, and retains spill allocation across `clear()` for reuse.

No existing maintained execution function was redirected to this ABI in Pass94.

## Verification

- `cargo fmt --all -- --check` — PASS
- `cargo check --workspace --all-targets` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace --all-targets` — PASS
- declared tests: **685**
- failures: **0**
- ignored: **8**

Frozen source fingerprint: `b28009479bcbfa8eb84ece69108f05c680d0f82d855b310d9e69d1dfd9386da8`.

# IMPLEMENTATION REPORT — PASS95

## V5 Stage 2 — validated transition frames

`kernel-query` now exports `CompiledDeltaEdgeIdentity` and generic `ValidatedTransitionFrame<P,D>`.

The legacy maintained-plan `RelationDelta` path was changed from:

1. validate leaf and compute `RelationMutationPlan`;
2. discard plan;
3. recurse to Scan;
4. recompute the same plan;
5. commit;

to:

1. validate leaf once;
2. retain `RelationMutationPlan + certified RelationDelta` in a frame keyed by deterministic DFS edge identity;
3. recurse with a `RelationDeltaApplyContext`;
4. consume the exact frame at Scan and commit the retained plan.

A hostile self-join test uses the same base relation at two Scan leaves and verifies that the two prepared frames do not alias and the final maintained result equals recomputation.

## V5 Stage 3 — compiled linear islands

Added `kernel-query/src/linear_island.rs` with:

- `LinearIslandPredicate`;
- `LinearIslandNormalForm`;
- `CompiledDeltaProgram`;
- Γ-aware normal-form execution over the universal `DeltaView` ABI.

`RelDifferentialProgram::compile` now derives this physical companion from the same validated `RelExpr + Γ`. Maximal linear chains rewrite predicates to source coordinates and collapse intermediate Bag projections. Zero-crossing and stateful operators remain barriers.

A differential hostile builds `Filter -> ProjectBag -> Filter`, verifies both predicates were rewritten to source column 0, executes the fused normal form, and compares its signed result with the current maintained engine.

## Deliberate stop before Stage 4

Pass95 does not begin ZeroCrossing/Annotation/OrderedBoundary/BilinearPullback/Blocker kernel migration. This avoids a checkpoint with mixed old/new state ownership. Stage 4 begins in the next pass from a clean Stage-2/3 substrate.

## Verification

- `cargo fmt --all -- --check` — PASS
- `cargo check --workspace --all-targets` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace --all-targets` — PASS
- declared tests: **687**
- failed: **0**
- ignored: **8**
- production source delta versus frozen Pass94: **3 files**
  - `crates/kernel-query/src/delta_abi.rs`
  - `crates/kernel-query/src/lib.rs`
  - `crates/kernel-query/src/linear_island.rs` — new
- frozen path-stable source fingerprint: `8a574d7e582b049a33f04de1359296ad1fee8daffef45a7afb9567e18e753ddc`

## Pass96 — V5 Stage 4 framework + ZeroCrossing

Changed only `kernel-query` production source. Added universal planned-effect unary/binary kernel contracts, explicit barrier classes on `CompiledDeltaProgram`, and migrated set-support maintenance to read-only planning plus patch commit over `DeltaView`. `Distinct` consumes the zero-copy compatibility view; set projection emits an adaptive signed carrier into the same planner. Added hostile plan-before-commit/underflow and complete barrier-classification tests. Full workspace fmt/check/strict-Clippy/tests pass. Annotation/Group and later barrier classes remain intentionally unported.

## Pass97 — V5 Stage 4.2 Annotation / Group

Group is now a certified Delta-ABI plan/commit barrier. Generic Γ-aware Group and exact-I64 Count consume `DeltaView`, emit `AdaptiveDelta`, and mutate only at explicit commit. Exact-I64 Count adds an admitted dense `ExactCount` window with pre-commit sparse fallback and singleton count-object move optimization. Full workspace gates pass; Stage 4.3 TopK remains untouched.

# IMPLEMENTATION REPORT — PASS98

## V5 Stage 4.3a — scalar-I64 TopK physical lowering

Added `crates/kernel-query/src/topk_i64.rs` and replaced the scalar-I64 `MaintainedTopKStorage::I64Scalar(BTreeMap<...>)` payload with `I64TopKState`.

`I64TopKState` owns three physical tiers: `DenseUnit`, `DenseCounted`, and `PagedRadix`. Dense admission uses a bounded span/memory window. Duplicate multiplicity causes fail-atomic unit-to-counted promotion; dense window escape causes fail-atomic paged-radix fallback. The semantic result is unchanged by the selected tier.

The scalar maintained path now plans from `RelationDeltaView` / `DeltaView<Row>` into a candidate `I64TopKState` and `AdaptiveDelta<Row,4>`, then commits the prepared physical state only after planning succeeds. Unit replacement uses cached threshold metadata and adjacent threshold repair rather than rebuilding the selected prefix. General signed packets retain exact semantics through the same state planner.

Two new hostiles cover physical promotion/fallback without mutating the source state and 2,000 sequential replacements in both order directions against a full TopK oracle. During development the latter exposed a bug in the hostile generator itself: a same-key replacement had been constructed with duplicate `BTreeMap::from` keys, retaining only `+1`; production coalescing was correct. The generator was fixed to accumulate signed weights before comparison.

Existing maintained TopK tests for ties, ascending/descending order, Set constraints, semantic ordering, threshold movement, atomic missing-removal rejection and Join->Group->TopK trees remain green.

## Deliberate stop

Stage 4.3 is not marked complete. `I64Rows` and `SemanticOrdered` still use the legacy mutating TopK branch and compatibility output calculation. Pass99 should migrate those branches to one read-only OrderedBoundary plan/commit patch and universal output carrier before beginning Stage 4.4 Join.

## Verification

- `cargo fmt --all -- --check` — PASS
- `cargo check --workspace --all-targets` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace --all-targets` — PASS
- declared tests: **693**
- failed: **0**
- ignored: **8**
- source delta vs frozen Pass97: **2 Rust files**
  - `crates/kernel-query/src/lib.rs`
  - `crates/kernel-query/src/topk_i64.rs` — new
- frozen path-stable source fingerprint: `a9db7920683bacfe6c910d09a95e4325cf1295d18252075bf40af3b811de1b76`


# IMPLEMENTATION REPORT — PASS99

## V5 Stage 4.3b — universal OrderedBoundary completion

Pass99 removes the remaining legacy mutation/output protocol from non-scalar `I64Rows` and generic Γ-ordered TopK. `MaterializedTopKDeltaState` now owns one planner that consumes `DeltaView<Row>` and returns `PlannedDeltaEffect<TopKDeltaPatch, AdaptiveDelta<Row,4>>` for all physical storage variants. Public `RelationDelta` construction happens only after successful planning; state mutation happens only in `commit_topk_patch`.

`I64RowsMutationPlan` contains only affected bucket replacements. Planning validates semantic removals and Set uniqueness on private bucket copies, normalizes signed removals before insertions, derives the selected WITH-TIES output effect, and leaves authoritative buckets untouched until commit. `SemanticOrdered` generalizes its existing indexed mutation planner to arbitrary `DeltaView` weights; candidate preview is private and the live semantic index is unchanged until the prepared id/key/row patch commits.

The old `apply_i64_delta` mutating path and the TopK-wide `before = output_value(); mutate; after = output_value(); relation_delta_between_values(...)` compatibility calculation are removed from the maintained transition path.

Hostile tests force both migrated storage classes and prove recomputation parity plus atomic failure. Full workspace fmt/check/strict-Clippy/tests pass: **695 declared / 0 failed / 8 ignored**. Frozen production-source fingerprint: `9e7645c1001bf32e04ad03717ac556923aa145396249127270ed9f699889434a`.

# IMPLEMENTATION REPORT — PASS100

## V5 Stage 4.4 — universal BilinearPullback / Join

Pass100 replaces the three maintained Join mutation protocols with one binary read-only `DeltaView` planner. `MaterializedJoinDeltaState::plan_delta_views` validates both carriers, produces one typed `JoinDeltaPatch` for the selected physical backend, and emits `AdaptiveDelta<Row,4>` directly from the bilinear differential. `apply_input_deltas` is now only the compatibility wrapper that materializes the effect after planning and then commits the patch.

Exact-I64 uses checked affected-bucket patches. Primitive Γ and structural Γ backends generalize their indexed side mutation planners to arbitrary signed `DeltaView` weights. The output path evaluates left changes against old right and right changes against planned left, preserving the simultaneous `ΔL ⋈ ΔR` cross-term exactly once.

A new hostile drives weighted `AdaptiveDelta` input (`+2`) through simultaneous left/right changes, proves recomputation parity, proves plan-before-commit immutability, then forces a valid-left/invalid-right binary plan and proves atomic rejection. Existing primitive Γ, structural Γ, two-sided I64, recursive maintained Join and Join→Group→TopK tests remain green.

Full workspace fmt/check/strict-Clippy/all-target tests pass: **696 declared / 0 failed / 8 ignored**. Frozen production-source fingerprint: `d21904012350dd116225d73e3e1c587287504be2926b3396a1f7dff26b7cb03b`.

# IMPLEMENTATION REPORT — PASS101

## V5 Stage 4.5 — universal BlockerZeroCrossing

Pass101 replaces the maintained Difference/AntiJoin `recompute whole blocker value -> diff snapshots -> overwrite value` path with a Γ-local binary read-only planner. `MaterializedBlockerDeltaState` owns the physical blocker state, consumes two `DeltaView<Row>` carriers, returns `PlannedDeltaEffect<BlockerDeltaPatch, AdaptiveDelta<Row,4>>`, and mutates only during explicit commit.

Difference stores per-full-row Γ classes with left/right fibers and emits only the change in truncated natural subtraction. AntiJoin stores per-join-key left fibers plus right support count and enumerates the whole left fiber only on a real blocker zero-crossing. The production AntiJoin implementation also fixes a weakness in the V2 oracle prototype: when a key remains unblocked, equal fiber cardinality is not treated as equality of effect; actual left removals/insertions are propagated, so a same-key row replacement remains observable downstream.

Hostile tests cover weighted Difference crossing (`+2` blocker support), plan-before-commit immutability, underflow rejection, same-key AntiJoin replacement at unchanged fiber size, weighted blocker activation, two-sided recursive Difference/AntiJoin parity with full recompute, and whole-tree fail-atomic rejection.

Full workspace fmt/check/strict-Clippy/all-target tests pass: **700 declared / 0 failed / 8 ignored**. Production source delta versus frozen Pass100 is exactly `crates/kernel-query/src/lib.rs`.

# Pass102 implementation — Stage 5 root-only compatibility materialization

## Production delta

Exactly one production source file differs from Pass101:

- `crates/kernel-query/src/lib.rs`

`MaterializedRelPlanState` now uses `AdaptiveDelta<Row,4>` as the recursive maintained carrier for both ordinary validated leaf deltas and storage-resolved leaf deltas. Internal Filter/FilterColumns/ProjectBag/ProjectSet/Distinct/PromoteToBag/Join/Group/TopK/Difference/AntiJoin propagation no longer returns or constructs compatibility `RelationDelta` objects.

The existing operator-specific two-phase planners from Stages 4.1–4.5 are consumed directly: their `planned.effect` is forwarded to the parent and their patch is committed only after planning succeeds. Join and blocker consume both child carriers directly. Set support, Group and TopK likewise consume `DeltaView` without an owned compatibility conversion.

Public leaf ingress remains `RelationDelta`; storage-resolved ingress remains `StorageResolvedRelationDelta`. The root public result is materialized once after successful recursive propagation. This deliberately preserves compatibility and revision/persistence boundaries while removing repeated internal allocation/materialization.

Bag projection keeps its previous Γ-normalization contract. Weighted child entries are projected, expanded only for the existing semantic cancellation algorithm, Γ-cancelled, then returned as the internal carrier; Stage 6 owns any further allocation optimization of this normalization implementation.

## Hostile falsification

The existing nontrivial recursive `Filter -> Project -> Join -> Project -> Group -> TopK` test now instruments compatibility materialization per test thread:

- ordinary maintained transition: exactly **1** `RelationDelta` materialization;
- storage-resolved maintained transition: exactly **1** materialization;
- invalid transition rejected before publication: exactly **0** materializations and state unchanged.

The full existing recompute-oracle assertions remain in the same test, so the allocation-boundary assertion is tied to exact output/state semantics rather than a synthetic kernel-only path.

## Verification

- `cargo fmt --all -- --check` — PASS;
- `cargo check --workspace --all-targets` — PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS;
- `cargo test --workspace --all-targets --quiet` — PASS;
- declared tests: **700**;
- passed: **692**;
- failed: **0**;
- ignored: **8**.

Production source fingerprint over Cargo/Rust production inputs: `dffeef1669763a37c9f48f7a753acc25d048513ea2005aefd2b17be1e5e64964`.

`crates/kernel-query/src/lib.rs`: `743e86dc8a96237d936e548944e2d57f511bb1ffc4808e6040f0ddce9a841e62`.

## Problem → hypothesis → implementation → falsification → result

**Problem:** Stage 4 kernels already produced carrier-native signed effects, but `MaterializedRelPlanState` immediately re-expanded them into owned `RelationDelta` objects at each recursive edge, paying allocation/copy cost and obscuring the universal Delta ABI.

**Hypothesis:** if recursive execution forwards one `AdaptiveDelta` carrier and only the root/public boundary materializes compatibility output, all maintained semantics and fail-atomicity remain unchanged while internal compatibility allocation disappears.

**Implementation:** replace recursive return types with `MaintainedDelta`, add carrier-native linear transforms, consume state-kernel `planned.effect` values directly, convert public leaf deltas only at ingress, and materialize once at root.

**Falsification:** exercise the deepest existing mixed linear/stateful recursive chain through ordinary and storage-resolved ingress, compare every output/state against recompute, count compatibility materializations, and inject an invalid transition.

**Result:** both successful paths materialize exactly once, the invalid path materializes zero times, all semantic oracles remain equal, and the complete workspace gate is green.

## Non-claim

Pass102 does not claim Stage 6 performance/allocation closure. ProjectBag Γ-cancellation still uses temporary row vectors, generic barriers may still have fallback allocation, and no corrected whole-chain benchmark comparison is claimed here. Historical #21/#22 therefore remain open pending Stage 6.
