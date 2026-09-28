# CFMD API / Product Design

The repository entered Rust-runtime productization after the Pass280 kernel closeout; Pass281 introduces `cfmd-runtime`.

Documents:

- [`PRODUCT_ROADMAP.md`](PRODUCT_ROADMAP.md) — active implementation sequence;
- [`CFMD_PYTHON_FACADE_THEORY.md`](CFMD_PYTHON_FACADE_THEORY.md) — full Python-first facade design source;
- [`RUST_API_ROADMAP.md`](RUST_API_ROADMAP.md) — primary Rust product/runtime roadmap.

The public API is designed from application semantics, not from the internal crate graph.
