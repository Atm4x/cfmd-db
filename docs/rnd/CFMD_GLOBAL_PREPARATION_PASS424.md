# CFMD PASS424 R&D — GLOBAL `RelExpr` PREPARATION HOSTILE

## Scope

PASS424 attacked the remaining whole-row staging in `prepare_general_relation_factorized` after PASS423 closed logical-root retention:

```text
factorized source columns
    -> complete source row model
    -> RelExpr execution
    -> complete target rows
    -> target physical columns
```

The pass deliberately tested whether the already-exact maintained Γ-DTC engine could serve as one universal streaming executor rather than adding operator-specific fallback paths.

## Rejected prototype

The prototype built `MaterializedRelPlanState` over an empty model, streamed factorized source rows in bounded batches as exact relation insert-deltas, and applied exact output deltas to a transient physical target sink. The sink used `RelationBaseWitness` and the same dense physical Scan order law as P420, including retracting output deltas from `Difference`-class execution. No SQL/source-A fallback was needed and semantic tests passed.

The architecture was nevertheless rejected on performance grounds.

100k-row Bag `Union` (50k + 50k), release:

```text
PASS423 accepted current preparation:
    311.676 ms

P424 maintained-Γ-DTC streaming prototype:
    3199.062 ms
    26 source batches
    max batch = 4096 rows

same prototype with target sink temporarily disabled:
    1907.967 ms
```

So the full prototype was about 10.3x the accepted preparation baseline, and about 6.1x remained even after removing the target sink. The dominant payer is therefore the maintained execution substrate itself, not output columnization.

## Hostile conclusion

`MaterializedRelPlanState` is the wrong owner for one-shot migration preparation. Its Scan state intentionally owns persistent rows plus canonical position authority so Query/Watch can accept future deltas. Reconstructing that long-lived incremental authority from factorized physical input is useful for a maintained query, but is unnecessary work for one-shot physical preparation.

The rejected law was effectively:

```text
one-shot migration preparation
    -> rebuild long-lived maintained Scan authority
    -> stream deltas through it
```

That duplicates a capability the preparation does not need.

## Selected next architecture

The next implementation should introduce one storage-neutral **one-shot relational execution source/sink law**, separate from maintained-query state:

```text
Prepared RelExpr
    + RelExecutionSource
        - relation type / semantic identity
        - bounded row/range visitor over physical/factorized storage
        - existing row-aligned Γ evidence when available
    -> one-shot operator DAG
        - operators own only algebraically necessary state
        - no persistent Scan row/position index merely for future mutation
    -> RelExecutionSink
        - physical target columns/segments
        - exact result occurrence evidence
```

Required complexity law:

```text
working memory = O(batch)
               + O(inherent blocking/operator state)
               + O(output physical authority)

not

O(all source rows)
+ O(maintained incremental Scan authority)
+ O(target rows)
+ O(target columns)
```

`Join`, `Group`, `Distinct`, `Difference`, `AntiJoin`, and `TopKWithTies` may inherently require indexes/support state. That state is legitimate only when required by the relational operator; a persistent future-update index at every Scan is not.

Per-operator lowerings are implementations of this one execution law, not fallback routing. Unsupported lowerings stay fail-closed until their exact state/certificate law exists. No path may silently call the old row-materializing evaluator as a compatibility fallback.

## Exact-evidence rule

The one-shot executor should consume already-owned `RelationScanOccurrenceSeed` / witness evidence when an operator can transport it exactly, following P408–P415 certificate work. It must not recanonicalize unchanged source rows merely to create a second authority.

The final sink should emit or carry the exact target occurrence evidence in physical output order so `RelationBaseWitness` can be rebound without a second Γ pass, following the P420 physical-seed law.

## Explicit rejection

Do not adopt any of these as the production answer:

- maintained-query bootstrapping from empty state for one-shot preparation;
- fixed 4096-row batching as an architectural performance remedy;
- generic row-model fallback when a one-shot lowering is absent;
- rebuilding `CanonicalRowPositionIndex` merely because maintained Query owns one;
- source-schema/current-world routing.

The PASS424 prototype source changes were fully reverted after measurement. Only this R&D decision and ledger updates are retained.
