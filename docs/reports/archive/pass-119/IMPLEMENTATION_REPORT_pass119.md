# Pass119 — Historical #20 surface-to-kernel mechanization closure

## Status

- Historical #20: **PROD CLOSED Pass119**.
- Historical ledger: **21 / 22 PROD CLOSED**.
- Proof/source freeze: `2026-09-23T18:32:34Z` (21:32:34 MSK).
- Production Rust/Cargo bytes: **unchanged from Pass118**.
- Production source fingerprint therefore remains: `12e09dcd9d9f1ea36a4d7f28b1079969e9e29e86b9f78d7b5575e2a450ea4f80`.
- Remaining historical problem: **#13 only**.

## Problem → hypothesis → implementation → falsification → result

### Problem
The normative CFMD surface table was executable only by convention. Production already had guarded structural `TypeExpr`, `RelExpr`, `Plan`, `CheckedCertificate`, and an exact round-trip `LoweringChecker`, but there was no proof-assistant theorem connecting the complete stated surface vocabulary to those kernel witnesses and no fail-closed binding that would detect vocabulary drift.

### Hypothesis
#20 can close without changing production Rust if the existing stable production contract is mechanized directly:

1. every normative surface feature must map to one trusted kernel witness;
2. surface structural types must preserve constructor structure and guarded-μ admission under elaboration;
3. every current relational surface operator must elaborate into the exact logical vocabulary;
4. the two predicates already enforced by `LoweringChecker` — exact logical round-trip and equal logical node count — must be proved sufficient for semantic preservation independently of physical algorithm choice;
5. a source-refinement checker must fail if the spec table, Rust type/query/plan vocabulary, or certificate boundary drifts away from the theorem artifact.

### Implementation
Added `formal/lean/CFMD/SurfaceKernel.lean` using Lean 4.34.0 core only.

Mechanized layers:

- complete normative `SurfaceFeature` table and `KernelWitness` vocabulary;
- `surface_feature_bijective` proves no stated surface feature is missing or conflated;
- structural `SurfaceType → KernelType` elaboration;
- `surface_type_weight_preserved` proves constructor-structure preservation;
- `surface_type_wellformed_preserved` proves free-variable / guarded-recursion admission is unchanged;
- complete current relational `SurfaceQuery` / `KernelQuery` vocabulary matching production `RelExpr`;
- physical `Plan` with algorithm annotations erased to the logical AST;
- `lower_roundtrip` and `lower_node_count`;
- `checked_plan_preserves_surface_semantics` and `checked_plan_certificate_sound`: for **any** physical plan, the exact two obligations enforced by production `LoweringChecker` imply semantic preservation for any logical evaluator;
- top-level `surface_to_kernel_preservation` theorem combines semantic, shape and type-admission preservation.

Added `formal/lean/check_surface_refinement.py` as a fail-closed production binding. It checks:

- the exact normative surface mapping table in `CFMD_IDEAL_DB_SPEC.md`;
- the exact production `TypeExpr` constructor set;
- scalar entity/reference carriers;
- the exact current `RelExpr` and `Plan` variant sets;
- production guarded recursion checks (`FreeVariable`, `UnguardedRecursion`, `validate_under_constructor`);
- `LoweringChecker::ExactLogicalRoundTrip` still enforces both exact `to_logical_expr()` equality and exact logical node count;
- logical `prepare()` still precedes lowering;
- production witnesses for capability/inclusion, retention, violation queries and typed rewrites;
- required Lean closure theorems still exist.

`formal/lean/check_all.sh` now checks both historical #18 and #20 artifacts plus both source-refinement binders in one offline gate.

`CFMD_IDEAL_DB_SPEC.md` now marks the surface→kernel preservation theorem `[VERIFIED]` and points at the proof/binder.

### Falsification
The first formal draft intentionally failed Lean on nested recursive type encodings and an overly implicit termination argument. It was rejected rather than papered over. The type model was replaced by a structurally recursive algebraic encoding, and the proof was rerun through the Lean kernel.

The final source binder is fail-closed:

- adding/removing a production `RelExpr` or `Plan` operator without updating the theorem fails exact variant-set equality;
- changing the `TypeExpr` constructor set fails;
- removing either production `LoweringChecker` obligation fails;
- changing the normative surface table without extending the mechanization fails;
- removing capability/rewrite/violation/retention witnesses fails.

### Result
Historical #20 is closed. Surface→kernel preservation is no longer only an executable Rust convention: the current normative surface vocabulary and checked lowering contract have a mechanically checked Lean theorem plus a production-source refinement gate.

The theorem is deliberately at the trusted architecture boundary. It does **not** claim that Lean verified arbitrary Rust machine code. Instead, it proves the semantic soundness of the exact certificate obligations enforced by the small production checker, and the binder guarantees the production checker/vocabulary still matches those mechanized obligations.

## Gates

### New Pass119 gates

- Lean 4.34.0 `CFMD/Publication.lean`: PASS.
- Lean 4.34.0 `CFMD/SurfaceKernel.lean`: PASS.
- #18 production refinement binder: PASS.
- #20 surface-to-kernel source refinement binder: PASS.
- Pass118 frozen production manifest subset: **85 / 85 Cargo/crate files byte-identical**.

### Production gates inherited exactly from Pass118

No production Rust/Cargo file changed in Pass119. Pass118's already-green gates therefore apply to the exact same production bytes:

- `cargo fmt --all -- --check`: PASS.
- `cargo check --workspace --all-targets`: PASS.
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS.
- full workspace: **758 passed / 0 failed / 8 ignored = 766 declared**.

The recycled Pass119 sandbox did not contain a Rust toolchain, so these Cargo commands were not redundantly rerun. Instead, the Pass118 manifest was verified directly against every current production Cargo/crate file; all 85 matched exactly.

## Formal artifact hashes

See `PASS119_FORMAL_SHA256.txt`.

## Remaining historical problem

Only **#13 supported-platform real durability assurance** remains OPEN.

It must not be closed by process-kill tests or filesystem mocks. Closure requires named supported OS/filesystem/device profiles and destructive/power-loss or equivalent real-platform fault evidence validating the filesystem assumptions consumed by #18 and the external authority persistence used by #17.
