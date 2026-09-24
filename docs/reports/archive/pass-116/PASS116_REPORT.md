# Pass116 — Historic #10 production integration and #17 frontier

## Status

- Historic #10 semantic implementation package/auth/deployment: **PROD CLOSED**.
- Historic ledger: **19/22 PROD CLOSED**.
- #17 authenticated durable store + external freshness/anti-rollback anchor: **OPEN, next frontier audited**.
- #13 and #20 remain OPEN.

## What was integrated

The R&D package was not blindly copied. Its `kernel-auth` layer was already present in Pass115 through the #16B integration, so Pass116 rebased only the missing deployment layer onto current mainline.

`kernel-deployment` now supplies:
- canonical bounded `SemanticPackageEnvelope`;
- exact caller-supplied pinned-Γ semantic descriptor match;
- SHA-256 + strict Ed25519 artifact verification through current `kernel-auth`;
- monotone signed deployment/revocation policy;
- policy-authorized refinement checker boundary;
- canonical bounded semantic ABI;
- untrusted `PackageRepository` and `ArtifactCas` boundaries;
- concrete filesystem package repository and filesystem CAS adapters;
- `ExternalDeploymentAuthority` end-to-end `verify -> authorize -> invoke` adapter;
- `LinuxNamespaceProcessRuntime` for `SandboxedProcess` profiles.

The Linux backend executes the authenticated artifact out of process under user/mount/PID/network/IPC/UTS namespaces, cleared environment, bounded request/response, and a wall-time bound derived from the runtime fuel budget. No native plugin is loaded into the DB process.

## Hostile evidence

- R&D deployment matrix retained: descriptor substitution, package swap, proof/checker substitution, runtime-profile substitution, policy rollback/noncanonical sets, malformed/trailing ABI and fuel escape all fail closed.
- Real namespace backend execution test: PASS.
- Host loopback reachability from the sandbox namespace: denied, PASS.
- Filesystem repository/CAS are tested as byte sources only; authority remains cryptographic/policy verification.
- `kernel-deployment`: 14/14 PASS.

## Final gates

- `cargo fmt --all -- --check`: PASS.
- `cargo check --workspace --all-targets --offline`: PASS.
- strict workspace Clippy before final test-only FS evidence: PASS; final `kernel-deployment --all-targets -D warnings`: PASS.
- full workspace after all changes: **756 declared / 748 passed / 0 failed / 8 ignored**.

Production fingerprint: `c7f75d6b9121851ff538609f5d2f9163e453dbc4f21d8f41cd48138705e122e7`.

## #17 handoff

Pass116 intentionally does not pretend to close #17. The existing substrate is unusually strong: signed generation records, signed WAL-frame chain, `compare_with_anchor`, and the `FreshnessAnchor` CAS interface already exist in `kernel-auth`; #18 proves local immutable-generation publication; #16 supplies authenticated distributed peer/quorum transport. The missing production boundary is an actually non-rollbackable freshness authority and store/recovery wiring that refuses local rollback/fork before publication/replay. See `PASS117_FRONTIER_17.md`.
