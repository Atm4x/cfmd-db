/-
CFMD surface-to-kernel preservation model.

This file is intentionally dependency-free (Lean 4 core only).  It models the
normative surface vocabulary from CFMD_IDEAL_DB_SPEC.md and the exact relational
operator vocabulary implemented by kernel-query::RelExpr / kernel-plan::Plan.
The companion check_surface_refinement.py binds these constructors and the
round-trip checker back to production Rust source.
-/

namespace CFMD.SurfaceKernel

abbrev Id := Nat
abbrev Column := Nat
abbrev ValueToken := Nat

inductive Scalar where
  | unit | bool | i64 | f64 | text
  | liveRef (entity : Id)
  | historicalId (entity : Id)
  deriving DecidableEq, Repr

inductive SurfaceType where
  | scalar (s : Scalar)
  | product (left right : SurfaceType)
  | sum (left right : SurfaceType)
  | option (child : SurfaceType)
  | set (element : SurfaceType) (equivalence : Id)
  | bag (element : SurfaceType) (equivalence : Id)
  | seq (element : SurfaceType)
  | map (key value : SurfaceType) (keyEquivalence : Id)
  | var (binder : Nat)
  | mu (binder : Nat) (body : SurfaceType)
  deriving DecidableEq, Repr

inductive KernelType where
  | scalar (s : Scalar)
  | product (left right : KernelType)
  | sum (left right : KernelType)
  | option (child : KernelType)
  | set (element : KernelType) (equivalence : Id)
  | bag (element : KernelType) (equivalence : Id)
  | seq (element : KernelType)
  | map (key value : KernelType) (keyEquivalence : Id)
  | var (binder : Nat)
  | mu (binder : Nat) (body : KernelType)
  deriving DecidableEq, Repr

def elaborateType : SurfaceType → KernelType
  | .scalar s => .scalar s
  | .product l r => .product (elaborateType l) (elaborateType r)
  | .sum l r => .sum (elaborateType l) (elaborateType r)
  | .option child => .option (elaborateType child)
  | .set element eq => .set (elaborateType element) eq
  | .bag element eq => .bag (elaborateType element) eq
  | .seq element => .seq (elaborateType element)
  | .map key value eq => .map (elaborateType key) (elaborateType value) eq
  | .var binder => .var binder
  | .mu binder body => .mu binder (elaborateType body)

def SurfaceType.weight : SurfaceType → Nat
  | .scalar _ | .var _ => 1
  | .product l r | .sum l r | .map l r _ => 1 + l.weight + r.weight
  | .option x | .set x _ | .bag x _ | .seq x | .mu _ x => 1 + x.weight

def KernelType.weight : KernelType → Nat
  | .scalar _ | .var _ => 1
  | .product l r | .sum l r | .map l r _ => 1 + l.weight + r.weight
  | .option x | .set x _ | .bag x _ | .seq x | .mu _ x => 1 + x.weight

/-- Type elaboration preserves every constructor occurrence exactly. -/
theorem surface_type_weight_preserved (t : SurfaceType) :
    (elaborateType t).weight = t.weight := by
  induction t with
  | scalar => rfl
  | product l r ihL ihR => simp [elaborateType, SurfaceType.weight, KernelType.weight, ihL, ihR]
  | sum l r ihL ihR => simp [elaborateType, SurfaceType.weight, KernelType.weight, ihL, ihR]
  | option x ih => simp [elaborateType, SurfaceType.weight, KernelType.weight, ih]
  | set x e ih => simp [elaborateType, SurfaceType.weight, KernelType.weight, ih]
  | bag x e ih => simp [elaborateType, SurfaceType.weight, KernelType.weight, ih]
  | seq x ih => simp [elaborateType, SurfaceType.weight, KernelType.weight, ih]
  | map k v e ihK ihV => simp [elaborateType, SurfaceType.weight, KernelType.weight, ihK, ihV]
  | var => rfl
  | mu b x ih => simp [elaborateType, SurfaceType.weight, KernelType.weight, ih]

/- The recursion obligation is explicit: a recursive occurrence must be under a
   data constructor after crossing its binder.  This mirrors TypeExpr::validate. -/
def SurfaceType.wellFormed : SurfaceType → List Nat → Bool → Bool
  | .scalar _, _, _ => true
  | .var v, bound, guarded => bound.contains v && guarded
  | .mu b body, bound, _ => body.wellFormed (b :: bound) false
  | .product l r, bound, _ | .sum l r, bound, _ =>
      l.wellFormed bound true && r.wellFormed bound true
  | .option x, bound, _ | .set x _, bound, _ | .bag x _, bound, _ | .seq x, bound, _ =>
      x.wellFormed bound true
  | .map k v _, bound, _ => k.wellFormed bound true && v.wellFormed bound true

def KernelType.wellFormed : KernelType → List Nat → Bool → Bool
  | .scalar _, _, _ => true
  | .var v, bound, guarded => bound.contains v && guarded
  | .mu b body, bound, _ => body.wellFormed (b :: bound) false
  | .product l r, bound, _ | .sum l r, bound, _ =>
      l.wellFormed bound true && r.wellFormed bound true
  | .option x, bound, _ | .set x _, bound, _ | .bag x _, bound, _ | .seq x, bound, _ =>
      x.wellFormed bound true
  | .map k v _, bound, _ => k.wellFormed bound true && v.wellFormed bound true

/-- Guarded-recursion/free-variable admission is unchanged by elaboration. -/
theorem surface_type_wellformed_preserved
    (t : SurfaceType) (bound : List Nat) (guarded : Bool) :
    (elaborateType t).wellFormed bound guarded = t.wellFormed bound guarded := by
  induction t generalizing bound guarded with
  | scalar => rfl
  | product l r ihL ihR =>
      simp [elaborateType, KernelType.wellFormed, SurfaceType.wellFormed, ihL, ihR]
  | sum l r ihL ihR =>
      simp [elaborateType, KernelType.wellFormed, SurfaceType.wellFormed, ihL, ihR]
  | option x ih => simp [elaborateType, KernelType.wellFormed, SurfaceType.wellFormed, ih]
  | set x e ih => simp [elaborateType, KernelType.wellFormed, SurfaceType.wellFormed, ih]
  | bag x e ih => simp [elaborateType, KernelType.wellFormed, SurfaceType.wellFormed, ih]
  | seq x ih => simp [elaborateType, KernelType.wellFormed, SurfaceType.wellFormed, ih]
  | map k v e ihK ihV =>
      simp [elaborateType, KernelType.wellFormed, SurfaceType.wellFormed, ihK, ihV]
  | var => rfl
  | mu b x ih => simp [elaborateType, KernelType.wellFormed, SurfaceType.wellFormed, ih]

inductive SurfaceFeature where
  | entityClass | recordValueObject | closedEnum | optional | sequence
  | set | multiset | map | recursiveDocument | reference
  | interfaceCapability | inheritance | computedProperty
  | businessConstraint | transactionMethod
  deriving DecidableEq, Repr

inductive KernelWitness where
  | nominalCarrier | product | sum | option | seq | set | bag | map | guardedMu
  | typedRef | capabilityInclusion | coherentInclusion | queryView
  | violationQuery | typedRewrite
  deriving DecidableEq, Repr

def elaborateFeature : SurfaceFeature → KernelWitness
  | .entityClass => .nominalCarrier
  | .recordValueObject => .product
  | .closedEnum => .sum
  | .optional => .option
  | .sequence => .seq
  | .set => .set
  | .multiset => .bag
  | .map => .map
  | .recursiveDocument => .guardedMu
  | .reference => .typedRef
  | .interfaceCapability => .capabilityInclusion
  | .inheritance => .coherentInclusion
  | .computedProperty => .queryView
  | .businessConstraint => .violationQuery
  | .transactionMethod => .typedRewrite

def surfaceOfWitness : KernelWitness → SurfaceFeature
  | .nominalCarrier => .entityClass
  | .product => .recordValueObject
  | .sum => .closedEnum
  | .option => .optional
  | .seq => .sequence
  | .set => .set
  | .bag => .multiset
  | .map => .map
  | .guardedMu => .recursiveDocument
  | .typedRef => .reference
  | .capabilityInclusion => .interfaceCapability
  | .coherentInclusion => .inheritance
  | .queryView => .computedProperty
  | .violationQuery => .businessConstraint
  | .typedRewrite => .transactionMethod

theorem surface_witness_left_inverse (f : SurfaceFeature) :
    surfaceOfWitness (elaborateFeature f) = f := by
  cases f <;> rfl

theorem surface_witness_right_inverse (w : KernelWitness) :
    elaborateFeature (surfaceOfWitness w) = w := by
  cases w <;> rfl

/-- The normative surface table is complete and one-to-one with trusted witnesses. -/
theorem surface_feature_bijective :
    Function.Injective elaborateFeature ∧ Function.Surjective elaborateFeature := by
  constructor
  · intro a b h
    have := congrArg surfaceOfWitness h
    simpa [surface_witness_left_inverse] using this
  · intro w
    exact ⟨surfaceOfWitness w, surface_witness_right_inverse w⟩

inductive Aggregate where
  | count | sumI64 | minI64 | maxI64
  deriving DecidableEq, Repr

inductive Direction where | asc | desc deriving DecidableEq, Repr

inductive SurfaceQuery where
  | scan (relation : Id)
  | filterConst (input : SurfaceQuery) (column : Column) (value : ValueToken) (equivalence : Id)
  | filterColumns (input : SurfaceQuery) (left right : Column) (equivalence : Id)
  | project (input : SurfaceQuery) (columns : List Column)
  | join (left right : SurfaceQuery) (leftCol rightCol : Column) (equivalence : Id)
  | difference (left right : SurfaceQuery)
  | antiJoin (left right : SurfaceQuery) (leftCol rightCol : Column) (equivalence : Id)
  | distinct (input : SurfaceQuery) (equivalences : List Id)
  | group (input : SurfaceQuery) (columns : List Column) (equivalences : List Id) (agg : Aggregate)
  | topK (input : SurfaceQuery) (column : Column) (ordering : Id) (direction : Direction) (k : Nat)
  | promoteToBag (input : SurfaceQuery)
  deriving DecidableEq, Repr

inductive KernelQuery where
  | scan (relation : Id)
  | filterConst (input : KernelQuery) (column : Column) (value : ValueToken) (equivalence : Id)
  | filterColumns (input : KernelQuery) (left right : Column) (equivalence : Id)
  | project (input : KernelQuery) (columns : List Column)
  | join (left right : KernelQuery) (leftCol rightCol : Column) (equivalence : Id)
  | difference (left right : KernelQuery)
  | antiJoin (left right : KernelQuery) (leftCol rightCol : Column) (equivalence : Id)
  | distinct (input : KernelQuery) (equivalences : List Id)
  | group (input : KernelQuery) (columns : List Column) (equivalences : List Id) (agg : Aggregate)
  | topK (input : KernelQuery) (column : Column) (ordering : Id) (direction : Direction) (k : Nat)
  | promoteToBag (input : KernelQuery)
  deriving DecidableEq, Repr

def elaborateQuery : SurfaceQuery → KernelQuery
  | .scan r => .scan r
  | .filterConst q c v e => .filterConst (elaborateQuery q) c v e
  | .filterColumns q l r e => .filterColumns (elaborateQuery q) l r e
  | .project q cs => .project (elaborateQuery q) cs
  | .join l r lc rc e => .join (elaborateQuery l) (elaborateQuery r) lc rc e
  | .difference l r => .difference (elaborateQuery l) (elaborateQuery r)
  | .antiJoin l r lc rc e => .antiJoin (elaborateQuery l) (elaborateQuery r) lc rc e
  | .distinct q es => .distinct (elaborateQuery q) es
  | .group q cs es a => .group (elaborateQuery q) cs es a
  | .topK q c o d k => .topK (elaborateQuery q) c o d k
  | .promoteToBag q => .promoteToBag (elaborateQuery q)

inductive ScanAlgorithm where | row | columnar deriving DecidableEq, Repr
inductive GroupAlgorithm where | generic | denseI64 deriving DecidableEq, Repr
inductive TopKAlgorithm where | generic | maintained deriving DecidableEq, Repr

inductive Plan where
  | scan (relation : Id) (algorithm : ScanAlgorithm)
  | filterConst (input : Plan) (column : Column) (value : ValueToken) (equivalence : Id)
  | filterColumns (input : Plan) (left right : Column) (equivalence : Id)
  | project (input : Plan) (columns : List Column)
  | join (left right : Plan) (leftCol rightCol : Column) (equivalence : Id)
  | difference (left right : Plan)
  | antiJoin (left right : Plan) (leftCol rightCol : Column) (equivalence : Id)
  | distinct (input : Plan) (equivalences : List Id)
  | group (input : Plan) (columns : List Column) (equivalences : List Id) (agg : Aggregate) (algorithm : GroupAlgorithm)
  | topK (input : Plan) (column : Column) (ordering : Id) (direction : Direction) (k : Nat) (algorithm : TopKAlgorithm)
  | promoteToBag (input : Plan)
  deriving DecidableEq, Repr

def lower : KernelQuery → Plan
  | .scan r => .scan r .row
  | .filterConst q c v e => .filterConst (lower q) c v e
  | .filterColumns q l r e => .filterColumns (lower q) l r e
  | .project q cs => .project (lower q) cs
  | .join l r lc rc e => .join (lower l) (lower r) lc rc e
  | .difference l r => .difference (lower l) (lower r)
  | .antiJoin l r lc rc e => .antiJoin (lower l) (lower r) lc rc e
  | .distinct q es => .distinct (lower q) es
  | .group q cs es a => .group (lower q) cs es a .generic
  | .topK q c o d k => .topK (lower q) c o d k .generic
  | .promoteToBag q => .promoteToBag (lower q)

def erase : Plan → KernelQuery
  | .scan r _ => .scan r
  | .filterConst q c v e => .filterConst (erase q) c v e
  | .filterColumns q l r e => .filterColumns (erase q) l r e
  | .project q cs => .project (erase q) cs
  | .join l r lc rc e => .join (erase l) (erase r) lc rc e
  | .difference l r => .difference (erase l) (erase r)
  | .antiJoin l r lc rc e => .antiJoin (erase l) (erase r) lc rc e
  | .distinct q es => .distinct (erase q) es
  | .group q cs es a _ => .group (erase q) cs es a
  | .topK q c o d k _ => .topK (erase q) c o d k
  | .promoteToBag q => .promoteToBag (erase q)

def KernelQuery.nodeCount : KernelQuery → Nat
  | .scan _ => 1
  | .filterConst q .. | .filterColumns q .. | .project q .. | .distinct q .. |
    .group q .. | .topK q .. | .promoteToBag q => 1 + q.nodeCount
  | .join l r .. | .difference l r | .antiJoin l r .. => 1 + l.nodeCount + r.nodeCount

def Plan.nodeCount : Plan → Nat
  | .scan .. => 1
  | .filterConst q .. | .filterColumns q .. | .project q .. | .distinct q .. |
    .group q .. | .topK q .. | .promoteToBag q => 1 + q.nodeCount
  | .join l r .. | .difference l r | .antiJoin l r .. => 1 + l.nodeCount + r.nodeCount

/-- Physical annotations/access paths erase to the exact elaborated logical query. -/
theorem lower_roundtrip (q : KernelQuery) : erase (lower q) = q := by
  induction q with
  | scan => rfl
  | filterConst q c v e ih => simp [lower, erase, ih]
  | filterColumns q l r e ih => simp [lower, erase, ih]
  | project q cs ih => simp [lower, erase, ih]
  | join l r lc rc e ihL ihR => simp [lower, erase, ihL, ihR]
  | difference l r ihL ihR => simp [lower, erase, ihL, ihR]
  | antiJoin l r lc rc e ihL ihR => simp [lower, erase, ihL, ihR]
  | distinct q es ih => simp [lower, erase, ih]
  | group q cs es a ih => simp [lower, erase, ih]
  | topK q c o d k ih => simp [lower, erase, ih]
  | promoteToBag q ih => simp [lower, erase, ih]

/-- Lowering cannot smuggle hidden physical logical nodes into a certified plan. -/
theorem lower_node_count (q : KernelQuery) : (lower q).nodeCount = q.nodeCount := by
  induction q with
  | scan => rfl
  | filterConst q c v e ih => simp [lower, Plan.nodeCount, KernelQuery.nodeCount, ih]
  | filterColumns q l r e ih => simp [lower, Plan.nodeCount, KernelQuery.nodeCount, ih]
  | project q cs ih => simp [lower, Plan.nodeCount, KernelQuery.nodeCount, ih]
  | join l r lc rc e ihL ihR => simp [lower, Plan.nodeCount, KernelQuery.nodeCount, ihL, ihR]
  | difference l r ihL ihR => simp [lower, Plan.nodeCount, KernelQuery.nodeCount, ihL, ihR]
  | antiJoin l r lc rc e ihL ihR => simp [lower, Plan.nodeCount, KernelQuery.nodeCount, ihL, ihR]
  | distinct q es ih => simp [lower, Plan.nodeCount, KernelQuery.nodeCount, ih]
  | group q cs es a ih => simp [lower, Plan.nodeCount, KernelQuery.nodeCount, ih]
  | topK q c o d k ih => simp [lower, Plan.nodeCount, KernelQuery.nodeCount, ih]
  | promoteToBag q ih => simp [lower, Plan.nodeCount, KernelQuery.nodeCount, ih]

/--
Soundness theorem matching production `LoweringChecker`: *any* physical plan
accepted by the two certificate obligations (exact logical erasure and no hidden
logical nodes) preserves the surface query meaning.  The proof is independent
of which physical algorithms/catalog choices produced the plan.
-/
theorem checked_plan_preserves_surface_semantics
    {α : Type} (eval : KernelQuery → α) (q : SurfaceQuery) (p : Plan)
    (exactRoundTrip : erase p = elaborateQuery q) :
    eval (erase p) = eval (elaborateQuery q) := by
  rw [exactRoundTrip]

theorem checked_plan_certificate_sound
    {α : Type} (eval : KernelQuery → α) (q : SurfaceQuery) (p : Plan)
    (exactRoundTrip : erase p = elaborateQuery q)
    (noHiddenExpansion : p.nodeCount = (elaborateQuery q).nodeCount) :
    eval (erase p) = eval (elaborateQuery q) ∧
      p.nodeCount = (elaborateQuery q).nodeCount := by
  exact ⟨checked_plan_preserves_surface_semantics eval q p exactRoundTrip,
    noHiddenExpansion⟩

/-- Exact AST preservation implies semantic preservation for any logical evaluator. -/
theorem surface_query_semantics_preserved
    {α : Type} (eval : KernelQuery → α) (q : SurfaceQuery) :
    eval (erase (lower (elaborateQuery q))) = eval (elaborateQuery q) := by
  rw [lower_roundtrip]

/-- Surface lowering also preserves exact logical node count. -/
theorem surface_query_no_hidden_expansion (q : SurfaceQuery) :
    (lower (elaborateQuery q)).nodeCount = (elaborateQuery q).nodeCount :=
  lower_node_count (elaborateQuery q)

/--
Top-level #20 preservation theorem: the complete normative feature vocabulary is
covered; type elaboration preserves admission; and relational surface queries
preserve both meaning and logical shape through physical lowering.
-/
theorem surface_to_kernel_preservation
    {α : Type} (eval : KernelQuery → α) (q : SurfaceQuery)
    (t : SurfaceType) (bound : List Nat) (guarded : Bool) :
    eval (erase (lower (elaborateQuery q))) = eval (elaborateQuery q) ∧
    (lower (elaborateQuery q)).nodeCount = (elaborateQuery q).nodeCount ∧
    (elaborateType t).wellFormed bound guarded = t.wellFormed bound guarded ∧
    (elaborateType t).weight = t.weight := by
  exact ⟨surface_query_semantics_preserved eval q,
    surface_query_no_hidden_expansion q,
    surface_type_wellformed_preserved t bound guarded,
    surface_type_weight_preserved t⟩

end CFMD.SurfaceKernel
