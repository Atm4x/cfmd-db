# IMPLEMENTATION REPORT — Pass63

Pass63 introduces one common physical-family inventory/memory-budget boundary without falsely claiming that every physical family now has an autonomous advisor.

## 1. Multi-family inventory

`PhysicalStore::artifact_memory_report()` reports `RelationLayout`, `SharedDenseIdentityMap`, `I64Index`, `SemanticIndex`, `SemanticQuotientFactor`, `SemanticQuotientSupport` and `SemanticStatistics`, including artifact counts, advisor-owned counts and deterministic estimated retained bytes.

Relation/layout state is part of the budget rather than an invisible constant. Shared `DenseEntityIds` backing used by multiple `DenseLiveEntityIds` columns is deduplicated by `Arc` identity and counted once.

## 2. Deterministic retained-size accounting

Retained-size estimators now cover current relation/algebraic storage, stable-row metadata, dense identity maps, I64 indexes, semantic bucket indexes, Γ-QCN factors/support and retained statistics.

All new size accumulation is saturating. The result deliberately excludes allocator/BTree-node overhead and is not presented as RSS or exact heap usage.

## 3. Memory-aware semantic-index advisor

`SemanticIndexAdvisorPolicy` adds:

```text
max_managed_estimated_bytes
max_total_estimated_bytes
```

beside the previous `max_managed_key_cells` compatibility/work-size ceiling.

Profitability remains a work-unit decision. Memory is an independent admission constraint. The global byte ceiling starts from the complete current physical inventory, subtracts only advisor-owned semantic indexes that this policy is allowed to replace, and treats all other state as fixed cost.

## 4. Unified ownership identity

The former `advisor_managed_semantic_indexes` set is replaced by typed internal `ManagedPhysicalArtifact` identities for I64/semantic indexes, Γ-QCN factors/support and statistics. Relation reinstall invalidation and inventory can therefore use a common artifact-identity boundary.

Only generic semantic indexes currently have an autonomous workload create/evict policy. Other families are inventory-visible but remain fixed/non-evictable to this advisor until their own benefit/maintenance laws exist.

## 5. Falsification

Coverage added/extended for:

- profitable semantic index rejected by managed estimated-byte budget;
- global budget counting retained statistics and relation state;
- shared dense identity backing counted once across two columns;
- Γ-QCN factor/support/statistics inventory;
- semantic bucket retained-size growth;
- pre-existing rule that manual indexes cannot be evicted by the advisor.

Final Rust 1.98.1 debug/release/Clippy/rustdoc/overflow gate passes. Cold release/overflow compilation timeouts were excluded; warmed retries completed. Frozen-source hashes match after the gate.

## 6. Boundary

Pass63 is **not** exact RSS accounting and is **not** full multi-family autonomous lifecycle. Those historical OPEN items are advanced, not closed. The production closure is the common inventory/ownership boundary plus a global memory-aware semantic-index policy that no longer pretends other physical families are free.

## 7. Post-freeze Program7 review

R&D Program7 was not integrated. Its delta-native relation-data ledger removes snapshot duplication, but a hostile post-compaction retry proves that `(source RevisionId, target RevisionId, delta, Γ)` is not sufficient to preserve the current exact intent contract while `RevisionTransitionRequest` still carries an independently supplied target Revision. Same txid/IDs/delta plus different untouched target content can be accepted as `AlreadyCommitted` before target validation. The space problem remains real; the submitted closure is production-OPEN pending an authority/content-identity redesign. See `PASS63_RND_PROGRAM7_REVIEW.md`.
