# PASS61 REPORT — structural Γ-QCN factor generalization

Status: **VERIFIED**

The Pass61 turn was interrupted before reporting/packaging. The exact source-start timestamp returned by the interrupted UI call is not recoverable from the surviving transcript, so this report does not invent a duration. The final production source byte is timestamped **2026-09-20 23:06:14 +03:00**. All work after that point was verification, documentation and packaging only.

## Problem

Pass53 established exact recursive/compositional `CanonicalEqKey` laws for structural Γ-equivalences, Pass54/56 consumed them in maintained Group/Join, and Pass59 closed local Γ-QCN change-shapes. However the Γ-QCN factor family itself still reused `MaterializedSemanticIndexState`, whose admission boundary was the generic semantic-index family and whose quotient-key build path explicitly required a primitive equivalence module.

That left an architectural mismatch: Γ could canonically quotient structural values, while multiway Γ-QCN factors still treated structural equivalence as non-materializable/fallback-only.

## Hypothesis

Γ-QCN factors should be a dedicated reconstructible derivative, not an alias of the generic semantic-index family. Its key law is narrower and cleaner:

```text
(relation row, quotient equivalence) -> exact Γ CanonicalEqKey
```

If that law is available through `SemanticRegistry::canonical_equivalence_key`, the factor can support both primitive and structural equivalences while preserving exact fallback for future equivalences that do not provide canonical keys.

## Implementation

### 1. Dedicated quotient-factor state

`PhysicalStore.semantic_quotient_factors` now stores `MaterializedSemanticQuotientFactorState` rather than `MaterializedSemanticIndexState`.

The dedicated state owns:

- its `SemanticIndexBinding` quotient coordinate;
- the exact pinned `SemanticContext`;
- `CanonicalEqKey -> Vec<PhysicalRowId>` buckets;
- reverse `PhysicalRowId -> CanonicalEqKey` mapping.

It is explicitly reconstructible and remains separate from the generic semantic-index family/advisor.

### 2. Structural Γ canonicalization

Factor build and fallback key-cache construction now use:

```text
SemanticRegistry::canonical_equivalence_key(context, equivalence, value)
```

Primitive equivalences remain admitted. Structural equivalences are admitted when present in the schema and their recursive domain is valid. A future/custom equality with no canonical representation still does not become a fake physical quotient.

### 3. Atomic delta maintenance

The dedicated factor validates removals against the stored reverse key, canonicalizes inserted rows under the exact pinned Γ, and mutates buckets/reverse state only through the existing candidate/COW transition boundary.

Context transitions require rebuild if the stored `SemanticContext` no longer matches.

### 4. Γ-QCN execution reuse

`quotient_key_cache` can now reuse maintained factors for structural coordinates. Surviving rows are addressed by stable `PhysicalRowId`, so factor reuse composes with Pass52–59 local support transport.

## Falsification

A hostile structural fixture uses three bag relations over:

```text
Option<TextAsciiCaseInsensitive>
```

with a structural `Option { inner: TextAsciiCaseInsensitive }` equivalence.

The fixture verifies:

- three structural quotient factors materialize successfully;
- `Alpha/alpha/ALPHA` and `Beta/beta/BETA` form the expected Γ classes;
- direct Γ-QCN execution consumes all six maintained structural factor keys rather than payload-canonicalizing them again;
- normal prepared execution remains equal to the logical evaluator;
- mixed `remove(ALPHA) + insert(Gamma)` updates the structural factor exactly;
- after the delta, direct Γ-QCN execution still reports six maintained quotient-key hits and matches the logical result.

During recovery from the interrupted turn, the first version of this test asserted factor-hit counters through the optimizer-selected execution path. On a 2×2×2 fixture the optimizer correctly chose the cheaper contiguous join, producing zero Γ-QCN factor hits. That was a **test design error, not a production failure**. The hostile check was corrected to exercise the Γ-QCN execution path directly while retaining prepared execution as the semantic oracle.

## Result

### CLOSED exactly in Pass61 — 2

1. **Γ-QCN materialized factors are no longer primitive-only.** Structural Γ equivalences with exact canonical keys now have first-class maintained quotient factors.
2. **Structural Γ-QCN factors are delta-maintained under the same candidate transition boundary.** They no longer require payload re-canonicalization/full factor rebuild after ordinary relation deltas.

### Historical OPEN accounting

Pass60 historical OPEN count: **22**.

- Historical OPEN fully closed this pass: **0 / 22**.
- Historical OPEN remaining: **22**.
- Genuinely new OPEN created: **0**.

Advanced but still OPEN:

- structural/custom-equivalence physical production is substantially narrower, but persisted recursive structural-key encoding and arbitrary/plugin canonical-law packaging remain open;
- Γ-QCN factor/support lifecycle/advisor and exact memory budgeting remain open;
- richer/unbounded multiway planning beyond the current verified bounded 3–8-leaf Γ-QCN remains open;
- persistent artifact-map metadata, remaining layouts/OrderedView, durability/distribution/authentication/formal-proof fronts are unchanged.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

Final frozen bytes PASS:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets --release`
- `cargo build --workspace --release`
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release`

Cold release/overflow invocations that hit the external compilation timeout were not counted; warmed retries completed successfully.

Static snapshot before packaging:

- **388 declared tests**;
- **138 `kernel-plan` tests**;
- **21 crates**;
- **53,085 Rust LOC**;
- **0 `unsafe` hits**;
- **19 existing `#[allow(...)]`**, no new suppression;
- **0 TODO/FIXME/todo!/unimplemented! hits**;
- **0 `GenericScan` hits**;
- **0 external registry/git Cargo sources**.

## Next frontier

The structural Γ law is now consumed by Group, maintained Join, relation multiset quotienting, and Γ-QCN factors. The remaining structural physical gap is no longer ordinary in-memory execution; it is primarily long-lived/durable canonical-key encoding/version migration plus advisor/lifecycle/budget policy. A useful next implementation pass can therefore attack persisted structural-key format discipline or measured physical-family lifecycle rather than adding another ad-hoc structural consumer.
