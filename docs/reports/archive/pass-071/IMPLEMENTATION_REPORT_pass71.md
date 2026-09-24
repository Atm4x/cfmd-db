# CFMD Implementation Report — Pass71

Pass71 adds durable, versioned physical-artifact *recipes* rather than persisting layout-local payloads. Checkpoint metadata can now retain reconstructible semantic indexes, Γ-QCN quotient factors and semantic statistics across compaction/reopen. Recovery rebuilds fresh payloads from the authoritative Revision and current pinned Γ under the recovery layout.

The design deliberately excludes raw `PhysicalRowId` buckets from durability. Physical row handles and layout-local representations remain reconstructible derivatives, not authority. Syntactically valid but semantically stale recipes are dropped as optional optimizations; corrupt or unknown recipe-format data fails closed. Equivalent recipes across layouts collapse deterministically, with manual pinning dominating advisor ownership.

I64 index recovery remains OPEN because the current recovery layout is generic RowStore while the specialized index requires typed-columnar lowering. A future durable physical-layout recipe must establish that prerequisite before I64 artifact reconstruction can be claimed.

The resumed packaging session reconstructed Pass71 from verified Pass70 because the original transient workspace was unavailable. The reconstructed source has the same four-file production scope and the same independent test inventory/invariants, and it was re-run through the full Rust 1.98.1 gate before publication.
