# IMPLEMENTATION REPORT — Pass34

## problem

Pass32 integrated a logical WAL and Pass33 repaired the hostile leaf-contract defects, but normal restart still depended on an externally supplied exact base `Revision`. There was no durable checkpoint generation owner, no manifest publication protocol, no segment rotation/compaction authority, and no production object that guaranteed a live runtime was paired with the correct durable head.

## hypotheses

1. Persist the complete current `Revision=(S,Γ,M)` as a versioned checkpoint and re-run `Revision::build` when decoding.
2. Treat checkpoint + WAL as immutable numbered generations selected only by immutable manifest publication.
3. Fsync checkpoint/WAL directory entries before publishing the manifest.
4. Publish a manifest only after an fsynced pending file is atomically renamed and the directory is fsynced.
5. Never rewrite/truncate the active WAL during checkpoint rotation; start a fresh WAL segment at the checkpoint revision and garbage-collect old generations later.
6. Bind `RuntimeRevisionCell` and `DurableRevisionStore` inside one `DurableRuntime` production owner.

## implementation

### kernel-schema

Added deterministic read-only schema iterators required by the checkpoint codec:

```text
Schema::symbols
Schema::type_definitions
Schema::capabilities
Schema::inclusions
```

### kernel-durability/checkpoint.rs

Added stable checkpoint codec for:

```text
RevisionId
SemanticContext
  Schema
    symbols
    types
    capabilities
    fields
    relations
    structural equivalences
    inclusions
  SemanticEnvironment module digests
DatabaseState
  lifecycle
  carriers
  fields
  relations
```

The decoder reconstructs ordinary domain values and then calls `Revision::build`, rather than returning unchecked checkpoint state.

### kernel-durability/store.rs

Added `DurableRevisionStore`:

```text
create(directory, base_revision)
open(directory, registry)
durably_prepare
durably_commit
rotate_checkpoint
compact_obsolete_generations
```

It owns an exclusive directory lock, one active checkpoint generation and one locked WAL segment.

Generation files:

```text
checkpoint-%020d.cfcp
wal-%020d.cfmw
manifest-%020d.cfmf
pending-manifest-%020d.tmp
.cfmd-durability.lock
```

Manifest protocol:

```text
checkpoint write + sync_all
WAL create + sync_data
parent directory sync_all
pending manifest write + sync_all
rename pending -> final immutable manifest
parent directory sync_all
```

The store tracks `durable_head`; a PREPARE whose source differs from it is rejected before WAL append. Checkpoint rotation is rejected unless the checkpoint revision equals that durable head.

### kernel-plan

Added `DurableRuntime`, owning:

```text
RuntimeRevisionCell
Mutex<DurableRevisionStore>
```

Public mainline operations:

```text
create
open
snapshot
commit_revision
checkpoint
compact_obsolete_generations
install_i64_index
```

`RuntimeRevisionCell::commit_revision_durable` is now crate-private. Normal callers can no longer arbitrarily pair a runtime with an unrelated durable backend.

`DurableRuntime::open` performs:

```text
published manifest selection
-> exact checkpoint decode/validation
-> WAL scan
-> logical replay
-> PhysicalStore rebuild
-> maintained materialization rebuild
-> fresh runtime lineage
```

## hostile falsification

Added/verified tests for:

1. full complex Revision checkpoint roundtrip;
2. exact checkpoint + committed WAL-tail reopen;
3. checkpoint rotation followed by a new WAL tail;
4. unpublished orphan checkpoint/WAL + torn pending manifest;
5. published checkpoint corruption;
6. published manifest with missing WAL;
7. obsolete generation compaction and reopen;
8. source revision mismatch against durable head through store invariants;
9. existing Pass32 WAL prefix/corruption/duplicate/LSN matrix under the expanded durability crate.

## rejected routes

### Rewrite one fixed checkpoint/WAL pair in place

Rejected. It creates a larger crash-sensitive overwrite/truncate protocol and destroys the previous known-good generation before the new base is fully published.

### Manifest overwrite

Rejected. Immutable generation manifests remove target-overwrite semantics and make the authority transition monotone.

### Trust checkpoint bytes as a Revision

Rejected. Decode must pass through normal schema/semantic/model validation.

### Let `FileRevisionWal::open_recovered` create a missing manifest-referenced WAL

Rejected. A published manifest referencing a missing WAL is corruption, not an empty valid tail.

### Fsync directory only after manifest rename

Rejected as insufficiently strong. Checkpoint/WAL names are directory-fsynced first, so a surviving published manifest cannot validly outrun prerequisite directory entries under the intended filesystem contract.

## verification

Final gate on Rust 1.98.1:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test ... --release PASS
```

Metrics:

```text
273 declared tests
70 kernel-plan tests
65 kernel-query tests
20 kernel-durability tests
20 crates
34,177 Rust LOC
0 external Cargo sources
0 unsafe
```

## result

Pass34 closes the missing durable base/generation/restart authority chain for the current relation-data transaction class. Recovery no longer needs a caller-provided base `Revision`; the base is encoded in the published generation itself.

The remaining durability blocker is now empirical hostile crash validation and durable configuration/transaction generalization, not absence of a checkpoint/manifest architecture.
