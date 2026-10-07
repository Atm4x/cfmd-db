# R&D — Semantic-Fiber Retention Synthesis Beyond Named Profiles

Date: 2026-10-06

## Scope

This R&D starts **after** the accepted finite-carrier result from PASS571. It does not repeat PASS571's production ExactFibers replacement/frontier work. The question here is more fundamental:

> Are `Cardinality / Quotient / Observable` actually the minimal physical basis, or can physical representation itself be derived from the observations a workload needs and the measured resource costs?

The result is stronger than the original three-profile unification: the three named profiles are useful coarse compatibility points, but they are **not the physical lattice** and should not be the final architecture.

## 1. Starting semantic theorem remains valid

For one pinned semantic binding there is one canonical map

`kappa : X -> K1 x ... x Kn`.

PASS571's carrier theorem remains accepted: mass, exact fibers and coordinate projections are exact retained views of this one map. Nothing in this R&D introduces another semantic authority.

The new finding is that the physically useful observations of `kappa` do not form the old total chain.

## 2. Observation algebra

The smallest useful observation vocabulary found in the current kernel is:

- `JointMass(k) = |kappa^-1(k)|`;
- `JointRows(k) = kappa^-1(k)`;
- `RowCanonicalKey(x) = kappa(x)`;
- `SlotMass(i, c) = |{x | kappa_i(x)=c}|`;
- `SlotRows(i, c) = {x | kappa_i(x)=c}`.

There are exact implications:

`JointRows => JointMass`

`SlotRows => SlotMass`

But important pairs are incomparable:

- `RowCanonicalKey` does not imply `JointRows`, and `JointRows` does not imply `RowCanonicalKey`;
- `SlotRows` does not imply `JointRows`, and `JointRows` does not imply `SlotRows`.

Therefore the true observation order is a **partial order**, not

`ExactCardinality < QuotientFiber < ObservableFiber`.

The old three-item chain bundles several incomparable operators into named capability packages.

## 3. Repository evidence for over-bundling

### Quotient

Current `MaterializedSemanticQuotientFactorState` stores both:

- `buckets: canonical joint key -> ordered rows`;
- `reverse: row -> canonical joint key`.

However the QCN retained-factor consumer path asks the maintained factor for `semantic_quotient_single_key(row_id)` and uses it to avoid re-reading/re-canonicalizing the authoritative value. The physical quotient bucket is not the observation that this hot path asks for.

This means the current label `QuotientFiber` conflates at least:

1. row-to-canonical-key routing;
2. joint-key-to-row fiber retention;
3. cardinality derived from the bucket.

A workload requiring only (1) should not be forced to retain (2).

### Observable / SAMF

The observable state exposes separate exact operations:

- exact joint-value probe;
- per-slot value probe;
- per-slot count.

These operations likewise do not require one universal maximal representation. For example a slot-count-heavy workload can retain a direct slot mass map without retaining projected row membership.

### Why PASS571 `Measure` lost to specialized statistics

`FiniteFiberCarrier` performs coordinate-class interning for every profile. That is necessary/useful for some composite projected shapes, but it is not semantically necessary for joint cardinality. A direct canonical joint-mass map is a different exact realizer and avoids the codec/interner resource atoms. The PASS571 result that `Measure` was larger/slower than specialized statistics is therefore explained structurally, not as an accident of implementation.

## 4. Physical retention is an AND/OR derivation problem

Let `A` be a finite set of retainable physical resource atoms. An observation `o` is realized when **one** exact derivation rule for `o` has all required atoms present.

This is a monotone Boolean function in DNF form:

`R_o(P) = OR_{r in Rules(o)} AND_{a in r} [a in P]`.

The R&D prototype uses the following witness atom universe:

- `DirectJointMass`;
- `DirectJointRows`;
- `DirectRowKey`;
- `ClassEncode`;
- `ClassDecode`;
- `InternedJointRows`;
- `InternedRowRoute`;
- `ProjectionIncidence`;
- `DirectSlotMass`;
- `DirectSlotRows`.

Examples of exact alternative derivations:

`JointMass <- DirectJointMass`

or

`JointMass <- DirectJointRows`

or

`JointMass <- ClassEncode + InternedJointRows`.

Similarly:

`RowCanonicalKey <- DirectRowKey`

or

`RowCanonicalKey <- ClassDecode + InternedRowRoute`.

And:

`SlotRows <- DirectSlotRows`

or

`SlotRows <- ClassEncode + InternedJointRows + ProjectionIncidence`.

These are alternative exact implementations of the same observation, not semantic fallback branches.

## 5. Minimal-realizer theorem

For a required observation set `D`, define a plan `P subseteq A` to be a **realizer** iff every observation in `D` has an exact derivation from `P`.

A realizer is minimal iff no strict subset is also a realizer.

The minimal realizers form an antichain under set inclusion.

### Consequence for named profiles

Suppose a fixed family of named profiles is claimed to be universally physically optimal for every positive atom cost assignment and every workload demand.

Every inclusion-minimal realizer can be made uniquely optimal by assigning low positive costs to its atoms and sufficiently high costs to atoms outside it. Therefore a universally optimal fixed family must contain every inclusion-minimal realizer that can occur.

So once the exact derivation graph has more than three minimal realizers, no three-profile family can be universally optimal.

This is independent of current benchmark constants.

## 6. Executable constructive result

Added R&D module:

`crates/kernel-semantics/src/fiber_retention.rs`

It contains:

- exact observation/atom vocabulary;
- exact AND/OR realization rules;
- implication checking over the complete finite atom universe;
- enumeration of inclusion-minimal realizers;
- an exact workload/resource selector;
- retained-byte hard budget;
- read-work benefit vs build + maintenance work objective.

The selector enumerates all `2^10 = 1024` atom sets in the current R&D universe, so it is exact rather than heuristic.

Across all 31 non-empty subsets of the five observations, this witness derivation graph contains **38 distinct inclusion-minimal physical shapes**. One demand set has **14** alternative minimal exact realizers.

This does **not** mean CFMD should expose or name 38 profiles. It proves the opposite: naming physical profiles is the wrong abstraction. The shapes should be compiler/advisor output.

## 7. Cost synthesis

For a candidate atom set `P` the prototype computes:

`gross_saved_work(P) = sum saved_work(o)` for observations exactly realized by `P`.

Then:

`net_work(P) = gross_saved_work(P) - build_work(P) - maintenance_work(P)`.

Candidates exceeding the hard retained-byte budget are rejected. Ties prefer lower retained bytes and then deterministic atom-mask order.

This is deliberately compatible with the existing kernel split between work units and memory admission. Production should reuse the existing `ResourceFootprint` union model so shared reconstructible atoms are counted once across candidates.

## 8. Important architectural correction to the finite carrier

The accepted `FiniteFiberCarrier` remains useful as a theorem/prototype for one family of exact lowerings, but it should **not** become the mandatory maximal container from which all profiles are carved.

In particular:

- direct joint mass should be allowed to exist without coordinate interning;
- row-to-key routing should be allowed without joint row buckets;
- direct slot mass should be allowed without slot row fibers;
- coordinate interning should be selected when key reuse/composite projection economics justify it;
- direct canonical-key storage should remain a valid exact lowering when interning costs more than it saves.

So the long-term object is not `FiniteFiberCarrier { profile: enum }`.

The better abstraction is conceptually:

`SemanticFiber(binding)`

plus

`RetentionPlan { resource_atoms, exact_realizers }`.

## 9. Proposed production architecture (not implemented here)

### Semantic identity

One identity only:

`SemanticFiber(binding)`.

No semantic identity split by `Cardinality / Quotient / Observable`.

### Consumer contract

Consumers ask for concrete observations, e.g.:

- `JointMass`;
- `RowCanonicalKey`;
- `JointRows`;
- `SlotMass(slot)`;
- `SlotRows(slot)`.

### Compiled physical plan

Advisor/compiler emits a `FiberRetentionPlan` containing resource atoms plus an exact derivation witness for each admitted observation.

A witness is important: execution should not branch through semantically different fallback implementations. It executes one certified exact derivation selected for that plan.

### Maintenance

One delta compiler emits maintenance operations for the selected atoms. This preserves the accepted one-`kappa` transition law while avoiding maintenance for atoms the workload did not select.

### Replanning

A new plan is reconstructible. Build candidate resources, verify the observation witnesses, atomically publish the new plan, then release no-longer-needed resources. Shared persistent atoms can survive the transition.

### Hard dependencies vs accelerations

Most observations are accelerations because authoritative relation evaluation remains available. Some higher-level maintained artifacts may require an observation for their own incremental algorithm. Those should be represented as **mandatory observations**; the selector must choose a realizer or reject that higher-level artifact.

## 10. Relation to PASS571

PASS571 should continue its own production tasks unchanged. Its `ExactFibers` and `ProjectedFibers` work is still valuable because those structures become validated candidate realizers/resource atoms in this larger synthesis model.

This R&D specifically does **not** ask PASS571 to stop proving its frontier. It changes what should happen after those proofs: successful lowerings should feed a retention compiler rather than harden into permanent named profiles.

## 11. Verification

Executed against Rust 1.98.1:

- `cargo test -p kernel-semantics fiber_retention --lib`: **7 passed / 0 failed**;
- full `cargo test -p kernel-semantics --lib`: **73 passed / 0 failed**;
- `cargo clippy -p kernel-semantics --lib -- -D warnings`: **PASS**;
- `cargo fmt --all`: applied, resulting source is formatted.

Executable hostile properties include:

- observation implication is partial, not the old three-profile total chain;
- minimal realizers form an antichain;
- row-key-only workload does not retain a quotient row bucket;
- slot-mass-only workload does not retain projected row membership;
- joint+slot-row workload discovers the shared interned projection lowering;
- more than three optimal shapes are selected across simple workloads;
- exhaustive 31-demand-set enumeration yields 38 distinct minimal shapes and max antichain width 14 in the witness graph.

## 12. CLOSED / OPEN / REJECTED

### CLOSED

- The three named profiles are **not** the minimal physical basis.
- The real observation order is not a total chain.
- Exact physical realization is naturally an AND/OR derivation graph.
- Minimal exact realizers can be enumerated independently of cost.
- Workload/resource costs can then select a plan without introducing semantic alternatives.
- A working exact selector/proof prototype exists.

### OPEN — next R&D

1. Extract a production-grade atom vocabulary from the real statistics / quotient / SAMF / carrier layouts, including shared allocations rather than the deliberately simple witness atoms used here.
2. Model composite-slot selectivity: per-slot observation demand should be indexed by slot/subset rather than one aggregate `SlotRows` symbol.
3. Model mutation shape explicitly: insert/remove frequency and key churn should affect atom maintenance cost.
4. Add snapshot/path-copy marginal cost, not only steady-state retained bytes.
5. Derive a deterministic delta-maintenance program from a selected atom set and prove that every atom transition is the image of the same `kappa` delta.
6. Determine the cutover threshold where exact enumeration should become branch-and-bound / Pareto dynamic programming as the atom universe grows.
7. Integrate existing `ResourceFootprint` shared-atom accounting into the R&D selector instead of scalar per-atom byte sums.

### REJECTED

- Treating `Cardinality / Quotient / Observable` as the final physical lattice.
- Replacing those three names with a larger fixed enum of profiles.
- One maximal carrier that always pays for coordinate interning.
- Selecting representation by semantic family name.
- Semantically different fallback branches inside one observation.
- Deleting the validated PASS571 lowerings; they should become candidate realizers.

## 13. R&D decision

**Continue toward retention synthesis / profile elimination.**

The finite-carrier theorem solved semantic unification. The next architectural step is to make physical retention a compiled consequence of observation demand and resource economics, not another hand-maintained profile taxonomy.
