# IMPLEMENTATION REPORT — PASS91

## Changed production files
1. `crates/kernel-semantics/src/lib.rs`
2. `crates/kernel-durability/src/lib.rs`
3. `crates/kernel-durability/src/store.rs`

No Cargo manifest changed. No Group/TopK production implementation changed.

## kernel-semantics
Added the production deployment boundary:
- `SemanticContractIdentity`;
- `ImplementationArtifactDigest`, `RuntimeProfileDigest` and builtin runtime profile;
- `SemanticRefinementCertificate`;
- `SemanticExecutableArtifact` and `SemanticImplementationPackage`;
- `ArtifactAuthenticationSet`;
- `SemanticExecutionPolicy`;
- typed `SemanticDeploymentError`;
- `ExecutionAuthorization`;
- `SemanticExecutionCapability`;
- `SemanticDeploymentRegistry` authorization/capability/installation paths.

Defined contracts require checked refinement. Opaque contracts pin artifact/runtime identity. Authentication, semantic refinement, runtime permission and revocation are orthogonal checks.

## kernel-durability
Store semantic-registry reconstruction now builds builtin packages, obtains authentication evidence and execution authorization, then installs only the authorized builtin. Direct durability-store installation of builtin specs was removed from reopen/replay paths.

Added durable-effect classification:
- `DurableEffectKind`;
- `DurableEffectCoordinationClass::OpaqueNonConfluent`;
- kind/coordination accessors on durable revision-effect records and intents.

This classification is intentionally conservative and does not add an automatic merge rule.

## Hostile verification
Covered:
- authenticated but semantically wrong implementation is rejected;
- semantically refined but unauthenticated implementation is rejected;
- revoked implementation can be replaced only by another certified implementation of the same defined contract;
- opaque execution requires exact artifact and runtime identity;
- capability checking ignores unrelated unavailable packages but reports unavailable required contracts;
- multi-parent resolution retains its exact durable effect kind and remains `OpaqueNonConfluent`.

Final source-state gates:
- `cargo fmt --all -- --check` — PASS;
- `cargo check --workspace --all-targets` — PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS;
- `cargo test --workspace --all-targets` — PASS on the same source state;
- 671 declared test attributes, 8 ignored, 0 failures.
