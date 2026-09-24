# Pass38 static authority audit

Checked the new exact-intent, combined migration and semantic deployment paths after the final Rust gate.

1. `DurableTransactionIntent::Exact` binds the exact canonical target Revision bytes. Optional materialization specs and builtin semantic implementation descriptors are equality-comparable parts of the retained intent.
2. New durable PREPARE rejects legacy/weak intent and re-decodes the exact target Revision using the reconstructed semantic registry before accepting it.
3. Recovery/checkpoint metadata retains exact committed intents; historical semantic module descriptors are installed before historical target matching/decoding.
4. `FullRevisionAndMaterializations` carries one canonical target Revision plus one exact materialization registry and is published through the same sealed whole-root boundary.
5. Physical layouts, row handles, indexes and materialized result contents are absent from durable semantic intent and deployment metadata.
6. Metadata v1 / mutation v2-v3 compatibility is explicit. Legacy target-only identities are not promoted to exact identities.
7. Builtin semantic deployment descriptors reconstruct only known compiled implementation families. Arbitrary executable/plugin code is not accepted from durable bytes.

No additional correctness/authority escape was found in this audit. Remaining limits are documented as OPEN in PASS38_REPORT.md.
