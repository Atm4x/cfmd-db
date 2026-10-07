# CFMD semantic-fiber shared-key frontier R&D — 2026-10-06

## Scope

Independent R&D continuation of the semantic-fiber retention compiler. PASS571 production routing/replacement work is intentionally not modified here.

Official UTC start: **19:41:23**.
Useful boundary: **20:01:23**.
Hard boundary: **20:05:23**.

The explicit question for this iteration was whether the compiler can preserve specialized performance on hostile workloads while still finding physical shapes that are materially better than the coarse Statistics / Quotient / SAMF points.

## New physical point: shared canonical row route

The previous compiler had two extremes for `RowCanonicalKey`:

1. `DirectRowKey` — one owned canonical tuple per row, close to the current quotient reverse map;
2. interned SAMF-style route — row -> joint class -> coordinate classes -> canonical class records.

This iteration adds a third exact realizer:

```text
SharedCanonicalKeyPool + SharedRowKeyRoute
```

Semantics:

```text
one canonical tuple payload per distinct joint key
row -> shared canonical tuple identity
```

It does **not** require coordinate class dictionaries, joint-signature dictionaries, or projection incidence.

Maintenance closure is explicit:

```text
SharedRowKeyRoute => SharedCanonicalKeyPool
```

The observation theorem remains exact: all three implementations realize the same `RowCanonicalKey` fact. The new point changes only storage/read mechanics.

## Deterministic payload frontier

For `N` rows, `D` distinct canonical tuples, arity `A`, and canonical payload size `K`:

Current quotient-like owned reverse payload is approximately proportional to:

```text
N * A * K
```

for the reverse side alone (in addition to keys retained by the forward bucket map).

A shared canonical row route retains canonical payload approximately proportional to:

```text
D * A * K
```

plus `N` shared identities/references.

Therefore payload compaction is approximately `N / D` when duplicate fibers are common. This is an exact structural distinction, not a timing estimate.

Probe fixture used in this iteration:

```text
N = 4096
D = 257
N / D ~= 15.94
```

For one 256-byte text coordinate, canonical text payload represented by the probe was:

```text
owned row keys:   4096 * 256 = 1,048,576 bytes
shared key pool:   257 * 256 =    65,792 bytes
```

This excludes map nodes, Vec/Arc headers and allocator overhead; it is intentionally only the canonical user-payload component. The payload ratio is still exact for the fixture.

## Primitive read-path calibration

A `kernel-semantics` ignored R&D microprobe was added. It measures the actual `PersistentOrdMap` access primitives and the real `SupportAtomFabric + RevisionObservableCatalog` reconstruction path used by SAMF's one-coordinate row-key reconstruction.

The kernel-plan end-to-end protected-path probe was attempted first, but the test target could not finish compiling inside the command timeout. Those unexecuted measurements are not claimed.

Seven debug-profile repetitions of the primitive probe produced these local ranges in ns/op:

```text
I64 owned reverse:     168.47 .. 550.33
I64 shared route:      179.54 .. 511.13
I64 SAMF reconstruction:469.41 .. 715.56

Text256 owned reverse: 234.47 .. 408.60
Text256 shared route:  182.33 .. 308.54
```

One representative run after the final probe implementation was:

```text
I64:     owned 192.58 ns, shared 187.88 ns, SAMF 509.65 ns
Text256: owned 331.39 ns, shared 228.54 ns
```

These numbers are **debug-profile local comparative measurements**, not production latency claims. The ranges intentionally remain broad because the environment is noisy and release-mode kernel-plan compilation did not complete inside the execution timeout.

The correct firewall interpretation is therefore conservative:

- owned vs shared I64 intervals overlap heavily => no proven latency dominance;
- owned vs shared Text256 intervals also overlap => no strict timing theorem yet;
- the deterministic payload advantage of shared storage is already established;
- SAMF has a structurally deeper row-key path (`row_to_atom -> atom signature -> class record`) than either direct route, but the debug intervals are still not strong enough to delete a protected implementation on timing alone.

This is exactly the intended behavior of the competitive firewall: uncertain measurements preserve the protected baseline.

## Important architectural consequence

The new point demonstrates that `direct canonical key` versus `coordinate interning` is not a binary architectural decision.

There is a middle representation:

```text
canonical tuple interning without coordinate-class interning
```

This is especially attractive when:

- `D << N`;
- row->key is hot;
- slot projection is cold or absent;
- canonical keys are large/nested;
- paying the full SAMF class/signature/projection machinery is unnecessary.

If projection demand later appears, the retention compiler can move to or augment with coordinate-class resources. Semantic identity remains unchanged.

## Frontier growth after adding the shared route

The exact production-shaped oracle was rerun after adding the new realizer. Across all non-empty demand subsets, the number of unique inclusion-minimal, maintenance-closed physical shapes becomes:

```text
arity 1:   51 unique shapes, max  21 realizers for one demand
arity 2:  171 unique shapes, max  51 realizers
arity 3:  579 unique shapes, max 129 realizers
arity 4: 1995 unique shapes, max 339 realizers
```

Previously the corresponding unique-shape counts were 38 / 128 / 434 / 1496. The increase is not a reason to expose more named profiles; it is additional evidence that the correct abstraction is a compiled retention plan over exact resource atoms.

## Factorized optimizer theorem check

The new shared-key family was inserted into the same AND/OR derivation graph and maintenance closure.

The `global backbone × per-slot` factorization was rerun exhaustively against the exact DNF oracle for all **2,716** non-empty demand sets at arity 1..4 and still matched exactly.

So the new independent row-key family does not break the previously discovered decomposition property.

## Competitive non-regression answer

This iteration strengthens the answer to "will we lose performance on other cases?":

1. Existing specialized layouts remain protected feasible points.
2. A synthesized plan may not replace one merely because it uses less memory or has a better aggregate score.
3. Read work is calibrated per derivation witness.
4. If measurement envelopes overlap, the old fast path remains selected.
5. The shared-key family can therefore be admitted as a new option without forcing it onto I64/small-key workloads where owned reverse lookup may remain best.
6. Conversely, large/repeated canonical keys now have a candidate that can obtain quotient-like row-key access without quotient's per-row key-payload duplication and without SAMF projection machinery.

This is the key architectural gain: **adding a better physical point no longer means globally replacing the previous one.** The compiler can retain the old optimum region and add a new region.

## Verification

Confirmed after the final source state:

```text
cargo fmt --all -- --check                                      PASS
cargo clippy -p kernel-semantics --lib -- -D warnings          PASS
shared_canonical_* targeted tests                               2 PASS / 0 FAIL
dnf_composition_matches_bruteforce...                           PASS
global_backbone_times_per_slot_factorization_matches_exact...  PASS (27.67 s)
protected_row_key_primitive_probe                               PASS when run explicitly
```

A full `kernel-semantics --lib` sweep printed all ordinary tests as passing up to the long factorization test, but the outer command timeout expired before the harness emitted its final summary. The long factorization test was then run separately and passed. Therefore no false full-suite green claim is made from the timed-out aggregate command.

The attempted kernel-plan end-to-end microbenchmark did not reach execution because release/debug test-target compilation exceeded repeated command timeouts. Its temporary test changes were reverted before freeze; no unverified kernel-plan source remains in this R&D delta.

## Files changed

Production routing/store/durability are unchanged.

R&D source delta:

```text
crates/kernel-semantics/src/fiber_retention.rs
```

Report:

```text
docs/rnd/CFMD_SEMANTIC_FIBER_SHARED_KEY_FRONTIER_RND_2026-10-06.md
```

## Decision

Continue the branch.

The next R&D question is now narrower and more decisive:

1. build a release-capable benchmark harness outside the heavyweight kernel-plan test binary so protected read paths can be calibrated with stable intervals;
2. measure `N/D`, key payload size, read frequency and snapshot churn regions where shared canonical routing strictly dominates owned quotient reverse storage;
3. add joint-row ownership as an independent shared resource rather than bundling it with row-key routing;
4. test whether the compiler can reproduce each current baseline's best region and introduce new strict-dominance regions without any protected regression;
5. only after stable release envelopes, consider productionizing the representation compiler.

The result is no longer merely code unification. The branch now has a concrete candidate for a **new Pareto point**: quotient-like lookup semantics, SAMF-like canonical-payload deduplication, but without full SAMF coordinate/projection overhead.
