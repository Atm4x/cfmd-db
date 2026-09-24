# Pass110 — #18 immutable-generation publication model

## Filesystem axioms used by the model

A1. `File::sync_all` makes that file's written bytes durable, but does not by itself establish a durable directory namespace entry.

A2. Directory `sync_all` persists the namespace state required by the protocol: newly created prerequisite entries, rename results, and removals performed before that directory sync.

A3. Rename is atomic in the live namespace. If a crash happens after rename but before directory fsync, recovery may expose either the pre-rename or post-rename namespace. The protocol therefore marks authority uncertain before attempting rename.

A4. A pending manifest is never considered authority. Recovery selects only final `manifest-*.cfmf` generations.

A5. Generation components are immutable once their publication protocol starts. The final manifest only references generation-local content that was file-synced and followed by prerequisite directory sync before rename.

A6. Before the final GC directory sync, any subset of attempted obsolete removals may become persistent. The model intentionally permits independent persistence rather than assuming all-or-nothing removal.

These are protocol axioms, not claims that every platform/filesystem honors them. Validation of the supported platform set remains historical #13.

## Production refinement map

- checkpoint `sync_all` -> `CheckpointFileSynced`
- WAL barrier/sync -> `WalFileSynced`
- metadata `sync_all` -> `MetadataFileSynced`
- prerequisite directory sync -> `PrerequisiteDirectorySynced`
- pending manifest `sync_all` -> `PendingManifestFileSynced`
- manifest rename -> `ManifestRenameUncertain`
- manifest directory sync -> `ManifestDirectorySynced`
- before obsolete remove -> `BeforeGcRemove`
- after obsolete remove -> `AfterGcRemoveUncertain`
- compaction directory sync -> `GcDirectorySynced`

This exact mapping is asserted in `publication_model.rs` against `StoreFaultPoint`.

## Current mechanically executable invariants

The bounded Rust model checks all crash projections it generates and rejects any state where recovery's selected highest final manifest lacks its durable prerequisites. It additionally checks pending non-authority, rename uncertainty, and partial-GC safety across multiple publication generations.

The real subprocess-kill tests provide implementation refinement evidence for the current host filesystem, but do not substitute for the final theorem artifact.

## Remaining closure obligations for #18

- mechanize this transition system in an external proof/model-checker environment;
- include streaming checkpoint/chunk/prepared-capsule prerequisites in the same publication theorem rather than treating them as a separate informal protocol;
- prove production event refinement, not merely enumerate matching fault-point names;
- state generation monotonicity/immutability explicitly in the mechanized model;
- keep unsupported filesystem semantics in #13 rather than weakening the axioms here.
