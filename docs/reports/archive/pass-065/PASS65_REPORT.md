# PASS65 REPORT — hostile-reviewed Incremental Revision Compiler

Status: **VERIFIED after final packaging gate**.

Production merge basis: frozen Pass64 plus R&D Program5, hostile-corrected before acceptance.
Source freeze: **2026-09-20 23:24:10 UTC**. The complete `crates/` SHA-256 snapshot captured at freeze is byte-identical to the post-gate snapshot.

## Problem

A pinned-Γ relation-data transition still paid generic revision construction costs after its logical target had already been derived from the current authoritative Revision:

- normalization compiled dense IDs and reverse LiveRef sensitivity, then `Revision::build` compiled them again;
- dense type extents were compiled for validation and discarded;
- all relations/fields were typed/semantically validated even when only a known relation subset changed;
- relation-data WAL replay called full `Revision::build` after every durable delta;
- reverse LiveRef sensitivity was one monolithic structure rather than sharing untouched relation partitions.

R&D Program5 proposed a sealed relation-only candidate. Hostile review found three correctness gaps in its submitted API, so the patch was rebased rather than merged verbatim.

## Hostile findings and corrections

1. **Cross-source rebinding:** the R&D candidate could be created from Revision A and built against Revision B. Pass65 makes `RelationUpdateCandidate<'a>` borrow its exact source Revision and removes caller-supplied source selection from the fast build path.
2. **Pinned Γ validation:** the submitted certified builder omitted `registry.validate_context`. Pass65 revalidates the full pinned semantic context before accepting touched-only compilation.
3. **Normalization equivalence:** full build removes touched relation rows containing dangling nested `LiveEntityRef` before typed validation, while the R&D path rejected them. Pass65 performs the same row-local normalization for touched relations and has a differential test requiring the incremental state to equal full `Revision::build`.

## Implementation

- `normalize_certified()` returns normalized state + final shared dense IDs + reverse LiveRef sensitivity.
- `Revision` retains `DenseTypeExtents` instead of recomputing/discarding them.
- `validate_relations_with_extents()` validates only the certified touched relation set.
- `LiveRefSensitivityIndex` has shared static field roots and `Arc` per-relation partitions; touched relations recompile, untouched partitions remain shared.
- `RelationUpdateCandidate<'a>` exposes only relation-row replacement and source-bound build.
- defensive arbitrary-state `build_relation_update` still checks lifecycle/carriers/fields and all untouched relations exactly.
- relation-data recovery applies WAL deltas to the current Revision's candidate and no longer invokes generic full `Revision::build` for each relation-data record.

## Performance falsification

The original R&D report measured 14.006× compiler-only. Pass65 reran an independent diagnostic after the hostile corrections rather than copying that number.

Fixture: 40 Bag<I64> relations × 5,000 rows, one touched relation, seven release runs. State clone and returned-Revision destruction are outside compiler timing.

- full compiler median: **3,406,633 ns**;
- hardened certified compiler median: **293,583 ns**;
- compiler-only ratio: **11.604×**;
- candidate `DatabaseState::clone()` median: **4,330,525 ns**.

The compiler boundary is therefore materially improved, but full logical-state cloning remains the dominant residual. Pass65 does not claim end-to-end O(|Δ|).

## CLOSED exactly in Pass65 — 3

1. **Duplicate revision derivative compilation after normalization.** Dense IDs and reverse LiveRef sensitivity produced by normalization are reused; dense type extents are retained by Revision.
2. **Generic full revision compiler/revalidation on sealed relation-only transitions and WAL recovery.** Certified source-bound candidates now validate/refresh only touched relation state while reusing exact revision-local derivatives.
3. **Monolithic reverse LiveRef sensitivity refresh for relation-only changes.** Static field roots and untouched relation partitions are structurally shared; only touched relation partitions are rebuilt.

These close the Program5 compiler boundary. They do not close persistent logical-state ownership or the broader recovery/rebuild historical item.

## Historical OPEN accounting

Pass64 historical active OPEN: **24**.

- Historical OPEN fully closed this pass: **0 / 24**.
- Historical OPEN remaining: **24**.
- Genuinely new OPEN formalized this pass: **1** — persistent/path-copy logical `DatabaseState` ownership for candidate construction.
- Total active OPEN after Pass65: **25**.

## Advanced but still OPEN

- historical recovery/rebuild economics advances: relation-data WAL replay no longer recompiles the full Revision, but physical reconstruction and broader recovery economics remain OPEN;
- persistent runtime work advances only on derivative sharing. `RelationUpdateCandidate` still clones the complete logical `DatabaseState` before the incremental compiler runs;
- compiled schema/Γ validation remains a future optimization; Pass65 preserves exact interpreter semantics and only narrows the validated data region constructively.

## Active OPEN after Pass65 — 25

1. Structural/custom-equivalence physical indexing: durable recursive-key encoding/versioning, persisted structural indexes, arbitrary/plugin canonical laws and structural ordering.
2. General nested/multiway/bushy Join planning beyond bounded 3–8-leaf Γ-QCN.
3. Complete multi-family physical lifecycle beyond semantic indexes and current Γ-QCN factor policy.
4. Autonomous workload telemetry/read-write rates/decay/hysteresis/lifecycle scheduling.
5. Exact allocator/RSS accounting, external memory pressure and rebuild scheduling.
6. Canonical-key/cache encoding-version migration and compatibility law.
7. Remaining physical layouts plus explicit `OrderedView`/pagination.
8. Secondary-index/alternate-layout rebuild economics and fast physical reconstruction after recovery.
9. Persistent outer physical artifact-map metadata instead of O(number of artifacts) map clones.
10. Durable revision DAG / branch+merge ancestry and merge replay.
11. General historical durable-format migration framework.
12. Arbitrary/plugin semantic executable artifact packaging/signing/authentication/deployment.
13. Transaction intent/outcome retention + GC; Program7 remains pending corrected exact-content design.
14. Streaming/chunked checkpoints and metadata.
15. Real machine power-loss assurance plus Windows/network-FS/FUSE durability semantics.
16. General lock-poison/restart policy.
17. Group commit / async durability.
18. Replication / consensus and broader distribution architecture.
19. Durable-store authentication/MAC; CRC32C is corruption detection only.
20. Formal power-loss proof for rename/fsync/GC protocol.
21. Transaction repair runtime.
22. Formal mechanization of remaining semantic/transport/retention/power-loss obligations.
23. Maintained I64 Group constant-factor gap.
24. Maintained I64 TopK constant-factor gap.
25. **Persistent/path-copy logical state:** source-bound relation candidates still obtain their immutable target snapshot through a full `DatabaseState::clone()`; compiler work is incremental but logical snapshot construction is not.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Final frozen bytes PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release`;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release`.

Cold long-running release/overflow invocations that hit the external execution timeout were not counted; completed warmed retries are the recorded gate results.

Static snapshot before packaging:

- **414 declared tests**;
- **159 `kernel-plan` tests**;
- **21 crates**;
- **57,570 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 `unsafe` hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

## Next frontier

Continue the main branch from Pass64's shared semantic-projection/multi-family direction unless corrected Program7 arrives first. Program5 makes a second path especially attractive: persistent/path-copy logical `DatabaseState` ownership can now be measured independently because the compiler itself is no longer the dominant relation-only rebuild tax.
