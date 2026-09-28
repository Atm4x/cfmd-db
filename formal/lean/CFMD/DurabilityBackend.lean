/-
CFMD durability backend separation model.

The semantic durability protocol owns the authoritative Revision and commit
ordering. A physical backend chooses only how an already-defined durable cut is
represented. Selecting directory or single-file storage cannot itself publish a
new semantic Revision.
-/

namespace CFMD.DurabilityBackend

inductive Backend where
  | directory
  | singleFile
  deriving DecidableEq, Repr

structure DurableCut where
  revision : Nat
  generation : Nat
  deriving DecidableEq, Repr

/-- Physical backend selection changes representation, not semantic authority. -/
def materialize (_backend : Backend) (cut : DurableCut) : DurableCut := cut

theorem backend_selection_preserves_revision
    (backend : Backend) (cut : DurableCut) :
    (materialize backend cut).revision = cut.revision := by
  rfl

theorem backend_selection_preserves_generation
    (backend : Backend) (cut : DurableCut) :
    (materialize backend cut).generation = cut.generation := by
  rfl

/-- Rejecting an unsupported physical capability cannot mutate the durable cut. -/
def rejectUnsupported (_backend : Backend) (cut : DurableCut) : DurableCut := cut

theorem capability_rejection_is_non_mutating
    (backend : Backend) (cut : DurableCut) :
    rejectUnsupported backend cut = cut := by
  rfl

end CFMD.DurabilityBackend
