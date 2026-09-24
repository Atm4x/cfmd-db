# Pass110 frontier — historical #18 then #16

## Recommended order

Attack **#18 before #16**. This is not because consensus depends entirely on checkpoint compaction; it is because #18 is a bounded formal problem whose result fixes the exact crash/durability axioms used by all higher durable-authority reasoning. #16 can then reuse those assumptions instead of carrying an informal fsync model alongside consensus safety.

## #18 — formal immutable-generation publication / rename / fsync / GC proof

### Current production facts

The store already follows the intended protocol:

1. generation component files are written/synced;
2. a pending manifest is written and `sync_all`'d;
3. authority becomes explicitly uncertain before the rename attempt;
4. pending manifest is atomically renamed to the generation manifest;
5. the directory is `sync_all`'d;
6. only later does compaction remove non-current generations/pending manifests and then sync the directory.

Runtime crash/fault tests exist, but the historical problem explicitly asks for a **proof**, not more fault-injection tests.

### Formal state machine to build

Model at minimum:

- stable file contents vs volatile file contents;
- stable directory entries vs volatile directory entries;
- `WriteFile`, `FileFsync`, `Rename`, `DirFsync`, `Remove`, `Crash`;
- immutable generation components and manifest references/checksums;
- `authority_uncertain`;
- recovery selection of a generation;
- compaction eligibility.

Filesystem axioms must be explicit rather than assumed silently: rename atomicity scope, what file fsync guarantees, what directory fsync persists, whether remove durability requires directory fsync, and crash projection of unsynced state. Unsupported filesystems/platforms remain #13 rather than being smuggled into #18.

### Proof obligations

- **P18.1 Unique authority:** after a successful manifest directory sync, recovery cannot choose two distinct authoritative generations.
- **P18.2 No premature authority:** pending manifests are never authoritative.
- **P18.3 Old-authority safety:** before rename is attempted, failure leaves the previous generation authoritative.
- **P18.4 Rename uncertainty:** after rename begins but before directory sync, code may report uncertainty but must not claim the old generation is uniquely authoritative.
- **P18.5 Publication closure:** after rename + directory fsync, the new manifest and every referenced component required for recovery are durable under the model axioms.
- **P18.6 Recovery closure:** every modeled crash point either recovers a fully valid generation or returns explicit authority uncertainty/corruption; never a silently mixed generation.
- **P18.7 GC non-interference:** compaction never removes a file required by the currently authoritative generation.
- **P18.8 GC crash safety:** a crash between arbitrary removals and the final directory fsync cannot make the authoritative generation unrecoverable.
- **P18.9 Generation monotonicity:** a published generation is never mutated in place.
- **P18.10 Production refinement:** every production `publish_manifest_with_hook` / generation-write / `compact_obsolete_generations_with_hook` event maps to one formal transition.

### Closure bar

Do not close #18 with a Rust model test alone. The final deliverable needs a mechanically checked finite transition system / theorem artifact plus a documented refinement mapping to `kernel-durability::store`. If the environment lacks a suitable prover, first produce the formal model and hostile executable checker, but keep #18 OPEN until mechanization actually exists.

## #16 — replication / consensus runtime

### Production substrate already present

- durable replicated effect envelopes and branch heads;
- membership epochs and strict-majority quorum contract;
- LocalDurable / QuorumDurable / Published stages;
- durable effect vote-once per `(membership epoch, decision position, voter)`;
- durable successor-membership vote-once;
- quorum certificate validation against durable votes;
- restart-safe journal replay and causal-prefix publication.

The current code still explicitly treats `DurableSequencerOrder` and peer authentication as externally supplied. Therefore #16 is correctly PARTIAL, not secretly complete.

### Next implementation waves

**16A — first-class term/election/locking authority**
- durable current term / promised term;
- candidate/leader identity;
- vote-for-leader once per term;
- per-slot accepted/locked value with a safe carry-forward rule;
- stale-term rejection;
- membership transition rule compatible with joint/previous quorum safety.

**16B — authenticated peer evidence**
- canonical signed/MAC'd vote transcript containing cluster/domain id, membership epoch, term, slot, effect id and voter id;
- verification against pinned membership credentials;
- replay/domain separation;
- only verified evidence may become durable vote authority.
Crypto trust-root rotation may couple to #10/#17; keep that dependency explicit rather than weakening #16's safety model.

**16C — quorum loss / recovery semantics**
- no publication without live eligible quorum;
- leader demotion/fencing on higher term;
- restart/rejoin from durable accepted/committed prefix;
- explicit unavailable vs unsafe states.

**16D — transport + anti-entropy**
- exchange durable frontiers/branch heads, request missing immutable effects, idempotent verified ingestion;
- never let network arrival order become semantic authority;
- repair converges to the certified durable prefix.

**16E — distributed fault assurance**
- multi-process replicas;
- crash/restart, message loss/duplication/reordering, partitions, stale leaders, concurrent proposals and reconfiguration;
- assert no two conflicting values become Published for one consensus slot/term lineage and no published causal prefix regresses.

### Suggested Pass sequence

- Pass110/111: #18 formal model + mechanized invariant core.
- Then #16A election/term/lock protocol on the existing durable journal.
- #16B authenticated evidence.
- #16C/#16D liveness/recovery + anti-entropy.
- Final #16 distributed-fault matrix before PROD CLOSED.
