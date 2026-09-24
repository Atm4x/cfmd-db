# IMPLEMENTATION REPORT — Pass61

Pass61 generalizes Γ-QCN factor materialization from primitive equivalences to the exact structural canonical laws introduced in Pass53, without merging quotient factors into the generic semantic-index authority.

## 1. Dedicated `MaterializedSemanticQuotientFactorState`

`PhysicalStore.semantic_quotient_factors` now stores a dedicated state with exact binding/context, canonical-key buckets and a stable-handle reverse map. This removes the previous accidental representation alias with `MaterializedSemanticIndexState`.

The factor family remains an explicit reconstructible physical derivative; it does not become a persisted semantic index and is not enrolled in the generic index advisor.

## 2. Canonicalization boundary

Build, insertion and fallback Γ-QCN key construction use `SemanticRegistry::canonical_equivalence_key`. Primitive modules continue to work. Schema-declared structural equivalences are admitted recursively through Γ. Equivalences without an exact canonical key are still rejected from quotient materialization rather than approximated.

## 3. Delta/context discipline

`validate_relation_derived_delta` and `apply_relation_derived_delta` now call quotient-factor-specific validation/application with the pinned `SemanticContext` and registry.

Removal checks the exact stored reverse key for the `PhysicalRowId`; insertion canonicalizes before publishing the candidate mutation. Context mismatch requires rebuild. Existing COW candidate publication keeps failed transitions from mutating the live root.

## 4. Structural Γ-QCN execution

`quotient_key_cache` recognizes schema structural equivalences and reuses maintained quotient-factor keys when binding/context/row count match. Without a maintained factor it still computes the exact canonical key from the row value; future unsupported custom equivalence remains outside this path.

## 5. Hostile fixture

The new test uses three `Bag<Option<Text>>` relations under structural `Option<TextAsciiCaseInsensitive>` equality. It checks materialization, canonical class behavior, direct factor reuse before and after a mixed relation delta, and equality with the logical evaluator.

The initial hostile assertion used optimizer-selected prepared execution to infer factor use. Because the tiny fixture makes contiguous execution cheaper, that produced a false negative. The final test directly invokes the Γ-QCN executor for the physical-use assertion and independently keeps prepared execution as the logical result oracle.

## 6. Verification

Final frozen bytes passed fmt, workspace check, full debug tests, strict Clippy, full release tests, release build, strict rustdoc and overflow-check release tests under Rust 1.98.1. Cold release/overflow compilation timeouts were not counted; warmed retries completed.
