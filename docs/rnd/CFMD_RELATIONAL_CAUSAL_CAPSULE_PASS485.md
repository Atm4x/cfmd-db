# CFMD R&D — persistent relational causal capsule (PASS485)

## Result

PASS484 proved that `query + observed OFC + source relations` is not exact causal authority: hidden Γ-DTC state distinguishes equal output fibers. The production answer is not a second serialized query history and not query replay. The existing `MaterializedRelPlanState` is already the correct state carrier because its arena/operator payloads are persistent/COW and its compiled ExecGraph owns the exact differential state requirements.

PASS485 therefore introduces `RelCausalCapsule` as an immutable `Arc<MaterializedRelPlanState>` boundary and adds two exact operations to the maintained plan:

```text
impact_relation_deltas(state_r, Δ)
    -> Unaffected | Changed

candidate_from_relation_deltas_for_revision(state_r, r', Δ)
    -> state_r'
```

The first plans Γ-DTC propagation against the hidden maintained state without committing or replaying the query/model. The second path-copies only affected persistent nodes and deliberately depends only on semantic `RelationDelta`, not storage handles.

## Reopen reconstruction law

For an exact durable forward effect

```text
r --Δ--> r+1
```

and a capsule state at `r+1`, the predecessor capsule is reconstructed by the exact inverse semantic delta:

```text
rewind(state_(r+1), Δ)
    = advance(state_(r+1), swap(inserted, removed), r)
```

This is derived/reconstructible authority. No maintained-state bytes are added to durable history. Reopen may rebuild a current capsule root once and walk the existing exact causal effect chain backwards to the revisions that actually need retained observation authority. Schema-changing/opaque boundaries remain fail-closed until their native capsule transport law exists.

## Sharing / scaling law

A capsule clone is one `Arc` clone. `MaterializedRelPlanState` itself stores a persistent arena of `Arc` nodes and persistent operator state. Therefore:

```text
N observations of the same capsule state
    -> 1 capsule payload + N compact refs

unrelated revision churn
    -> same capsule Arc, zero state copy

relevant exact delta
    -> path-copy touched Γ-DTC nodes only
```

Source routing is exposed as compact `relation -> source occurrence count` metadata from the compiled transition program. A repeated source in a self-join is represented as one relation route with two compiled occurrences; no query replay/routing fallback is needed.

## Hostile exclusions

Rejected:

- serializing one `MaterializedRelPlanState` per committed observation;
- full query/model replay during each retroactive braid proof;
- static source-relation/OFC-key authority;
- physical row handles as durable causal meaning;
- a second relational query engine or history store.

## Remaining integration boundary

The next pass must bind durable relational observation identity to shared capsule identity and add runtime relation/source-occurrence indexes. The durable record should carry only compact semantic reconstruction authority (query/capsule identity + observation identity), while the exact maintained state stays reconstructible. Reopen should instantiate each unique capsule lineage once and share it across all observations that reference it.
