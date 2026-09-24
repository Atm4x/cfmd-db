# Implementation report — Pass105

Production changes are confined to `kernel-query` and preserve the Pass104 semantic authority.

- `crates/kernel-query/src/sealed_group_v3.rs`: total sealed Group patch planning/commit.
- `crates/kernel-query/src/lib.rs`: State V3 integration, unified storage-resolved Scan patch path, one-root compile/share, V4 graph ingress use.
- `crates/kernel-query/src/linear_island.rs`: physical program owns the one compiled `PreparedRelGraph`.
- `crates/kernel-query/src/execgraph.rs`: V4 unified graph compiler and scheduling machine.

No old ExecGraph V3 route-choice types remain. No legacy unrevisioned detached maintained-transition type remains.

The V4 scheduler is production code but runtime kernel dispatch is intentionally not cut over yet; recursive whole-tree planning remains the semantic oracle until NodeId state-arena migration is proven barrier by barrier.
