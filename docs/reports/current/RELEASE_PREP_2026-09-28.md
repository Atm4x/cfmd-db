# CFMD repository release-prep — 2026-09-28

## Scope

Prepare the Pass280 repository as the next GitHub baseline, preserving newer repository-infrastructure fixes from public `Atm4x/cfmd-db` while **not** importing its older kernel code over Pass280.

## GitHub baseline inspected

At review time the public repository's latest large code commit on `main` was:

```text
1dd571b50f7f49413128420a9a163d4ebd263c47  Pass205 iteration
```

The local release baseline is Pass280 and is therefore authoritative for kernel source.

Repository/infrastructure commits inspected and reconciled include:

```text
cc20a4c16669ac6fe71c0e0d93feeb3bdbb5237b  fix: mark CI scripts executable
57ca58cd179cc84790755903461e86b06ab87672  Update script execution command to use bash
1e2dd635ea4ddcb10489c3c3376ffa45a92c01bd  Update rust.yml
8bd4f4f7a0f23a8f26fbeec4b659af7d676ea1d0  fix: preserve complete Cargo vendor snapshot
50b83dc4b06e3864e03bccb250670442757dba32  Gitignore update
```

Reconciled behavior:

- Rust and Lean workflow scripts run through explicit `bash`, so CI does not depend on executable-bit preservation;
- shell scripts in this snapshot are also marked executable for normal Unix checkouts;
- `.gitignore` retains `vendor/**/Cargo.toml.orig` and `vendor/**/.cargo-checksum.json` despite the broad `*.orig` scratch rule;
- Pass280 already contains the complete vendor metadata/files required by those checksums.

No Pass205 kernel source is merged into Pass280.

## GitHub Actions `physical_store/core.rs` failure

Observed failure:

```text
error: couldn't read `crates/kernel-plan/src/storage_impl/physical_store/core.rs`:
No such file or directory
 --> crates/kernel-plan/src/storage_impl/physical_store.rs:1:1
  |
1 | include!("physical_store/core.rs");
```

The GitHub API confirms the exact inconsistent remote state:

- `crates/kernel-plan/src/storage_impl/physical_store.rs` exists and contains six literal `include!()` declarations;
- `crates/kernel-plan/src/storage_impl/physical_store/core.rs` is **404 / absent** on `main`;
- the other five child files are present.

Pass280/release-ready contains all six children, including `core.rs`. Therefore this failure is an **incomplete GitHub snapshot/upload**, not a current Rust implementation bug.

### Prevention added

`scripts/verify-repository.sh` now checks, before Cargo:

1. required repository entry points;
2. every literal Rust `include!("...")` target below `crates/`;
3. every file/hash listed by each vendored `.cargo-checksum.json`;
4. the repository-wide SHA-256 manifest.

`ci-rust.sh` invokes this verifier first, and the GitHub Rust workflow has an explicit `Repository completeness` step before installing the Rust toolchain.

A future partial upload of `physical_store.rs` without `physical_store/core.rs` therefore fails with a direct repository-completeness message instead of a later compiler error.

## Documentation refresh

Current-facing documentation now matches the Pass280 baseline:

- root `README.md`;
- root `SPEC.md`;
- `docs/spec/CFMD_CORE_SPEC.md` front matter/current product handoff;
- `docs/status/PROJECT_STATUS.md`;
- new `docs/status/KERNEL_HOSTILE_LEDGER.md`;
- `docs/status/REPOSITORY_LAYOUT.md`;
- `docs/architecture/ARCHITECTURE.md`;
- regenerated `docs/architecture/CRATE_MAP.md` from the current 27 crate manifests;
- new `docs/api/PRODUCT_ROADMAP.md`;
- rewritten `docs/api/RUST_API_ROADMAP.md`;
- retained full `docs/api/CFMD_PYTHON_FACADE_THEORY.md` design source;
- `docs/reports/current/PASS280_REPORT.md`;
- stale Pass121 current reports moved into `docs/reports/archive/pass-121/`.

## Product direction

The product sequence is now **Python-first** at the application surface, with a compact Rust facade/runtime service below it.

The roadmap preserves the facade theory's central laws:

- explicit `db.users`-style database context;
- familiar lazy queries with deep symbolic relationship navigation;
- no hidden I/O on materialized Python objects;
- explicit semantics for many-valued paths;
- exact async query-result watch rather than table callbacks/polling;
- current/historical/Candidate worlds;
- Plan → Candidate → commit;
- history inverse through the same pipeline;
- one authoritative runtime shared by app and local Studio/tooling.

## Verification on release-ready tree

Repository integrity:

```text
bash ./scripts/verify-repository.sh                          PASS
```

Targeted reproduction gate:

```text
cargo check -p kernel-plan --lib --locked --offline         PASS
```

Rust workspace:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets --locked --offline    PASS
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
                                                             PASS
cargo test --workspace --all-targets --locked --offline     PASS
```

Source refinement:

```text
formal/lean/check_refinement.py                             PASS (10 fault points)
formal/lean/check_surface_refinement.py                     PASS
```

The sandbox does not contain a Lean executable, so the two `.lean` theorem files were not recompiled locally in this release-prep step. They and the Rust kernel source are unchanged from the Pass280 baseline whose formal gate was PASS; GitHub `lean.yml` installs the pinned Lean 4.34.0 toolchain and reruns the complete formal gate.

## Upload instruction

Replace the GitHub repository contents from the complete release-ready snapshot rather than selectively uploading only changed parent files. In particular, retain all nested module directories, `vendor/`, `.github/`, scripts and the regenerated manifest.
