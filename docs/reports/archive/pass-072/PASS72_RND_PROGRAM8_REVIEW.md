# Pass72 hostile review — R&D Program8 Quotient Hypergraph Engine

Date: 2026-09-21
Base reviewed: verified Pass71 reconstruction checkpoint
R&D input: `CFMD_RND_PROGRAM8_CLOSEOUT_PASS68_2026-09-21.zip`
Decision: **ACCEPTED AFTER SEMANTIC REBASE + HOSTILE FIX**

## Package integrity

All files in the Program8 package matched its supplied `SHA256SUMS.txt`. The package declared Program8 incremental on Program6 and a one-file production delta in `crates/kernel-plan/src/lib.rs`.

On Pass71, the incremental patch did not apply mechanically: 27/33 hunks applied with `--fuzz=0`; 6 hunks overlapped later Pass69–71 changes. The production integration therefore used semantic rebase rather than fuzzy patching.

## Claims reproduced

The following Program8 decisive hostiles pass on the Pass72 integration:

- `quotient_key_support_counts_leaves_not_duplicate_rows`;
- `quotient_hypergraph_gyo_accepts_chain_and_rejects_cycle`;
- `nine_way_acyclic_quotient_join_preserves_bag_order_and_multiplicity`;
- `nine_way_bushy_quotient_join_preserves_logical_bag_order`;
- `nine_way_raw_cycle_collapses_to_certified_common_quotient_without_semantic_drift`.

The full `kernel-plan` suite passes with 173 passed / 4 ignored.

## Rebase invariants preserved

The merge preserves later-mainline semantics absent from the Program8 R&D base:

1. Pass69/70 canonical-key binding and exact structural-definition closure remain the compatibility authority for long-lived Γ-derived caches.
2. Pass70 compiled Γ remains a reconstructible executable derivative, not semantic authority.
3. Pass71 durable physical recipes/reopen behavior remains green, including structural semantic-index recovery and actual persisted-index consumption after reopen.
4. Program8 does not persist quotient-support row handles as durable authority; QCN support remains reconstructible physical state.

## Hostile finding fixed during integration

Program8 adds three retained structures to materialized quotient support:

- per-leaf `live_rows_by_key`;
- static `key_leaf_support`;
- dynamic `live_key_leaf_support`.

The R&D patch did not extend Pass63 retained-byte estimation to account for those maps. In production this would cause global physical-memory budgets to undercount Γ-QCN support after Program8.

Pass72 extends `quotient_support_estimated_retained_bytes()` to account for key/value storage in all three new support-counter maps, including canonical-key heap bytes. This keeps Program8 consistent with the existing multi-family memory-budget contract.

## Claim boundary

Program8 is accepted for the certified quotient-hypergraph branch only. It does not establish:

- arbitrary cyclic join planning;
- general hypertree-width / worst-case-optimal planning;
- universal runtime speedup;
- global late materialization for unrelated join implementations.

For >8 leaves, QCN admission remains gated by complete quotient coverage and successful GYO reduction. Otherwise the existing fallback remains authoritative.
