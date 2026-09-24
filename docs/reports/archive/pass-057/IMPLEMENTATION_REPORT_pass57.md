# IMPLEMENTATION REPORT — Pass57

Pass57 rebases the merge-ready portion of R&D Program 2 onto authoritative Pass56. Only `crates/kernel-query/src/lib.rs` and `crates/kernel-plan/src/lib.rs` change in production.

## `kernel-query`

`MaterializedRelPlanState::node` changed from owned `MaintainedRelPlanNode` to `Arc<MaintainedRelPlanNode>`.

Read-only traversal uses `self.node.as_ref()`. Mutating recursive operations use `Arc::make_mut(&mut self.node)`, so a clone shares the complete maintained subtree until an actual mutation reaches a path.

No semantic query law changes. Prepared-state equality/output/delta behavior remains governed by the same pinned `SemanticContext` and existing maintained node implementations.

New hostile test: `maintained_plan_clone_uses_cow_and_isolates_mutation`.

## `kernel-plan`

`PhysicalStore` heavy payload maps now hold `Arc<State>`:

- `relations: BTreeMap<..., Arc<InstalledRelation>>`
- `i64_indexes: BTreeMap<..., Arc<MaterializedI64IndexState>>`
- `semantic_indexes: BTreeMap<..., Arc<MaterializedSemanticIndexState>>`
- `semantic_quotient_factors: BTreeMap<..., Arc<MaterializedSemanticIndexState>>`
- `semantic_quotient_supports: BTreeMap<..., Arc<MaterializedSemanticQuotientSupportState>>`
- `semantic_statistics: BTreeMap<..., Arc<MaterializedSemanticStatisticsState>>`

All affected mutation paths use `Arc::make_mut`. Read APIs expose `&State` through `Arc::as_ref`, preserving caller contracts.

`PhysicalStore` also owns an in-process `state_identity: Arc<()>`. `PreparedPhysicalStoreTransition` keeps `source_identity` instead of `Box<PhysicalStore>`. Commit freshness checks pointer identity + epoch + optional source revision. Every physical mutation/publication path that advances state rotates `state_identity`.

This witness is deliberately non-durable and does not enter revision semantics.

`RuntimeRevisionBundle::validate_target_logical_state` now validates unchanged global fields and untouched relations directly, and reconstructs only changed relations. It no longer clones the full logical `DatabaseState` as a temporary mutation target.

New hostile test: `physical_store_clone_cow_isolates_relation_mutation`. It verifies relation-root and I64-index-root sharing before mutation and COW separation plus exact original/candidate index contents after mutation.

Integrated R&D test: `logical_transition_validation_reconstructs_only_affected_relation`.

Integrated diagnostic benchmark: `benchmark_persistent_physical_store_clone_tax_with_and_without_index`.

## Explicit non-integration

The R&D Program-1 relation multiset code is not replayed: Pass55 already owns that implementation. `EqClassId`, `TransientEqClassInterner`, and the R&D Set-validation rewrite are also not merged because they do not yet justify a second runtime classification layer and the validation form lacks the production future-custom exact fallback discipline.

## Remaining architectural limit

The heavy values are persistent/COW, but the artifact maps themselves are ordinary `BTreeMap`s. `PhysicalStore::clone()` therefore still copies map entries. The current change removes payload-size-dependent deep copying; it does not claim `O(1)` clone with respect to artifact count.
