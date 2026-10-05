# PASS507 GitHub repository checkpoint

This is a repository-maintenance checkpoint, not PASS508 and not a new semantic architecture pass.

## Mainline

The source baseline is the completed PASS507 repository. Product semantics remain PASS507: scoped `Context<M>` ownership, schema-neutral authoritative `Database` creation, DB-owned semantic-coordinate authorization, one prepared schema-aware publication law for relation and pure field-only classes, and explicit hosted `SemanticRevision` formation identity. PASS508 remains the next R&D target.

## GitHub synchronization

The public GitHub repository was still based on PASS472 plus subsequent CI-only fixes. This checkpoint deliberately does **not** merge PASS472 source over the newer mainline. It ports only repository/CI fixes that remain valid:

- Rust CI installs `ripgrep` before `scripts/ci-rust.sh`.
- Ubuntu CI enables unprivileged user namespaces before the namespace sandbox gate.
- repository manifest tooling can verify exact Git-index bytes in a real checkout while source archives still verify SHA-256 directly.
- Lean refinement checking recursively follows split Rust `mod`/`include!` source closure and the current streaming checkpoint publication cuts.
- workspace Rust sources were normalized with pinned rustfmt 1.98.1.

## Verification boundary

The updated Lean refinement checker passes against the current durability source. `cargo fmt --all -- --check` passed after normalization. Strict workspace Clippy cleanup was started against Rust 1.98.1 and exposed additional current-mainline pedantic debt beyond the old PASS472 patch set. The external 24-minute hard wall-clock boundary elapsed during the connection interruption, so a fresh full Clippy/test sweep was not started after that boundary. This checkpoint therefore records the exact hard-stop state rather than claiming a CI-green result that was not completed.

## Next

Before pushing as a release-quality GitHub mainline, resume from this checkpoint and finish `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, then run the complete Rust/formal/repository gates and regenerate the manifest from the final staged Git bytes. No PASS508 semantic work should be mixed into that maintenance closure.
