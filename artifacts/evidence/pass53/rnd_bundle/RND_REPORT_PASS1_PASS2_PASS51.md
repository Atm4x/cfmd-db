# CFMD non-Join R&D — combined Pass1 + Pass2 report against Pass51

## Scope

Authoritative rebase baseline for this package: `cfmd_workspace_pass51_verified(1).zip`.

The main agent owns Join / `ProvenanceJoinRow` replacement and the Γ-QCN line. This R&D deliberately does not modify Join planning, Γ-QCN, prepared quotient support, Join enumeration, or Join access-path economics.

The goal is to find places where the physical implementation still behaves like a generic DB/runtime despite stronger CFMD mathematics already being available.

## Result summary

Four production files form the integration candidate:

1. `kernel-semantics/src/lib.rs` — compositional exact Γ-canonical keys for structural equivalences.
2. `kernel-query/src/lib.rs` — `MaterializedSetSupportState` lowered from semantic vector scans/full clone to exact canonical support lookup + validated mutation plan.
3. `kernel-fixpoint/src/lib.rs` — relation reachability lowered once to adjacency instead of rescanning the whole edge relation per frontier vertex.
4. `kernel-schema/src/lib.rs` — subtype/inclusion reachability maintains direct parent adjacency instead of rescanning the entire inclusion relation at every BFS step.

A fifth candidate, replacing `SemanticBucketIndex`'s linear bucket removal, was hostile-tested and deliberately NOT integrated because the first exact ordered representations traded mutation speed for unacceptable read/probe regressions. The experiment is retained separately as rejected evidence.

---

## A. Structural Γ-canonical keys

### Problem

CFMD already defines exact structural equivalence compositionally in pinned Γ for Product / Option / Sum / Set / Bag / Seq / Map / guarded μ. Production canonical key support remained primitive-only. Physical consumers therefore paid repeated `registry.equivalent(...)` scans even when Γ already determines an exact quotient coordinate.

This is a generalization tax caused by physical lowering lagging behind the logical algebra.

### Implementation

`CanonicalEqKey` gains structural variants and `SemanticRegistry::canonical_equivalence_key(...)` recursively lowers a value using the exact structural equivalence definition pinned by schema + Γ.

Important boundaries:

- primitive semantic modules remain the base authority;
- Rust `Eq/Hash/Ord` is not substituted for semantic equality;
- Set/Bag/Map keys canonicalize unordered logical structure independently of physical iteration order;
- guarded μ/Var reuse the already-admitted recursive equivalence structure;
- this package does NOT make structural keys a durable persisted semantic-index encoding, so `KEY_ENCODING_REVISION` is intentionally untouched. If long-lived/durable structural index keys are later admitted, encoding migration/versioning must be handled explicitly.

`ensure_unique` now uses the exact canonical quotient key rather than pairwise O(n²) semantic equality.

### Consumer lowering

`MaterializedSetSupportState` now keeps:

```text
CanonicalRowKey -> support slot
```

instead of locating every support class with a semantic linear scan.

Delta application groups all changes by canonical class, prevalidates every underflow, and only then mutates the state. This removes the full `supports.clone()` atomicity workaround while retaining all-or-nothing behavior.

### Diagnostic evidence from Pass1

The stored raw repeats show the structural-support diagnostic moving from approximately 1.77 s build to ~22–26 ms and semantic probes from ~864–875 ms to ~3.4–3.6 ms. This is workload-specific evidence, not a universal performance claim.

Pass51 explicitly still listed structural/custom canonical laws as OPEN/external R&D; the Pass1 patches apply cleanly to Pass51.

---

## B. Fixpoint relation -> adjacency lowering

### Problem

`kernel-fixpoint::solve` represented a finite edge relation correctly, but traversal scanned the complete edge `BTreeSet` for every dequeued vertex. The logical relation was therefore not erased to the standard specialist physical form implied by the CFMD spec.

### Implementation

The solver deterministically compiles:

```text
BTreeSet<(source,target)>
    -> BTreeMap<source, Vec<target>>
```

once, then performs the existing least-fixpoint traversal over outgoing adjacency.

The certificate/result boundary, parent/rank witness semantics and checker are unchanged.

### Diagnostic evidence from Pass1

Stored N=8000 chain repeats show roughly 15–17x improvement over the whole-edge rescan path.

---

## C. Schema inclusion/subtype relation -> adjacency lowering

### Problem found in Pass51

`Schema::is_subtype` still did this for every frontier node:

```text
for (child,parent) in ALL inclusions
    if child == current ...
```

Therefore a transitive subtype query behaved like O(V*E), and `Schema::include` also paid the same tax during cycle checks.

The schema is already an immutable-revision-oriented finite relation, so this is another case where the physical representation ignored the obvious relation->adjacency erasure available from the mathematical model.

### Implementation

`Schema` now maintains both:

```text
inclusions        // existing exact relation / direct-inclusion authority
inclusion_parents // derivative adjacency for reachability
```

`include` updates adjacency only when a genuinely new direct pair is inserted. `is_subtype` traverses only direct parents. Cycle semantics are unchanged.

No transitive closure is stored, so update cost remains small and there is no second semantic authority.

### Pass51 benchmark

Fixture: chain of 3000 semantic IDs, 20 bottom->top subtype queries, release, Rust 1.98.1, five process runs.

Median:

```text
schema build/cycle checks:
  baseline 11,431,034 ns
  R&D       1,100,687 ns
  ~10.39x

20 transitive subtype queries:
  baseline 456,506,683 ns
  R&D        9,230,385 ns
  ~49.46x
```

Raw data: `evidence/pass2_subtype_repeats_pass51.tsv`.

---

## D. SemanticBucketIndex hostile experiment — defect confirmed, first replacements rejected

### Existing defect

`SemanticBucketIndex::remove` does:

```text
bucket.iter().position(identity)
bucket.remove(position)
```

so a skewed semantic class has O(bucket) lookup + O(bucket) shift per deletion.

### Why `swap_remove` is invalid

Bucket order is intentionally stable and is consumed by maintained/indexed execution paths. A simple `swap_remove` changes deterministic enumeration and is therefore not a valid drop-in physical refinement.

### Experiment 1: stable linked slots

An exact stable-slot list made deletion approximately ~70x faster on the hostile fixture, but sequential bucket iteration became about 20–25x slower because every element became pointer-chasing. Rejected as a universal replacement.

### Experiment 2: linked chunks

A linked list of contiguous chunks preserved exact insertion order and bounded each local shift. A chunk-size sweep is retained in `pass2_bucket_chunk_sweep_pass51.tsv`.

At chunk size 32, five-run medians on N=20k / 5k middle deletions were:

```text
remove:
  Vec baseline  25,467,233 ns
  chunk32        2,528,821 ns
  ~10.07x faster

read/probe before deletes:
  Vec baseline   1,432,141 ns
  chunk32       10,861,766 ns
  ~7.58x slower

build:
  Vec baseline   3,222,031 ns
  chunk32        5,332,794 ns
  ~1.66x slower
```

This is a hostile falsifier against globally replacing the bucket representation.

### R&D conclusion

The correct next design is a multi-family/adaptive bucket representation selected from workload/skew statistics, not one supposedly superior container. Candidate directions worth testing:

- compact Vec bucket for read-heavy/stable classes;
- mutation-oriented ordered tombstone/chunk form for high-churn skewed classes;
- hysteresis/retention policy owned by the existing multi-family physical advisor;
- byte/RSS accounting before automatic promotion.

The rejected experiment is isolated under `experiments/rejected_semantic_bucket/` and MUST NOT be applied as part of this package.

---

## Verification on Pass51 rebase

Integration candidate (`kernel-semantics`, `kernel-query`, `kernel-fixpoint`, `kernel-schema`):

- `cargo fmt --all -- --check` — PASS
- `cargo check --workspace --all-targets` — PASS
- `cargo test --workspace --all-targets` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- release tests for all four modified crates — PASS

A complete `cargo test --workspace --all-targets --release` was attempted twice. Both invocations hit the external execution timeout while the entire workspace was compiling. They are therefore recorded as INCOMPLETE, not PASS and not FAIL. The warmed targeted release gate over every modified crate completed successfully.

---

## Problem ledger for this R&D package

### Closed / integration candidate

1. Structural Γ equivalence existed logically but lacked compositional exact canonical physical keys.
2. Set-support maintenance used semantic vector scans and full support cloning instead of Γ quotient keys + validated local plan.
3. Fixpoint traversal rescanned the complete relation for every frontier node instead of lowering Relation -> adjacency.
4. Schema subtype traversal/cycle checks rescanned the complete inclusion relation instead of lowering Relation -> adjacency.

### Confirmed but not closed

5. `SemanticBucketIndex` high-skew deletion is O(bucket). Two exact ordered replacement families were tested; both expose a material read/write tradeoff, so a global replacement is rejected. Needs advisor-controlled/adaptive physical family.
6. Clone-heavy runtime revision/materialization candidate construction remains OPEN; this package did not attempt a partial COW rewrite.
7. Structural canonical persisted-index encoding/version migration remains OPEN; this package only uses structural canonical keys in non-durable derived state.

### Join boundary

No Join / Γ-QCN production code is modified by the integration candidate. The current Pass51 Join work remains authoritative.
