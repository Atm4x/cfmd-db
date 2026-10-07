# PASS569 R&D — semantic capability lattice for cyclic planning

## Problem

`SemanticStatistics`, `SemanticQuotientFactor`, and SAMF/`ObservableAtom` historically became separate physical artifact families even though they expose overlapping facts about the same semantic key partition. Production cyclic planning still asked specifically for retained statistics, while quotient execution already accepted either a dedicated quotient factor or SAMF. That artifact-name coupling blocks safe retirement and encourages family routing.

## Selected law

For one `SemanticIndexBinding`, the semantic object is the finite partition induced by the pinned Γ equivalence product. Physical retention is a capability projection of that object:

```text
ExactCardinality  <=  QuotientFiber  <=  ObservableFiber
```

where `<=` means "the stronger retained representation can answer every observation required by the weaker capability". In particular:

- SAMF/`ObservableAtom` supplies exact cardinality, row-to-quotient key/fiber, and full observable fiber;
- the quotient projection supplies row-to-quotient keys/fibers and therefore exact row/distinct-key cardinality;
- the cardinality-only projection supplies only exact row/distinct-key cardinality.

The semantic consumer must request a capability, never a concrete artifact family. Physical policy may choose the cheapest representation that satisfies the demanded capability and workload/resource constraints. This is implementation specialization of one law, not semantic fallback.

## PASS569 implementation

- `PhysicalStore::semantic_cardinality(...)` is the exact-cardinality owner used by join/cyclic costing.
- Resolution order is strongest available compatible representation: SAMF -> quotient projection -> cardinality-only projection. Every branch is checked against the same pinned semantic context and authoritative row count.
- `MaterializedSemanticQuotientFactorState` now exposes an exact statistics snapshot from its maintained reverse map + key buckets; no second statistics artifact is required merely because quotient state already exists.
- cyclic optimizer/admission and join-access costing no longer name or require `SemanticStatistics` as a semantic concept.
- capability metadata now records that `ObservableAtom` supplies `{ObservableFiber, QuotientFiber, ExactCardinality}` and a quotient projection supplies `{QuotientFiber, ExactCardinality}`.

## Why the physical families are not all deleted in PASS569

The existing representations have materially different retained-memory shapes. The cardinality-only state stores key multiplicities; the quotient projection additionally stores row->key and key->row mappings; full SAMF retains catalog/class/fiber/projection structures. Replacing every weaker projection by full SAMF without a resource proof could increase retained memory and maintenance work. That would violate the hostile rule "one universal principle, specialized physical implementation when justified".

Therefore PASS569 removes **consumer semantic duplication** first. Remaining artifact-family names are implementation profiles/advisor/durability debt, not alternate semantic authorities. The next retirement step must unify their identity/admission under one `SemanticFiber` representation/profile vocabulary and benchmark the profile cost frontier before deleting the compact profiles.

## Status

- CLOSED: cyclic optimizer/admission dependence on concrete `SemanticStatistics`.
- CLOSED: exact-cardinality semantic duplication between statistics and quotient/SAMF consumers.
- CLOSED: capability lattice declaration in runtime/durable capability metadata.
- OPEN: replace `UnifiedArtifactId::{SemanticStatistics, SemanticQuotientFactor, ObservableAtom}` with one semantic-fiber artifact identity + representation/profile.
- OPEN: move advisor telemetry/admission to capability demand/profile choice instead of family-specific advisors.
- OPEN: once profile unification is complete, delete family-specific durable recipe variants and install APIs.
- REJECTED: blindly replace compact cardinality/quotient projections with full SAMF merely to remove enum variants; this can regress retained memory/maintenance and is not required by the semantic law.
