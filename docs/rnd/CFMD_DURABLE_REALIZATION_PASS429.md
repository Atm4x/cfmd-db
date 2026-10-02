# CFMD durable realization authority — PASS429

P429 establishes the first durable boundary for the Physical Realization Algebra.

Selected law:

```text
semantic RevisionId R
+ reachable PhysicalAtoms P
+ direct FactorizedRealizationRoot rho
    -> independently versioned CFPR image
    -> SingleFile PhysicalArtifact section
    -> authenticated/encrypted generation root
```

Reopen decodes `(R, P, rho)` and validates physical topology directly. `DatabaseState` remains a semantic verification oracle but is not required to reconstruct the current physical read authority.

The first codec deliberately accepts only direct factorized roots. Derived transforms, chunk overlays, witnesses, Scan seeds and maintained-query caches are not silently serialized. A non-direct root must be materialized or gain an explicit durable expression law in a later pass.

Crash/publication hostile:

```text
old published physical root
+ prepared/unpublished bytes at file tail
    -> reopen old root

new complete generation + root-slot publication
    -> reopen new physical root
    -> same semantic RevisionId
```

A materialized field replacement was used to ensure this is a real representation change: the root dependency set changes while field value and semantic revision remain equal. The image stores only reachable atoms, so the superseded atom is not retained merely because it still exists in the in-memory construction store.

P394 law remains mandatory: stable semantic relation-column identity is not ordinal order. Durable decode preserves encoded `column_order` literally and rejects duplicates; it never sorts by semantic ID.
