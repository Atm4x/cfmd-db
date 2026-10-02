# PASS427 R&D — sealed Γ evidence and canonical-position ownership

## Problem

P426 one-shot Set Union/Project/Distinct/Difference necessarily computed the full Γ-canonical output row key to implement Set quotient/subtraction semantics. The physical target then called `RelationBaseWitness::build_columnar`, canonicalizing the same output rows again. Passing a public `CanonicalRowKey` into the witness would remove work but create a forgeable second Γ authority.

`CanonicalRowPositionIndex` had a separate ownership problem: the ordered class map owned a full canonical row key, while every physical row position also owned another full key payload.

## Selected law

A certified canonical key is evidence, not data supplied by storage code.

```text
RelationRowCanonicalizer(result type, semantic context, compiled Γ modules)
        |
        +-- certify_row(row) -> CertifiedCanonicalRowKey
                                  [opaque authority + key]

same certified key:
  - decides quotient/subtraction membership
  - is emitted with the accepted row
  - is sealed into RelationOccurrenceCertificate
  - is adopted by RelationBaseWitness
```

Tokens from different compiled authority instances cannot be combined, even if their visible key values happen to compare equal. This prevents accidental/raw-key injection from becoming semantic authority.

For maintained physical position indexing:

```text
Γ class map: Arc<CanonicalRowKey> -> positions
by_position: Arc<CanonicalRowKey>
```

Each Γ-class canonical payload is allocated once; positions share it by pointer. Borrowed ordered-map lookup preserves lookup by `&CanonicalRowKey` without cloning a key merely to search an `Arc` key map.

## Rejected alternatives

- trusted/public raw canonical-key constructor;
- re-canonicalize at witness boundary "for safety";
- position -> class lookup by linear scan;
- temporary full canonical-key clone for every ordered-map lookup;
- LRU/eviction of duplicated key payloads.

## Remaining proof/work

- carry sealed evidence through unchanged-row Filter/AntiJoin/nested paths;
- Group/TopK one-shot exact state/evidence laws;
- release memory/perf hostile for high-cardinality Set and repeated-class Bag position indexes;
- only then durable PhysicalAtoms / RealizationRoot.
