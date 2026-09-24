# CFMD Pass71 Report — durable physical artifact recipes and recovery rebuild

Date: 2026-09-21
Status: **VERIFIED (reconstructed checkpoint)**
Authoritative base: verified Pass70
Reconstruction freeze: see `evidence/pass71/PASS71_RECONSTRUCTION_FREEZE_V2_UTC.txt`

## Goal

Close the runtime-only gap left after Pass69/70: selected reconstructible physical artifacts must survive checkpoint/compaction/reopen without turning `PhysicalRowId`, layout-local buckets or allocator representation into durable logical authority.

## Problem → hypothesis → implementation → falsification → result

### 1. Persisting raw physical index payloads would violate authority

**Problem.** Semantic-index buckets contain layout-local `PhysicalRowId`. Recovery rebuilds a new `PhysicalStore` and can assign different physical handles. Persisting those buckets directly would smuggle stale physical identity across reopen.

**Hypothesis.** Persist a versioned *artifact recipe* describing the logical physical derivative, then rebuild a fresh payload from authoritative `Revision=(S,Γ,M)` under the recovery layout.

**Implementation.** Metadata codec v5 carries durable physical-artifact recipes with an independent recipe-format revision. Production recovery recipes cover:

- semantic indexes;
- Γ-QCN quotient factors;
- semantic statistics.

Recipes encode logical relation/key semantics and advisor/manual ownership, never raw row handles.

**Falsification.** Reopen tests require the reconstructed semantic index to serve a real `FilterEqConst` and increment `persisted_index_hits`, rather than merely appearing in a catalog. Structural `Option<Text>` + ASCII-CI index recovery is covered as well.

**Result.** Reconstructible physical intent survives checkpoint/compaction/reopen without making physical payload authority durable.

### 2. Stale derived metadata must not block logical recovery

**Problem.** A syntactically valid recipe can become semantically stale after WAL-tail/full-revision migration beyond the checkpoint.

**Hypothesis.** Separate metadata-format validity from optimization compatibility. Corrupt/unknown recipe format must fail closed; a valid but stale recipe must be dropped while logical Revision recovery proceeds.

**Implementation.** Recovery decodes recipe metadata first, then attempts exact reconstruction against the recovered authoritative Revision and pinned Γ/key-binding. Incompatible recipes are omitted as derived state.

**Falsification.** A stale recipe naming an unknown relation is checkpointed and reopened. The Revision recovers exactly while the stale optimization disappears.

**Result.** Derived physical metadata cannot make logical recovery unavailable.

### 3. Layout-independent recipe collapse must preserve pinning semantics

**Problem.** Multiple physical instances of the same logical semantic artifact can exist on different layouts. A layout-independent durable recipe collapses them. If one instance is manual-pinned and another advisor-owned, arbitrary iteration order must not decide future eviction semantics.

**Hypothesis.** Manual ownership dominates when equivalent recipes collapse.

**Implementation.** Recipe canonicalization groups without layout identity and marks the recovered artifact advisor-managed only when all collapsed source instances were advisor-managed.

**Falsification.** Duplicate equivalent artifacts with mixed ownership are serialized in adversarial order.

**Result.** Reopen never turns a manually pinned artifact into an advisor-evictable artifact.

### 4. I64 persistence deliberately remains outside this pass

Persisted exact-I64 indexes require typed-columnar physical lowering. Recovery currently boots a generic row-store layout. Rebuilding an I64 index recipe on the wrong layout would be a false closure. I64 durable recovery therefore remains tied to the broader durable physical-layout recipe frontier.

## CLOSED exactly in Pass71 — 3 narrower production defects

1. Reconstructible semantic physical artifacts lost their materialization intent across checkpoint/reopen.
2. Valid-but-stale physical recipes could otherwise threaten logical recovery instead of being discarded as optional derived state.
3. Layout-independent recipe collapse lacked an explicit manual-pin dominance law.

These do **not** close a whole historical OPEN item.

## Historical OPEN accounting

Pass70 ended with **22 active historical OPEN**.

- Historical OPEN fully closed in Pass71: **0 / 22**.
- Genuinely new OPEN: **0**.
- Total active OPEN after Pass71: **22**.

Advanced: structural/durable physical indexing, recovery rebuild economics, and multi-family physical lifecycle. Still OPEN include durable typed-layout recipes/I64 recovery, raw-payload acceleration if ever justified by stable layout generation identity, arbitrary/plugin semantic executable deployment, structural ordering, and the rest of the historical durability/planning/formal frontier.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Full reconstruction V2 gate PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release`;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release`.

Cold release/overflow compilation exceeded external command windows and was not counted; warmed reruns completed successfully. `PASS71_RECONSTRUCTION_FREEZE_V2.sha256` and post-gate source inventory are byte-identical.

Reconstruction note: the original transient Pass71 workspace was unavailable when packaging resumed. Pass71 was reproduced from verified Pass70 plus the recorded four-file Pass71 design/diff, then independently reverified. The published checkpoint is authoritative for these reconstructed bytes; no byte-identity claim is made with the lost transient workspace.

Frozen snapshot:

- **439 declared tests**;
- **172 `kernel-plan` declared tests**;
- **21 crates**;
- **62,186 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 unsafe hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

Production source diff relative to Pass70:

- `crates/kernel-durability/src/lib.rs`;
- `crates/kernel-durability/src/metadata.rs`;
- `crates/kernel-durability/src/store.rs`;
- `crates/kernel-plan/src/lib.rs`.

## Next frontier

The most direct continuation is a versioned durable physical-layout recipe/lowering boundary, which is required before exact-I64 artifacts can be reconstructed after reopen. Program8 is reviewed separately on top of this Pass71 checkpoint rather than being mixed into the Pass71 recovery change.
