# PASS112 — Historical #18 mechanized closure (Lean 4.34.0)

Freeze: 2026-09-23T15:15:55.396937+00:00
Production Rust fingerprint: ab29f02d899a7f563a870179b03a190b01829520ff2136d30f1cd55b21657ca9
Lean proof SHA-256: 50ac70af7dc7557de18c8790c8f12ec117d0f48cba8aa7a5d99ffbc754f7857f

## Result
Historical **#18 formal immutable-generation publication / rename / fsync / GC proof is PROD CLOSED in Pass112**.

The production durability protocol itself is byte-identical to Pass111. The Lean model now also contains an explicit finite `PublishTransition`/`Reachable` state machine. Pass112 adds an offline, mechanically checked Lean artifact plus a hostile source-refinement binder to the existing Rust finite model and subprocess crash matrix.

## Mechanized artifact

`formal/lean/CFMD/Publication.lean` is checked by Lean 4.34.0 without Mathlib or network dependencies. It proves the recorded P18.1–P18.10 obligations:

- P18.1 unique recovery authority;
- P18.2 pending manifest never authoritative;
- P18.3 old authority before rename;
- P18.4 rename uncertainty admits old/new only and both are recoverable;
- P18.5 manifest-directory fsync closes publication on recoverable new authority;
- P18.6 every modeled publication crash cut is recoverable;
- P18.7 GC does not remove current authority;
- P18.8 arbitrary partial persistence of obsolete removals cannot break recovery;
- P18.9 immutable generation identity/payload and monotone generation order;
- P18.10 one-to-one production fault-point/refinement mapping.

The source binder additionally checks generation writers remain `create_new(true)` and generation allocation remains monotone via `next_generation`.

The model explicitly enumerates ordinary prerequisites (`checkpoint`, `wal`, `metadata`) and streaming prerequisites (`checkpointRoot`, complete `checkpointChunks`, shadow `wal`, `metadata`, `preparedCapsule`).

## Filesystem assumptions

The theorem is conditional on the supported durability model: file fsync persists contents; directory fsync persists namespace entries; rename is atomic but pre-directory-fsync crash may expose old or new namespace; remove persistence is likewise uncertain until directory fsync. Unsupported filesystem/platform assurance remains historical #13 and is not silently absorbed by #18.

## Production refinement binding

`formal/lean/check_refinement.py` binds the mechanization back to `kernel-durability::store` and fails if:

- the 10 `StoreFaultPoint` variants change;
- prerequisite fsync hook order changes;
- authority uncertainty moves after rename;
- pending sync / rename / manifest-directory sync order changes;
- GC remove / directory-sync order changes;
- streaming checkpoint no longer closes PreparedCutCapsule, metadata, shadow WAL, chunks/root and directory durability before calling the shared manifest publisher;
- monotone generation creation no longer uses `next_generation` on both generation-rotation paths.

`formal/lean/check_all.sh` runs Lean kernel checking and this production-binding gate together.

## Independent dynamic evidence

Re-run in Pass112:

- Rust finite publication/GC model: 4/4 PASS;
- subprocess kill checkpoint/manifest matrix: PASS;
- subprocess kill compaction matrix: PASS;
- full workspace: 703 passed / 0 failed / 8 ignored = 711 declared;
- fmt/check/strict Clippy: PASS.

## Historical status

The ledger moves from **16/22 to 17/22 PROD CLOSED**.

Next historical frontier: #16 replication/consensus runtime. #13 remains the platform-level durability-assurance boundary for the filesystem axioms used here.
