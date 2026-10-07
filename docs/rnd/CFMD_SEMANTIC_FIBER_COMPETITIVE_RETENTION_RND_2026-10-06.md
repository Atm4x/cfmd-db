# CFMD Semantic-Fiber Competitive Retention R&D — 2026-10-06

## Status

R&D continuation only. No production routing, durable format, advisor publication, or PASS571 production replacement was changed.

Official UTC start: **19:16:36**.

Functional source freeze: **19:36:41**. Useful boundary: **19:36:36**. Hard boundary: **19:40:36**. The source freeze landed five seconds after the useful target while terminating/checking the final verification command; no source edits were made after freeze.

This iteration answers the hostile question: can retention synthesis preserve the fastest existing specialization instead of trading a local latency regression for global architectural uniformity?

## Executive result

Yes, the architecture can be made non-regressive by construction rather than by hope.

The previous retention compiler already showed that the physical representation space is richer than three named profiles. This iteration adds a **competitive performance firewall** around that compiler:

1. existing specialized implementations are represented as protected feasible reference plans;
2. read cost is attached to an exact derivation witness, not to an observation globally;
3. measured read work is an interval `[lower, upper]`, not a falsely exact scalar;
4. a synthesized plan may replace a protected plan only when its conservative read bound is no worse than the protected plan's optimistic bound;
5. the same synthesized plan must Pareto-dominate at least one protected feasible reference in build work, lifecycle maintenance work, snapshot peak, and atomic transition peak;
6. its conservative lifecycle net-work score must also be no worse than the best optimistic protected reference;
7. if measurement intervals overlap and dominance is not proved, a protected plan remains admissible and wins instead of forcing synthesis.

This means profile elimination does **not** require deleting a specialized fast path before a better lowering is proved.

## 1. Important correction: observation cost belongs to the realizer

The previous model stored `retained_work` on `FiberObservationWorkload`.

That silently assumed every exact derivation of one observation had the same probe cost. Production code disproves that assumption.

For example, exact `RowCanonicalKey` can be supplied by:

- a direct row -> canonical-key route;
- a row -> joint-class route followed by signature/class decoding.

Both are semantically exact, but their read paths are different.

The R&D model now has:

`FiberRealizerKey { observation, resources } -> FiberReadWorkEnvelope { lower, upper }`.

The compiled witness is selected by read cost among exact realizers retained by the plan. Physical resource count is only a deterministic tie-breaker.

### Consequence

A memory-cheaper realizer can no longer masquerade as latency-equivalent merely because it answers the same semantic observation.

## 2. Production audit: stronger representation is not necessarily faster

The current `PhysicalStore::semantic_quotient_single_key(...)` resolves SAMF first and quotient second.

The quotient single-key path is structurally:

`reverse.get(row_id) -> key.first() -> clone`.

The SAMF single-key path is structurally:

`row_to_atom.get(row_id)`
`-> atoms.get(atom)`
`-> signature`
`-> class_record(class)` for each coordinate
`-> clone canonical coordinate key(s)`.

This is a code-path operation count, not a nanosecond benchmark. It is nevertheless enough to reject the assumption that a stronger capability owner is automatically the fastest implementation of every weaker observation.

The same issue motivates a retention compiler: retain a direct row-key route when that observation is hot even if projection machinery is also retained for other observations.

## 3. Production audit correction: global cardinality != keyed joint mass

PASS569's exact cardinality capability is `(row_count, distinct_key_count)`, not a lookup of `mass(k)` for one canonical joint key.

The R&D model previously conflated those facts under `JointMass`.

This iteration adds `FiberObservationKey::GlobalCardinality` separately from keyed `JointMass`.

Exact global cardinality can be observed from:

- direct joint-mass state;
- direct joint-row fibers;
- interned joint-row fibers.

For retained fibers this observation does **not** require canonical-key lookup. Maintenance closure may still require interning resources to keep those fibers exact under arbitrary delta, but the read witness itself is O(1)-style metadata/fiber-map size observation rather than a key-probe path.

This correction matters both for cost calibration and for proving non-regression against current statistics/SAMF cardinality consumers.

## 4. Competitive performance firewall

The new R&D API introduces protected reference plans and a guarded compiler configuration.

A protected reference is a physical resource set corresponding to an existing implementation or otherwise trusted fast path.

Protected references are explicitly appended to the feasible candidate set. Therefore synthesis cannot make an old implementation unreachable merely by changing the optimizer search space.

### Read guard

For observation `q`, let a protected realizer have measured interval:

`R_q in [R_lower, R_upper]`.

Let a synthesized realizer have:

`S_q in [S_lower, S_upper]`.

A synthesized candidate is allowed to displace protected execution only if:

`S_upper <= R_lower`

for all requested observations of at least one protected feasible reference (plus the aggregate protected ceiling check).

Thus an overlapping interval is treated as **not proved**, not as a speedup.

### Physical-axis Pareto guard

For the same protected reference `R`, synthesized `S` must satisfy:

`build(S) <= build(R)`

`maintenance(S) <= maintenance(R)`

`peak(S) <= peak(R)`

`transition_peak(S) <= transition_peak(R)`.

The read guard above is checked per requested observation.

In addition:

`conservative_net(S) >= best_optimistic_protected_net`.

This is intentionally strict R&D policy. A future production policy may explicitly permit controlled tradeoffs, but it must do so as policy rather than accidentally hiding them inside one scalar score.

## 5. What the guard proves and what it does not

Under the supplied cost measurements/envelopes and lifecycle assumptions:

- a synthesized plan cannot replace a protected plan through an unmeasured read-latency tradeoff;
- it cannot replace it while being worse on build, maintenance, snapshot peak, or transition peak relative to the protected reference it claims to dominate;
- uncertainty defaults to retaining the protected implementation;
- a specialized fast path can remain in the feasible set even after named profile identity disappears from semantic routing.

It does **not** prove that a calibration measured on one CPU/key distribution is valid on another. Real production admission still needs representative calibration dimensions such as key width, nested canonical payload, `N/D`, arity, cache behavior, read/write ratio, and live-snapshot pressure.

The architecture now has a safe response to that uncertainty: widen the envelope and keep the protected plan until dominance is established.

## 6. Executable non-regression cases

New tests prove three critical behaviors.

### Unknown/overlapping performance keeps the old fast path

A synthesized slot-row representation is much smaller in retained bytes, but its read interval is `[1, 3]` while protected direct slot rows are exactly `1`.

The synthesized layout is rejected; protected direct rows remain selected.

### Measured strict dominance allows replacement

Protected direct slot rows have exact read work `5` while the synthesized interned route has conservative upper bound `3`, with no worse lifecycle axes and lower retained footprint.

The synthesized layout is admitted and selected.

### Coarse legacy points remain a reachable envelope

Cardinality, Quotient, and Observable resource closures are supplied as protected references. The selected result remains no worse than the protected envelope on requested read observations and lifecycle objective.

This is the migration strategy for eventual profile elimination: old profiles become protected physical points, not semantic authorities.

## 7. Exact structured decomposition

The previous R&D concluded that unrestricted AND/OR retention optimization is NP-hard in general, but CFMD's semantic-fiber graph has a small global backbone plus repeated per-slot choices.

This iteration implements:

`factorized_minimal_realizers(required, arity)`.

It partitions observations into:

- global backbone observations;
- independent observation groups keyed by slot.

Each group is solved exactly. Group plans are composed and maintenance closure is re-applied after every composition, so shared interning dependencies that couple slots are not incorrectly treated as free.

### Exhaustive oracle equivalence

The factorized realizer frontier was compared against the original exact DNF oracle for **every non-empty demand set** through arity 4.

Demand counts:

- arity 1: 31;
- arity 2: 127;
- arity 3: 511;
- arity 4: 2,047.

Total: **2,716 complete demand sets**.

For every one:

`factorized_minimal_realizers == production_minimal_realizers`.

The arity-1..4 exhaustive equality test completed in about **21.09 s** in the current debug test build.

This is executable finite evidence, not yet a Lean/general proof. The algebra strongly suggests a proof based on monotone/idempotent maintenance closure plus associative union/composition, but that proof was not claimed in this iteration.

## 8. Why this changes the risk assessment

Before this iteration, profile elimination had a plausible failure mode:

> the compiler chooses a more general/memory-efficient structure and silently slows a hot specialized observation.

That failure mode is now representable and rejectable in the architecture.

The target is no longer "one structure that is hopefully good everywhere".

It is:

`existing fast points + synthesized points -> protected competitive feasible set`.

A specialized implementation can disappear only after another physical shape proves dominance for the workloads/lifecycle region where the specialization used to win.

This provides a path to remove family identity without throwing away family-specific performance.

## 9. Current strongest opportunity

The production audit makes `RowCanonicalKey` especially interesting.

Current quotient has a naturally cheap reverse-map path. Current SAMF is a stronger representation but reconstructs canonical key material through atom/class indirection. If projection observations and row-key observations are simultaneously hot, a synthesized plan can retain:

- SAMF-like/interned projection backbone where useful;
- a direct/shared row-key route as an additional small atom where it wins.

That hybrid point is not naturally represented by the old three-profile chain. It may outperform "SAMF owns everything" without giving up SAMF's projection strengths.

This needs measurement, not assertion.

## 10. Verification

After the final refactor:

- pre-final-regression `cargo test -p kernel-semantics fiber_retention --lib`: **21 passed / 0 failed**;
- final added `GlobalCardinality` read/maintenance-separation regression: **targeted PASS**;
- last completed full `cargo test -p kernel-semantics --lib` before that final test-only addition: **88 passed / 0 failed**;
- final attempted full sweep started **89 tests** but the command timeout interrupted it before completion, so no green full-suite claim is made for that last invocation;
- exhaustive factorization-vs-exact test through arity 4: **PASS**, 2,716 demand sets;
- final `cargo clippy -p kernel-semantics --lib -- -D warnings`: **PASS**;
- final `cargo fmt --all -- --check`: **PASS**.

No production `kernel-plan` source was modified.

## 11. Immediate next R&D

The next iteration should stop using synthetic read-work numbers for the protected envelope and calibrate real paths:

1. benchmark current statistics global-cardinality observation;
2. benchmark quotient direct `row -> single canonical key`;
3. benchmark SAMF `row -> canonical key` reconstruction across arity/key shapes;
4. benchmark joint probe and per-slot projection probe;
5. add the direct **shared canonical joint-key** physical family (fiber-owned `Arc<KeyTuple>` / row route) separately from coordinate-class interning;
6. feed measured confidence envelopes into the competitive selector;
7. build a factorized **cost** optimizer (not only factorized minimal-realizer enumeration) and compare it against the exact guarded oracle on small arity.

If synthesized shapes cannot strictly dominate protected points, they do not replace them. That is now an architectural rule of the R&D line rather than an informal caution.

## Decision

Continue the branch.

The key result of this iteration is not a claim that every workload is faster. It is stronger for architecture: the representation compiler can be designed so that **no existing specialized performance point needs to be sacrificed in order to gain synthesized layouts elsewhere**. The remaining challenge is empirical calibration and scalable cost selection, not semantic correctness or forced uniformity.
