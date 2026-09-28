namespace CFMD.SecurityAuthority

structure Grants where
  read : Bool
  historicalRead : Bool
  historyRead : Bool
  watch : Bool
  write : Bool
  deriving DecidableEq, Repr

/-- Product-value derivation carries the same authority; it never adds grants. -/
def deriveValue (grants : Grants) : Grants := grants

/-- Write authorization is exactly the trusted host-issued write grant. -/
def mayWrite (grants : Grants) : Bool := grants.write

/-- Read/history/watch value derivation cannot manufacture write authority. -/
theorem derived_value_preserves_grants (grants : Grants) :
    deriveValue grants = grants := by
  rfl

/-- A session without Write remains denied after arbitrary product-value derivation. -/
theorem no_write_grant_stays_denied (grants : Grants) (denied : grants.write = false) :
    mayWrite (deriveValue grants) = false := by
  simpa [deriveValue, mayWrite] using denied

/-- Conversely, authorization does not depend on a transport or endpoint label. -/
theorem write_authority_is_grant_exact (grants : Grants) :
    mayWrite grants = grants.write := by
  rfl

end CFMD.SecurityAuthority
