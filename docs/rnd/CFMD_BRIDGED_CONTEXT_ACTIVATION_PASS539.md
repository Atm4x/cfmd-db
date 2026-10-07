# CFMD PASS539 — bridged typed Context activation

## Problem
A client compiled against schema-language A may first connect after authoritative HEAD already moved to B. The server must not reconstruct historical A, expose a second current schema, guess aliases, or run a per-query compatibility router.

## Selected law
`Context<A>` may activate against current B only when A carries an explicit contract schema revision and retained verified migration lineage compiles a `CurrentSchemaBridge<A,current>`.

Admission is the only bridge-resolution point. The admitted `ReadContext` still owns the current authoritative revision/state/Session authority, while an optional contract semantic context describes the old typed language used to construct queries. Before prepare, the complete source `RelExpr` is compiled through `CurrentSchemaBridge`; prepare, read-footprint authorization, causal observation capture and execution then operate only on current-world coordinates.

Ordinary current-schema Context remains the direct specialization: no bridge object, no migration lookup and no extra query compilation branch beyond the `None` check.

## Typed contract revision
`CfmdSchema::contract_schema_revision()` is explicit bridge provenance, defaulting to `None`. `#[derive(CfmdSchema)]` supports `#[cfmd(schema_revision = N)]`. This is not a handshake and not a server-side client registry: generated code carries the language revision it was compiled for. Shape/name inference is rejected because several retained schemas can be superficially compatible while having different migration semantics.

## Read theorem
For a bridged read `q_A`, the existing kernel bridge must prove

```text
q_A = q_B o M
```

for the admitted row-representation-identity fragment. The bridge rewrites exact scan coordinates, independently typechecks the current expression and preserves the result type. Value-changing/split/merge/global relation rewrites remain `ContractNotRepresentable`.

The runtime never evaluates `q_A` over historical state. `q_A` exists only as client-language IR until compilation; `q_B` is the only prepared/executed query.

## Mutation theorem admitted in PASS539
A bridged object field patch may lower directly into current Plan coordinates only when every required coordinate is definitionally preserved:

```text
source relation --row identity--> one current relation
source model field --value identity--> one current model field
```

The row representation and ordinal meaning are unchanged, so the existing exact object-field patch plan may use the current relation coordinate without converting the row. The model field coordinate is separately transported by a new definitionally-identity field law. Authorization therefore observes current-B Plan/read footprints; grants are never transported.

PASS539 deliberately does not claim general old-language writes. Bridged create/delete, relationships and semantic preconditions fail with `ContractNotRepresentable` until their lifecycle/relationship/expression coordinates have their own exact transport theorem. This is preferable to lowering a partially understood object contract or rebuilding A.

## Hostile findings
- Inferring A from `S::definition()`'s ordinary builder revision is unsound because current Rust consumer schemas historically default to revision 1. Bridge provenance must be explicit; hence `contract_schema_revision`.
- A bridge must not change `ReadContext::semantic_context()` to A. Query execution, Γ-DTC observation state and authorization must see current B. PASS539 therefore separates the optional contract/language context from the execution semantic context.
- Retargeting only reads would leave field patches writing source relation IDs. PASS539 maps exact relation and model-field identities before constructing the current Plan.
- Field dependency transport is too broad for direct assignment: a source field can fan out or feed a value transform. Direct patch lowering therefore uses a narrower definitionally-identity field theorem, not dependency transport.
- Create/delete/relationship mutation cannot be inferred from row identity alone because lifecycle/object contracts carry additional semantic coordinates. They remain fail-closed.

## Complexity
Bridge lookup occurs once at Context admission and is proportional to retained migration depth/program metadata. Query compilation is proportional to query descriptor/scanned relation coordinates. No data rows, historical revisions or query results are scanned to activate a Context. Current-schema Contexts retain their existing path.

## Result
PASS539 closes the first real post-cutover product activation path: a newly arriving explicitly-versioned A contract can execute an exact A read directly against authoritative B through retained migration proof. The next theorem is the remaining object/lifecycle/relationship mutation surface plus remote endpoint activation syntax; only after that should P0 client lifecycle be declared complete.
