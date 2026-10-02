# PASS437 R&D — FACTORIZED MODEL DELTA ACTION

`DurableModelDelta` is now an exact action on a direct factorized historical read snapshot.

For each transition, the implementation rewrites only coordinates carried by the delta: lifecycle graph, changed carrier entity orders and changed field columns. The final direct root is structurally validated and the atom store is pruned to exact dependencies. Relation effects compose with this action independently at the same target revision.

The law is intentionally representation-level but not a second semantics engine: `DurableModelDelta` remains the durable semantic authority; the factorized action is its exact lowering. Schema/semantic-change/full effects remain outside this law.

This closes the ordinary reversible mixed-history payer from P436 and removes storage/history from the immediate architecture roadmap absent new hostile evidence.
