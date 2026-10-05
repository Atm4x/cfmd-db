# CFMD PASS489 R&D — Shared Maintained Relational Causal DAG

Status: selected architecture / theorem sketch backed by executable hostile regressions.

## 1. Question

PASS488 made reopen output-sensitive: one exact-history sweep plus only relation-to-lineage influences. The remaining hostile case is a hot relation referenced by many distinct relational observation lineages. Current reconstruction still advances every affected `MaterializedRelPlanState` independently.

Question: can exact CFMD Γ-DTC semantics share maintained work across those lineages without query replay, a second history store, or a recovery-only fallback?

## 2. Result

Yes, but the correct object is **not** a cache of whole query capsules and it is not keyed by source relation.

The exact reusable object is a **canonical semantic RelExpr subtree state cell**. Query roots remain independent observation identities, while structurally identical maintained subtrees may be represented once and may fan out their exact differential output to multiple parent occurrences.

This does not erase the worst-case cost of arbitrary distinct semantics. It changes the payer from:

```text
all query roots × all relevant effects
```

toward:

```text
unique maintained DAG nodes × effects that influence those nodes
+ observation-root fanout/output work
```

and admits stronger compiler lowerings for parameter families where the node-local law supports them.

## 3. Structural Sub-DAG Sharing Theorem

Fix semantic context `C`, revision `r`, current source world `S`, and a relational subtree expression `E`.

Let `State(E,C,r,S)` be the exact maintained Γ-DTC state produced by the existing deterministic maintained semantics.

For any two occurrences of the **same canonical semantic subtree** `E` at the same `(C,r,S)`:

1. their maintained state is equal;
2. for every exact source delta `δ` on `E`'s dependency closure, their output differential is equal;
3. their successor maintained state is equal;
4. therefore one state transition may be evaluated once and its exact output delta fanned out to every parent occurrence.

Proof is structural induction over `RelExpr`.

- `Scan`: state is determined by the same relation world plus Γ canonicalization authority.
- stateless unary/binary nodes: output delta is a deterministic function of child delta(s), operator parameters and semantic modules.
- Set projection/Distinct/Set Union: support state is a deterministic function of the same subtree history.
- blocker/Difference/AntiJoin: blocker state is determined by equal child states and identical blocker parameters.
- Join: join indexes/fibers are determined by equal child states and identical join semantics.
- Group: group support/aggregate state is determined by equal child state and identical group/aggregate semantics.
- TopKWithTies: maintained ordered-cut state is determined by equal child state and identical ordering/k.

The existing Γ-DTC kernels already supply the deterministic local transition laws. PASS489 therefore does not need a new query algebra.

## 4. Hostile rejection — relation-keyed sharing is unsound

`(source relation set, revision)` is not a state identity.

Example over one relation:

```text
q1 = FilterEqConst(Scan(R), value = "alpha")
q2 = FilterEqConst(Scan(R), value = "beta")
```

At the same empty revision both have the same source relation envelope. Proposed insertion of `"ALPHA"` changes `q1` and leaves `q2` unaffected.

Therefore any cache/pool that merges lineages merely because they read the same hot relation is unsound. Executable regression:

`p489_source_relation_and_revision_do_not_define_a_shareable_causal_state_class`.

Conversely, independently built identical query semantics at the same world/revision remain exactly equal through a rewind. Executable regression:

`p489_identical_relational_semantics_form_one_shareable_maintained_state_class`.

## 5. Required representation split

Current `MaterializedRelPlanState` is a single-root local ExecGraph arena. A flat node simultaneously means:

```text
operator occurrence in this root graph
+
maintained state cell
```

That representation prevents cross-root computational sharing even when two occurrences have identical semantic subtrees.

Selected target representation:

```text
RelObservationForest
    roots: ObservationId -> NodeRef

CanonicalNodePool
    CanonicalSubtreeKey -> MaintainedStateCell

Occurrence/Fanout graph
    NodeRef -> parent occurrences / root observers
```

The distinction is essential:

- one state cell may have many parents;
- one query may contain the same semantic subtree in several occurrences (for example self-join scans);
- observation/root identity must never collapse merely because state cells are shared;
- source occurrence multiplicity remains execution topology, not duplicated source state.

## 6. Durable law

The forest/DAG is **derived runtime authority only**.

Durable representation remains the PASS486 law:

```text
committed effect
+ observation-local id
+ observed revision
+ exact RelExpr
```

No durable node IDs, DAG IDs, cache keys or maintained-state snapshots are introduced. Reopen recompiles canonical node identities and reconstructs the forest from existing durable exact effects.

Thus future compiler changes do not become on-disk compatibility obligations.

## 7. Complexity target

Let:

- `H` = exact effects in the reconstruction suffix;
- `N` = number of unique canonical maintained DAG nodes reachable from all observation roots;
- `J` = actual `(effect, unique-node)` influence count;
- `O` = observation/root routes emitted.

Target reconstruction complexity:

```text
O(H + N + J + O)
```

rather than root-lineage `O(H + Q + I + O)` when many roots share maintained substructure.

This is output-sensitive to **unique semantic maintained nodes**, not query count.

## 8. What structural sharing cannot remove

There is no theorem that arbitrary distinct query roots can all be updated in sublinear-in-distinct-semantics time.

If roots contain independent stateful semantic quotients, future continuations can distinguish those states independently. By the future-context quotient law from the zero-downtime R&D, an exact runtime must retain enough information to distinguish them. Structural interning therefore cannot merge non-equivalent stateful nodes.

So the worst case can still satisfy:

```text
N ≈ Q
J ≈ H × Q
```

and that cost is semantic rather than routing overhead.

PASS489 therefore rejects the stronger hypothesis "shared sub-DAG makes every fully-hot query family sub-HQ".

## 9. Parameter-family fusion is a separate exact lowering

Some distinct nodes have no independent hidden state and admit stronger batch execution.

Examples:

- many `FilterEqConst` nodes over the same input/column/equivalence can use one canonical-value dispatch index;
- many `FilterOrderConst` nodes over the same ordered input can use ordered-cut/range routing;
- identical projections or linear islands can share compiled transforms.

This is allowed only as a lowering of the same multi-root forest semantics, with equivalence against independent Γ-DTC execution. It must not become recovery-specific routing or a fallback path.

Stateful Join/Group/Blocker/TopK nodes may be fused only when a specific exact family theorem exists. Otherwise only canonical-identical subtrees share state.

## 10. Selected implementation sequence

PASS490 should implement the representation split at the smallest universal boundary:

1. derive canonical semantic subtree keys in `kernel-query`, independent of durability encoding;
2. separate maintained **state cell** identity from ExecGraph **occurrence/fanout** identity;
3. compile multiple relational observation roots into one derived `RelObservationForest`;
4. execute each unique influenced node once per transition and fan out its exact delta;
5. keep current single-root `MaterializedRelPlanState` as a lowering/client of the same node transition machinery during refactor, not as a fallback engine;
6. prove forest results/capsules equal independent per-root execution before switching reopen reconstruction;
7. then replace PASS488 per-lineage rewind with forest rewind and benchmark fully-hot shared-prefix workloads.

Only after the structural forest is exact should parameter-family fusion be attempted.

## 11. Rejected directions

- relation-envelope keyed capsule/state sharing;
- durable DAG/node IDs;
- serializing maintained sub-DAG snapshots;
- recovery-only query batching that has different semantics from live Γ-DTC execution;
- hashing physical node IDs from local ExecGraphs as semantic identities;
- merging non-identical stateful nodes because they happen to have equal current output;
- claiming `I = H×Q` is universally irreducible: shared structure can reduce it;
- claiming it is universally removable: independent stateful semantics preserve the worst case.

## 12. Conclusion

The exact next abstraction is not "more capsule interning". It is a **multi-root maintained relational DAG** with canonical semantic state cells and separate occurrence/root identity.

That is a universal Γ-DTC execution principle usable by causal reconstruction, watches and future multi-query execution; it is not an SQL-style cache and does not add a fallback path.
