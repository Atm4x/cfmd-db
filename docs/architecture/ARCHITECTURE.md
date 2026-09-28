# CFMD Architecture

## Product/runtime layering

```text
Python public facade
        │
        │ EntitySet / Query / Plan / Candidate / Watch
        ▼
Python binding bridge
        │
        │ compact typed IR + runtime operations
        ▼
Rust facade / authoritative runtime service
        │
        ├── query compile / revision contexts / exact watch
        ├── Plan / Candidate / history / diagnostics
        └── local tooling protocol
        ▼
internal kernel crates
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

The repository intentionally keeps internal concerns split into small crates. This crate graph is an implementation architecture, **not** the public application API. The Pass280 global kernel hostile/refactor campaign is frozen; future kernel changes are evidence-driven.

## Public/runtime architectural laws

1. **One authoritative runtime owns writes.** An external Studio/tooling process submits Plans to the same runtime; it does not independently mutate database files.
2. **Current, historical and Candidate worlds share query semantics.** The same logical query may be evaluated against current state, `db.at(revision)` and `db.preview(plan)`.
3. **Exact watch is language-neutral.** Python async iteration, .NET adapters and Studio subscriptions sit over a revision-tagged runtime protocol rather than separate callback systems.
4. **Bindings construct compact typed IR.** Python should not replicate planner/kernel logic or expose Rust crate topology one-to-one.
5. **Materialized objects do not perform hidden I/O.** Relationship traversal that may touch storage is symbolic in query construction or explicit through references/runtime calls.

## Major crate groups

### Logical and semantic model

- `kernel-types` — nominal/revision identifiers and shared primitive types.
- `kernel-exact` — exact coefficient arithmetic for finite measures/signed deltas.
- `kernel-schema` — type vocabulary, definitions, subtype closure and schema context.
- `kernel-model` — finite structural values, carriers, relation/COW storage and normalized state.
- `kernel-semantics` — pinned semantic-context/module execution.
- `kernel-identity`, `kernel-lifecycle`, `kernel-retention` — identity/lifecycle/history policy.

### Query/change/write system

- `kernel-query` — logical relational expressions, preparation and maintained query state.
- `kernel-plan` — physical plans, checked lowering and physical execution strategies.
- `kernel-change` — typed changes/rewrites.
- `kernel-lens` — writable/dependent view calculus.
- `kernel-aggregate`, `kernel-fixpoint`, `kernel-grounded-closure` — maintained higher-order calculi.
- `kernel-validation`, `kernel-violation` — validation and violation-query boundaries.

### Physical semantic indexing

- `kernel-semantic-index` — Γ-bound exact semantic index structures/lifecycle.
- `kernel-persistent` — persistent ordered/radix containers shared by maintained kernels.
- `storage-memory` — in-memory physical storage implementation and revision graph support.

### Revision, durability and distribution

- `kernel-revision` — revision transition/publication contracts.
- `kernel-durability` — immutable generations, WAL, recovery, platform assurance/certification.
- `kernel-transport` — typed/semantic transport authority.
- `kernel-integration` — sealed cross-layer runtime integration boundary.

### Trust/proof/deployment

- `kernel-auth` — cryptographic identity/authentication/trust-root lifecycle.
- `kernel-deployment` — authenticated semantic-package/runtime-profile deployment contracts.
- `kernel-proof` — checked proof/certificate fragments used by runtime layers.

## Kernel/runtime invariants

1. A published runtime revision owns one coherent logical revision, physical store and maintained state.
2. Pinned `Γ` defines semantic equality/order; physical indexes bind to the same semantic revision.
3. Prepared/compiled immutable metadata is reused rather than silently reconstructed during transitions.
4. Recoverable validation/planning/auth/durability failures precede authoritative publication.
5. Detached Candidates use explicit versioned ownership and cannot mutate authoritative state before commit.
6. Durable authority is recovered only through the accepted generation/WAL protocol.
7. Platform durability certification is scoped to exact evidence/fingerprints.
8. Authentication proves package/evidence authority; it does not substitute for semantic refinement.
9. Specialized physical implementations and universal correctness implementations must erase to the same logical semantics; error-driven fallback is not an authority model.

## Formal boundary

`formal/lean/CFMD/Publication.lean` and `SurfaceKernel.lean` mechanize selected architectural obligations. Python refinement binders tie theorem artifacts to concrete Rust source vocabulary. CI runs the formal gate when proof-relevant files change.

## Historical architecture document

The previous append-only pass-oriented architecture narrative is retained at [`ARCHITECTURE_LEGACY_CURRENT.md`](ARCHITECTURE_LEGACY_CURRENT.md) for provenance. It is not the primary current architecture entry point.
