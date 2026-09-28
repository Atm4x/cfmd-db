/-
CFMD single-file authority publication model.

A generation becomes authoritative only after its complete bytes are durable
and a checksummed root slot naming that generation is published. A torn root
slot is not authority. A valid published root whose referenced generation is
corrupt must fail closed rather than roll back to an older root.
-/

namespace CFMD.SingleFile

inductive Phase where
  | oldRoot
  | generationDurable
  | rootSlotTorn
  | rootSlotPublished
  deriving DecidableEq, Repr

inductive Authority where
  | old
  | new
  deriving DecidableEq, Repr

structure CrashImage where
  oldRootValid : Bool
  newGenerationValid : Bool
  newRootValid : Bool
  deriving DecidableEq, Repr

def authority (image : CrashImage) : Option Authority :=
  if image.newRootValid then some .new
  else if image.oldRootValid then some .old
  else none

def crashImage : Phase → CrashImage
  | .oldRoot => ⟨true, false, false⟩
  | .generationDurable => ⟨true, true, false⟩
  | .rootSlotTorn => ⟨true, true, false⟩
  | .rootSlotPublished => ⟨true, true, true⟩

/-- Appending/syncing the new generation alone never changes authority. -/
theorem durable_generation_is_not_publication :
    authority (crashImage .generationDurable) = some .old := by
  rfl

/-- A torn/incomplete replacement slot leaves the previous root authoritative. -/
theorem torn_root_falls_back_to_old :
    authority (crashImage .rootSlotTorn) = some .old := by
  rfl

/-- Publication occurs only once the new root slot itself is valid. -/
theorem valid_root_publishes_new :
    authority (crashImage .rootSlotPublished) = some .new := by
  rfl

/-- A valid new root is never interpreted as authority for the old generation. -/
theorem published_root_never_rolls_back
    (image : CrashImage) (h : image.newRootValid = true) :
    authority image = some .new := by
  simp [authority, h]

/-- Recovery must reject a valid root whose referenced generation is invalid. -/
def recoverable (image : CrashImage) : Prop :=
  match authority image with
  | none => False
  | some .old => image.oldRootValid = true
  | some .new => image.newGenerationValid = true

theorem published_root_requires_its_generation
    (image : CrashImage)
    (hroot : image.newRootValid = true)
    (hrecoverable : recoverable image) :
    image.newGenerationValid = true := by
  simp [recoverable, authority, hroot] at hrecoverable
  exact hrecoverable

end CFMD.SingleFile

namespace CFMD.SingleFile

/-- The active generation's WAL is either open for appends or sealed at an exact durable cut. -/
inductive JournalState where
  | open (firstLsn : Nat)
  | sealed (firstLsn nextLsn journalEnd : Nat)
  deriving DecidableEq, Repr

def canPublishNextGeneration : JournalState → Bool
  | .open _ => false
  | .sealed firstLsn nextLsn _ => firstLsn > 0 && nextLsn >= firstLsn

def nextGenerationFirstLsn : JournalState → Option Nat
  | .open _ => none
  | .sealed _ nextLsn _ => some nextLsn

/-- Generation rotation is impossible while the live journal is still open. -/
theorem open_journal_cannot_publish_generation (firstLsn : Nat) :
    canPublishNextGeneration (.open firstLsn) = false := by
  rfl

/-- A valid seal carries the exact next WAL LSN into the next generation. -/
theorem sealed_journal_carries_lsn
    (firstLsn nextLsn journalEnd : Nat) :
    nextGenerationFirstLsn (.sealed firstLsn nextLsn journalEnd) = some nextLsn := by
  rfl

/-- Appending an unpublished generation after a sealed journal cannot change the old root authority. -/
theorem orphan_generation_after_seal_keeps_old_authority :
    authority (crashImage .generationDurable) = some .old := by
  rfl

/-- Reopening a sealed old journal after discarding an orphan generation preserves its first LSN. -/
def reopenJournal : JournalState → JournalState
  | .open firstLsn => .open firstLsn
  | .sealed firstLsn _ _ => .open firstLsn

theorem reopen_preserves_first_lsn
    (firstLsn nextLsn journalEnd : Nat) :
    reopenJournal (.sealed firstLsn nextLsn journalEnd) = .open firstLsn := by
  rfl

end CFMD.SingleFile

namespace CFMD.SingleFile

/-- A live store reopen derives its head from the published checkpoint plus the
    exact durable WAL suffix; packaging that state in one file does not create
    a second logical commit authority. -/
structure StoreCut where
  checkpointRevision : Nat
  walHeadRevision : Nat
  deriving DecidableEq, Repr

def recoveredHead (cut : StoreCut) : Nat := cut.walHeadRevision

/-- Rotating the physical generation at the already-durable head preserves the
    logical database head before any later WAL commit is appended. -/
theorem checkpoint_rotation_preserves_head (head : Nat) :
    recoveredHead ⟨head, head⟩ = head := by
  rfl

end CFMD.SingleFile

namespace CFMD.SingleFile

/-- Replication authority shares the physical WAL stream but is auxiliary to the
    linear database head. Appending one replication record cannot publish a new
    database Revision. -/
structure ReplicationLane where
  databaseHead : Nat
  replicationFrames : Nat
  deriving DecidableEq, Repr

def appendReplicationAuthority (lane : ReplicationLane) : ReplicationLane :=
  { lane with replicationFrames := lane.replicationFrames + 1 }

theorem replication_frame_preserves_database_head (lane : ReplicationLane) :
    (appendReplicationAuthority lane).databaseHead = lane.databaseHead := by
  rfl

end CFMD.SingleFile

namespace CFMD.SingleFile

/-- A streaming checkpoint may carry a suffix that starts before the old WAL
    endpoint, but it must end at exactly the same certified endpoint. -/
structure CarryForward where
  oldFirstLsn : Nat
  cutFirstLsn : Nat
  endpointNextLsn : Nat
  deriving DecidableEq, Repr

def validCarry (c : CarryForward) : Prop :=
  0 < c.oldFirstLsn ∧
  c.oldFirstLsn ≤ c.cutFirstLsn ∧
  c.cutFirstLsn ≤ c.endpointNextLsn

/-- Carry-forward does not invent a later endpoint: the new journal's next LSN
    is exactly the sealed old journal endpoint. -/
def carriedNextLsn (c : CarryForward) : Nat := c.endpointNextLsn

theorem carry_forward_preserves_endpoint (c : CarryForward) :
    carriedNextLsn c = c.endpointNextLsn := by
  rfl

/-- The pinned cut of a valid carry-forward transition can never precede the
    physical WAL authority from which it is derived. -/
theorem valid_carry_cut_is_within_old_journal
    (c : CarryForward) (h : validCarry c) :
    c.oldFirstLsn ≤ c.cutFirstLsn := by
  exact h.2.1

end CFMD.SingleFile

namespace CFMD.SingleFile

/-- Physical compaction may relocate the already-authoritative generation and
    sealed WAL bytes, but it does not create a new logical generation, Revision,
    generation digest, or WAL digest. -/
structure LocatedAuthority where
  revision : Nat
  generation : Nat
  generationDigest : Nat
  walDigest : Nat
  generationOffset : Nat
  journalOffset : Nat
  deriving DecidableEq, Repr

def relocateAuthority
    (authority : LocatedAuthority)
    (generationOffset journalOffset : Nat) : LocatedAuthority :=
  { authority with generationOffset, journalOffset }

theorem relocation_preserves_revision
    (authority : LocatedAuthority) (generationOffset journalOffset : Nat) :
    (relocateAuthority authority generationOffset journalOffset).revision = authority.revision := by
  rfl

theorem relocation_preserves_generation
    (authority : LocatedAuthority) (generationOffset journalOffset : Nat) :
    (relocateAuthority authority generationOffset journalOffset).generation = authority.generation := by
  rfl

theorem relocation_preserves_freshness_material
    (authority : LocatedAuthority) (generationOffset journalOffset : Nat) :
    ((relocateAuthority authority generationOffset journalOffset).generationDigest,
      (relocateAuthority authority generationOffset journalOffset).walDigest) =
    (authority.generationDigest, authority.walDigest) := by
  rfl

/-- A copied compacted image is not authority until a root naming its new
    physical offsets is published. -/
inductive CompactionPhase where
  | oldAuthority
  | copyDurable
  | relocationRootPublished
  deriving DecidableEq, Repr

def compactionAuthority : CompactionPhase → Authority
  | .oldAuthority => .old
  | .copyDurable => .old
  | .relocationRootPublished => .new

theorem durable_compaction_copy_is_not_publication :
    compactionAuthority .copyDurable = .old := by
  rfl

theorem relocation_root_is_publication :
    compactionAuthority .relocationRootPublished = .new := by
  rfl

end CFMD.SingleFile
