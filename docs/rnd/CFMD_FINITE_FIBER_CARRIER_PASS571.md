# PASS571 R&D — Finite Semantic-Fiber Carrier Integration

## Input

External R&D established that retained semantic-key structures are finite materializations of one pinned canonical map `kappa : X -> KΓ`, with exact forgetful laws `Projected -> Exact -> Measure`. PASS571 integrated that theorem into mainline R&D code and measured the first production frontier.

## Implemented carrier

`kernel-semantics::fiber_carrier::FiniteFiberCarrier<RowId, Key>` now owns one exact delta law over already-canonical coordinate keys.

Profiles:

- `Measure` — exact joint-key masses/cardinality;
- `ExactFibers` — joint fibers plus row-to-joint routing;
- `ProjectedFibers` — exact fibers plus coordinate-class -> joint-fiber incidence.

Coordinate canonical classes are interned once per coordinate. Exact reverse routing shares `Arc<[u32]>` joint-class signatures instead of duplicating canonical key tuples per row.

Projected retention deliberately does **not** duplicate rows into every coordinate projection. Projection stores only `coordinate class -> joint fibers`; projected rows are the exact union of those joint fibers. This is the structural projection certificate for this finite carrier.

## Proof / hostile verification

The carrier has finite hostile-state tests over every subset of a 4-row / 2-coordinate universe. After every insert/remove transition:

`count_M(k) == |fiber_Q(k)| == |fiber_O(k)|`.

Coordinate classes whose last retained row disappears are retired from the interning table; stale canonical-key classes are not accumulated indefinitely.

## Frontier measurement

Debug R&D microbenchmark, 4,096 rows, 257 joint classes, single i64 semantic coordinate. Numbers are local comparative measurements, not product performance promises.

### Current production shapes

| profile | retained bytes from PASS570 estimator | build ns (PASS571 run) |
|---|---:|---:|
| Cardinality / statistics | 39,456 | 16,031,418 |
| Quotient | 848,368 | 52,945,137 |
| Observable / SAMF | 230,192 | 66,871,059 |

### Unified carrier

| profile | structural retained bytes | build ns |
|---|---:|---:|
| Measure | 46,440 | 18,982,368 |
| ExactFibers | 376,176 | 43,406,992 |
| ProjectedFibers | 400,864 | 39,742,326 |

The first projected lowering duplicated row sets per coordinate and measured ~519,600 B / 68.3 ms build. Replacing that with `projection class -> joint fibers -> rows` reduced it to 400,864 B / 39.7 ms.

Carrier direct remove+insert transition measured approximately 0.21–0.26 ms across profiles. Current `PhysicalStore::apply_relation_delta` measured 55–123 ms in the same debug run, but those values are **not directly comparable**: current timings include candidate-store transition and authoritative relation mutation, while carrier timings isolate the retained semantic-fiber transition. They are kept only as an upper/lower diagnostic, not as a claimed speedup.

## Decision

### CLOSED

- One finite carrier/delta law exists in mainline R&D code.
- Coordinate canonical-class interning exists.
- Per-row canonical joint-key duplication is removed in ExactFibers.
- Product-projection mapping is unnecessary for the carrier lowering; structural coordinate position + incidence is sufficient.
- Forgetful-law commutation is executable and hostile-tested.

### OPEN

- `Measure` is still ~18% larger and slower to build than the specialized current statistics profile on the measured shape.
- `ProjectedFibers` is much faster to build than current SAMF in this debug run but remains ~1.74x larger by retained-byte estimator.
- Hostile distributions still need measurement: `D≈N`, multi-column high Cartesian cardinality, large/nested canonical keys, update-heavy churn, slot-probe-heavy reads.
- Production `MaterializedSemanticQuotientFactorState` has not yet been replaced. ExactFibers is the strongest immediate replacement candidate.
- Production SAMF remains the reference Observable lowering until projected retained-memory parity/non-regression is demonstrated.

### REJECTED

- One monolithic always-maximal layout.
- Projection-owned duplicate row sets.
- Routing/fallback with different semantic meaning.
- Deleting SAMF based only on identity unification before physical frontier parity.

## Next

PASS572 should benchmark hostile distributions and, if ExactFibers remains non-regressive, replace the production quotient state first. Projected/SAMF replacement remains gated on retained-memory + probe + delta-work frontier rather than naming cleanup.
