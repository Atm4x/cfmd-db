# PASS81 CHECKPOINT K — ATOMIC SCHEMA MIGRATION + COMPLEMENT WAL

Status: **INTEGRATED / DEBUG+CLIPPY VERIFIED**.

Closed:
1. Schema-transition target bytes and migration complement are one exact durable transaction intent (`SchemaMigrationExact`).
2. WAL mutation codec v7 persists the complement in PREPARE; v5/v6 remain backward-readable.
3. Metadata codec v8 persists the exact transaction identity; v7 remains readable.
4. Prepare validates source-chain continuity and target-schema agreement against the decoded exact target revision.
5. COMMIT installs complement authority idempotently; recovery rebuilds the complement ledger from committed WAL descriptors in commit order.
6. Hostile: COMMIT without checkpoint rotation -> reopen recovers both target head and complement.

Verification: full workspace fmt/check/tests/clippy `-D warnings` PASS. No lint suppressions.
