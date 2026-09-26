# CFMD Architecture

## Layering

```text
future public Rust facade
        │
        ▼
kernel-integration / revision publication
        │
        ├── logical model + schema + semantic Γ
        ├── query / plan / maintained execution
        ├── change / rewrite / lens / validation
        ├── semantic indexes / grounded closure / aggregates
        ├── durability / recovery / retention
        ├── transport / replication authority
        └── authentication / deployment / proof boundaries
                │
                ▼
        physical storage implementations
```

The repository intentionally keeps internal concerns split into small crates. This crate graph is an implementation architecture, not the eventual public API.

## Major crate groups

### Logical and semantic model

- `kernel-types` — stable nominal/revision identifiers and basic shared types.
- `kernel-exact` — shared exact `ℕ`/`ℤ` coefficient arithmetic for finite measures and signed deltas.
- `kernel-schema` — type expressions, schema/semantic symbols and versioned `Γ` structures.
- `kernel-model` — finite structural values, carriers, relations and model state.
- `kernel-semantics` — pinned semantic-context/module execution.
- `kernel-identity`, `kernel-lifecycle`, `kernel-retention` — identity/lifecycle/history policy.

### Query/change/write system

- `kernel-query` — logical relational expressions, preparation and maintained query state.
- `kernel-plan` — physical plans and checked lowering.
- `kernel-change` — typed changes/rewrites.
- `kernel-lens` — writable/dependent view calculus.
- `kernel-aggregate`, `kernel-fixpoint`, `kernel-grounded-closure` — higher-level maintained operators/calculi built over shared exact coefficients where multiplicity is unbounded.
- `kernel-validation`, `kernel-violation` — validation and violation-query boundaries.

### Physical semantic indexing

- `kernel-semantic-index` — Γ-bound exact semantic index structures and lifecycle.
- `storage-memory` — in-memory physical storage implementation.

### Revision, durability and distribution

- `kernel-revision` — revision transition/publication contracts.
- `kernel-durability` — immutable generations, WAL, recovery, platform assurance and certification.
- `kernel-transport` — authenticated replication transport/authority machinery.
- `kernel-integration` — cross-layer runtime integration boundary.

### Trust/proof/deployment

- `kernel-auth` — cryptographic identity/authentication primitives and trust-root lifecycle.
- `kernel-deployment` — authenticated semantic-package/runtime-profile deployment contracts.
- `kernel-proof` — checked proof/certificate fragments used by the runtime.

## Runtime invariants

1. A published runtime revision owns one coherent logical revision, physical store and maintained state.
2. The pinned semantic environment `Γ` defines semantic equality/order; physical indexes must bind to the same semantic revision.
3. Prepared/compiled logical metadata must be reused rather than silently reinterpreted during transitions.
4. Authoritative publication is fail-closed: recoverable validation/planning/durability errors precede commit/publication.
5. Detached candidates use explicit COW/versioned ownership and cannot mutate the authoritative runtime before sealing/publication.
6. Durable authority is recovered only from the immutable-generation/WAL protocol accepted by the recovery rules.
7. Platform durability certification is scoped to exact fingerprints/evidence, not inferred from filesystem names alone.
8. Authentication and semantic refinement are separate trust obligations.

## Formal boundary

`formal/lean/CFMD/Publication.lean` and `SurfaceKernel.lean` mechanize selected architectural obligations. Python refinement binders tie those theorem artifacts to concrete Rust vocabulary/source landmarks. CI runs the formal gate when proof-relevant files change.

## Historical architecture document

The previous append-only pass-oriented architecture narrative is retained at [`ARCHITECTURE_LEGACY_CURRENT.md`](ARCHITECTURE_LEGACY_CURRENT.md). It remains useful for provenance but is no longer the primary architecture entry point.
