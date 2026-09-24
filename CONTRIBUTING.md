# Contributing

CFMD is currently in a kernel-to-product transition. Changes should preserve the explicit semantic/formal boundaries already established.

## Required local gates

```bash
./scripts/ci-rust.sh
./scripts/ci-formal.sh   # when Lean is installed / proof-relevant files changed
```

## Change discipline

- Do not use incidental Rust `Eq`/`Hash`/`Ord` where pinned `Γ` defines semantic equality/order.
- Do not change the logical query/type vocabulary without updating the #20 Lean/refinement boundary.
- Do not change durable publication fault points/order without updating the #18 Lean/refinement boundary.
- Do not broaden a durability support claim without a new exact platform fingerprint and certification campaign.
- Prefer a stable facade addition over exposing another internal kernel type publicly.
- Add tests for both positive behavior and hostile/fail-closed cases when changing authority, durability or semantic boundaries.
