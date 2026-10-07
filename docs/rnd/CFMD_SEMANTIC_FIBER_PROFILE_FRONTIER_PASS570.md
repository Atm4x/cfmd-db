# PASS570 R&D — SemanticFiber identity and physical profile frontier

## Selected law

One semantic binding denotes one semantic-fiber artifact identity. Retention shape is a physical profile, not a second semantic artifact family:

```text
SemanticFiber(binding)
  profile = Cardinality | Quotient | Observable

capabilities:
  Cardinality -> {ExactCardinality}
  Quotient    -> {ExactCardinality, QuotientFiber}
  Observable  -> {ExactCardinality, QuotientFiber, ObservableFiber}
```

Consumers express `SemanticFiberDemand`; a profile is admissible iff it satisfies the demand. Physical policy then compares actual resource/work cost among admissible profiles.

## Hostile measurement

Same binding, 4,096 rows, 257 equivalence classes, deterministic retained-byte estimator:

| profile | retained bytes |
|---|---:|
| Cardinality | 39,456 |
| Quotient | 848,368 |
| Observable | 230,192 |

The capability lattice is monotone, but retained-memory cost is not. Observable is strictly stronger than Quotient yet materially smaller for this shape. Therefore “pick the weakest sufficient profile” is not a valid universal optimizer law. Selection must compare a multidimensional frontier: retained memory, build work, incremental maintenance work, and read work saved.

## PASS570 boundary

CLOSED: semantic artifact identity, capability owner, telemetry/memory family vocabulary, inherited derived-commit retry debt.

OPEN: family-specific state structs and durable recipe variants are still physical implementations. They may be unified structurally only after build/update benchmarks prove the replacement does not lose a useful point on the cost frontier.
