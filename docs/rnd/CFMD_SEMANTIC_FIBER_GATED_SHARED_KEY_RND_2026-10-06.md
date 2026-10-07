# CFMD semantic-fiber gated shared-key R&D — 2026-10-06

## Scope

Independent continuation of the retention-synthesis R&D line. Production routing/store code was not modified.

Official UTC start: **20:24:45**.  Functional source freeze: **20:43:23**.  Useful boundary: **20:44:45**.  Hard boundary: **20:48:45**.

This iteration answers two questions:

1. Is the new shared canonical row-key representation ready to be handed to production integration?
2. Does it preserve the non-regression requirement outside its favorable workload region?

## Decision

**Do not productionize the whole retention compiler yet.**

**The shared canonical-key lowering is now suitable for a narrow gated production-integration task as an additional physical point, not as a replacement for Quotient/SAMF.**

The required production shape is:

```text
SharedCanonicalKeyPool
+ SharedCanonicalKeyMass
+ SharedRowKeyRoute
```

The existing `DirectRowKey` point must remain available.  The advisor/firewall may select shared routing only inside a calibrated region or when the direct point cannot satisfy the hard memory budget.

## Hostile correction: shared payload alone is not maintenance-closed

The previous R&D model had:

```text
SharedRowKeyRoute => SharedCanonicalKeyPool
```

That is insufficient under arbitrary insert/remove churn.  If a row leaves a canonical key, the implementation must know when that row was the last owner; otherwise dead pool entries accumulate forever.

The exact maintenance closure is therefore at least:

```text
SharedRowKeyRoute
    => SharedCanonicalKeyPool
     + SharedCanonicalKeyMass
```

`SharedCanonicalKeyMass` is live-row mass / reference-count authority for the shared canonical tuple.  It is not needed to answer `RowCanonicalKey`, but it is required to keep the retained representation exact and bounded under deltas.

The executable retention algebra was updated accordingly.  This is the main correctness correction from this iteration.

## Fair release harness

The previous primitive shared-key probe started from a pre-deduplicated key pool.  That favored shared routing by giving interning for free.

A standalone release harness was added at:

```text
crates/kernel-semantics/examples/rnd_fiber_frontier.rs
```

Both sides now begin from the same logical canonical tuple stream.

Direct:

```text
row -> owned Vec<CanonicalEqKey>
```

Shared:

```text
content-addressed Arc<[CanonicalEqKey]> pool with live mass
row -> shared Arc identity
```

Shared build/update timings include content lookup, mass update, row-route mutation, and dead-key retirement logic.  This materially changes the frontier and prevents the R&D from declaring a false universal win.

## 4,096-row release frontier: shared is not universally faster

Five release repetitions were collected for `N=4096`, payload sizes 16/64/256/1024 bytes, and duplicate ratios 1x/4x/16x/128x.

Median shared/direct ratios illustrate the shape of the frontier:

```text
payload  dup     build      read       update
16 B     1x      1.96x      1.04x      2.76x
16 B   128x      1.29x      1.06x      1.36x

256 B    1x      1.32x      1.13x      2.69x
256 B   16x      1.19x      0.89x      1.97x
256 B  128x      1.03x      0.80x      1.37x

1024 B   1x      1.50x      0.95x      1.90x
1024 B  16x      1.13x      0.59x      1.43x
1024 B 128x      0.73x      0.68x      0.98x
```

Ratios below 1 favor shared.  These medians are local benchmark evidence, not hard architectural constants.  The important result is qualitative: after fair maintenance accounting, the representation has a real crossover rather than universal dominance.

Therefore replacing direct row-key storage globally would be a regression.

## Whole-state RSS frontier

A process-isolated RSS probe was used at `N=65,536`.  This includes persistent-map structure and allocator-visible retained state rather than only canonical user payload.

### 256-byte canonical key

Direct owned route:

```text
~26.0 MiB RSS delta
```

Shared:

```text
D=N       (1x reuse):   ~33.0 MiB   -- worse than direct
D=N/2     (2x reuse):   ~19.1 MiB
D=N/4     (4x reuse):   ~12.0 MiB
D=N/8     (8x reuse):    ~8.5 MiB
D=N/16   (16x reuse):    ~6.8 MiB
D=N/128 (128x reuse):    ~5.2 MiB
```

The whole-state memory crossover is already between 1x and 2x reuse in this fixture.

### 1 KiB canonical key

Direct owned route:

```text
~74.1 MiB RSS delta
```

Shared:

```text
D=N       (1x reuse):   ~84.4 MiB   -- worse than direct
D=N/2     (2x reuse):   ~44.8 MiB
D=N/4     (4x reuse):   ~24.8 MiB
D=N/8     (8x reuse):   ~14.8 MiB
D=N/16   (16x reuse):   ~10.0 MiB
D=N/128 (128x reuse):    ~5.8 MiB
```

Again, `D=N` is a protected direct-layout region; repeated keys rapidly move into a strong shared-layout memory region.

## Large-key strict Pareto region

The decisive hostile case is:

```text
N = 65,536 rows
D = 512 distinct canonical keys
reuse = 128x
canonical payload = 4 KiB
```

Three release repetitions:

```text
                    direct owned               shared pool+mass+route
build ns/row        8,876 .. 12,817             4,193 .. 6,629
row-key read ns     2,988 .. 4,783                 906 .. 1,290
update ns          12,553 .. 23,358             5,755 .. 5,952
RSS delta              ~266.1 MiB                  ~7.3 MiB
```

The intervals are non-overlapping on all three timed axes, and RSS is about **36.7x lower**.

This is the first measured region in this R&D line where the new physical point is not merely a memory/CPU tradeoff: it is a strict local Pareto improvement over the direct owned row-key representation for the measured operations.

### Lower reuse at 4 KiB

At 4x and 16x reuse, shared build/read are already strongly favorable in the local release runs, but update intervals remain noisy/overlapping.  The competitive firewall must therefore keep those regions unresolved until more stable end-to-end calibration exists.

## Non-regression consequence

The result is exactly the architecture the branch was trying to obtain:

```text
small/high-distinct canonical keys
    -> DirectRowKey remains protected

large/repeated canonical keys
    -> SharedCanonicalKeyPool + Mass + Route can dominate

projection-heavy workloads
    -> interned/SAMF-like resources remain separate candidates
```

No one representation is forced onto every workload.

A future production advisor can admit the new point conservatively:

1. keep current Quotient/SAMF/direct implementations as protected feasible references;
2. require calibrated read/build/maintenance envelopes;
3. use shared routing when it strictly dominates a protected point, or when hard memory budget makes the protected point infeasible;
4. fall back to the protected point whenever intervals overlap and memory pressure does not require a different layout.

## Production-readiness boundary

### Ready for gated integration

The following concept is mature enough to hand to the production branch for an **additional, non-default physical lowering**:

```text
SharedCanonicalKeyPool
SharedCanonicalKeyMass
SharedRowKeyRoute
```

Required integration properties:

- canonical tuple payload is owned once per distinct key;
- row route carries shared identity;
- live mass is exact and dead identities are retired;
- current direct Quotient row-key path remains available;
- no resolver rule such as "strongest representation first" may force the shared point globally;
- admission is protected by resource/performance envelopes.

### Not ready for production replacement

Do not yet ship:

- deletion of DirectRowKey / Quotient globally;
- full retention compiler as production authority;
- automatic removal of SAMF;
- uncalibrated synthesis from R&D-local timing constants;
- a global claim that shared routing is faster.

The remaining production-facing proof is an end-to-end `kernel-plan` benchmark after the lowering exists behind a gate.  The standalone harness is sufficient to justify creating that gated implementation task, but not sufficient to delete the protected baselines.

## Verification

Final source state:

```text
cargo test -p kernel-semantics --lib
91 passed / 0 failed / 1 ignored

cargo clippy -p kernel-semantics --lib -- -D warnings
PASS

cargo clippy -p kernel-semantics --example rnd_fiber_frontier -- -D warnings
PASS

cargo fmt --all -- --check
PASS
```

The ignored test is the explicit R&D primitive microbenchmark.

The full-suite run includes the exact `global backbone x per-slot` factorization/oracle comparison; it still passes after `SharedCanonicalKeyMass` was added to maintenance closure.

## Files changed

R&D algebra:

```text
crates/kernel-semantics/src/fiber_retention.rs
```

Release frontier harness:

```text
crates/kernel-semantics/examples/rnd_fiber_frontier.rs
```

Report:

```text
docs/rnd/CFMD_SEMANTIC_FIBER_GATED_SHARED_KEY_RND_2026-10-06.md
```

Production routing/store/durability remain unchanged.

## Next R&D line

The general compiler still needs one deeper step before production authority:

1. model joint-row ownership independently from row-key ownership;
2. calibrate composite arity and nested canonical keys, not only one-coordinate text payloads;
3. measure snapshot-overlap/COW transition peaks for direct/shared/interned hybrids;
4. derive a small stable set of measurable sufficient statistics (`N`, `D`, payload work/bytes, projection demand, churn, snapshot pressure) from which the advisor can choose a region without benchmarking online;
5. prove the factorized optimizer remains exact after these additional resource families.

The narrow shared-key lowering can be integrated in parallel behind the firewall while this broader R&D continues.
