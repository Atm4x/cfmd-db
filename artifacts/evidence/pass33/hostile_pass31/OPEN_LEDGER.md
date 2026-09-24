# Hostile OPEN ledger after Pass31

## Reopened / newly demonstrated

- [ ] **H31-01 BLOCKER — bootstrap row↔handle binding.** `RuntimeRevisionBundle::build` accepts semantically equal but reordered physical Bag state, then positional handle attachment corrupts maintained Scan identity.
- [ ] **H31-02 HIGH — maintained Scan exact logical order.** `swap_remove` leaks derived storage order after non-tail deletion.
- [ ] **H31-03 HIGH — resolved evidence row↔handle validation.** Validate handle membership *and* payload association before candidate mutation/propagation.
- [ ] **Pass27 leaf contract status correction.** O(n) semantic lookup is closed; correctness of initial binding and order-preserving maintenance is not fully closed.
- [ ] **Pre-WAL regression gate.** Add exact unsorted root/materialization oracle tests for reordered bootstrap, first/middle deletion, duplicates, semantic-equivalent representatives, and multi-Scan/multi-materialization trees.

## Pass31 / Pass30 transaction-core OPEN retained

- [ ] COW/persistent immutable candidate subroots instead of correctness-first deep clones.
- [ ] Schema/Γ-changing revision transaction / rebuild-migration path.
- [ ] Process/filesystem crash atomicity.
- [ ] Production WAL/replay/checkpoint/segment recovery integration.
- [ ] Durable stable encoding/versioning/checksum for logical revision commit descriptors.
- [ ] Selected-layout multiplicity / derived layout registry.
- [ ] Certified digest/root bootstrap fast path with exact fallback.
- [ ] General logical change descriptor for lifecycle/field/schema/semantic changes or typed migration transactions.
- [ ] Runtime root identity must remain process-local freshness identity, never durable authority.
- [ ] RwLock poisoning/recovery policy.
- [ ] Publication primitive performance / reader blocking across future fsync.
- [ ] Legacy unbound PhysicalStore source/candidate clone cleanup.

## Historical Pass26 production backlog retained

- [ ] Generic Text/F64 maintained TopK order-statistics.
- [ ] I64 TopK constant-factor gap.
- [ ] Group/TopK as typed-batch producers.
- [ ] Maintained I64 Group constant-factor gap.
- [ ] Indexed generic/Text Group.
- [ ] Persisted Text/F64/Bool/entity indexes + planner.
- [ ] Nested/multiway/mixed-key joins.
- [ ] Remaining physical layouts + OrderedView/pagination.
- [ ] WAL/recovery/durable materializations/crash tests (Agent-1 R&D verified; production integration OPEN).
- [ ] Transaction repair runtime, distribution, formal mechanization.
- [ ] Semantic indexing/canonical-key strategy for generic maintained Join (Agent-2 R&D may exist; production integration OPEN).
