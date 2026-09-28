# Project Status

## Current phase

The CFMD **global kernel hostile/refactor campaign is COMPLETE / FROZEN after Pass280** for the current declared scope.

This is an engineering freeze, not a claim that undiscovered bugs are impossible. A frozen kernel is reopened only when there is concrete evidence: a correctness counterexample, proof/authority seam, measured asymptotic/performance regression, new mathematical requirement, or a public API/DX requirement that cannot be satisfied cleanly above the kernel.

### Kernel state

- **27** Rust workspace crates under `crates/`;
- historical problem ledger: **22 / 22 closed** for the declared scope;
- historically heavy `kernel-query`, `kernel-plan`, `kernel-semantics` and `kernel-durability` were revalidated during the final global closeout;
- dedicated/grouped hostile audits cover the remaining kernel crates; current inventory is in [`KERNEL_HOSTILE_LEDGER.md`](KERNEL_HOSTILE_LEDGER.md);
- Lean proof artifacts remain active for the declared publication/surface boundaries;
- vendored crypto/dependency closure is retained for offline CI;
- the declared durability profile retains destructive VM certification evidence.

## Active engineering phase: productization

The product target is now **Python-first embedded/local CFMD**, backed by a compact language-neutral runtime protocol and a Rust facade/runtime layer. The internal `kernel-*` crates remain implementation details rather than the application API.

Primary product laws:

1. familiar lazy `where/select/match` vocabulary rather than SQL-shaped manual join plumbing;
2. deep symbolic relationship traversal inside query construction;
3. no hidden I/O on ordinary materialized Python objects;
4. exact query-result watch where the kernel can certify an incremental derivative;
5. explicit current / historical / speculative Candidate worlds;
6. writes represented as inspectable Plans before commit for advanced flows;
7. history inverse/undo goes through the same Plan → Candidate → validation pipeline;
8. one authoritative runtime owns writes; external Studio/tooling attaches through a local protocol;
9. unsupported exact-watch/preview capabilities fail explicitly rather than silently degrading to polling or weaker semantics.

The source design is [`../api/CFMD_PYTHON_FACADE_THEORY.md`](../api/CFMD_PYTHON_FACADE_THEORY.md). The implementation sequence is [`../api/PRODUCT_ROADMAP.md`](../api/PRODUCT_ROADMAP.md).

## Immediate roadmap

1. Establish the stable Rust runtime/facade boundary needed by bindings, without exposing internal crate topology.
2. Produce installable Python wheels and a typed Python schema/query surface.
3. Implement deep path query IR and stable `get/require/match/where/select/order/limit/aggregate` execution.
4. Expose Plan + Candidate preview and revision/history access.
5. Deliver a certified exact-watch subset and revision-tagged async delta protocol.
6. Add local tooling transport and CFMD Studio on the same authoritative runtime.
7. Extend exact-watch coverage, undo/conflict explanation, adapters and production packaging incrementally.

## What remains intentionally unfinished

- no stable end-user Python package/wheels yet;
- no stable public Rust facade crate yet;
- no complete language-neutral tooling protocol yet;
- no CFMD Studio yet;
- exact-watch coverage must be explicitly defined per operator/capability;
- production platform support remains narrower than logical/kernel capability;
- API compatibility/semver, binary-size and public benchmark baselines are not yet release-frozen.

## Support-scope policy

Formal and empirical assurance claims are scoped. A new storage profile, semantic surface constructor, lowering vocabulary, publication protocol or facade capability must extend its corresponding evidence/proof boundary instead of inheriting a broader claim implicitly.
