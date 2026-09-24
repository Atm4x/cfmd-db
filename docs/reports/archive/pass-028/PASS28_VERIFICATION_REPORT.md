# PASS28 VERIFICATION REPORT

Final classification: **VERIFIED AFTER REOPEN/CORRECTION on Rust 1.98.1**.

The original 20-minute Pass28 artifact was source-audited without a toolchain. Before Pass29, that exact line of work was reopened under the user-supplied Rust 1.98.1 toolchain, compiler/test defects were corrected, and the resulting Pass28 baseline passed the full verification gate. The original frozen-source section below is preserved as provenance for the first artifact; it is not the final Pass28 status.

## Frozen source boundary

Production source editing stopped after the requested wall-clock window:

```text
start:  2026-09-19 19:54:31 UTC
freeze: 2026-09-19 20:14:32 UTC
elapsed: ~20m01s
```

After freeze only evidence, reports and packaging files were created.

Frozen source hashes:

```text
dace2013c9873de2e7a23be69443b087e41e098ee25f1f43121bbdc0b646f4c6  crates/kernel-plan/src/lib.rs
5b93c8fa7edd974a0f42baa43000107ac9ab05a7537b9ea6c52f53a8a47c8eae  crates/kernel-query/src/lib.rs
```

These hashes matched the pre-freeze audit values after reporting began, confirming no post-boundary production-source edit.

## Pass27→Pass28 changed production files

```text
crates/kernel-plan/src/lib.rs
crates/kernel-query/src/lib.rs
```

No other pre-existing workspace file differed from the pristine Pass27 baseline before post-freeze reports/evidence were added.

## Static checks performed

PASS at source-text level:

- 235 `#[test]` declarations across Rust sources;
- 29,140 Rust source lines;
- 19 workspace crates;
- Cargo.lock contains zero external `source =` entries;
- zero `unsafe {` occurrences;
- zero TODO/FIXME;
- zero `panic!`, `todo!`, `unimplemented!` occurrences;
- no tabs/trailing whitespace in changed files;
- `{}`, `()`, `[]` raw counts balance in both changed files;
- lightweight scanner ignoring line comments, nested block comments, strings and chars reports balanced delimiters in both changed files;
- no unchecked `commit_prepared_after_joint_validation` API remains;
- prepared storage implementation is private to `kernel-plan`;
- bound legacy semantic mutation guards are present.

Patches are retained in:

```text
evidence/pass28/kernel-plan_pass27_to_pass28.patch
evidence/pass28/kernel-query_pass27_to_pass28.patch
```

## Runtime/compiler gate in the original source-audit environment

The container contains none of:

```text
cargo
rustc
rustfmt
clippy-driver
```

Attempted commands therefore fail before inspecting the source:

```text
cargo fmt --all -- --check
cargo test --workspace
```

Raw output: `evidence/pass28/PASS28_TOOLCHAIN_EVIDENCE.txt`.

No external network/toolchain bootstrap was possible in the session environment.

## Claims explicitly NOT made

Pass28 does **not** claim:

- successful parsing by rustc;
- successful compilation;
- test execution;
- fmt compliance;
- Clippy compliance;
- rustdoc compliance;
- benchmark result;
- process/filesystem crash atomicity;
- multi-relation revision atomicity;
- capability-sealed storage certificates.

At the time of the original source-audit, Pass27 was still the last fully VERIFIED checkpoint. This was superseded by the post-reopen Pass28 verification below.

## Required first action in a Rust-capable environment

Run, in order:

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --release
cargo build --workspace --release
```

Then run the project's strict rustdoc and overflow-check gates used by Pass27. Any compiler/fmt/clippy failure must be treated as a Pass28 defect, not papered over in the report.

## Post-reopen Rust 1.98.1 correction and final verification

Before Pass29 was forked, Pass28 was reopened with:

```text
rustc 1.98.1 (48a229cea 2026-09-01)
cargo 1.98.1 (797e8a9bc 2026-08-05)
rustfmt 1.9.0-stable
clippy 0.1.98
```

Real verification exposed and closed four issues:

1. `rustfmt --check` formatting differences;
2. Clippy `too_many_arguments` on the transaction API, fixed structurally via a request object rather than lint suppression;
3. one hostile test incorrectly compared physical row order for bag semantics, corrected to bag-equivalence;
4. stale maintained-plan failure was surfaced as a nested query error instead of the central stale-transition error, normalized at the transaction boundary.

After those corrections Pass28 passed:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release PASS
```

The corrected Pass28, not the original source-only artifact, is the baseline from which Pass29 was forked.

