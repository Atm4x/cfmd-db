# PASS536 R&D — value-changing maintained-state transport boundary

## Question

For a verified row-local migration `m : R_A -> R_B`, can an already-maintained Γ-DTC query state cross `A -> B` exactly without scanning/rebuilding maintained rows?

The required law is stronger than exact delta transport.  Let `S_A` and `S_B` be maintained states, `step_A/step_B` their exact delta transitions, and `T_m` the row-local delta transport induced by the verified migration.  A zero-row-touch cutover needs a state homomorphism `H_m` such that

```text
observe_B(H_m(s)) = transported_observation(observe_A(s))
H_m(step_A(s, d)) = step_B(H_m(s), T_m(d))
```

and constructing `H_m` must be O(plan/schema metadata), not O(number of maintained/source rows).

## What already exists

`SchemaMigrationTransport::transport_relation_delta_exact` gives exact pointwise transport for future row-local deltas.  PASS534/535 additionally prove the special case where `H_m` is identity on row payload/support and only semantic coordinates/context move.

PASS536 adds `ObservationRelationTransport` so the verified migration kernel distinguishes these two facts explicitly:

```text
RowIdentity
    => metadata-only maintained-state transport theorem exists

RowLocalStateTransform
    => exact pointwise row transform exists,
       but no zero-row-touch maintained-state theorem is implied
```

This distinction is derived from the verified migration program, not frontend names or a compatibility table.

## Hostile obstruction in the current representation

`MaterializedRelPlanState` currently stores semantic values concretely:

- Scan leaves own `PersistentVec<Row>` in the current row value domain;
- Scan mutation/search owns Γ-canonical lookup/support for those values;
- Distinct/set operators own exact support state;
- join/group/top-k own operator-specific maintained state derived from semantic values/keys.

If `m` genuinely changes row values or row shape, a metadata-only rebind cannot make a source-domain stored `Row` become a target-domain `Row`.  Even if future deltas commute pointwise through `T_m`, the already-maintained state does not.  Under the current representation, exact conversion therefore requires visiting affected maintained payload/support state.  That is precisely the O(rows)/rebuild fallback prohibited by the product law.

The obstruction is representation-level rather than a missing branch in `MigratableWatch`.  Adding a generic `map(all maintained rows)` path would be correct only by abandoning the zero-rebuild theorem and would also duplicate transformation work already owned by migration/realization.

## Clean universal direction, if revisited later

The no-fallback route would be to factor maintained observation state itself, analogous to Physical Realization Algebra:

```text
stable observation/occurrence carriers
    + ObservationRealizationRoot
    -> current semantic row/key domain
```

A migration could then publish a new observation realization root and transform only touched carriers/deltas.  To be production-valid this would also need exact laws for operator support (set/distinct/join/group/top-k), stable occurrence identity across migration, target Γ-key realization, and bounded retained transform chains.  That is a query-state architecture change, not a PASS536 patch.

For DB 1.0 the selected decision is therefore: do not build that subsystem now.  Value/shape-changing maintained-state migration remains fail-closed, with an explicit diagnostic distinguishing it from an unrecognized/general rewrite.  Continue to zero-downtime Context/long-lived-reader lifecycle closure.

## Selected law

```text
exact future delta transport != exact maintained-state transport
```

Only a proved metadata/state homomorphism may keep a watch alive across a migration.  Row identity has one; genuine value-domain change currently does not.  No query replay, full-result rebuild, old-schema current-world routing, or generic transform fallback is permitted.
