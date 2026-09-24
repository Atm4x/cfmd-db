# Pass65 hostile review — R&D Program5 Incremental Revision Compiler

Input: `CFMD_RND_PROGRAM5_CLOSEOUT_PASS63_2026-09-21.zip`.
Baseline for production merge: frozen Pass64.
Verdict: **accepted after hostile corrections; not merged verbatim**.

## What survived review

Program5's core construction is sound: a relation-only transition can reuse revision-local dense identity, dense lifecycle and dense type extents when the target candidate is obtained from the authoritative source Revision and the only public mutation surface replaces relation rows. Touched relations alone require typed/semantic validation and LiveRef sensitivity refresh. Recovery can use the same sealed path for relation-data WAL records instead of rebuilding the whole Revision.

The R&D partitioning of `LiveRefSensitivityIndex` also survives: immutable field roots and untouched per-relation partitions are shared through `Arc`; only touched relation partitions are recompiled.

## Hostile defects found in the submitted candidate

### 1. Candidate was not source-bound

The submitted API accepted `build_certified_relation_update(id, source, registry, candidate)` where `candidate` did not record which Revision created it. A candidate cloned from Revision A could therefore be passed with Revision B as `source`, allowing untouched state from A to be accepted under derivative roots from B without the deep equality checks that the certified path intentionally removes.

Production correction: `RelationUpdateCandidate<'a>` now borrows its exact source Revision and owns the build method. There is no caller-supplied source at build time, so cross-source rebinding is structurally unavailable.

### 2. Certified path skipped pinned-Γ registry validation

The submitted certified builder validated touched relations but omitted `registry.validate_context(source.semantic_context())`. The old full `Revision::build` always performed that check. A relation update whose touched values did not force semantic module lookup could therefore be accepted with a registry that could not satisfy the pinned Γ environment.

Production correction: the source-bound candidate build revalidates the complete pinned semantic context before touched-only compilation. A hostile test uses a source built with pinned I64 equivalence and verifies that the same candidate is rejected by an empty registry.

### 3. Touched relation normalization was not equivalent to full `Revision::build`

Full revision construction normalizes before typed validation. A newly supplied relation row containing a dangling nested `LiveEntityRef` is removed during normalization. The submitted incremental path skipped local normalization and immediately typed-validates the touched relation, producing `TypeMismatch` instead of the normalized target Revision.

Production correction: before touched-only validation, the candidate filters only touched relation rows whose values contain a `LiveEntityRef` outside the already-normalized source lifecycle. Historical IDs remain unaffected. A hostile differential test requires the certified result state to equal a full `Revision::build` result on this case.

## Performance re-check

Independent release diagnostic on the hardened Pass65 source:

- 40 Bag<I64> relations × 5,000 rows;
- one touched relation;
- logical snapshot clone performed outside compiler timing;
- returned Revision destruction performed outside compiler timing.

Median of seven runs:

- full compiler: 3,406,633 ns;
- hardened certified compiler: 293,583 ns;
- compiler-only ratio: 11.604×;
- `RelationUpdateCandidate` full `DatabaseState::clone()` alone: 4,330,525 ns.

The Program5 compiler boundary therefore remains materially faster, but the logical-state clone is now the dominant residual cost. Pass65 does **not** claim end-to-end O(|Δ|) revision transition complexity.

## Accepted production boundary

- `DatabaseState::normalize_certified()` may return reusable final dense IDs and LiveRef sensitivity.
- `Revision` retains `DenseTypeExtents`.
- arbitrary externally supplied relation-only states continue through the defensive `build_relation_update` equality checks.
- fast certified updates are created only by `Revision::relation_update_candidate()` and are source-bound by lifetime.
- recovery creates candidates only from the current authoritative Revision.
- touched relations receive local LiveRef normalization, typed/semantic validation and per-relation sensitivity refresh.
- untouched relation sensitivity partitions, field sensitivity roots, dense identity, dense lifecycle and type extents are reused.

## Residual

`relation_update_candidate()` still clones the complete `DatabaseState`. Persistent/path-copy logical state is therefore a separate OPEN and is not hidden inside the compiler closeout.
