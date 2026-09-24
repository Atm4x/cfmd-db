# IMPLEMENTATION REPORT — Pass48

## problem

Pass47's first safe leaf permutation was semantically correct but physically awkward: reordered tuples carried provenance and a final global sort reconstructed reference Bag order. Generalizing that mechanism would scale provenance copying and `O(output log output)` restoration rather than addressing the underlying execution-order problem.

## hypotheses

1. For the current primitive equality fragment, pinned-Γ canonical keys can represent exact pair compatibility without becoming semantic authority.
2. Non-contiguous selectivity can be exploited as semijoin/compatibility pruning while final enumeration remains in original leaf order.
3. A bounded subset DP can decide whether such pruning is worthwhile without requiring the executor to materialize the chosen bushy tree.
4. Exact predicate revalidation at completed tuples keeps the compatibility structure derivative even if physical state is stale/corrupt.

## implementation

- retired production `ProvenanceJoinRow`, `ProvenanceJoinFragment` and final restoration sort;
- added canonical-key predicate compatibility buckets represented as row-ordinal bit masks;
- added semijoin support masks that eliminate rows with no possible counterpart before enumeration;
- added original-order DFS enumeration with mask intersection against already chosen leaves and forward viability checks;
- completed tuples are revalidated under pinned Γ before output;
- generalized subset selectivity search to 3–8 leaves;
- existing persisted singleton-side access wins when unified `JoinAccessDecision` says it is preferable;
- renamed physical observability from restoration count to `multiway_join_order_preserving_enumerations`.

## hostile falsification

- the initial Pass48 stable-handle provenance + final-sort route was rejected before packaging after hostile review identified post-hoc order repair as the wrong physical boundary;
- old provenance/restoration symbols are absent from production source;
- three-way non-contiguous and four-way subset fixtures exactly match logical reference Bag order and multiplicity;
- duplicate rows remain distinct physical ordinal choices;
- canonical compatibility is accepted only for resolved primitive pinned-Γ equality modules; unsupported semantics fall back;
- final tuples receive exact Γ equality revalidation;
- no new lint suppression or assertion-hidden side effect was introduced;
- release diagnostic retained the previous adversarial optimization (~148.7x on that fixture) without final sorting.

## verification/result

Final Rust 1.98.1 fmt/check/debug/release/strict-Clippy/release-build/strict-rustdoc/overflow-release gate: PASS.

Metrics: 349 declared tests, 121 `kernel-plan` tests (120 normal + 1 ignored diagnostic benchmark), 21 crates, 47,692 Rust LOC, 0 external Cargo sources, 0 `unsafe`.

## rejected routes

- no permanent `ProvenanceJoinRow` result carrier;
- no global post-join order-restoration sort;
- no claim that subset DP must dictate physical tuple materialization order;
- no host `Eq/Hash/Ord` substitution for Γ equality;
- no immediate unbounded subset search: current cap is 8 leaves until planner/mask memory policy is stronger;
- no WCOJ primitive added to logical semantics; future WCOJ/bitset/sparse implementations remain physical refinements.

## recommended next step

Unify compatibility construction with the physical access/lifecycle layer: choose dense bit masks, sparse ordinal lists, persisted semantic/I64 indexes or future WCOJ-style structures from one subset-node cost interface; add richer correlation/selectivity statistics before relaxing the current 8-leaf search cap.

## remaining risks

The broad multiway item remains OPEN. Current order-preserving compatibility execution is limited to the builtin primitive canonical equality fragment and rebuilds execution-local masks. General indexed/typed subset nodes, structural/custom equivalence, adaptive search-space control, memory accounting and richer statistics remain future work.
