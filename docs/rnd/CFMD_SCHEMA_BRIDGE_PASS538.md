# CFMD PASS538 — certified current-world SchemaBridge foundation

## Question

Can a client language formed against schema A be compiled directly against authoritative current schema B without resurrecting historical A as a live database, keeping dual schemas, or introducing a generic per-query migration router?

## Selected law

PASS538 isolates one exact bridge object from the already-verified migration theorem:

```text
SchemaMigrationProgram M : A -> B
        verify(A)
           |
           v
     SchemaBridge<A,B>
```

The bridge contains semantic certificates only. It owns no source database state.

For relational reads, the first admitted class is the strongest class that can be compiled without an inverse transform or result reconstruction:

```text
q_A
  scans A_i

forall A_i:
  migration proves exact row-representation identity A_i -> B_i

then
  q_B = q_A with Scan(A_i) -> Scan(B_i)
```

The entire relational operator tree is preserved verbatim. The rewritten query is independently typechecked under B and must have exactly the same result type as q_A. Distinct source scan relations may not alias onto one target relation.

This is a current-world compilation theorem, not a maintained-watch theorem. It reuses the same row-identity proof discovered for MigratableWatch, but no maintained state is involved.

## Why value-changing reads remain fail-closed

A row-local migration can have an exact forward transform and exact future delta transport while an arbitrary old read is still not representable in the old result language from current B.

Example:

```text
A.age : i64
M      : i64 -> f64
B.age : f64
```

`transport_relation_delta_exact` can map an A delta to B exactly. That does not prove that an arbitrary query returning A rows can be answered from B and returned as A rows. Doing so would require a certified factorization/inverse-like read theorem for the specific observation. PASS538 therefore returns `UnrepresentableReadRelation` rather than fabricating A values or reading historical A.

## Exact relation intent side

The same `SchemaBridge` delegates exact relation delta and relation-column write-footprint transport to the already-verified migration transport. This means read and write representability share one migration authority rather than two frontend-specific engines.

The retained-lineage `CurrentSchemaBridge` composes one or more verified `SchemaBridge` steps:

```text
A -> B -> C -> ... -> Current
```

It supports:

- `compile_read_exact` by successive certified read compilation;
- `transport_relation_delta_exact` by successive exact delta transport;
- `transport_relation_write_footprint_exact` by successive semantic-coordinate transport.

Authorization is deliberately not transported. The output coordinates belong to the current authoritative schema and must be authorized there.

## Retained-lineage law

`DurableRuntime::current_schema_bridge(source_schema_revision)` searches only retained schema-epoch authority already owned by the durability/runtime kernel. It never calls historical revision materialization.

The chain is accepted only when:

1. a retained epoch begins at the requested source schema revision;
2. every next epoch's source context exactly equals the preceding target context;
3. every migration program verifies against that exact context;
4. the chain reaches the authoritative current semantic context.

Missing/released/non-contiguous lineage yields `None`; callers must treat that as non-representable rather than falling back to history.

## Hostile findings

Rejected:

- historical A snapshot as current client compatibility — stale;
- inverse migration B -> A — not generally defined and violates current-world authority;
- dual live A/B schemas — duplicates semantic authority;
- name/existence mapping — names are not semantic identity;
- per-row/per-query fallback router — puts migration decisions on the hot path and recreates mixed-schema architecture;
- generic transform-result-then-decode-as-A — unsound for non-injective/value-changing migration;
- transporting old authorization grants — authorization remains current-B coordinate authority.

## Performance law

Bridge construction is proportional to retained schema migration depth and migration-program metadata, not data cardinality. Row-identity read compilation walks the query descriptor and scanned relation set only. Exact relation-intent transport is proportional to touched delta rows/coordinates already required by the mutation itself. No source/current relation scan or historical state reconstruction is introduced.

## What remains

PASS538 does not yet expose a post-cutover `Context<A>` product facade. The next pass must bind the old typed contract to a current-world bridge and lower object/model-field operations through it. That integration must preserve current-B authorization and reject unsupported model-field/type/split/merge cases as `ContractNotRepresentable`.
