# CFMD Implementation Report — Pass69

## Problem

Long-lived canonical-key artifacts depended on exact Γ semantics but lacked one stable recursive persisted key grammar and one explicit compatibility/rebuild law. Different physical families carried partially overlapping metadata, and generic maintained semantic indexes still treated structural equivalence as a special gap.

## Hypothesis

Make key bytes and cache compatibility explicit derived contracts of pinned Γ: compile a structural equivalence binding to semantic revision + exact module dependency closure + key-format revision, encode canonical keys with a versioned stable grammar, and force every cache consumer to rebuild on any mismatch.

## Implementation

`kernel-semantics` now owns canonical-key codec v1 and bounded fail-closed decode. The grammar recursively encodes scalar and structural key constructors with fixed tags/lengths and canonical collection ordering. A golden fixture pins v1 bytes.

`kernel-semantic-index` binds maintained semantic indexes to the shared key contract and exposes explicit compatibility/rebuild causes. Structural equivalences use the same authoritative `SemanticRegistry::canonical_equivalence_key` semantics as existing structural consumers.

`kernel-plan` replaces ad-hoc compatibility metadata for semantic indexes, Γ-QCN factors, Γ-QCN support and semantic statistics with the shared binding. Structural maintained Filter is covered end-to-end before/after exact relation delta maintenance.

## Falsification

The implementation was attacked for malformed lengths, unknown versions, non-canonical ordering, case-normalization ambiguity, trailing bytes and stack exhaustion through recursive nesting. All fail closed. Runtime structural index consumption was checked against the existing semantic evaluator rather than host `Eq`/`Ord`.

## Result

Historical canonical-key/cache encoding-version migration debt is closed. Durable storage of physical index payloads is not implemented by this pass and remains part of structural persistence/recovery work.
