# PASS402 R&D — current-B relation endpoint detachment

## Question

After a general migration `q : A -> B.R`, can writes to `B.R` remain exact without a source-row inverse/lens for `Union`, `Distinct`, `Group`, or Bag outputs?

## Result

Yes at the semantic level. The migration transform initializes the current semantic coordinate; it is not a permanent writable view law.

After cutover, `B.R` is an ordinary current-world relation. A write is therefore an exact current-B relation rewrite:

```text
current B relation value R
    + Γ-certified RelationDelta_B
    -> exact endpoint R'
```

There is no obligation to find an A-row whose mutation would reproduce `R'`. Physical realization may still depend on old atoms, but those atoms are representation ancestry, not current semantic write authority.

PASS402 wires this law directly to the existing intent-bearing `PreparedRelationRewrite`. `kernel-query` remains responsible for Γ-canonical Set/Bag semantics and exact-base validation; `kernel-realization` only materializes the certified endpoint.

## Physical reference lowering

The reference lowering is:

```text
PreparedRelationRewrite_B
    -> verify exact current B support
    -> evaluate exact B endpoint
    -> columnize endpoint into native B atoms
    -> replace only B.R realization rule
```

This immediately detaches `B.R` from any derived migration ancestry. Old target-realization atoms disappear from the current dependency frontier unless another current/historical root still needs them.

A stale prepared write is rejected when the current realized B support no longer matches the support against which the rewrite was prepared.

## Hostile performance result

The full-endpoint reference lowering is mathematically universal but is **not accepted as the final production write lowering**.

100k-row bag-Union, one-row insertion, three warm release runs:

```text
92.918 ms
98.899 ms
94.569 ms
```

The cost is O(|B.R|): the reference implementation reconstructs/verifies the current relation and publishes a complete new native column. That is too expensive for a one-row write.

This is not a reason to revive source-schema routing or add SQL update fallbacks. It identifies the next physical R&D target: a bounded B-native relation-delta overlay keyed by Γ relation classes/multiplicity, with exact compaction back into native columns.

## Consequence

The earlier P401 concern is refined:

- arbitrary general-query outputs indeed do not provide a universal **source A row identity**;
- but source identity is not required for current-B writes after semantic cutover;
- the remaining problem is only the efficient physical representation of exact B deltas.

The desired production law is therefore:

```text
PreparedRelationRewrite_B
    -> bounded immutable B-native delta overlay
    -> query/read lowering over base + overlay
    -> compaction/materialization when profitable
```

No operator-specific writable-view router and no inverse migration are needed.
