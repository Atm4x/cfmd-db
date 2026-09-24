# Project Status

## Current phase

The CFMD kernel historical backlog is closed for the declared supported scope. The project is transitioning from architecture/R&D convergence to productization.

### Kernel state

- 25 Rust workspace crates.
- historical problem ledger: 22 / 22 closed.
- Lean proof artifacts retained for #18 and #20.
- offline vendored crypto/dependency closure retained.
- supported durability profile has destructive VM certification evidence.

### What is intentionally not finished

The repository does **not** yet expose a stable end-user Rust API. Internal `kernel-*` crates should be treated as unstable implementation details. Documentation still contains a large provenance archive because it records the R&D lineage and falsification evidence.

## Next engineering phase

1. Define a single stable public Rust facade crate.
2. Freeze error/resource/transaction lifecycle semantics at that facade.
3. Design ergonomic schema, query, rewrite and transaction APIs without leaking internal NodeId/plan/revision ownership types.
4. Add bulk/batch APIs so high-throughput use does not devolve into per-row abstraction overhead.
5. Add public examples and end-to-end application tests.
6. Establish release packaging, semver policy, API compatibility checks, binary-size tracking and benchmark baselines.
7. Only after the Rust facade stabilizes, add language bindings such as Python/PyO3.

## Support-scope policy

Formal and empirical assurance claims are scoped. A new storage platform profile, new semantic surface constructor, new lowering vocabulary or durability publication protocol must extend the corresponding evidence/proof boundary rather than inheriting a broader claim implicitly.
