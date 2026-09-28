namespace CFMD.SessionLifecycle

structure Grants where
  read : Bool
  historicalRead : Bool
  historyRead : Bool
  watch : Bool
  write : Bool
  deriving DecidableEq, Repr

structure Authority where
  revision : Nat
  generation : Nat
  grants : Grants
  revoked : Bool
  deriving DecidableEq, Repr

/-- Host-issued refresh changes grants only while the authority remains live. -/
def refresh (state : Authority) (grants : Grants) : Authority :=
  if state.revoked then state
  else { state with generation := state.generation + 1, grants := grants }

/-- Revocation is monotone and does not mutate database Revision authority. -/
def revoke (state : Authority) : Authority :=
  if state.revoked then state
  else { state with generation := state.generation + 1, revoked := true }

/-- Effective write authority requires both a current grant and a live session. -/
def mayWrite (state : Authority) : Bool :=
  !state.revoked && state.grants.write

theorem refresh_preserves_revision (state : Authority) (grants : Grants) :
    (refresh state grants).revision = state.revision := by
  cases h : state.revoked <;> simp [refresh, h]

theorem revoke_preserves_revision (state : Authority) :
    (revoke state).revision = state.revision := by
  cases h : state.revoked <;> simp [revoke, h]

theorem revoke_is_monotone (state : Authority) :
    (revoke state).revoked = true := by
  cases h : state.revoked <;> simp [revoke, h]

theorem refresh_cannot_revive_revoked
    (state : Authority)
    (grants : Grants)
    (revoked : state.revoked = true) :
    (refresh state grants).revoked = true := by
  simp [refresh, revoked]

theorem revoked_authority_cannot_write
    (state : Authority)
    (revoked : state.revoked = true) :
    mayWrite state = false := by
  simp [mayWrite, revoked]

end CFMD.SessionLifecycle
