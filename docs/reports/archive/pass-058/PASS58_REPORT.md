# PASS58 REPORT — dense revision-local identity + Γ-QCN insertion/resurrection Dq

Status: **VERIFIED**

Source window: **2026-09-20 21:40:52 → 21:47:40 +03:00 = 6:48**.
Production source was frozen at 21:47:40. All later changes are reports/spec/evidence/package only.

## R&D bundle review

Reviewed bundle:
`CFMD_RND_PROGRAM2_CLOSURE_PROGRAM3_START_PASS56_2026-09-20(1).zip`
SHA-256: `7df66c8e90ae091d46c2b798246b27e4dfba7974eb6dd9b842099df7db3a3e44`.

Disposition:

- **Program 2:** no new production merge above Pass57. The bundle independently confirms the COW/path-copy/root-identity work already promoted in Pass57. The only stated residual is O(number of artifacts) outer `BTreeMap` metadata clone. Per the R&D recommendation, Pass58 does not invent a persistent-map implementation without an artifact-cardinality falsifier.
- **Program 3:** **MERGED safe first slice** — deterministic revision-local dense IDs, compact dense sets and lifecycle adjacency/reachability projection. Logical/external `EntityId` remains authority.
- **Program 3 future slices:** dense nominal/capability/subtype extents, LocalId-backed reference columns and incremental lifecycle/SCC maintenance remain OPEN.

## Closed exactly in Pass58

### 1. Reusable revision-local dense identity substrate

`kernel-identity` now provides:

- opaque `LocalEntityId(u32)`;
- deterministic `DenseEntityIds` external↔local bijection compiled from the current finite entity set;
- compact `DenseEntitySet` bitset over local ordinals.

These objects are reconstructible physical state. They do not redefine identity and are valid only under the dense mapping that produced them.

### 2. Lifecycle relation lowering to dense adjacency

`DenseLifecycleProjection` compiles one `LifecycleGraph` to:

- roots as `Vec<LocalEntityId>`;
- `KeepsAlive` as dense adjacency vectors;
- reachability over local ordinals rather than repeated `EntityId` B-tree traffic.

Hostile coverage was strengthened beyond the R&D bundle: dense reachability is compared with `LifecycleGraph::live_entities()` for **all 512 root/edge configurations** of a three-node directed graph, in addition to a reachable cycle + unreachable dead cycle fixture.

Rebased Pass58 release diagnostic on the 50k-entity chain:

- tree walk: `88,050,596 ns`;
- dense projection walk: `2,807,426 ns`;
- ratio: about **31.363x**.

This is workload-specific direction evidence, not a universal speed claim.

### 3. Γ-QCN pure insertion/resurrection uses local greatest-fixed-point derivative

Pass52 already gave pure deletions a stable-handle local derivative. Pass58 removes the pure-insertion `Replace(fresh fixed point)` fallback.

For pure insertion:

1. old stable handles are transported into the new handle sequence;
2. inserted endpoint keys are read from the already delta-maintained Γ quotient factors;
3. only quotient leaves touched by inserted rows rebuild their key/bucket representation;
4. the connected quotient-constraint component containing those leaves is reset optimistically to full support;
5. the existing monotone support-pruning queue computes that component's **greatest fixed point**;
6. disconnected components retain their previous fixed point unchanged.

This is intentionally not symmetric bit activation. Optimistic component reset + monotone pruning handles mutually supporting resurrection components soundly.

Hostile `gamma_quotient_local_insertion_resurrects_greatest_fixed_point` first deletes the only supporting value until the maintained QCN output is empty, then reinserts it. The locally maintained state is compared directly with a freshly rebuilt `MaterializedSemanticQuotientSupportState` and is exactly equal; logical output resurrects to the 100-row reference result.

Pure deletion + pure insertion therefore both increment the same local-Dq counter. **Mixed delete+insert remains the exact Replace fallback** in Pass58.

## Authority and fallback boundaries

- `EntityId` remains logical/external identity authority; `LocalEntityId` is revision-local physical evidence.
- Dense lifecycle projection is not durable semantic state.
- Γ-QCN factors/support remain reconstructible physical derivatives of `(query, Γ, revision data)`.
- Pure insertion local Dq requires the maintained quotient factors already owned by a materialized support state. If the exact preconditions do not hold, production falls back to exact fixed-point rebuild.
- Mixed delete+insert is intentionally not claimed local yet.

## Historical OPEN accounting

Pass57 historical OPEN count: **22**.

- Historical OPEN fully closed this pass: **0 / 22**.
- Historical OPEN remaining: **22**.
- Genuinely new OPEN created: **0**.

Advanced but still OPEN:

- historical #9 persistent runtime root: heavy payload clone tax is closed, but outer artifact-map metadata remains O(number of artifacts);
- dense identity/graph runtime: subtype/capability extents, LocalId-backed reference columns and lifecycle incremental SCC/support remain incomplete;
- Γ-QCN derivative: pure delete and pure insert are local, but mixed delete+insert and revision-level multi-relation coalescing remain fallback/rebuild cases;
- structural/custom Γ-QCN factors and broader multi-family advisor/lifecycle remain OPEN.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

PASS:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets --release`
- `cargo build --workspace --release`
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release`

The first release and first overflow-check invocations hit external compilation timeouts and were not counted; warmed retries completed PASS.

Static snapshot before packaging:

- **375 declared tests**;
- **134 `kernel-plan` tests**;
- **73 `kernel-query` tests**;
- **8 `kernel-lifecycle` tests** (7 normal + 1 ignored diagnostic);
- **5 `kernel-identity` tests**;
- **21 crates**;
- **51,379 Rust LOC**;
- **0 `unsafe` hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 external registry/git Cargo sources**.

## Next frontier

Pass58 makes the Γ-QCN derivative asymmetric only for **mixed** change batches now:

- pure deletion → local stable-handle loss propagation;
- pure insertion/resurrection → local connected-component greatest-fixed-point refresh;
- mixed delete+insert → exact Replace fallback.

A clean Pass59 target is either revision-batch coalescing/mixed Dq, or the next Program3 production consumer: dense nominal/subtype/capability extents under one mapping. These should not be conflated unless the source window leaves a clean falsification boundary.
