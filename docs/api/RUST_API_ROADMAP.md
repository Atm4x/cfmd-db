# Rust Runtime / Facade Roadmap

## Role after Pass280

Rust remains the implementation/runtime language and needs a stable boundary above the internal `kernel-*` crates. It is **not** the reason to delay the Python-first product surface until a large standalone Rust application API is perfected.

The intended layering is:

```text
Python public facade
        │
        │ compact typed query/rewrite/runtime IR
        ▼
Python binding bridge
        │
        ▼
Rust facade / runtime service
        │
        ├── query compile / execution
        ├── revision contexts
        ├── Plan / Candidate
        ├── exact watch protocol
        ├── history / inverse
        └── tooling transport
        ▼
internal CFMD kernel crates
```

## Required Rust boundary

The Rust facade/runtime layer must provide stable ownership/lifecycle contracts for bindings without exposing internal crate topology:

- database open/create/close and authoritative runtime ownership;
- schema introspection and typed identifier handles;
- compact query IR submission/compilation/execution;
- revision/historical contexts;
- Plan/rewrite submission;
- Candidate creation/query/delta/validation/rebase/commit/discard;
- exact watch register/resume/cancel and revision-tagged delta delivery;
- history access/inverse construction;
- diagnostics/explainability;
- local tooling transport entry points;
- stable error/resource-limit taxonomy.

## Boundary rules

- applications/bindings do not import internal `kernel-*` types as public contracts;
- no public NodeId/maintained-plan/COW ownership leakage;
- bulk operations cross the facade in batches rather than per-row FFI calls;
- internal kernel refactors may continue behind the facade without changing application semantics;
- Python/.NET/Rust clients must share one semantic runtime protocol rather than separate implementations of watch/Candidate/history.

## Rust application API

A direct Rust application facade remains desirable after the runtime boundary stabilizes. It can then expose idiomatic typed builders over the same IR/protocol used by Python instead of creating a competing semantic surface.

## Acceptance gate

The boundary is ready when Python bindings can implement the product roadmap without calling internal kernel crates directly, without per-row abstraction overhead, and without reproducing query/watch/Candidate logic in Python.
