# Pass110 engineering revalidation after Flat NodeId Arena

| Item | Pass110 status | Action |
|---|---|---|
| Repeated recursive subtree typecheck | ACTIVE | Merge with direct-flat build into one typed postorder compile artifact. |
| Temporary recursive construction tree | ACTIVE | Same closure as above; debug oracle may remain separately under `cfg(debug_assertions)`. |
| `attach_storage_rows` whole-state candidate clone | CLOSED | Two-phase validate -> targeted COW commit, hostile atomicity test added. |
| Revision candidate shallow clone | INTENTIONAL | Detached candidate/publication semantics; shallow Arc COW. |
| I64 Group hand-baseline gap | NON-BLOCKING | Keep as optional optimization evidence; #21 remains CLOSED. |
