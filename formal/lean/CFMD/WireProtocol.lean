/-
CFMD transport-neutral wire negotiation model.

The wire layer may select only a protocol version explicitly contained in both
client and server ranges. Framing/resource admission is observational: it does
not itself acquire database-state authority.
-/

namespace CFMD.WireProtocol

structure VersionRange where
  minimum : Nat
  maximum : Nat
  deriving DecidableEq, Repr

def contains (range : VersionRange) (version : Nat) : Bool :=
  range.minimum ≤ version && version ≤ range.maximum

/-- Current foundation has one hosted semantic version and selects it iff offered. -/
def negotiate (serverVersion : Nat) (client : VersionRange) : Option Nat :=
  if contains client serverVersion then some serverVersion else none

theorem negotiate_never_selects_unoffered
    (serverVersion : Nat) (client : VersionRange) (selected : Nat)
    (h : negotiate serverVersion client = some selected) :
    selected = serverVersion ∧ contains client selected = true := by
  unfold negotiate at h
  split at h <;> simp_all

/-- Frame admission is metadata-only and cannot alter authoritative Revision. -/
structure WireObserver where
  revision : Nat
  admittedFrames : Nat
  deriving DecidableEq, Repr

def admitFrame (state : WireObserver) : WireObserver :=
  { state with admittedFrames := state.admittedFrames + 1 }

theorem frame_admission_preserves_revision (state : WireObserver) :
    (admitFrame state).revision = state.revision := by
  rfl

end CFMD.WireProtocol
