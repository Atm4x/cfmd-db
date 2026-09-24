# CFMD Lean proof artifacts

This directory contains offline Lean 4.34.0 mechanizations for historical
formal-assurance problems.  It intentionally depends only on Lean core; no
Mathlib/package registry/network access is required.

Run all formal/refinement gates:

```bash
LEAN_BIN=/path/to/lean-4.34.0/bin/lean ./check_all.sh
```

## Historical #18 — immutable publication / fsync / GC

`CFMD/Publication.lean` proves the ten P18 obligations recorded in
`PASS110_FRONTIER_18_16.md`: publication crash projections, rename uncertainty,
prerequisite durability, immutable generations, GC safety and the exact
production-fault-point mapping.  Its filesystem assumptions remain explicitly
platform-dependent; validating those assumptions on supported real systems is
historical #13.

`check_refinement.py` binds the theorem artifact to the production
`kernel-durability::store` publication protocol and streaming-checkpoint cuts.

## Historical #20 — surface-to-kernel preservation

`CFMD/SurfaceKernel.lean` mechanizes the normative surface vocabulary and the
current relational lowering contract.  It proves:

1. the complete normative surface-feature table maps bijectively to trusted
   kernel witnesses;
2. surface type elaboration preserves constructor structure and the guarded-μ /
   free-variable admission obligation;
3. the complete current relational surface vocabulary lowers without hidden
   logical expansion;
4. exact physical-plan erasure implies semantic preservation for *any* logical
   evaluator;
5. therefore the two predicates enforced by production `LoweringChecker`
   (`to_logical_expr == logical` and exact logical node count) are sufficient
   for surface semantic preservation independent of physical algorithm choice.

`check_surface_refinement.py` is a fail-closed source-binding gate.  It verifies
that the normative table in `docs/spec/CFMD_CORE_SPEC.md`, production `TypeExpr`, the
full current `RelExpr`/`Plan` variant sets, guarded recursion validation,
`LoweringChecker`, and the capability/rewrite/violation/retention witnesses
still match the mechanized model.  Adding a new surface/query/type constructor
without extending the proof artifact makes this gate fail.


The repository pins this toolchain in `/lean-toolchain`, and `.github/workflows/lean.yml` runs this gate only for proof-relevant changes (or manual dispatch).
