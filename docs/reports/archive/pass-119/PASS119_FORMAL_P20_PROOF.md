# Pass119 formal #20 proof map

Mechanized artifact: `formal/lean/CFMD/SurfaceKernel.lean`

| Historical obligation | Lean theorem / binding |
|---|---|
| Complete normative surface vocabulary | `surface_feature_bijective` + spec-table source binding |
| Structural surface type preservation | `surface_type_weight_preserved` |
| Guarded μ / free-variable preservation | `surface_type_wellformed_preserved` |
| Exact logical erasure after lowering | `lower_roundtrip` |
| No hidden logical plan expansion | `lower_node_count` |
| Checker premises imply semantic preservation | `checked_plan_preserves_surface_semantics` |
| Exact production checker contract is sound | `checked_plan_certificate_sound` |
| End-to-end surface query/type preservation | `surface_to_kernel_preservation` |
| Production Rust matches theorem vocabulary | `check_surface_refinement.py` |

The production source binding checks the exact `TypeExpr`, `RelExpr`, `Plan`,
`LoweringChecker`, guarded-recursion and non-relational witness boundaries.  It
is intentionally fail-closed on new operator/type/surface vocabulary.
