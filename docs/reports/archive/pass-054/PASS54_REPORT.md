# CFMD Pass54 Report — Structural Γ Canonical Group Production

**Status:** VERIFIED on Rust 1.98.1.

**Production source window:** 2026-09-20 20:15:03 → 20:22:39 +03:00 (**7m36s**). Production source was frozen after that point; final verification found no correctness defect, so the freeze was not removed.

## 1. Scope

Pass54 takes exactly one post-Pass53 consumer: maintained `Group` in `kernel-query`.

Pass53 proved compositional structural Γ-canonical keys, but `MaterializedGroupDeltaState` still admitted indexed lookup only when every group key part had a `ResolvedPrimitiveEquivalence`. A structural or mixed structural+primitive key therefore fell through to exact recursive semantic scans despite Γ already supplying an exact canonical quotient representation.

Only `crates/kernel-query/src/lib.rs` changed in production. Structural Join/index and durable recursive-key encoding are intentionally left for separate passes.

## 2. Canonical Group admission

`MaterializedGroupDeltaState` now distinguishes three physical paths:

```text
single I64 exact key                  -> specialized i64_lookup
all-primitive canonical key parts     -> pre-resolved primitive encoders + semantic_lookup
structural/mixed canonical key parts  -> Γ canonical_equivalence_key + semantic_lookup
unsupported future/custom key law     -> exact semantic fallback
```

The state retains the existing primitive fast encoders. For structural/mixed keys it uses the Pass53 certified `SemanticRegistry::canonical_equivalence_key` against the exact pinned `SemanticContext` stored by the materialization.

No host `Eq/Hash/Ord` substitutes for Γ equality.

## 3. Delta-side semantic-scan removal

Pass54 does not stop at build-time group lookup. When canonical lookup is admitted, maintained-delta key equality also compares canonical Γ keys rather than recursively invoking `rows_semantically_equal` for every affected-key comparison.

Therefore the structural consumer no longer pays a hidden semantic-scan tax during local replay after the initial materialization.

The exact semantic fallback remains reachable only when no certified canonical group key law is available.

## 4. Hostile consumer test

Pass54 adds a mixed composite maintained Group fixture:

```text
[ Set<TextAsciiCaseInsensitive>, I64Exact ]
```

The hostile input uses:

- structurally equal sets with different physical element order;
- different ASCII case representatives;
- a primitive I64 coordinate that must remain independently discriminating;
- group birth/death across a model replacement.

The test verifies:

- `semantic_lookup` is active;
- primitive-only `group_encoders` are absent, proving the structural path is actually exercised;
- canonical structural admission is active;
- equivalent structural representatives collapse to the same group;
- the maintained delta is semantically equivalent to full recomputation;
- final group cardinality matches the oracle.

The complete `kernel-query` suite and full workspace suite remain green.

## 5. Authority and durability boundaries

Pass54 does **not** change:

- logical `Group` semantics;
- Γ equality laws;
- `KEY_ENCODING_REVISION`;
- durable semantic-index format;
- structural Join execution;
- Γ-QCN structural factors;
- custom/plugin canonicalization.

Structural Group keys are an in-memory reconstructible physical derivative. The representative row retained by each group remains ordinary semantic payload; canonical keys are lookup evidence, not semantic authority.

## 6. Verification

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

The first combined release invocation and the first overflow-check invocation hit external compilation timeouts. Neither was counted as PASS/FAIL; the unfinished commands were rerun on warmed targets to actual PASS.

Workspace metrics after Pass54:

- 363 declared Rust tests;
- 130 `kernel-plan` tests;
- 70 `kernel-query` tests;
- 21 crates;
- 50,062 Rust LOC under `crates/`;
- 19 pre-existing `#[allow(...)]` sites, no new suppression;
- 0 `unsafe` in `crates/`;
- 0 external Cargo registry/git sources.

## 7. Problem ledger

### CLOSED exactly in Pass54 — 2

1. ✅ Maintained Group admitted canonical lookup only for all-primitive keys, forcing structural/mixed Γ group keys onto exact semantic group scans despite Pass53's certified structural canonical law.
2. ✅ Even after a canonical group lookup existed, generic delta-side key matching could still invoke recursive semantic equality repeatedly. Canonical-admitted Group replay now compares exact Γ canonical keys throughout the maintained path.

### Historical OPEN from Pass53 fully closed this pass

**0 / 22.** Historical item #1 is materially advanced: structural maintained Group now consumes the certified law. It still includes custom/plugin canonicalization, persisted structural index encoding, structural Join/index execution and related typed production, so the broad item remains OPEN.

### Historical / active OPEN after Pass54 — 22

The count remains **22**. Notably:

- structural maintained Group is now canonical-keyed;
- structural maintained Join remains on exact `GenericScan` fallback;
- persisted recursive structural-key encoding/version migration remains OPEN;
- Γ-QCN structural/custom factors remain OPEN;
- Pass52 insertion/resurrection activation remains OPEN;
- all prior lifecycle/memory/durability/distribution/formal items remain unchanged.

### New OPEN created in Pass54

**0.** This pass consumes an already-proved structural key law and introduces no new subsystem.

## 8. Result

Pass54 turns the Pass53 structural canonical law into actual Group production rather than leaving it as a semantic utility. The physical compilation chain is now real for another operator family:

```text
pinned Γ structural equality
        -> compositional CanonicalEqKey
        -> maintained composite Group lookup/delta matching
        -> exact logical Group semantics unchanged
```

The next isolated consumer should be maintained structural Join / planner access selection. That is deliberately not bundled into Pass54 so Join costing, persisted/transient family choice and durable-key boundaries can be hostile-reviewed independently.
