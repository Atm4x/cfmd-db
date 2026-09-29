# CFMD API / Product Design

The repository entered Rust-runtime productization after the Pass280 kernel closeout; Pass281 introduced `cfmd-runtime`, Pass337 adds the public Rust application crate `cfmd`, and Pass338 adds its `CfmdEntity` derive generator.

Documents:

- [`PRODUCT_ROADMAP.md`](PRODUCT_ROADMAP.md) — active implementation sequence;
- [`CFMD_PYTHON_FACADE_THEORY.md`](CFMD_PYTHON_FACADE_THEORY.md) — retained Python UX/design source (not the current implementation-order authority);
- [`RUST_API_ROADMAP.md`](RUST_API_ROADMAP.md) — primary Rust product/runtime roadmap.

The public API is designed from application semantics, not from the internal crate graph.
