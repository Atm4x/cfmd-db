# CFMD Project Specification

This file is the concise, repository-facing specification. The full normative model remains in [`docs/spec/CFMD_CORE_SPEC.md`](docs/spec/CFMD_CORE_SPEC.md). Historical pass-by-pass corrections are archived under `docs/history/` and are not the primary entry point for new development.

## 1. State model

A logical CFMD revision is treated as a coherent revisioned state, conventionally written as `Revision = (S, Γ, M)`:

- `S` — structural/schema state;
- `Γ` — pinned semantic environment defining certified semantic equality, ordering, tokenization and related meaning-bearing operations;
- `M` — logical model/data state.

Runtime publication additionally owns the authoritative physical store and maintained materializations associated with that exact semantic revision. A published runtime revision must never mix components from different revisions.

## 2. Type and surface model

The logical type universe is structural and nominal. Core structural constructors include products, sums, option, set, bag, sequence, map and guarded recursive `μ` types. Entities/references remain nominal where identity semantics require it.

Object/document/relational forms are surface views over the same typed kernel rather than independent peer database models.

## 3. Semantic environment Γ

Meaning is revisioned data. Operations whose behavior depends on equality, ordering, canonicalization or other semantic modules must be evaluated against the pinned `Γ` for the revision. Kernel code must not substitute incidental Rust equality/hash/order for a certified semantic operation.

Semantic implementations are authenticated/deployed through the dedicated trust boundary. Authentication proves package identity/authority; it does not replace semantic refinement/correctness checks.

## 4. Query and change calculus

Queries are exact typed expressions over the logical model. Physical plans may vary, but checked lowering must erase exactly to the logical query and may not introduce hidden logical nodes.

Every logical type has a change semantics. Incremental execution, maintained views and writable/rewrite paths operate through typed changes rather than through ad-hoc page mutation semantics.

## 5. Transactions and rewrites

Writes are typed rewrites over revisioned state. Publication boundaries are designed so recoverable errors occur before authority changes; sealed/committed transitions are total across the authoritative publication step.

## 6. Physical/runtime model

The current runtime uses compiled/prepared logical and physical metadata, NodeId-addressed maintained state, exact semantic indexes, persistent/COW roots where applicable, and maintained delta execution for the supported relational operators.

Internal physical representations are implementation details. The upcoming user-facing Rust facade must not expose kernel ownership/layout types as stable API.

## 7. Durability and recovery

Durability uses immutable generations, WAL/prerequisite ordering, explicit directory/file synchronization, authenticated durable evidence and fail-closed recovery/publication rules.

The publication/GC model is mechanically checked in Lean (#18) and source-bound to the Rust implementation by refinement scripts. Real durability claims are profile-scoped: a filesystem/device/kernel/QEMU configuration is supported only when its platform fingerprint and destructive campaign evidence are certified.

Current certified profile: QEMU 8.2.2 TCG, Alpine Linux 3.24.2 / Linux 6.18.52-0-virt, dedicated raw virtio device, ext4 `data=ordered`, QEMU `cache=none,aio=threads`. This does not imply bare-metal NVMe/SATA certification.

## 8. Distribution, consensus and authenticity

The kernel contains replication/consensus authority machinery with authenticated evidence. Trust-root/key lifecycle, signed evidence and external freshness/anti-rollback boundaries are distinct from logical query semantics.

## 9. Proof boundary

Lean artifacts currently mechanize:

- immutable-generation publication/fsync/rename/GC obligations (#18);
- surface-to-kernel preservation and the checked-lowering boundary (#20).

The proof artifacts do not claim arbitrary Rust machine-code verification. Source-refinement binders fail closed if the production vocabulary/protocol drifts away from the mechanized model.

## 10. Current product boundary

The historical kernel problems are closed for the declared scope. The next engineering phase is the stable user-facing Rust API and release surface:

- one public facade crate (`cfmd` or equivalent);
- stable error taxonomy and resource lifecycle;
- ergonomic schema/query/transaction APIs;
- bulk/batch boundaries that avoid per-row abstraction overhead;
- packaging, documentation, examples, benchmarks and compatibility work.

The internal `kernel-*` crates remain unstable implementation modules until that facade is defined.
