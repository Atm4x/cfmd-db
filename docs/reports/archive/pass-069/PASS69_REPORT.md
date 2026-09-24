# CFMD Pass69 Report — structural canonical-key/version discipline

Date: 2026-09-21
Status: **VERIFIED**
Authoritative base: verified Pass68
Production source window: **2026-09-21 01:11:05–01:31:14 UTC (20m09s)**
Production source freeze: **2026-09-21 01:31:14 UTC**

## Goal

Close the long-lived canonical-key/cache compatibility gap without making physical representation semantic authority: define a stable recursive Γ-canonical key encoding, explicit compatibility/rebuild reasons, and one binding contract consumed by structural semantic indexes, Γ-QCN factors/support and semantic statistics.

## Result

Pass69 establishes a versioned canonical-key format and migration boundary.

- `CanonicalEqKey` has a stable recursive byte codec v1 with explicit format revision, fixed tags, big-endian numeric representation, deterministic constructor grammar and canonical collection ordering.
- The decoder fails closed on unknown revisions, malformed/non-canonical data, hostile declared lengths, trailing bytes and recursive nesting beyond the bounded parser depth.
- A golden-byte fixture prevents accidental wire-format changes without a revision bump.
- Structural equivalence bindings now record exact primitive/module dependency closure rather than relying only on an in-memory semantic context value.
- `SemanticIndexBinding` reports explicit compatibility outcomes: semantic revision mismatch, dependency mismatch, key-encoding mismatch, or compatible.
- Historical persisted binding metadata can therefore request rebuild rather than silently reinterpret an old key representation.
- Generic semantic indexes now support schema-declared structural equivalences through the same exact Γ canonical-key law used by the maintained structural/query paths.
- Semantic indexes, Γ-QCN endpoint factors, Γ-QCN support and semantic statistics share the same key-binding compatibility contract.

The codec is a persistence-ready key-format law, not a claim that physical index payloads are already durable. Durable storage/recovery of structural index payloads remains outside Pass69.

## Hostile / falsification

- Recursive Product/Option/Sum/Seq/Set/Bag/Map key round trips match the Γ oracle.
- Non-canonical Set/Bag/Map ordering is rejected rather than normalized during decode.
- ASCII-CI canonical leaves reject non-canonical persisted representations.
- Unknown key revisions trigger rebuild/incompatibility rather than reinterpretation.
- Hostile lengths and nesting >256 fail closed.
- Structural maintained Filter consumes the persisted semantic index and remains exact across relation delta maintenance.
- Long-lived canonical-key artifact families no longer use separate ad-hoc compatibility rules.

## CLOSED exactly in Pass69 — 1 historical item

Historical OPEN #6: **canonical-key/cache encoding-version migration and compatibility law**.

A concrete format revision, stable codec, dependency binding and fail-closed rebuild decision now exist and are shared by the long-lived canonical-key artifact families.

## Historical OPEN accounting

Pass68 ended with **23 active OPEN**.

- Historical OPEN fully closed in Pass69: **1 / 23**.
- Genuinely new OPEN: **0**.
- Total active OPEN after Pass69: **22**.

Historical structural/custom-equivalence persistence remains OPEN because durable physical index payloads, arbitrary/plugin executable deployment and structural ordering are not closed by the key codec alone.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Frozen-source final gate PASS:

- `cargo fmt --all -- --check`;
- `cargo check --workspace --all-targets`;
- `cargo test --workspace --all-targets`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace --all-targets --release`;
- `cargo build --workspace --release`;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`;
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release`.

Cold release/overflow compilation that exceeded the external command window was not counted; warmed reruns completed successfully. Freeze/post-gate SHA-256 inventories of `crates/` are byte-identical.

Frozen snapshot:

- **431 declared tests**;
- **168 `kernel-plan` tests**;
- **21 crates**;
- **60,752 Rust LOC**;
- **0 external registry/git Cargo sources**;
- **0 unsafe hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**.

Production source diff relative to Pass68:

- `crates/kernel-semantics/src/lib.rs`;
- `crates/kernel-semantic-index/src/lib.rs`;
- `crates/kernel-plan/src/lib.rs`.

## Next frontier

The key-format/migration law is no longer the blocker. The remaining structural frontier is durable physical structural-index payload persistence/rebuild plus arbitrary/plugin semantic deployment and structural ordering. The other high-value mainline candidates remain general multiway/bushy planning and the incomplete cross-family advisor/telemetry boundary.
