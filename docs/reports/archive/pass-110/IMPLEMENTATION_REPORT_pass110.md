# IMPLEMENTATION REPORT — Pass110

Production changes:
- `crates/kernel-query/src/lib.rs`: removed setup-wide candidate clone from `attach_storage_rows`; added validate-before-COW commit path and hostile failure-atomicity/COW test.
- `crates/kernel-durability/src/store.rs`: registers the #18 finite publication model as a store-private test submodule.
- `crates/kernel-durability/src/store/publication_model.rs`: explicit immutable-generation publication/rename/fsync/GC crash model and production fault-point mapping.

No #16 production behavior was changed. Historic #18 remains open because the final external mechanization requirement is intentionally not waived.
