# IMPLEMENTATION REPORT — Pass39

## problem

Pass38 completed the single-head durable control plane, leaving the historical maintained-query performance backlog. Agent-2 had already proved exact primitive canonical-key laws in an isolated prototype, but production generic Join/Group/TopK still lacked a shared Γ-bound semantic index layer.

## hypotheses

1. Canonical equality/order encoders must be resolved from pinned semantic modules, never chosen ad hoc by the planner.
2. A reusable index must be generic over derivative identity so storage `StableRowHandle` does not leak into internal query semantics.
3. Current builtin primitive equivalences/orderings can be indexed exactly; structural/custom semantics should remain on the known-correct fallback until an exact canonical law exists.
4. Join, Group and TopK can share the same key/index foundation while retaining their existing I64 specializations.

## implementation

### `kernel-semantics`

Added `CanonicalEqKey`, `CanonicalOrderKey`, `ResolvedPrimitiveOrdering`, `SemanticRegistry::resolve_primitive_ordering`, and canonical-key encoding on resolved primitive equality/ordering modules.

F64 equality remains bitwise. F64 ordering key matches `total_cmp`, including signed zero, infinities, subnormals and NaN sign/payload ordering.

### `kernel-semantic-index`

Added a new zero-dependency crate with:

- `SemanticModuleBinding`;
- `SemanticIndexBinding` pinned to semantic revision, module digests and `KEY_ENCODING_REVISION`;
- generic `SemanticBucketIndex<Key, Identity>` with ordered buckets and a reverse identity map.

The identity parameter is deliberately not `StableRowHandle`.

### `kernel-query` Join

Added local monotone `IndexedRowId` and indexed side state. Current primitive non-I64 equality modules now use canonical semantic buckets for removal verification and opposite-side delta probing. I64 keeps its specialized path. Structural/custom equality stays on `GenericScan`.

### `kernel-query` Group

Added composite `Vec<CanonicalEqKey>` lookup for all-primitive grouping keys. Existing I64 lookup remains. Structural/custom group equality retains semantic scan fallback.

### `kernel-query` TopK

Added `IndexedOrderedRows` using `CanonicalOrderKey -> tie bucket`. Current non-I64 builtin ordering modules no longer use a generic sorted-row mutation path. Output traverses ordered buckets and includes the entire threshold tie bucket.

### release-only repair discovered during benchmark

The first benchmark run exposed that some new `insert/remove` side effects were embedded in `debug_assert!`, so release optimized the mutations away. Side effects were moved into unconditional statements and assertions now inspect already-produced results. A release regression covers remove+insert replacement, and a static grep audit confirms the new paths have no mutating operations hidden in debug assertions.

## hostile falsification

Verified:

- equality-key iff equivalence for every current builtin primitive equality contract;
- ordering-key comparator equivalence for every current builtin ordering contract;
- 100,000 deterministic F64 bit-pair order checks;
- Text ASCII-CI and Unicode-boundary hostile cases;
- entity namespace separation;
- duplicate buckets/rebinding/invalidation;
- Join replacement/removal semantics;
- composite primitive Group;
- hostile F64 TopK;
- structural Join fallback rather than unsound canonicalization;
- debug and release behavior match after the release-only repair.

## benchmark

`semantic_index_join_bench` compares the integrated TextAsciiCaseInsensitive Join update to a deliberately minimal full key scan.

Median across three release runs:

```text
right rows   indexed ns   minimal scan ns   scan/indexed
1,000        1,631        374               0.23x
10,000       1,439        4,620             3.21x
50,000       1,438        18,800            13.07x
```

The indexed path is effectively flat across 1k to 50k in this rerun (1,631 ns -> 1,438 ns; timing noise dominates), while the minimal scan scales ~50.27x. This closes the asymptotic scan gap but exposes a planner-threshold optimization for small relations.

## verification

Rust 1.98.1:

```text
fmt                         PASS
check --workspace           PASS
debug all-target tests      PASS
strict Clippy               PASS
release all-target tests    PASS
release build               PASS
strict rustdoc              PASS
overflow-checked release    PASS
```

308 declared tests; 69 kernel-query; 30 kernel-semantics; 3 kernel-semantic-index; 21 crates; 39,584 Rust LOC; zero external Cargo sources; zero `unsafe`.

## result

Pass39 closes four actual production gaps: exact primitive canonical/index infrastructure, primitive generic Join full-scan maintenance, primitive/composite Group lookup, and generic Text/F64 maintained TopK ordering. It does not claim persisted storage semantic indexes, structural/custom canonical keys, cost-based index selection, typed-batch Group/TopK output, multiway join planning, or the existing I64 constant-factor gaps.
