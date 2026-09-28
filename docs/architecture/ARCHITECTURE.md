# CFMD Architecture

## Product layering after Pass281

```text
Rust applications         Python / .NET / Studio
       |                         |
       +-----------+-------------+
                   |
              cfmd-runtime
                   |
       stable product/runtime IR
                   |
       +-----------+------------+
       |                        |
 query/change/revision      durable/runtime
       |                        |
       +-----------+------------+
                   |
             kernel-* graph
```

The Pass280 kernel graph is frozen for the declared scope. Pass281 introduces `cfmd-runtime` as an anti-corruption layer: external consumers see facade-owned IDs, values, query/plan/runtime handles and errors, not kernel ownership types.

## Authority rules

1. **One Rust runtime owns product semantics.** Language bindings project this runtime; they do not reproduce query/write/Candidate/watch/history logic.
2. **Kernel types do not leak.** `SemanticId`, `RelExpr`, runtime bundle/cell types, physical layouts and durability protocol types are internal.
3. **Prepared authority stays in Rust.** Immutable query compilation is retained as `PreparedQuery` instead of being reconstructed per FFI invocation.
4. **Revision worlds are explicit.** Current snapshots, historical contexts and future Candidates are distinct handles.
5. **Writes cross one authoritative transition path.** Product Plans map to the validated durable kernel transition protocol.
6. **Exact watch remains language-neutral.** Async Rust/Python/.NET/Studio adapters will share one revision-tagged subscription protocol.
7. **Bulk boundaries are mandatory.** FFI and tooling transports batch rows/deltas; no one-call-per-cell design.

## Current public/product owner

`crates/cfmd-runtime` currently provides the complete Rust-first product foundation: create/open, immutable current and historical `ReadContext` worlds, typed object/relation query surfaces, `Plan -> Candidate -> commit`, durable history/undo/rebase, and exact maintained `watch()` with provider-neutral wake delivery, cancellation and causal catch-up status. Internal kernel ownership/layout remains hidden.

Hosted protocol/authentication/authorization/transports and language/UI adapters are next-stage compositions above this runtime; they do not redefine database semantics.

## Internal architecture

Internal crate responsibilities and dependency edges are documented in [`CRATE_MAP.md`](CRATE_MAP.md). Kernel hostile/freeze state is documented separately in [`../status/KERNEL_HOSTILE_LEDGER.md`](../status/KERNEL_HOSTILE_LEDGER.md).

## Formal boundary

`formal/lean/CFMD/Publication.lean`, `Notification.lean` and `SurfaceKernel.lean` mechanize selected architectural obligations. Source-refinement gates bind theorem artifacts to Rust source vocabulary. Notification proofs now explicitly separate publication authority from duplicate/spurious wake, cancellation and runtime-shutdown liveness signals.

## Hosted composition after Pass303

```text
kernel-* -> cfmd-runtime -> cfmd-protocol -> cfmd-host -> transport providers
```

`cfmd-host` owns server/session composition but no concrete I/O. Authentication maps provider evidence to a principal; authorization maps the principal to grants; only then does the host construct the restricted runtime/session/wire stack. Local IPC and hosted network servers are future adapters of this boundary, not dependencies of the kernel/runtime.

## Hosted lifecycle after Pass304

`kernel-* -> cfmd-runtime(shared session authority) -> cfmd-protocol -> cfmd-host -> transport providers`. Security providers may refresh or revoke authority and issue expiry deadlines; transports provide channel-binding evidence and scheduling, but cannot mint database grants or semantic conflict certificates. Graceful drain is host lifecycle, not database mutation.

## Durability backend boundary after Pass310

`kernel-durability` now separates logical durability authority from physical layout ownership:

```text
DurableRevisionStore
  prepare / commit / recovery / retry / causal authority
                  |
                  v
          DurabilityBackend
           /             \
  DirectoryBackend   SingleFileBackend
```

The backend owns physical roots/locks/container state and declares physical capabilities; it does not decide whether a transaction commits or what Revision is authoritative. Replication frame persistence is delegated to the backend while the replication state machine remains common. New layouts must implement this boundary rather than add optional layout fields to `DurableRevisionStore`.
