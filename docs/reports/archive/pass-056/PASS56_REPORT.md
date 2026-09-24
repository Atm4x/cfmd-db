# CFMD Pass56 Report — Structural Maintained Join Without GenericScan

**Status:** VERIFIED on Rust 1.98.1.

**Production source window:** 2026-09-20 20:49:33 → 20:56:43 +03:00 (**7m10s**). Production source remained frozen during final verification and packaging.

## 1. Objective

Pass56 removes the remaining maintained-Join `GenericScan` storage family for the currently admitted semantic universe. Pass53 established exact compositional structural Γ canonical keys; Pass54 and Pass55 proved that those keys can safely lower Group and relation-multiset operations. Maintained Join still ignored that result and retained a structural scan fallback.

## 2. Structural canonical Join storage

`MaterializedJoinDeltaState` now has three physical maintained families:

```text
I64 specialized
Primitive SemanticIndexed
Structural StructuralIndexed
```

The old `GenericScan { left, right }` variant is deleted.

`StructuralIndexedJoinSide` stores:

- stable maintained `IndexedRowId -> Row` bindings;
- `IndexedRowId -> CanonicalEqKey` reverse keys;
- `CanonicalEqKey -> Vec<IndexedRowId>` buckets preserving insertion order;
- monotone next identity.

Structural keys are derived only by `SemanticRegistry::canonical_equivalence_key(context, equivalence, value)` under the pinned Γ. No host `Eq`/`Hash`/`Ord` is semantic authority.

## 3. Exact output semantics

Join output remains left-major/right-minor for the maintained state:

- left rows are traversed in maintained identity insertion order;
- matching right bucket identities are traversed in insertion order;
- canonical-key equality is exact for the admitted structural law established in Pass53;
- Bag multiplicity is preserved.

No final sort, representative rewriting or persisted key format was introduced.

## 4. Delta maintenance

Structural Join deltas no longer scan the opposite relation semantically.

For each changed row:

1. derive its structural Γ canonical key;
2. probe only the corresponding opposite-side bucket;
3. emit joined changed rows in deterministic bucket order;
4. prevalidate row removals/set uniqueness using canonical bucket narrowing plus full row semantic equality where identity-level validation is required;
5. commit both maintained sides only after both mutation plans succeed.

A hostile structural Product fixture with `TextAsciiCaseInsensitive` proves initial structural equivalence is indexed, then applies independent left/right insertions and compares the maintained Join delta against full recomputation under Γ.

## 5. Fallback removal boundary

Workspace search after freeze finds **zero `GenericScan` symbols** in `crates/`.

For the current semantic universe this is sound because `resolve_primitive_equivalence` returns an encoder for primitive equivalences and returns `None` exactly for schema structural equivalences, which are now handled by the structural canonical path.

Future arbitrary/plugin semantic equality remains OPEN. If such a law can provide exact `equivalent` without a certified canonical representation, it must enter through an explicit future physical-family/admission boundary rather than silently resurrecting the deleted current fallback.

## 6. Durability boundary

Pass56 does not persist structural recursive keys and does not change `KEY_ENCODING_REVISION`. Structural maintained Join state is reconstructible in-memory physical evidence tied to the exact `SemanticContext` already stored in `MaterializedJoinDeltaState`.

## 7. Verification

Final Rust 1.98.1 gate on frozen source:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test ... --release PASS
```

Compilation-timeout invocations were not counted; release/overflow commands were rerun to actual completion.

Workspace metrics after Pass56:

- 365 declared Rust tests;
- 72 `kernel-query` tests;
- 130 `kernel-plan` tests;
- 21 crates;
- 50,534 Rust LOC under `crates/`;
- 19 pre-existing `#[allow(...)]` sites, no new suppression;
- 0 `unsafe` in `crates/`;
- 0 `GenericScan` symbols in `crates/`.

## 8. Problem ledger

### CLOSED exactly in Pass56 — 2

1. ✅ Maintained structural Join still stored both sides as full `RelationValue` and evaluated output through exact quadratic/general semantic scanning despite Pass53 certified structural canonical keys.
2. ✅ Maintained structural Join delta maintenance still scanned opposite-side rows for every changed row; it now probes exact Γ-canonical buckets and maintains those buckets atomically.

### Historical OPEN from Pass55 fully closed this pass

**0 / 22.** Historical structural/custom physical-production remains broader: persisted structural indexes, structural Γ-QCN factors, custom/plugin canonical laws and durable recursive-key encoding remain open.

### Historical / active OPEN after Pass56

**22.** Count unchanged.

### New OPEN created in Pass56

**0.** Future plugin/custom semantics without certified canonical keys were already part of the existing custom-semantic frontier.

## 9. Result

Pass56 removes the last current maintained-Join generic semantic scan family:

```text
Γ equality law
    -> exact canonical quotient key
    -> maintained structural bucket index
    -> exact Join output + exact Join delta
```

The next structural frontier is no longer maintained Join fallback. It is persisted structural physical indexes / structural Γ-QCN factors / durable recursive-key encoding, plus the independent Pass52 insertion-activation Dq and broader lifecycle/COW work.
