# CFMD CQ semantic identity + factorized aggregate R&D over PASS591

**Date:** 2026-10-07  
**Base:** `CFMD_PASS591_PERSISTENCE_TRANSITION_HOSTILE_CLEANUP(1).zip`  
**Base SHA-256:** `f6768da58ac5967a2042ca9326698ebb7a6f4d02ac15fd7a2d9d28e41b71a311`  
**Mode:** production-shaped R&D. No durable format or public API change.

## Status

Two distinct results survived hostile measurement:

1. **CQ semantic interning** is viable as a production execution optimization for multiple equivalent Set-CQ roots. It is integrated into the in-memory `RelObservationForest` path in this R&D tree and remains fail-closed outside its admitted fragment.
2. **Γ-factorized join aggregates** are a substantially larger opportunity than CQ-root dedup for hierarchical equijoins feeding `COUNT` / `GROUP BY key + COUNT`. This remains R&D-only, but exactness and resource frontier are already strong enough to recommend a dedicated productionization line.

The two mechanisms are complementary:

```text
CQ semantic core
    -> deduplicate equivalent logical observations
    -> classify structural shape

hierarchical aggregate shape
    -> select factorized Γ-mass maintenance
    -> never materialize the join cross-product for count-only observations
```

## A. Production-shaped CQ semantic interning

### A.1 Selected law

The existing structural identity remains authoritative for exact reconstructible AST identity.
A second semantic identity is admitted only for a restricted conjunctive-query fragment and only after explicit proof checking.

```text
CanonicalRelExprStructuralIdentity
    !=
CertifiedCqSemanticIdentity
```

The semantic identity is never a durable replacement for the structural expression. It is an in-memory interning key for maintained observation state.

### A.2 Admitted fragment

The lowering accepts only:

- `Scan`;
- `FilterEqConst`;
- `FilterEqColumns`;
- `Project`;
- `JoinEq`.

Additional restrictions:

- every base relation is `RelationSemantics::Set`;
- every equality operator must use exactly the pinned column equivalence of the participating coordinates;
- constants are encoded through the pinned Γ canonical key;
- query output coordinate order remains part of identity;
- unsupported operators fail closed to the old structural path.

Rejected from CQ identity:

- Bag semantics;
- coarser query equality over finer stored equality;
- ordering filters;
- Difference / Union / AntiJoin;
- Distinct;
- Group / TopK;
- PromoteToBag;
- expressions whose bounded canonical variable relabeling would exceed the current R&D admission bound.

### A.3 Proof boundary

Discovery and proof checking are separate.

The R&D module discovers mutual homomorphism witnesses, but reuse is admitted only after the witnesses pass `kernel-proof::CertificateChecker`.
Redundant atom removal likewise requires a checked equivalence certificate before the atom is removed from the candidate core.

This preserves the project law:

```text
expensive/untrusted discovery
    -> small witness
    -> deterministic checker
    -> semantic reuse
```

### A.4 Production forest integration

`RelObservationForest::build_with_stats` now has a second interning map for certified CQ identities.

The existing structural interning is checked first. Semantic interning is enabled only when the forest has more than one root, because a single root has no cross-root semantic reuse opportunity in this implementation.

For each admitted expression:

1. lower to CQ;
2. minimize to a checked core;
3. canonicalize variable names while preserving Γ-equivalence types and output coordinate order;
4. intern the resulting semantic identity;
5. reuse an already-built maintained cell if present.

Each original root remains a separate route and keeps its original source-relation envelope. Multiple roots may point to the same maintained state cell.

### A.5 Exact correctness coverage

The production path is tested over:

- 3 unary Set relations;
- all 6 join-order permutations;
- all 64 states of a two-element finite universe;
- all admitted single-row insert/remove transitions.

For every world and transition, semantic interning preserves every root result and exact delta.

Pinned Γ identity is part of variables/constants; the identity is therefore not Rust-value equality masquerading as semantic equality.

### A.6 PASS591 A/B release frontier

Exactly the same benchmark source was compiled against:

- clean supplied PASS591;
- this R&D tree.

Fixture: one five-relation Set query, emitted in multiple semantically equivalent join-order ASTs. CQ identity construction is included in modified build time.
Five alternating release process runs were collected; table uses median times.

| Equivalent roots | PASS591 cells | CQ cells | PASS591 build | CQ build | build speedup | PASS591 delta | CQ delta | delta speedup |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 10 | 10 | 10.385 ms | 10.349 ms | 1.00x | 33.139 µs | 42.884 µs | 0.77x* |
| 2 | 13 | 10 | 12.580 ms | 10.170 ms | 1.24x | 41.211 µs | 33.980 µs | 1.21x |
| 6 | 27 | 10 | 24.102 ms | 10.229 ms | 2.36x | 53.258 µs | 34.031 µs | 1.56x |
| 24 | 93 | 10 | 76.509 ms | 10.658 ms | 7.18x | 107.298 µs | 47.110 µs | 2.28x |
| 120 | 445 | 10 | 355.601 ms | 12.966 ms | **27.43x** | 1.081 ms | 52.257 µs | **20.69x** |

`*` One-root semantic interning is disabled. The very small delta timing is therefore noise / binary-layout variance rather than an admitted semantic optimization; no one-root regression claim is made from this micro-timing.

At 120 roots:

- maintained cells: `445 -> 10` = **44.5x fewer**;
- visited nodes for the measured update: `69 -> 2` = **34.5x fewer**.

### A.7 CQ peak RSS

Separate release processes were measured with `/usr/bin/time -v` over the same benchmark program; the program reaches the 120-root case before exit.
Three runs were stable:

```text
clean PASS591 peak RSS: 271308 .. 271448 KiB
CQ tree peak RSS:        14404 .. 14532 KiB
```

Using 271332 / 14532 KiB gives approximately **18.67x lower peak RSS** in this deliberately duplicate-heavy workload.

### A.8 Coverage signal, not workload claim

Static source census over current Rust code found 1,597 `RelExpr::<variant>` constructor mentions, of which 1,156 (72.38%) are constructors from the admitted CQ family (`Scan`, equality filters, `Project`, `JoinEq`).

This is **not** a claim that 72% of runtime queries are CQ-admissible. One unsupported node makes the whole candidate subtree ineligible. It only establishes that the admitted operators are not an exotic unused corner of the IR.

## B. Stronger R&D: Γ-factorized join aggregates

### B.1 Hostile finding

CQ interning only helps when multiple logical roots are equivalent.
A single observation such as:

```text
R(key, payload_r)
JOIN S(key, payload_s)
JOIN T(key, payload_t)
-> COUNT(*)
```

still drives the current maintained forest through Join states and a Group state.
Even though the final observation is one exact number, the generic path pays work proportional to large join intermediates.

For count-only observation this is unnecessary.

### B.2 Exact Γ law

For one pinned equivalence Γ and canonical class `k`:

```text
r_k = multiplicity of R rows in Γ-class k
s_k = multiplicity of S rows in Γ-class k
t_k = multiplicity of T rows in Γ-class k
```

Then exact bag join cardinality is:

```text
COUNT(R ⋈ S ⋈ T)
    = Σ_k r_k * s_k * t_k
```

For `GROUP BY join_key + COUNT` the exact output for each class is simply:

```text
count_k = r_k * s_k * t_k
```

A delta to one relation and one key changes only that class contribution. No cross-product rows are required.

The R&D carrier uses:

- `CanonicalEqKey` from the pinned CFMD equivalence;
- `ExactNatural` for multiplicities/products;
- exact insert/remove maintenance.

This is not an i64-only arithmetic shortcut.

### B.3 Exhaustive correctness

For three Bag relations, two semantic keys, and per-key multiplicities in `{0,1,2}`:

```text
3^(3*2) = 729 finite database worlds
```

were exhaustively checked.

Results:

- global `COUNT` factorization: exact match to full `RelExpr` evaluation in all 729 worlds;
- keyed `GROUP BY join-key + COUNT`: exact match in all 729 worlds;
- insert/remove delta maintenance: matches full rebuild.

### B.4 Release frontier using CFMD-native keys and exact arithmetic

Fixture:

- 32 Γ key classes;
- 3 Bag relations;
- equal fanout per key in each relation;
- current production `RelObservationForest` for `Join -> Join -> global Count`;
- factorized candidate using `CanonicalEqKey + ExactNatural`;
- seven samples inside each process; three complete release frontier runs; table uses medians across those complete runs.

| Fanout | Rows/relation | Logical 3-way join rows | production build | factorized build | build speedup | production delta | factorized delta | delta speedup |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 2 | 64 | 256 | 0.705 ms | 24.396 µs | 28.9x | 33.610 µs | 0.331 µs | 101.5x |
| 4 | 128 | 2,048 | 3.008 ms | 43.965 µs | 68.4x | 71.686 µs | 0.420 µs | 170.7x |
| 8 | 256 | 16,384 | 15.174 ms | 102.962 µs | 147.4x | 214.257 µs | 0.781 µs | 274.3x |
| 16 | 512 | 131,072 | 99.216 ms | 209.350 µs | **473.9x** | 598.665 µs | 0.791 µs | **756.8x** |
| 24 | 768 | 442,368 | 308.636 ms | 295.427 µs | 1,044.7x | 1.187 ms | 0.922 µs | 1,287.2x |
| 32 | 1,024 | 1,048,576 | 694.280 ms | 356.838 µs | **1,945.6x** | 2.121 ms | 0.941 µs | **2,254.0x** |

The single-delta factorized measurement is sub-microsecond and therefore sensitive to timer/microbenchmark effects. It is reported for shape only, not as a universal latency promise.

A more stable batch measurement at fanout 16 applies 256 sequential inserts to the same Γ class and advances the production forest through 256 revisions:

```text
production forest: median 127.943 ms
factorized Γ state: median 30.676 µs
ratio:              ~4,170.8x
```

This is still a favorable count-only workload, but the measurement is large enough that timer granularity is not the explanation.

### B.5 Peak RSS

Separate release processes, identical source model:

#### Fanout 16

```text
production forest: 98936, 99192, 99192 KiB
factorized state:    2032, 2032, 2128 KiB
median ratio: ~48.8x less peak RSS
```

#### Fanout 32

```text
production forest: 711948, 711952, 712080 KiB
factorized state:      2416,   2532,   2544 KiB
median ratio: ~281.2x less peak RSS
```

The reason is structural: the factorized state retains per-class multiplicities rather than expanding join combinations required by the generic maintained pipeline.

These ratios are **not** claims about arbitrary joins. They are a resource frontier for the admitted count/group-count shape.

## C. Why this is not a SQL-shaped special case

The candidate is better understood as an exact semantic-fiber aggregate lowering:

```text
pinned Γ quotient classes
    -> class masses per relation occurrence
    -> semiring product inside a class
    -> semiring sum across classes
```

The semantic authority is the same pinned Γ equality already used by CFMD.

The current code base already has a physical observation vocabulary containing `JointMass`, and catalog-free SAMF retains atom masses. Therefore a production factorized aggregate should not create an unrelated duplicate indexing subsystem.

Preferred integration:

```text
existing retained JointMass / semantic-fiber capability
        -> seed class multiplicities directly when available
        -> FactorizedAggregateCarrier
```

The missing seam is an execution capability that exposes exact class masses to the aggregate lowering without exposing SAMF implementation identity.

When no retained mass capability is selected, an exact carrier can still be built from source rows at preparation time; this is the same semantic law with a more expensive physical construction, not a semantic fallback.

## D. q-hierarchical classification

The CQ R&D now computes whether the certified core is q-hierarchical.

This is relevant because the dynamic-query literature gives a structural dichotomy: q-hierarchical CQ cores admit linear preprocessing and constant-time updates/counting for the covered setting, whereas non-q-hierarchical cores have conditional lower bounds against generic sublinear dynamic maintenance (under OMv/OV assumptions).

Reference:

- Christoph Berkholz, Jens Keppeler, Nicole Schweikardt, *Answering Conjunctive Queries under Updates*, arXiv:1702.06370.

This should be used as a **physical-selection guardrail**, not as a claim that every CFMD Bag/Γ extension inherits the paper automatically.

Selected architecture direction:

```text
certified CQ core
    |
    +-- q-hierarchical aggregate shape
    |       -> exact factorized Γ carrier
    |
    +-- other shape
            -> existing DTC / Γ-QCN / WCOJ / other exact physical lowering
```

No universal "fast CQ" mechanism should be promised for shapes that cross the known hard boundary.

Related implementation family:

- Nikolic & Olteanu, *Incremental View Maintenance with Triple Lock Factorization Benefits* (F-IVM), arXiv:1703.07484.

CFMD should reuse the factorization principle, but keep its own Γ semantic authority and exact-delta laws rather than importing a SQL-specific execution model.

## E. Hostile boundaries

### E.1 CQ semantic identity is not universal query identity

Do not extend it by analogy to:

- Bags without an explicit multiplicity theorem;
- Difference/negation;
- order-dependent operators;
- arbitrary aggregation;
- coarser equalities not identical to the stored coordinate equality;
- constraints/FDs unless a separate certified containment law is added.

### E.2 Factorized aggregate is not a generic join replacement

The current prototype specifically proves an equality-class count law.
It does not prove that arbitrary projected join rows, arbitrary aggregates, non-equality predicates, or non-hierarchical joins can be maintained with the same bounds.

### E.3 Do not run CQ core search in the update hot path

Core discovery is preparation work. The update path consumes an already selected representative/state.

### E.4 Do not replace durable AST identity

Semantic interning is an additional execution identity. Durable reconstructibility and exact query definition remain structural.

## F. Verification

Toolchain: Rust 1.98.1 retained in the environment; source tar is not part of the R&D package.

Final gates:

- `cargo test -p kernel-query --lib --offline`: **166 passed / 0 failed / 10 ignored**;
- `cargo test -p kernel-plan --lib --offline`: **316 passed / 0 failed / 11 ignored**;
- `cargo test -p cfmd-runtime --lib --offline`: **30 passed / 0 failed**;
- factorized exact finite-world tests: included in kernel-query green suite;
- `cargo check --workspace --offline`: **PASS**;
- strict Clippy over `kernel-query`, `kernel-plan`, `cfmd-runtime`, all targets, `-D warnings`: **PASS**;
- `cargo fmt --all -- --check`: **PASS**.

No PASS591 persistence-transition behavior or durable format was changed by this R&D.

## G. Decision

### GO — CQ semantic interning, with a narrow admission boundary

The actual production-shaped path demonstrates a material region:

- 2 duplicates: already modestly positive;
- 6 duplicates: ~2.36x build;
- 24 duplicates: ~7.18x build;
- 120 duplicates: ~27.43x build / ~20.69x delta / ~18.67x peak RSS.

It should remain gated to multiple roots and exact proof-checked Set-CQ identities.

### STRONG GO R&D — factorized Γ aggregates

This is the larger result.

A moderate fanout-16 exact workload already shows:

- ~**474x** build improvement;
- ~**757x** single-delta improvement;
- ~**4,171x** over a 256-update batch;
- ~**48.8x** lower peak RSS.

At fanout 32 the build frontier reaches ~**1,946x** and peak RSS ~**281x** lower.

These are not global database speedups. They demonstrate that materializing join intermediates for count-only hierarchical observations is the wrong physical law.

## H. Recommended next production R&D

1. Introduce a narrow `FactorizedAggregateCarrier` for exact equality-join `COUNT` and `GROUP BY join-key + COUNT` only.
2. Compile/admit it from an explicit query-shape certificate; do not pattern-match loosely at runtime.
3. Use pinned Γ canonical class identity and `ExactNatural` multiplicity throughout.
4. Add a `JointMass` capability bridge so retained SAMF/semantic-fiber mass can seed the carrier without row recanonicalization.
5. Keep existing DTC as the exact reference in hostile tests until exhaustive/random state-space parity is broad enough.
6. Benchmark low-fanout / high-distinct / skew / churn / self-join / multi-key hostile distributions; retain the generic path where factorization does not dominate.
7. Measure actual duplicate-CQ rate in real history/watch workloads before making CQ core computation unconditional for all multi-root forests.

## Ledger

### CLOSED THIS R&D

- production-shaped certified CQ semantic interning;
- Γ-typed CQ identity rather than host equality;
- exact finite-state/delta parity for production forest reuse;
- release PASS591 A/B frontier;
- CQ peak-RSS frontier;
- q-hierarchical core classifier;
- exact Γ-factorized global count prototype;
- exact Γ-factorized grouped count prototype;
- exhaustive 729-world Bag correctness;
- production-vs-factorized build/delta/RSS frontier;
- connection to existing `JointMass` semantic-fiber capability.

### OPEN

- narrow production `FactorizedAggregateCarrier` integration;
- `JointMass` capability bridge;
- real workload duplicate-CQ census;
- hostile skew/D≈N/self-join/multi-key frontier;
- optional extension from count payload to other lawful commutative semiring aggregates only after exact laws are proved.

### REJECTED

- universal semantic identity for all `RelExpr`;
- Bag CQ dedup by importing Set-CQ theorem without multiplicity proof;
- hot-path CQ core discovery;
- replacing structural/durable query identity with CQ identity;
- generic "all joins become O(1)" claim;
- separate SQL-style factorization authority competing with Γ/SAMF;
- reporting the 1,000x-class favorable aggregate frontier as an average CFMD speedup.

## Checkpoint completion timing

The exploratory R&D work before the chat interruption did **not** have a compliant declared 20/24-minute wall-clock envelope, so no such compliance is claimed for that earlier work.

A separate checkpoint-completion cycle was explicitly bounded and changed no source code:

- official UTC start: **15:49:46**;
- source was already functionally frozen before this completion cycle;
- package integrity completed: **15:51:30 UTC**;
- useful boundary: **16:09:46 UTC**;
- hard boundary: **16:13:46 UTC**.

The completion cycle only verified preserved results, checked PASS591-relative packaging hygiene, repeated the Rust 1.98.1 smoke compile, removed the source toolchain tar, and produced the final delta archive.
