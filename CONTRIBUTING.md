# Contributing

CFMD has frozen the Pass280 global kernel hostile/refactor campaign and is now in productization. Changes should preserve the explicit semantic/formal boundaries already established.

## Required local gates

Before running the gates, stage the intended changes and rebuild the manifest
from Git's staged bytes. This avoids line-ending differences and includes every
vendored dependency in the committed snapshot.

```bash
git add <changed-files>
python3 scripts/update-repository-manifest.py
git add REPOSITORY_MANIFEST.sha256
```

```bash
bash ./scripts/ci-rust.sh
bash ./scripts/ci-formal.sh   # when Lean is installed / proof-relevant files changed
```

## Change discipline

- Do not use incidental Rust `Eq`/`Hash`/`Ord` where pinned `Γ` defines semantic equality/order.
- Do not change the logical query/type vocabulary without updating the #20 Lean/refinement boundary.
- Do not change durable publication fault points/order without updating the #18 Lean/refinement boundary.
- Do not broaden a durability support claim without a new exact platform fingerprint and certification campaign.
- Prefer a stable facade addition over exposing another internal kernel type publicly.
- Add tests for both positive behavior and hostile/fail-closed cases when changing authority, durability or semantic boundaries.

- Frozen kernels reopen only on concrete evidence (counterexample, proof/authority seam, measured regression, R&D requirement, or facade/DX requirement), not cleanup-by-inertia.
