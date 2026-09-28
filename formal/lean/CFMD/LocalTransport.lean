/-
CFMD first-party local transport conformance model.

Locality and endpoint admission are transport facts only. They do not establish
identity, mint permissions or mutate database Revision authority.
-/

namespace CFMD.LocalTransport

structure State where
  revision : Nat
  acceptedConnections : Nat
  deriving DecidableEq, Repr

def acceptLocal (state : State) : State :=
  { state with acceptedConnections := state.acceptedConnections + 1 }

theorem local_accept_preserves_revision (state : State) :
    (acceptLocal state).revision = state.revision := by
  rfl

/-- Local transport forwards evidence to the configured authenticator unchanged. -/
def authenticateLocal
    (authenticate : α → Option Nat)
    (_isLocal : Bool)
    (evidence : α) : Option Nat :=
  authenticate evidence

theorem locality_cannot_upgrade_rejected_evidence
    (authenticate : α → Option Nat)
    (evidence : α)
    (rejected : authenticate evidence = none) :
    authenticateLocal authenticate true evidence = none := by
  simpa [authenticateLocal] using rejected

end CFMD.LocalTransport
