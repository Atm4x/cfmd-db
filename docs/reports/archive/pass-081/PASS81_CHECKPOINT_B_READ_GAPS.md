# PASS81 CHECKPOINT B — READ GAPS

Status: **DEBUG VERIFIED**.
Baseline: Pass81 Checkpoint A (`54e76a985239d3f47d00ec64f8e77e064639690b7baa00c499a3e9e8cc636a6d`).

## Closed at this checkpoint

1. **Finite nonrecursive non-monotone exact reads**
   - first-class Difference and AntiJoin in logical, physical, DTC, maintained-state, transport and durable metadata layers;
   - Bag Difference is Γ-class monus, Set Difference is Γ-support subtraction;
   - AntiJoin is right zero/nonzero blocker semantics with left multiplicity preservation.

2. **Structural semantic ordering**
   - compositional ordering for guarded μ, Product/Sum/Option/Seq/Set/Bag/Map;
   - semantic order classes are distinct from physical tie-break keys;
   - structural TopKWithTies consumes the semantic ordering;
   - checkpoint v2 durably persists structural ordering while v1 remains decodable.

## Hostile evidence

- ASCII-CI Set Difference removes `A` when right contains `a`.
- ASCII-CI Bag Difference: `A×3 - a×2 = A×1`.
- AntiJoin treats duplicate right blockers as one presence condition and preserves left duplicates when unblocked.
- explicit Sum rank defeats accidental SemanticId order.
- Product/Text ASCII-CI TopKWithTies preserves `a ~ A` tie.
- physical and logical evaluators agree on the new operators.

## Verification

- fmt PASS
- check workspace/all-targets PASS
- debug workspace/all-targets: **521 passed / 0 failed / 8 ignored**
- Clippy workspace/all-targets `-D warnings` PASS

## Deliberate carryover

- optimized SAMF blocker-mass/zero-cross materialization;
- recursive/stratified negation;
- SAMF Ordered overlay, range indexes and pagination;
- write-side FineChange/Rewrite + dependent Lens/complement is next.
