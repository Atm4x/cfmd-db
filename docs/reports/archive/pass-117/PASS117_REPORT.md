# Pass117 — Historic #17 external freshness integration (PARTIAL)

## Status

- Historic #17: ACTIVE / PROD PARTIAL.
- Source freeze: 2026-09-23 before 17:50 UTC hard wall.
- No claim of #17 closure yet.

## Implemented

1. `kernel-auth` now defines a canonical signed `FreshnessCut` binding:
   - store id;
   - generation + predecessor generation authority digest;
   - generation material authority digest;
   - WAL LSN + WAL prefix authority digest;
   - trust-root epoch;
   - deployment-policy epoch.
2. Strict Ed25519 verification and record digest for external freshness cuts.
3. Metadata codec v13 persists `DurableExternalFreshnessBinding` inside the generation published before manifest authority.
4. `DurableRevisionStore::adopt_external_freshness` rotates into an externally-bound generation rather than pretending an older unbound generation was anchored.
5. `open_with_external_freshness` verifies the external signed record and performs rollback/fork/gap/WAL-prefix preflight before normal store recovery/publish path.
6. Plain `open()` rejects externally-bound stores, preventing accidental bypass.
7. Every durable PREPARE/COMMIT and group durability barrier advances the WAL freshness cut.
8. Ordinary and streaming checkpoint publication advance the generation freshness cut only after local manifest publication. Failure poisons authority rather than serving uncertain state.
9. Generation digest binds manifest/checkpoint root/checkpoint chunks/metadata/prepared capsule. WAL freshness uses exact valid frame-prefix digest, permitting authenticated prefix extension while detecting truncation/fork.

## Evidence completed in this pass

- focused external-freshness anchored-mode test: PASS.
- `kernel-auth` tests: PASS.
- `kernel-durability` tests: 92/92 PASS.
- replication multi-process tests: 2/2 PASS.
- strict Clippy for kernel-auth + kernel-durability: PASS.

## Still OPEN before #17 closure

1. Production genuinely separate rollback-domain backend (process/TCP/provider adapter) rather than only the authority trait/test authority.
2. Independent-process anchor restart/fault matrix.
3. Full hostile reopen matrix: generation rollback, same-generation valid fork, WAL truncation/replay, old trust/deployment epoch, CAS response-loss before/after apply, anchor unavailable.
4. Full workspace fmt/check/clippy/test gate after those additions.

Historical closed count remains 19/22.
