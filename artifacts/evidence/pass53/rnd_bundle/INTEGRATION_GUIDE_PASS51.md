# Integration guide — Pass51

## Files safe to review independently

Apply/review these four patches against Pass51:

1. `patches/kernel-semantics.patch`
2. `patches/kernel-query.patch`
3. `patches/kernel-fixpoint.patch`
4. `patches/kernel-schema.patch`

They are also present as complete files under `modified/`.

## Recommended order

1. `kernel-semantics`
2. `kernel-query`
3. `kernel-fixpoint`
4. `kernel-schema`

The query patch depends on the new structural canonical-key API. Fixpoint and schema patches are independent of it and independent of each other.

## Important non-actions

- Do not apply anything from `experiments/rejected_semantic_bucket/`.
- Do not change `KEY_ENCODING_REVISION` merely because in-memory structural keys now exist. A durable/persisted structural-key family requires an explicit encoding-version decision and rebuild/migration boundary.
- Do not modify Pass51 Γ-QCN / Join code to consume these keys until the main Join agent deliberately integrates structural/custom quotient factors.

## Verification after manual integration

At minimum rerun:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p kernel-schema -p kernel-semantics -p kernel-fixpoint -p kernel-query --release
```
