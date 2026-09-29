# PASS325 REPORT — Replication authority segment identity / relocatable replay foundation

## Wall clock
- Start: 2026-09-29 06:11:08 UTC
- Functional freeze: 2026-09-29 06:29:50 UTC
- Useful boundary: 06:31:08 UTC
- Hard boundary: 06:35:08 UTC

## Goal
Turn P324's immutable replication-authority segment design into executable code before changing physical authority: introduce stable content/parent-bound segment identities, bounded canonical segment envelopes, a relocatable physical index, and replay the indexed chain through the existing replication authority evaluator. Do not delete P323's proven archive-copy product path until the replacement physical authority is real.

## Delivered

### 1. Content/parent-bound `ReplicationAuthoritySegmentId`
Added the exact P324 identity law:

```text
SHA256(
    "CFMD/replication-authority-segment/v1" ||
    parent_segment_id_or_zero ||
    canonical_delta_length ||
    canonical_delta_frame_count ||
    exact canonical replication frames
)
```

The same delta attached to a different parent therefore has a different identity. Zero IDs are rejected rather than being ambiguous with the no-parent sentinel.

### 2. Bounded `CFAS` segment envelope
Added a fixed 88-byte segment header carrying parent ID, segment ID, bounded delta length and bounded frame count. The delta reuses the existing replication journal frame grammar unchanged; there is no second authority format/evaluator.

Replay is deliberately two-pass per segment: a bounded first pass verifies the complete content-addressed segment digest before any authority state is mutated; only then does a second bounded pass feed exact existing frames through maintained `ReplicationAuthorityJournal` replay semantics. This avoids partial semantic mutation from a segment whose final digest would fail. Declared length/frame count, index/header binding and final content digest are all fail-closed.

### 3. Canonical relocatable `CFAI` index
Added `ReplicationAuthoritySegmentIndex`:

```text
root SegmentId
SegmentId -> { parent SegmentId, physical offset, physical length }
```

Canonical encode/decode is bounded to 65,535 entries. Validation rejects:
- a root missing from the index;
- missing parents;
- cycles;
- entries not reachable from the root;
- overlapping extents;
- zero identities;
- conflicting duplicate ID -> extent/parent bindings;
- offset/length arithmetic overflow.

`relocate(id, new_extent)` changes only the physical location. Stable identity and parent edges are not rewritten.

### 4. Executable replay/relocation equivalence
A two-segment authority history is constructed with real maintained journal operations:

```text
Segment 1: membership bootstrap
Segment 2: quorum-loss fence, parent = Segment 1
```

The regression establishes:

```text
semantic_state(flat live journal)
    == semantic_state(indexed segment-chain replay)
    == semantic_state(the same chain after both physical extents move)
```

Chain replay is oldest-to-newest and reuses the existing frame replay semantics.

### 5. Hostile segment/index regressions
Added direct regressions proving:
- one-byte segment tamper is rejected;
- unreachable indexed authority is rejected;
- parent binding changes the segment identity.

### 6. Scope honesty: copy amplification is not yet closed
P323's product `ReplicationArchiveSectionSource` remains the active SingleFile checkpoint representation. P325 deliberately does not replace it with a test-only segment store.

The missing production step is physical authority: current roots/generations do not yet reference immutable segment extents outside the successor generation, and current compaction only preserves the active generation/WAL closure. Deleting the old archive path now would weaken durability rather than close the payer.

The segment module is therefore compiled as the next physical architecture foundation but is not advertised as the current product persistence path. Hostile follow-up also rejects serializing the complete relocatable index on every checkpoint: an N-entry index rewritten at checkpoint N would itself accumulate O(N²) metadata writes. The full index is a recovery/compaction view; ordinary publication needs persistent O(new-delta) locator nodes/tree.

## Verification
- `kernel-durability --all-targets`: **193/193 PASS**, plus multiprocess suites PASS.
- targeted segment suite: **4/4 PASS**.
- workspace `cargo check --all-targets` with `RUSTFLAGS=-D warnings`: PASS.
- workspace strict Clippy `--all-targets -- -D warnings`: PASS; final `kernel-durability --all-targets -- -D warnings` rerun after the verification-before-replay hostile fix: PASS.
- `kernel-durability` rustdoc `-D warnings`: PASS.
- `cargo fmt --all -- --check`: PASS.

## Architectural result
P324's key law is now executable rather than only documented:

```text
logical identity != physical address
```

and authority replay is invariant under relocation as long as the current index resolves each stable segment ID to the exact bytes that hash to that ID.

This is the prerequisite for moving replication history out of every successor generation without inventing a second semantic evaluator or a generic fallback.

## Next target — P326
Make the P325 chain the actual SingleFile replication authority:

1. append new immutable authority delta segment(s) without copying ancestors;
2. append O(new-delta) persistent locator metadata and publish only the current segment/locator root from the successor generation (never rewrite the full index on every checkpoint);
3. recover product stores through the indexed segment chain;
4. preserve exact streaming-checkpoint cut semantics;
5. extend physical compaction to compute the reachable segment closure, copy exact segment bytes, rebuild only physical index extents, publish the relocated root/index, and only then reclaim old extents;
6. delete P323's monolithic historical archive-copy path once those crash/recovery regressions pass.

The target cost law remains:

```text
ordinary checkpoint = O(new replication-authority delta)
explicit physical compaction = O(reachable retained authority)
```
