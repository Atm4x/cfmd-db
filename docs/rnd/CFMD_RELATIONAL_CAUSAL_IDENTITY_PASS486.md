# CFMD R&D — relational causal identity and reopen interning (PASS486)

The rejected design was a durable `capsule_id`. Such an id would make a runtime memory strategy part of durable semantics and force lifecycle/compatibility machinery around a reconstructible object.

The selected authority is:

```text
DurableRelationalCausalObservation {
    observation_id,
    observed_revision,   // Γ-DTC state frontier
    query: RelExpr,
}
```

Together with the committed effect id this uniquely identifies the causal observation. `RelCausalCapsule` remains derived state.

For reopen, observations are partitioned by canonical structural `RelExpr` identity. For each unique query, construct one maintained state at the epoch boundary and perform one exact backwards sweep through durable relation effects. Requested observation frontiers take shared clones from that persistent lineage. If an effect is irrelevant to the query, PASS485 structural sharing keeps the same payload. Runtime then interns by `(capsule frontier, query identity)` and relation indexes store only compact `(effect_id, observation_id)` refs.

For a proposed retroactive relation delta P, collect observation refs from touched source relations, deduplicate the refs, resolve each shared capsule once, and evaluate `Impact(C, P)` through compiled Γ-DTC propagation. `Changed` creates coordination with the observing committed effect; `Unaffected` permits that anti-dependency edge. No query replay, full-state diff, source-envelope guess or old-schema interpretation is involved.

The remaining product gap is capture/orchestration rather than authority: ordinary query execution must emit these observations automatically and the canonical history normalizer must invoke this certificate in its late-effect braid path.
