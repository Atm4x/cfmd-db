# IMPLEMENTATION REPORT — Pass38

## problem

Pass37 left three connected durable-control-plane gaps: retained transaction outcomes did not encode exact historical request intent strongly enough, a semantic Revision migration and its required maintained-materialization registry could not publish as one transaction, and restart still depended on externally supplied implementations for builtin semantic-module digests.

## hypotheses

1. Exact idempotency should retain canonical semantic target content rather than rely on nominal `RevisionId`.
2. Materialization registry is part of the client-visible migration intent when it changes together with a Revision, so both must share one PREPARE/COMMIT/publication boundary.
3. Current builtin semantic implementations are deterministic from `(builtin contract, implementation_revision)` and can therefore be durably described without serializing arbitrary executable code.
4. Legacy target-only records must remain explicitly weaker; compatibility code must not silently promote them to exact authority.

## implementation

### `kernel-durability`

Added `DurableTransactionIntent::{Exact, LegacyTargetOnly}`. Exact intents retain canonical target Revision bytes, optional materialization specs and exact `BuiltinSemanticModuleSpec` descriptors.

Added `DurableRevisionChange::FullRevisionAndMaterializations` and mutation codec v4. PREPARE encoding carries exact intent plus typed change; v2/v3 decoding remains explicit.

Recovery transaction ledger now stores durable intents rather than only target Revision IDs. Generation metadata codec v2 persists the exact committed-intent ledger plus builtin semantic deployment descriptors. Metadata v1 remains readable as `LegacyTargetOnly` with no invented exact identity.

`DurableRevisionStore::open` reconstructs current/historical builtin semantic implementation descriptors before checkpoint/WAL Revision decoding. `open_with_legacy_registry` remains the explicit compatibility route for generations that predate the durable semantic deployment manifest.

The final cleanup refactored semantic-registry reconstruction, recovered-intent merge and current PREPARE decoding into focused helpers to satisfy strict Clippy without suppression.

### `kernel-semantics`

Added `BuiltinSemanticModuleSpec` for equivalence, tokenizer and ordering implementations. Each descriptor carries the builtin contract and exact implementation revision, deterministically reproduces the expected `ModuleDigest`, can be installed into `SemanticRegistry`, and can be enumerated for a pinned `SemanticContext`.

### `kernel-plan`

Added `RevisionAndMaterializationsTransitionRequest`, prepared/sealed combined transition support and durable commit/retry paths.

`DurableRuntime::{commit_revision,replace_revision,replace_revision_and_materializations}` now constructs exact requested intent and compares it with the retained committed intent before returning `AlreadyCommitted`. Same client ID with different exact target content/configuration is an explicit conflict.

Recovery supports `FullRevisionAndMaterializations` by rebuilding the authoritative physical representation and the exact desired maintained registry from durable semantic authority.

## hostile falsification

Production tests cover:

- historical exact intent surviving later heads, checkpoint rotation, compaction and restart;
- conflicting same client ID + same nominal RevisionId + different Revision contents;
- one atomic Revision + materialization-registry transition and restart;
- exact retry of the combined transition;
- historical semantic implementation manifest surviving checkpoint/compaction/reopen;
- legacy mutation decoding without silent authority promotion;
- exact builtin descriptor digest/installation roundtrip.

Static authority audit confirms that physical row handles/layouts/indexes/materialized results remain absent from durable transaction intent and semantic deployment metadata.

## verification

Rust 1.98.1 final gate:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test ... --release PASS
```

297 declared tests; 81 kernel-plan; 65 kernel-query; 32 kernel-durability; 20 crates; 38,065 Rust LOC; zero external Cargo sources; zero `unsafe`.

## result

Pass38 closes three actual problems:

1. new-format client transaction identity is exact and remains exact through later heads/checkpoint/compaction/restart;
2. semantic Revision plus materialization registry can migrate atomically as one durable transaction;
3. current builtin semantic implementation families have a durable self-describing deployment manifest, so normal reopen no longer relies on caller-supplied registry authority.

It does **not** claim arbitrary plugin executable persistence, universal historical-format migration, revision-DAG durability, distributed durability, authenticated storage, or machine-power-loss proof.
