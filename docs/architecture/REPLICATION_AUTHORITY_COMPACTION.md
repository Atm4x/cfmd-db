# Replication authority compaction — P324 R&D result

## Problem

The P323 single-file writer removed whole-archive RAM materialization, but checkpoint rotation still
rewrites the complete historical replication-authority byte stream.  For a history `H_n` that grows
by `Δ_n` per checkpoint, the physical work is

```text
W(N) = |H_1| + |H_2| + ... + |H_N|
```

and therefore becomes quadratic when `|H_n| = Θ(n)`.

A generic "compact old frames" pass is not acceptable: replication frames are authority evidence,
and dropping a frame because it looks superseded can silently destroy a vote-once fence, a lock,
a membership owner, authenticated evidence, or causal history.

## Exact semantic state

`ReplicationAuthoritySemanticSnapshot` in the executable P324 tests enumerates the complete semantic
state produced by journal replay.  It intentionally excludes only persistence mechanics:

```text
path / File handle
pending single-file frames
live single-file frames
poison state
```

Everything else is authority state and must either be represented directly by a compact image or be
mechanically derivable from it.

The executable round-trip property is:

```text
capture(replay(history))
    ==
capture(restore(capture(replay(history))))
```

This guards the inventory itself: adding a new semantic field to the journal without adding it to the
snapshot makes the snapshot test fail to compile or compare unequal once exercised.

## Negative result: a monolithic semantic snapshot is not the asymptotic solution

The journal contains history-bearing state which is part of the public/causal semantics, not merely
replay scaffolding.  In particular:

- `effects` retains replicated `DurableRevisionEffectRecord`s;
- `revision_frontiers` retains replicated causal coverage;
- branch ideals traverse historical prerequisite effects;
- durable decision locks and membership epochs can remain safety evidence.

Existing production readers call `replication.effect(id)` and `replication.revision_frontier(revision)`
when constructing causal ideals and validating new transitions.  Therefore a lossless semantic
snapshot has a lower bound proportional to retained replicated causal history in the worst case.

Consequently replacing `history frames` with `one current snapshot` would reduce redundant votes,
promises and superseded transition evidence, but it would still rewrite an `Ω(retained history)` image
at every checkpoint.  It does **not** by itself remove the quadratic write-amplification class.

## Required architecture: immutable authority segments

The correct physical primitive is a persistent segment chain / tree, not repeated concatenation and
not a fallback to the old monolithic journal.

```text
AuthorityRoot
    │
    ▼
Segment N  = canonical delta N + parent digest
    │
    ▼
Segment N-1
    │
    ▼
...
    │
    ▼
Semantic base segment
```

A checkpoint publishes only the new canonical delta plus a cryptographic reference to already-durable
immutable authority.  Historical bytes are not copied merely because a new database generation was
published.

Required properties:

1. **Persistent authority** — a new root never mutates or re-encodes an already-authoritative segment.
2. **Content binding** — every parent edge binds the exact parent digest and logical length, not only
   a physical offset.
3. **Exact cut** — a streaming checkpoint root references exactly the authority prefix visible at its
   begin cut; later live frames become a successor segment.
4. **One replay semantics** — recovery folds segments oldest-to-newest through the existing
   `ReplicationAuthorityJournal::apply_replay_frame` semantics.  There is no second generic authority
   evaluator.
5. **Bounded memory** — segment replay and publication are frame/chunk bounded.
6. **Linear lifetime write cost** — each durable authority delta is written O(1) times between
   explicit physical compactions; checkpoint count alone does not multiply old history.
7. **Compaction is semantic, explicit and certified** — a segment-tree compactor may replace a prefix
   by an equivalent semantic base only after proving the exact snapshot projection.  Normal operation
   never silently falls back to replaying/re-copying a monolithic archive.

## Why raw physical offsets are insufficient

P323's `SingleFileSectionSnapshot` captures a generation-local extent.  A new authority segment must
not identify its parent solely by that absolute offset: single-file physical compaction relocates the
active generation.  Parent identity therefore has to be content-addressed (digest + logical identity),
with physical location resolved by the current authority-segment index.

This also cleanly separates two concerns:

```text
logical parent identity  = stable digest / segment id
physical extent          = relocatable storage implementation
```

## Next implementation step

P325 should introduce a first-class `ReplicationAuthoritySegmentId` and a bounded segment envelope,
plus a root-local segment index that can be relocated during single-file compaction.  The first
production conversion should preserve the current replication frame grammar inside each delta
segment and prove:

```text
replay(flat historical frames)
    ==
replay(flatten(authority segment chain))
```

Only after that equivalence is executable should P323's composite old-archive copy path be deleted.

## Single-file compaction closure

Current `compact_active_generation()` copies one contiguous active generation and its WAL to
`DATA_OFFSET`.  Once authority parents may refer to immutable segments outside the newest generation,
that copy rule is no longer sufficient: a root could remain syntactically valid while one of its
content-addressed ancestors is discarded.

The segment design therefore adds an explicit reachability obligation.  Let `R(a)` be the transitive
closure of segment IDs reachable from authority root `a`.  A physical compaction is valid iff:

```text
for every s in R(a):
    bytes'(index'(s)) == bytes(index(s))
    SHA256(bytes'(index'(s))) == s.digest
```

and the newly published root/index pair is durable before any old extent in `R(a)` can be reclaimed.
The index is relocation metadata, not semantic authority: changing an extent is permitted only when
the segment ID and exact bytes remain unchanged.

This yields the intended cost model:

```text
ordinary checkpoint:  O(new authority delta)
explicit compaction:   O(reachable retained authority)
```

rather than paying `O(retained authority)` at every checkpoint.

## Segment identity candidate

P324 fixes the logical identity contract before implementation.  A segment ID should be the SHA-256
of a domain-separated canonical envelope, not a counter and not its storage address:

```text
segment_id = SHA256(
    "CFMD/replication-authority-segment/v1" ||
    parent_segment_id_or_zero ||
    canonical_delta_length ||
    canonical_delta_frame_count ||
    canonical_delta_bytes
)
```

The parent ID participates in the digest, so the same delta attached to a different authority prefix
is a different segment.  `delta_length` and `frame_count` are bounded canonical fields and prevent an
index/parser from inventing alternate segmentation of the same byte tail.  The physical index may
map this stable ID to a different extent after compaction, but it may never change the segment bytes
or parent edge for that ID.

## P325 executable segment foundation

P325 implements the segment identity/index/replay layer selected above without yet switching the
single-file product path away from P323's monolithic archive section.

The executable foundation now contains:

```text
ReplicationAuthoritySegmentId
    = SHA256(domain || parent_id || delta_len || frame_count || exact frame bytes)

ReplicationAuthoritySegmentIndex
    root SegmentId
    SegmentId -> { parent SegmentId, physical extent }

CFAS segment envelope
    parent id
    segment id
    bounded delta length
    bounded frame count
    exact existing replication frames

CFAI canonical index
    stable ids/parents
    relocatable physical extents
```

The index rejects missing parents, cycles, unreachable entries, overlapping extents, zero IDs and
conflicting duplicate IDs. Relocation changes only physical extents; segment IDs and parent edges stay
unchanged.

Recovery is executable and uses one authority evaluator: the index chain is resolved oldest-to-newest,
each bounded segment is digest-checked, and its existing replication frames are passed through
`ReplicationAuthorityJournal::apply_replay_frame` via the maintained replay path.

The regression constructs a two-segment chain (`membership bootstrap -> quorum-loss fence`) and checks:

```text
semantic_state(flat journal)
    == semantic_state(indexed segment-chain replay)
    == semantic_state(the same segments after physical relocation)
```

It also checks that attaching the same delta to a different parent changes `SegmentId`.

### Remaining physical payer

P325 deliberately does not delete P323's product archive-copy source yet. The new chain is executable,
but current single-file roots/generations still do not publish immutable segment extents outside the
successor generation. Deleting the old path before that physical authority/refinement exists would
replace a proven durable representation with a test-only format.

P326 should therefore make segment publication real: append immutable segment bytes before successor
generation authority, publish a small segment-root/index binding from the generation, recover through
the P325 chain, and extend active-generation compaction to relocate exactly the reachable segment
closure before reclaiming old extents. Only then is the historical archive copy path removable.

### P325 hostile follow-up: the full relocatable index is not a per-checkpoint object

The executable `CFAI` map is useful as a canonical recovery/compaction view, but persisting the complete
map again in every successor generation would reintroduce the same asymptotic class in smaller form:

```text
checkpoint N writes N index entries
=> 1 + 2 + ... + N = O(N^2) index-entry writes
```

Therefore P326 must not put a freshly serialized full `CFAI` beside every generation. Ordinary
publication needs a persistent locator structure as well: at minimum one immutable locator node per
new segment, linking to the previous locator/root, or a persistent authenticated tree. Recovery may
materialize the bounded/indexed view in memory by traversing those nodes; explicit physical compaction
may rebuild the full map because compaction is already `O(reachable authority)`.

This preserves the actual target law: both authority bytes **and their location metadata** are O(new
delta) during an ordinary checkpoint.

## P327 authenticated immutable-object boundary

P326 proved the linked-segment/locator physical shape but rejected its candidate because external
`CFAS` bytes sat outside generation sections and therefore outside CFMD AE v1. P327 closes that
security prerequisite before physical authority is switched again.

External authority segments are now defined to live inside a relocatable `CFAO` immutable-object
envelope. `ReplicationAuthoritySegmentId` remains the identity of the canonical plaintext segment;
encryption nonce and physical location are deliberately excluded from that identity.

For encrypted databases CFMD AE v1 derives a third HKDF-separated key domain,
`immutable-object`, alongside `section` and `wal`. The `CFAO` header carries object kind, segment ID,
parent segment ID, canonical plaintext length and fixed chunk geometry. Every 64 KiB plaintext chunk
is an independent AE v1 envelope whose AAD binds the complete object header plus chunk index and
chunk plaintext length. Therefore header substitution, parent/identity substitution, chunk reorder,
truncation and ciphertext tamper fail closed.

The object AAD intentionally does **not** bind generation number, physical offset or locator offset.
Consequently compaction may copy exact authenticated `CFAO` bytes to a new extent and rebuild only
physical locator locations; it must not decrypt/re-encrypt reachable immutable authority merely to
relocate it.

Plaintext and encrypted stores use the same object grammar but have explicit, non-interchangeable
modes. An encrypted database never error-routes to plaintext object replay, and a plaintext database
does not silently accept encrypted objects without its database key.

P327 does not reactivate the rejected P326 physical root/locator candidate. The maintained P323
replication archive remains product authority until the linked locator/root publication and
segment-aware compaction are reintroduced on top of `CFAO` and pass crash/torn/relocation tests in
both plaintext and encrypted configurations.
