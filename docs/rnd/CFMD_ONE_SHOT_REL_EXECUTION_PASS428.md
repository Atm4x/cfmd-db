# PASS428 R&D — complete one-shot RelExpr execution + sealed unchanged-row Γ propagation

## Selected law

The current relational IR is executed for one-shot physical preparation by one exhaustive storage-neutral lowering:

```text
RelExecutionSource
    -> exact RelExpr operator state
    -> RelExecutionSink
    -> physical columns + witness
```

There is no compatibility route to the complete-row evaluator and no maintained Query/Watch bootstrap. `execute_expr` now exhaustively matches every current `RelExpr` variant; adding a future variant is therefore a compile-time obligation to define its one-shot semantics.

## P428 additions

- `Group` stores only first-seen semantic group coordinates, a Γ group-key lookup, and exact aggregate state (`ExactCount` or `ExactF64Sum`). Empty global groups preserve the existing zero-input aggregate law.
- `TopKWithTies` stores canonical ordering buckets only for the current top-K boundary. Whole worse buckets are discarded as soon as they cannot participate; the retained bound is `K + all rows tied at the boundary`, which is mathematically inherent.
- `FilterEqConst`, ordered Filter, `FilterEqColumns`, and `AntiJoin` recursively preserve sealed `CertifiedCanonicalRowKey` evidence when their unchanged-row input already owns it. No raw canonical key injection was added.
- The obsolete `UnsupportedOneShotRelExpr` error was removed because every current relational operator now has an exact one-shot lowering. Future unsupported syntax fails at compile time in this owner rather than falling back at runtime.

## CanonicalRowPositionIndex hostile

P427 changed row-position ownership from full canonical-key payload copies to shared class payloads. P428 compared the current implementation against a frozen P426 copy using the same release hostile: 100k rows, then 10k middle-position remove + append operations.

```text
P426 old full-payload index:
unique-100k                 build 1199.109 ms   churn10k 431.408 ms
1024 classes / 100k rows    build 1143.129 ms   churn10k 416.803 ms

P428 shared-payload index:
unique-100k                 build  524.350 ms   churn10k  97.649 ms
1024 classes / 100k rows    build  284.426 ms   churn10k  86.803 ms
```

The structural memory improvement from P427 therefore does not trade memory for update cost; avoiding repeated canonical-key clones materially reduces persistent map work in this fixture.

## Remaining evidence boundary

TopK output rows are unchanged semantically, but the current certified-output fast path is required only when an upstream operator already exposes a sealed full-row Γ token. Filters and AntiJoin now preserve that token. Group and Join produce new rows and therefore require new result evidence; Group's grouping key alone is not a valid full-row output certificate. No forgeable shortcut is allowed.
