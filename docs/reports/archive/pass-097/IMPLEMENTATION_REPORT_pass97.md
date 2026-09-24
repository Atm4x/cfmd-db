# IMPLEMENTATION REPORT — PASS97

## Changed production surface

- `crates/kernel-query/src/lib.rs`

## Annotation / Group kernel

1. Added `DenseWindowGroupCount` using `Vec<ExactCount>` with bounded physical admission.
2. Added `GenericGroupPatch`, `I64CountGroupPatch`, and `GroupDeltaPatch`.
3. Replaced the old mutating fast-I64 Group path with read-only signed-delta planning plus explicit commit.
4. Replaced generic Group's combined planning/mutation path with `DeltaView` planning, universal `AdaptiveDelta` output, and explicit commit.
5. Dense window misses switch to sparse fallback before commit and never surface as semantic errors.
6. Added exact dense singleton move optimization using `mem::take`.
7. Preserved all pinned-Γ validation and existing public `RelationDelta` compatibility.

## Verification

Targeted:
- dense plan immutability + recompute parity — PASS;
- dense outlier fallback + recompute parity — PASS;
- existing generic Group Count/ExactF64Sum/underflow suites — PASS;
- strict `kernel-query` Clippy — PASS.

Final workspace:
- `cargo fmt --all -- --check` — PASS;
- `cargo check --workspace --all-targets` — PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS;
- `cargo test --workspace --all-targets` — PASS;
- 691 test attributes, 8 ignored, 0 failures.

No lint suppression was added. Build artifacts remain outside the workspace via `CARGO_TARGET_DIR=/mnt/data/cfmd_target_pass97`.
