# CFMD Project Specification

This is the concise repository-facing specification. The full normative model and append-only implementation record remain in [`docs/spec/CFMD_CORE_SPEC.md`](docs/spec/CFMD_CORE_SPEC.md). Historical pass artifacts are provenance, not the primary current-status interface.

## 1. State model

A logical CFMD revision is a coherent revisioned state, conventionally:

```text
Revision = (Schema S, SemanticEnvironment Γ, finite Model M)
```

- `S` defines structural/nominal types, relations, constraints and semantic dependencies;
- `Γ` pins meaning-bearing equality/order/canonicalization and certified semantic modules;
- `M` is the finite typed model for the revision.

Runtime publication additionally owns physical roots, maintained materializations and durable authority for that same revision. Published state must not mix components from different revisions.

## 2. Type and surface model

The logical universe is structural plus nominal identity. Core structural constructors include products, sums, option, set, bag, sequence, map and guarded recursive `μ` types. Entities/references remain nominal where identity semantics require it.

Object/document/relational forms are surface views over one typed kernel, not independent database models.

## 3. Semantic environment Γ

Meaning is revisioned data. Operations depending on semantic equality, ordering or canonicalization are evaluated against pinned `Γ`. Kernel code must not substitute incidental Rust `Eq`/`Hash`/`Ord` when a certified semantic operation is authoritative.

Authentication/deployment of semantic modules and semantic correctness are separate obligations.

## 4. Query and change calculus

Queries are exact typed expressions over the logical model. Physical plans may specialize aggressively, but checked lowering must erase to the same logical query.

Every logical type has change semantics. Incremental execution, maintained views, exact watch and writable/rewrite paths operate through typed changes rather than ad-hoc page mutation semantics.

## 5. Transactions, Plans and Candidates

Writes are typed rewrites over revisioned state. Recoverable errors occur before authoritative publication.

At the product surface, advanced writes are exposed as **Plans** before commit. `preview(plan)` creates a queryable **Candidate** future world. Candidate validation, query delta, explanation, freshness/rebase and commit must reuse the same kernel/revision semantics rather than form a parallel transaction model.

## 6. Exact watch

Exact watch observes the **result of a logical query**, not merely “a table changed”. A commit publishes one coherent revision-tagged delta batch. If an operator lacks a certified exact derivative, exact watch fails explicitly; optional full recomputation must be an explicit caller choice.

The watch protocol is language-neutral so Python, GUI adapters, .NET and Studio can share it.

## 7. Physical/runtime model

The runtime uses prepared logical/physical metadata, exact semantic indexes, persistent/COW roots where appropriate, and maintained delta execution for supported operators.

Internal `kernel-*` representations remain implementation details. Public bindings consume a compact stable runtime/facade boundary, not crate ownership/layout types.

## 8. Durability and recovery

Durability uses immutable generations, WAL/prerequisite ordering, explicit sync/publication steps, authenticated durable evidence and fail-closed recovery rules.

Publication/GC obligations are mechanically checked in Lean for the declared boundary. Real durability claims remain platform-profile-scoped.

Current certified profile remains the repository support-matrix profile; this does not imply blanket bare-metal/filesystem certification.

## 9. Distribution, trust and deployment

Replication/transport authority, trust-root/key lifecycle, signed evidence and freshness/anti-rollback are distinct from logical query semantics. Large external artifacts are authenticated metadata-first and read through bounded contracts where applicable.

## 10. Formal boundary

Lean artifacts mechanize selected publication and surface-to-kernel obligations. Source-refinement binders fail closed when proof-relevant Rust vocabulary/protocol drifts from those artifacts. They do not claim arbitrary machine-code verification.

## 11. Kernel status

The global hostile/refactor campaign is **COMPLETE / FROZEN after Pass280** for the current declared scope.

Every kernel crate has received a dedicated or grouped hostile audit proportional to its size, and the historically heavy `query/plan/semantics/durability` line was revalidated against the final workspace. `FROZEN` is evidence-driven: reopen on a counterexample, proof/authority seam, measured complexity regression, new mathematical requirement or public API/DX requirement—not for cleanup by inertia.

Current inventory: [`docs/status/KERNEL_HOSTILE_LEDGER.md`](docs/status/KERNEL_HOSTILE_LEDGER.md).

## 12. Product boundary

The active product direction is **Python-first embedded/local CFMD**, backed by a Rust facade/runtime service and language-neutral protocol.

Primary surface goals:

- explicit database-bound entity sets (`db.users`);
- familiar lazy `match/where/select/order/aggregate` operations;
- deep symbolic relationship traversal without manual join plumbing;
- no hidden I/O on materialized Python objects;
- Plan → Candidate → commit workflows;
- revision/history access and semantic inverse/undo preview;
- exact async query-result watch;
- one authoritative runtime shared by the application and local Studio/tooling;
- inspectable dependencies/capabilities/explainability.

Detailed design: [`docs/api/CFMD_PYTHON_FACADE_THEORY.md`](docs/api/CFMD_PYTHON_FACADE_THEORY.md). Implementation sequence: [`docs/api/PRODUCT_ROADMAP.md`](docs/api/PRODUCT_ROADMAP.md).
