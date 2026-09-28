namespace CFMD.HostedBoundary

structure Grants where
  read : Bool
  historicalRead : Bool
  historyRead : Bool
  watch : Bool
  write : Bool
  deriving DecidableEq, Repr

structure Session where
  principal : Nat
  grants : Grants
  deriving DecidableEq, Repr

/-- Authentication establishes identity only; authorization remains a separate authority. -/
def issueSession
    (authenticate : α → Option Nat)
    (authorize : Nat → Option Grants)
    (evidence : α) : Option Session :=
  match authenticate evidence with
  | none => none
  | some principal =>
      match authorize principal with
      | none => none
      | some grants => some { principal, grants }

/-- Any issued grants come exactly from the authorizer for the authenticated principal. -/
theorem issued_grants_are_authorizer_exact
    (authenticate : α → Option Nat)
    (authorize : Nat → Option Grants)
    (evidence : α)
    (session : Session)
    (issued : issueSession authenticate authorize evidence = some session) :
    authorize session.principal = some session.grants := by
  cases hAuth : authenticate evidence with
  | none =>
      simp [issueSession, hAuth] at issued
  | some principal =>
      cases hAuthorize : authorize principal with
      | none =>
          simp [issueSession, hAuth, hAuthorize] at issued
      | some grants =>
          simp [issueSession, hAuth, hAuthorize] at issued
          cases issued
          exact hAuthorize

/-- Connection lifecycle is observational with respect to database Revision authority. -/
structure ConnectionState where
  revision : Nat
  connected : Bool
  deriving DecidableEq, Repr

def disconnect (state : ConnectionState) : ConnectionState :=
  { state with connected := false }

theorem disconnect_preserves_revision (state : ConnectionState) :
    (disconnect state).revision = state.revision := by
  rfl

end CFMD.HostedBoundary
