# Pass121 Verification

## Repository transformation

- production crate files inherited from Pass120: **85 / 85 byte-identical**;
- Lean theorem artifacts inherited unchanged:
  - `Publication.lean` SHA-256 `50ac70af7dc7557de18c8790c8f12ec117d0f48cba8aa7a5d99ffbc754f7857f`;
  - `SurfaceKernel.lean` SHA-256 `e09b95d9f41d77e91d5eb786249894299ee6fa51abea878a5d7b674469b5a48c`;
- `check_surface_refinement.py` changed only to follow the projectized normative spec path `docs/spec/CFMD_CORE_SPEC.md`;
- root Cargo metadata gained the pinned `rust-version = "1.98.1"` workspace field.

## Rust gates

Executed offline with Rust/Cargo 1.98.1:

- `cargo fmt --all -- --check` — **PASS**;
- `cargo check --workspace --all-targets --locked --offline` — **PASS**;
- strict `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` — **PASS**;
- `cargo test --workspace --all-targets --locked --offline` — **766 passed / 0 failed / 8 ignored = 774 declared**.

## Formal/refinement gates

- #18 Rust↔Lean source refinement binder — **PASS**;
- #20 surface/kernel source refinement binder — **PASS**;
- Lean theorem source bytes are unchanged from their mechanically checked closing snapshots.

The local projectization sandbox does not currently contain a Lean executable, so Lean kernel elaboration was not redundantly rerun in Pass121. CI pins Lean `v4.34.0` and runs `formal/lean/check_all.sh` automatically for proof-relevant changes or manual dispatch.

## Project statistics

- workspace crates: **25**;
- project Rust files under `crates/`: **60**;
- Rust lines under `crates/`: **118,076**;
- nonblank Rust lines: **112,080**;
- Rust `src/` lines: **115,297**;
- Lean files: **2**, **814 lines**;
- formal source-refinement Python: **250 lines**;
- Rust test attributes in source tree: **769**;
- actual Cargo test suite: **774 declared**, of which **766 pass** and **8 intentionally ignored benchmarks**;
- full normative core spec: **2,191 lines**;
- archived report/spec/history files: **618+**;
- retained artifact/evidence files before final Pass121 logs: **820+**.

## Preservation audit

A SHA-256 preservation scan verified that **every regular file that existed at the Pass120 repository root has an exact byte-identical preserved copy somewhere in the projectized repository** (current location or historical archive): **0 lost root files**.

Additional tree checks:

- `crates/`: 85 / 85 Pass120 production files byte-identical;
- `vendor/`: 1,046 / 1,046 files byte-identical;
- former `diagnostics/`: 6 / 6 files byte-identical under `artifacts/diagnostics/`;
- former `evidence/`: 744 / 744 files byte-identical under `artifacts/evidence/`.

The only intentional active-source/document transformations are repository metadata/docs, the moved-spec path in the #20 Python refinement binder, and project tooling/CI. The two Lean theorem files remain byte-identical to their closed proof snapshots.
