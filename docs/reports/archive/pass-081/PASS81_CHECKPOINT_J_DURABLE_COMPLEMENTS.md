# PASS81 CHECKPOINT J — DURABLE MIGRATION COMPLEMENTS

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

Closed in this checkpoint:

1. Added `DurableMigrationComplement`: migration step metadata is retained permanently while local complement payload is a separately releasable authority.
2. `Forever` requires local payload; `UntilRevision` and `UntilEpoch` may release only at an explicit declared boundary; `ExternalArchive` and `Forget` never persist local complement payload as authority.
3. Released steps remain as tombstone metadata so migration-chain continuity/history is not silently erased.
4. Durable metadata codec is v7 and persists the migration-complement ledger; v6 metadata remains readable with an empty complement ledger.
5. `DurableRevisionStore::stage_migration_complement` publishes the capsule in a fresh checkpoint generation and requires source-schema agreement plus chain continuity.
6. `release_due_migration_complements` publishes payload release in a new checkpoint generation and is no-op when no retention boundary is due.
7. Restart tests prove payload survival before release and tombstone-only persistence after release.

Verification:
- fmt/check workspace PASS;
- workspace tests: **547 passed / 0 failed / 8 ignored**;
- workspace clippy `-D warnings` PASS;
- **555 declared tests**;
- no lint suppressions added.

Not claimed: atomic attachment of the complement capsule to the schema-transition WAL descriptor. J establishes durable complement/retention authority; transactional schema-migration binding remains the next write checkpoint.
