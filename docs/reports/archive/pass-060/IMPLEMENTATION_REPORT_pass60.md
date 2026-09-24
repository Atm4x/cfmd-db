# IMPLEMENTATION REPORT — Pass60

Pass60 rebases and integrates the independent Program3 closeout on top of verified Pass59 without overwriting Pass58/59 work.

## 1. Shared revision-local identity ownership

`Revision` now retains `Arc<DenseEntityIds>` compiled from normalized lifecycle entities. Validation, lifecycle and reverse-reference derivatives are constructed against that same coordinate system through `compile_with_ids` APIs. Public/durable data continues to use `EntityId`.

`DenseEntitySet` gains exact removal and iteration primitives required by maintained LocalId algorithms.

## 2. Dense lifecycle derivative

`MaintainedDenseLifecycle` extends the Pass58 static projection with:

- dense root set;
- forward `keeps_alive` adjacency;
- reverse `kept_by` adjacency;
- maintained dense live set;
- decrease-region recomputation for removals;
- activation propagation for additions/resurrection.

For a removal, the algorithm finds the forward candidate region, identifies nodes with support entering from outside that region or from roots, computes the surviving closure inside the region, then replaces only that portion of the live set. Cycles without external support die together; cycles with surviving external support remain live.

## 3. Dense validation extents

`DenseTypeExtents` precomputes subtype-aware entity membership over one LocalId universe. `validate_state_with_ids` builds the extents once and threads them through recursive value validation. Scalar `LiveEntityRef` checks and field-owner checks no longer perform repeated carrier scans.

The existing public `validate_state` remains available and internally creates a dense map when no revision-owned map was supplied.

## 4. Reverse live-reference sensitivity

`LiveRefSensitivityIndex` indexes references recursively through structural values and relation rows. `DatabaseState::normalize` uses it to restrict fields/rows affected by lifecycle death instead of rescanning every surviving value for each dead target. Unknown/dead reference semantics remain unchanged.

`Revision` retains the normalized sensitivity derivative for future incremental consumers.

## 5. Dense physical live-reference column

`NativeColumn::DenseLiveEntityIds` contains:

- the declared entity type;
- `Arc<DenseEntityIds>` identity map;
- `Vec<LocalEntityId>` payload.

Construction maps external IDs through the supplied revision map and rejects unknown IDs. Materialization maps LocalIds back to external IDs. Projection/mutation/schema checking/bound matching/equality filtering all have explicit branches for the new representation. Equality filtering translates the external predicate ID to LocalId once and compares compact IDs in the loop.

The older `LiveEntityIds` representation remains valid.

## 6. Rebase discipline

The R&D patch was based on Pass57 and overlapped the Pass58 dense slice. The integration therefore:

- retained the Pass58 exhaustive three-node projection law;
- adopted the R&D maintained lifecycle and closeout tests;
- preserved all Pass59 Γ-QCN code;
- rejected the unrelated missing `algebraic_native` module reference;
- regenerated dependency metadata through Cargo rather than accepting stale lockfile hunks.

## 7. Falsification and diagnostics

All modified runtime crates passed targeted tests and strict Clippy before source freeze. The full workspace debug suite then passed on the frozen source.

Additional hostile coverage includes:

- exact `Arc::ptr_eq` reuse of revision-owned dense IDs;
- nested reverse sensitivity (`Product -> Option -> LiveEntityRef`);
- relation-row removal on lifecycle death;
- dense physical live-column round trip/projection/filtering;
- maintained cycle death/resurrection;
- all single-fact lifecycle toggles on three-node graphs;
- preserved exhaustive 512 static projection configurations.

Rebased release diagnostics:

- lifecycle local cut: `353310285 ns -> 1530465 ns` (~230.85x);
- type membership hot probes: `198211042 ns -> 4941448 ns` (~40.11x), with `7810815 ns` one-time extent compilation.

These are diagnostic fixtures, not general performance guarantees.

## 8. Verification

Final frozen bytes passed fmt, workspace check, full debug tests, strict Clippy, full release tests, release build, strict rustdoc, and overflow-check release tests under Rust 1.98.1. Cold release/overflow calls that timed out during compilation were not counted; warmed retries completed.
