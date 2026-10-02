# CFMD PASS435 — Shared Physical Atom + Read Authority R&D

## Selected ownership law

`PhysicalAtomStore` is an immutable/versioned authority over `PhysicalAtomId -> Arc<PhysicalAtom>` carried by `kernel_persistent::PersistentOrdMap`.

This gives:

- O(1) snapshot clone of the map root;
- O(log N) structural path copy per sparse atom insertion/removal;
- one physical atom payload allocation per live atom lineage;
- exact structural reclamation after the last owning root/read handle drops;
- no O(all atoms) clone when a historical `ReadContext` is created.

A whole-map `Arc<BTreeMap<Id, Arc<Atom>>>` COW was rejected as the authority model. It has cheap clone, but the first mutation while a snapshot is pinned copies O(N) map topology. Release hostile at N=100,000 showed the expected crossover: persistent wins strongly for sparse root rewrites, while a 1024-update batch can be CPU-faster with one linear COW clone. CFMD selects persistent ownership because historical/current/read branches require bounded retained-memory growth, not only one-batch throughput.

## Read authority law

`ReadContext` now has one semantic query surface over two representation forms:

- logical committed `Revision` (live and legacy history);
- exact factorized historical snapshot (`RevisionId + SemanticContext + FactorizedRealizationRoot + shared PhysicalAtomStore`).

For a complete retained historical realization root, `Database::at(revision)` binds the factorized form directly. `PreparedQuery::execute` uses the existing one-shot `kernel-realization` evaluator and does not materialize a full historical `DatabaseState`.

This is representation routing beneath one semantic read API, not a second historical query engine.
