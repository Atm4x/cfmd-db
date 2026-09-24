# PASS60 REPORT — Program3 dense-runtime closeout integration

Status: **VERIFIED**

Source window: **2026-09-20 22:31:03 → 22:37:09 +03:00 = 6:06**.
Production source was frozen at 22:37:09. All later work is verification, reports/spec/evidence and packaging only.

## Closed exactly in Pass60

### 1. One revision-owned dense identity coordinate system

Pass58 introduced `LocalEntityId`, `DenseEntityIds`, `DenseEntitySet` and a reconstructible dense lifecycle projection, but individual consumers could still compile their own coordinate systems.

Pass60 integrates the R&D closeout so `Revision` owns one `Arc<DenseEntityIds>` and all revision-scoped physical derivatives can bind to that exact map. `EntityId` remains the logical/durable identity; `LocalEntityId` remains reconstructible and revision-local only. `Revision::dense_entity_ids()` returns shared clones and the hostile test checks `Arc::ptr_eq`.

### 2. Dense type/capability extents replace repeated carrier/subtype scans in validation

`DenseTypeExtents` compiles subtype-aware membership into dense bitsets over the revision-owned LocalId universe. Field-owner validation and `LiveEntityRef` type membership now query these extents rather than rescanning model carriers and subtype closure for every probe.

The extent compiler has both an independent constructor and `compile_with_ids`, and revision validation uses the latter so no second LocalId coordinate system is created.

### 3. Maintained dense lifecycle is an exact local LFP derivative

`MaintainedDenseLifecycle` stores LocalId forward/reverse adjacency, dense roots and the current live set. Fact removals recompute only the forward-affected region and retain nodes with surviving external support; additions/resurrections propagate only from newly supported nodes.

Pass60 preserves the Pass58 exhaustive 512-case three-node projection law and adds the R&D closeout hostile suite: all single-fact toggles on all three-node graph shapes plus explicit cycle death/resurrection. The maintained result is checked against the reference `LifecycleGraph::live_entities()` exact least fixed point.

### 4. Reverse LiveRef sensitivity removes repeated whole-model dangling-reference rescans

`LiveRefSensitivityIndex` recursively indexes nested live references from fields and relation rows by LocalId target. `DatabaseState::normalize` builds the sensitivity derivative once, runs lifecycle normalization, then restricts affected model locations through the reverse index.

Existing semantics are preserved: surviving fields that reference unknown/dead live entities remain errors, while relation rows containing dead/unknown live references retain the existing removal behavior.

### 5. LocalId-backed physical LiveEntityRef columns

`NativeColumn::DenseLiveEntityIds` stores `Vec<LocalEntityId>` together with the exact shared `Arc<DenseEntityIds>` needed for reconstruction. Logical materialization still returns external `Value::LiveEntityRef { id: EntityId, ... }`; typed equality filtering maps the external predicate ID once and compares LocalIds in the hot loop.

Legacy `NativeColumn::LiveEntityIds<Vec<EntityId>>` remains supported. This is an additive physical family, not a logical/durable compatibility break.

## Rebase/hostile review of the R&D package

The package targeted Pass57, while Pass58 had already integrated the first dense-identity/lifecycle slice. It was therefore **not** applied blindly.

- overlapping identity/lifecycle hunks were rebased semantically;
- the Pass58 exhaustive 512-state projection test was retained;
- Pass59 Γ-QCN changes were preserved;
- an unrelated stale `pub mod algebraic_native;` reference from the old R&D branch was explicitly rejected because Program4 is not part of this closeout and the module file is absent;
- no persisted LocalId encoding was introduced.

Changed production surfaces relative to Pass59 are limited to `kernel-identity`, `kernel-lifecycle`, `kernel-model`, `kernel-validation`, `kernel-revision`, `kernel-plan` and their dependency metadata.

## Diagnostic evidence on the rebased Pass60

Release diagnostics are fixture-specific, not universal speed claims.

Maintained lifecycle local-cut fixture, 50k-node chain:

- reference tree recomputation: **353,310,285 ns**;
- maintained dense LFP: **1,530,465 ns**;
- ratio: **~230.85x**.

Dense subtype/type membership fixture, 128 carriers × 256 entities:

- one-time extent compilation: **7,810,815 ns**;
- repeated carrier/subtype scan: **198,211,042 ns**;
- dense membership: **4,941,448 ns**;
- hot lookup ratio: **~40.11x**.

The R&D package reported lower final frozen ratios on its Pass57 environment (~115.4x lifecycle and ~9.98x type lookup); Pass60 records its own rebased numbers rather than treating either measurement as universal.

## Authority boundary

- semantic/durable identity remains `EntityId`;
- `LocalEntityId` is revision-local and reconstructible;
- no LocalId is serialized into checkpoint/WAL logical formats;
- `DenseLiveEntityIds` carries the exact map required to reconstruct external IDs;
- dense extents/lifecycle/sensitivity are physical derivatives, not semantic authority;
- legacy external-ID physical columns remain admitted.

## Historical OPEN accounting

Pass59 historical OPEN count: **22**.

- Historical OPEN fully closed this pass: **0 / 22**.
- Historical OPEN remaining: **22**.
- Genuinely new OPEN created: **0**.

Advanced but still OPEN:

- dense LocalId is not yet the representation of every graph/operator hot path; adoption remains measurement-driven;
- Program4 algebraic native layout remains R&D, not production;
- Γ-QCN structural/custom factors and durable recursive structural-key encoding remain OPEN;
- per-key support counters can make touched Γ-QCN components finer-grained;
- outer artifact-map metadata remains ordinary `BTreeMap<K, Arc<State>>` after Pass57 COW;
- remaining physical layouts/OrderedView, durability/distribution/authentication/formal-proof fronts remain unchanged.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Final frozen bytes PASS:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets --release`
- `cargo build --workspace --release`
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release`

Cold release and overflow invocations that hit the external compilation timeout were not counted; warmed retries completed successfully.

Static snapshot before packaging:

- **387 declared tests**;
- **137 `kernel-plan` tests**;
- **11 `kernel-lifecycle` tests**;
- **11 `kernel-validation` tests**;
- **21 crates**;
- **52,819 Rust LOC**;
- **0 `unsafe` hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**;
- **0 external registry/git Cargo sources**.

## Next frontier

Program3 is now coherent across revision ownership, validation, lifecycle, reverse LiveRef sensitivity and a typed physical LiveEntityRef family. The next pass should therefore return to a broader remaining frontier rather than duplicating dense identity infrastructure: structural Γ-QCN factors, per-key QCN support counts, measured adoption of LocalId in another graph-heavy consumer, or the artifact-cardinality falsifier before any persistent-map replacement.
