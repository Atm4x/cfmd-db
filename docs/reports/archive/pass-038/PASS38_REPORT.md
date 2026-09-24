# PASS38 REPORT — exact durable intent + combined revision/config transaction + builtin semantic deployment manifest

Status: **VERIFIED** on Rust **1.98.1**.

Pass38 closes three durable-control-plane correctness/authority problems left after Pass37. Tests are evidence for those fixes; individual test cases are not counted as separate closed problems.

## 1. Problem

Three concrete gaps remained after Pass37.

1. **Historical idempotency retained too little request identity.** A committed `ClientTransactionId` was retained primarily with nominal target identity. After later heads/checkpoint rotation/compaction, a retry needed an exact proof that it named the same semantic target rather than merely a reused `RevisionId`.
2. **Semantic Revision and maintained-materialization configuration could not change in one atomic transaction.** Pass37 could replace a full Revision or reconfigure materializations separately, but a schema/Γ migration that simultaneously invalidated the old maintained registry still required a staged sequence.
3. **Builtin semantic implementation deployment remained external.** Durable revisions pinned module digests, but restart still depended on a caller-provided registry to supply the implementation revision corresponding to those digests.

## 2. Design

### 2.1 Exact retained durable transaction intent

New commits persist a `DurableTransactionIntent::Exact` containing:

```text
target RevisionId
+ canonical encoded target Revision=(S,Γ,M)
+ optional exact materialization registry
+ exact builtin semantic implementation descriptors required by that target
```

The exact intent is present in the WAL PREPARE and carried into generation metadata when checkpoint rotation compacts the WAL tail. Therefore transaction identity survives:

```text
WAL -> later committed heads -> checkpoint -> compaction -> restart
```

Retry matching is now equality of retained exact intent, not equality of nominal target ID. Reusing one `ClientTransactionId` with the same `RevisionId` but different Revision content or different combined materialization registry is a conflict.

Legacy metadata/WAL entries remain explicit `LegacyTargetOnly`; old weak identity is never silently upgraded to exact identity.

### 2.2 One atomic Revision + materialization-registry transaction

`DurableRevisionChange` adds:

```text
FullRevisionAndMaterializations {
    encoded_target_revision,
    materializations,
}
```

`RuntimeRevisionCell::prepare_revision_and_materializations` builds one candidate root containing the target semantic Revision and the desired maintained registry before any durable publication.

The normal authority ordering remains:

```text
build complete candidate
-> durable PREPARE(exact Revision + exact materialization registry)
-> seal live root
-> durable COMMIT
-> infallible whole-root publish
```

Recovery decodes/revalidates the target Revision, obtains the materialization registry from the committed descriptor/generation metadata, rebuilds physical state and maintained state, and returns one coherent root.

### 2.3 Durable builtin semantic implementation manifest

`kernel-semantics` now exposes `BuiltinSemanticModuleSpec` for the current builtin implementation families:

- equality/equivalence;
- tokenizer;
- ordering.

A descriptor contains the semantic contract variant plus its exact `implementation_revision`; recomputing its digest yields the pinned `ModuleDigest`.

`SemanticRegistry::builtin_modules_for_context` collects the exact builtin implementations required by a semantic context. New transaction intents and generation metadata persist these descriptors. On open/recovery, `DurableRevisionStore` reconstructs the builtin registry before decoding/revalidating checkpoint and WAL targets.

Historical exact transaction intents also retain their semantic implementation descriptors. This is required so an old transaction can still be proven identical after later heads change Γ or implementation revisions.

This is deliberately **not** arbitrary executable-code persistence. Unknown/plugin module binaries, signatures and deployment remain OPEN.

### 2.4 Compatibility boundary

- mutation codec advances to v4;
- metadata codec advances to v2;
- mutation v2/v3 payloads remain explicitly decodable;
- metadata v1 decodes committed entries as `LegacyTargetOnly` and has no semantic deployment manifest;
- stores lacking that manifest require the explicit `open_with_legacy_registry` compatibility path at the durability layer.

This is controlled backward compatibility, not a universal automatic migration framework.

## 3. Hostile falsification / authority audit

The production suite and static audit verify the important boundaries:

- an old exact transaction remains idempotently recognizable after unrelated later heads, checkpoint rotation, obsolete-generation compaction and restart;
- the same `ClientTransactionId` and same nominal `RevisionId` with different Revision contents is rejected;
- combined Revision + materialization-registry commit publishes and recovers as one unit;
- retry of that combined transaction matches the exact registry as part of intent;
- historical semantic implementation descriptors survive checkpoint/compaction/reopen and allow the old target to be reconstructed/matched;
- current exact PREPARE cannot be created without a complete exact transaction intent;
- legacy target-only entries are not accepted as proof of an exact new retry;
- durable semantic deployment persists only builtin descriptors, not physical state or arbitrary executable code.

Final cleanup also refactored the durability decoder/open path rather than suppressing strict Clippy warnings: semantic-registry reconstruction, recovered-intent merge, and current mutation-codec decoding are isolated helpers.

## 4. Verification

Final Rust 1.98.1 gate:

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

Metrics:

```text
297 declared tests
81 kernel-plan tests
65 kernel-query tests
32 kernel-durability tests
20 crates
38,065 Rust LOC
0 external Cargo sources
0 unsafe
```

## 5. Problem ledger

### Closed exactly in Pass38

1. ✅ **Historical client transaction identity was not exact enough after later heads/checkpoint/compaction.** New transactions retain the canonical target Revision, applicable materialization registry and builtin semantic implementation descriptors as `DurableTransactionIntent::Exact`; exact retry survives checkpoint/compaction/restart and conflicting same-ID intent is rejected.
2. ✅ **Revision replacement and materialization-registry replacement were not one transaction class.** `FullRevisionAndMaterializations` now prepares, durably commits, publishes and recovers the semantic Revision plus maintained registry atomically.
3. ✅ **Restart depended on externally supplied implementations for current builtin semantic modules.** Published generations and exact historical intents now persist `BuiltinSemanticModuleSpec` descriptors sufficient to reconstruct the current builtin `SemanticRegistry` and revalidate pinned digests without caller-supplied registry authority.

### Closed from the Pass37 OPEN list

1. ✅ Combined semantic Revision + materialization-registry transaction.
2. ✅ Exact long-lived historical transaction-intent identity for new-format transactions.
3. ✅ Durable deployment manifest for the currently supported builtin semantic implementation families.

### Partially advanced, still OPEN

1. 🟨 **Durable format migration.** Mutation v2/v3 and metadata v1 have explicit compatibility decoding, but there is no general migration engine covering arbitrary historical checkpoint/manifest/metadata/version combinations.
2. 🟨 **Semantic implementation deployment.** Builtin implementation families are self-describing by durable descriptor; arbitrary plugin/external executable artifacts, authenticity and lifecycle are still OPEN.

### Historical Pass26 OPEN backlog still active

1. ⬜ Generic Text/F64 maintained TopK order-statistics.
2. ⬜ I64 TopK constant-factor gap.
3. ⬜ Group/TopK as typed-batch producers.
4. ⬜ Maintained I64 Group constant-factor gap.
5. ⬜ Indexed generic/Text Group.
6. ⬜ Persisted Text/F64/Bool/entity indexes + planner.
7. ⬜ Nested/multiway/mixed-key joins.
8. ⬜ Remaining physical layouts + OrderedView/pagination.
9. ⬜ WAL/recovery/durable materializations/crash tests — logical WAL, full semantic revisions, combined Revision/config commits, checkpoint/manifest/restart, exact client retry, builtin semantic deployment manifest and Linux process-kill matrix are integrated; machine power-loss, cross-filesystem assurance, arbitrary semantic artifacts and durable DAG history remain OPEN.
10. ⬜ Transaction repair runtime, distribution, formal mechanization.
11. ⬜ Semantic indexing/canonical-key strategy for generic maintained Join — Agent-2 R&D VERIFIED; production integration OPEN.

### Remaining / newly clarified OPEN after Pass38

1. ⬜ Durable revision DAG / branch+merge ancestry and parent encoding. Current durable runtime remains a linear committed-head protocol.
2. ⬜ General automatic migration framework for historical checkpoint/manifest/metadata/mutation versions.
3. ⬜ Arbitrary/plugin semantic executable artifact packaging, authenticity/signing and deployment lifecycle. Pass38 closes builtin descriptors only.
4. ⬜ Transaction-outcome retention/GC policy; correctness-first metadata still retains exact committed intents indefinitely.
5. ⬜ Streaming/chunked checkpoint and metadata codecs; current bounded codecs buffer complete payloads.
6. ⬜ Machine power-loss validation; subprocess kill cannot prove persistence through volatile controller/device caches.
7. ⬜ Cross-platform/filesystem durability contracts: Windows, network filesystems, FUSE and equivalent rename/directory-sync semantics.
8. ⬜ Rebuild policy/performance for secondary indexes and alternate layout replicas after recovery/full semantic replacement.
9. ⬜ Candidate runtime roots remain clone-heavy; COW/persistent roots are still a performance/scale requirement.
10. ⬜ General Rust lock-poison/restart policy beyond the explicit durability fail-stop supervisor path.
11. ⬜ Group commit / async durability / replication / distributed consensus.
12. ⬜ Authenticated durable storage. CRC32C detects accidental corruption but is not a MAC/signature.
13. ⬜ Formal/power-loss proof of rename/fsync/GC semantics on deployment filesystems.
14. ⬜ Legacy metadata v1 has no builtin deployment manifest and therefore needs an explicit trusted legacy registry during compatibility open; automatic one-step upgrade remains OPEN.

## 6. Result / next boundary

The current **single-head durable control plane is functionally coherent** for:

- relation-data commits;
- arbitrary complete semantic Revision replacement;
- materialization configuration;
- combined Revision + materialization-registry migration;
- exact client retry identity;
- builtin semantic implementation deployment descriptors;
- checkpoint/manifest/restart and process-kill recovery.

The remaining durability work is mostly history/distribution, general migration, arbitrary executable deployment, scale, authentication and external filesystem/power-loss assurance rather than a missing single-head correctness path.

The next productive mainline is therefore the historical data-plane backlog, beginning with Agent-2 semantic canonical keys/indexes, unless a new independent hostile review produces a correctness counterexample.
