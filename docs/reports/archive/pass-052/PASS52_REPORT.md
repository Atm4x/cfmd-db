# CFMD Pass52 Report — Stable-Handle Local Dq for Γ-QCN Deletions

**Status:** VERIFIED on Rust 1.98.1.

**Production source window:** 2026-09-20 19:18:35 → 19:31:13 +03:00 (**12m38s**). Production source was frozen after that point. Final verification did not uncover a correctness defect, so the freeze was not removed.

**Scope:** refine the Pass51 exact `Replace` derivative for materialized Γ-QCN support into a true local derivative where the mathematics is monotone and prove that the refinement does not create a second authority/invalidation system.

## 1. Problem

Pass51 made the Γ-QCN common-domain/support fixed point a reconstructible maintained artifact. Correctness was complete, but every affected relation delta rebuilt the dependent support state:

```text
Fine input delta -> Replace(fresh complete support fixed point)
```

For a pure deletion this throws away information already present in the old fixed point. Support cannot be born from deletion, so a global replacement is stronger work than the derivative requires.

The dangerous shortcut would be to special-case convenient physical positions, e.g. "deleting the final ordinal is local". Hostile review rejected that route: locality must follow semantic/physical identity transport, not dense-vector accident.

## 2. Stable-handle coordinate transport

Pass52 uses stable `PhysicalRowId` as the derivative coordinate.

For each affected Γ-QCN leaf the new logical handle sequence must be a strict ordered subsequence of the old sequence. From this it derives:

```text
old ordinal -> Option<new ordinal>
```

This transport supports arbitrary removed rows, including interior deletions and dense-position movement, without requiring ordinal identity.

The existing support state is then transported rather than reconstructed:

- base support masks are remapped to surviving ordinals;
- quotient-leaf canonical-key arrays are remapped in current logical scan order;
- only changed quotient-leaf key buckets are rebuilt;
- no surviving payload row is canonicalized again.

If exact subsequence transport cannot be proved, maintenance immediately returns to the Pass51 `Replace` fallback.

## 3. Monotone Γ-QCN support derivative

Pure deletion cannot create support. Pass52 therefore treats the old converged support fixed point as an upper bound and propagates only losses.

A constraints-by-leaf dependency map drives a work queue:

1. enqueue quotient constraints touching a changed leaf;
2. recompute viable keys of that constraint against current masks/buckets;
3. clear rows that lost all viable quotient support;
4. enqueue every constraint incident to a leaf whose mask shrank;
5. continue until the queue is empty.

This is monotone finite descent over the already verified Γ-QCN support lattice. It reuses the same pinned Γ quotient coordinates and canonical keys as Pass49–51; it does not introduce host `Eq/Hash/Ord` semantics.

`common_keys` may conservatively retain a key that disappeared from a changed bucket. This is safe because a key with no live bucket cannot become viable. The physical artifact can therefore remain a sound superset representation while masks converge exactly.

## 4. Atomicity / authority boundary

The relation transition remains one unpublished candidate transition:

```text
validate relation delta
    -> mutate candidate physical relation
    -> maintain canonical quotient factors
    -> try local Γ-QCN deletion derivative
       or exact Replace fallback
    -> publish one physical transition epoch
```

The local derivative never becomes semantic authority. `Revision=(S,Γ,M)` remains authoritative; canonical factors/support state remain reconstructible physical evidence; final output still rechecks the original Γ predicates.

`semantic_quotient_support_local_delta_updates` is diagnostic physical observability only.

## 5. Why insertion is deliberately not mirrored

An insertion can resurrect support. Two or more currently unsupported rows may become valid only together, so support growth can contain a mutually supporting SCC of the greatest fixed point.

A naïve mirror operation such as "set bits for inserted rows and propagate additions" would therefore need a separately proved activation-closure law. Pass52 does not invent one.

Current exact behavior is:

```text
delete-only fine delta -> local Dq
insert or mixed delta  -> Pass51 exact Replace
```

That is a deliberate partial derivative with an exact universal fallback.

## 6. Hostile / regression evidence

The Pass52 hostile fixture checks one maintained Γ-QCN program through sequential transitions:

1. suffix deletion takes the local derivative and matches fresh logical evaluation;
2. interior deletion also takes the local derivative, disproving dependence on physical suffix position;
3. deleting one representative from a 100-row duplicate key bucket exercises multi-word ordinal masks and remains exact;
4. a cross-coordinate fixture proves support loss cascades through the dependency queue to another quotient coordinate;
5. a following insertion does **not** increment the local-derivative counter and instead uses exact Replace;
6. every transition result equals fresh logical evaluation;
7. strict Clippy remains clean with no new suppression.

The original suffix-only prototype was discarded before source freeze rather than preserved as a second path.

## 7. Verification

Final Rust 1.98.1 gate on frozen source:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test ... --release PASS
```

Release/overflow invocations that hit external compilation timeouts were not counted as results; they were rerun separately on warmed targets to actual PASS.

Workspace metrics:

- 358 declared Rust tests;
- 130 `kernel-plan` tests (129 normal + 1 ignored diagnostic benchmark);
- 21 crates;
- 49,389 Rust LOC under `crates/`;
- 19 pre-existing `#[allow(...)]` sites, no new suppression;
- 0 external Cargo registry/git sources;
- 0 `unsafe` in `crates/`;
- 0 TODO/FIXME/todo!/unimplemented! markers in production crates.

## 8. Problem ledger

### CLOSED exactly in Pass52 — 2

1. ✅ **Pure Γ-QCN-support deletion still rebuilt the entire fixed point through Pass51 `Replace`.** Arbitrary pure deletions now use exact stable-handle transport of the existing support state and do not rebuild surviving endpoint canonicalization or the complete fixed point.
2. ✅ **There was no dependency-local propagation of support loss across quotient coordinates.** A monotone quotient-constraint work queue now propagates only causally reachable support removals to convergence, including interior/duplicate deletions and cross-coordinate cascades.

### Historical OPEN from Pass51 fully closed this pass

**0 / 22.** The broad Γ-QCN/multiway incremental frontier still includes insertion/resurrection activation, mixed-delta local maintenance, revision-batch coalescing, lifecycle/memory policy, structural/custom quotient factors and wider planner coverage.

### Advanced but still OPEN

1. 🟨 **Fine Γ-QCN Change/Dq maintenance.** Delete-only fine deltas are now locally maintained; insertion/resurrection and mixed deltas still use exact Replace.
2. 🟨 **Revision-batch coalescing.** Multi-relation candidate construction may maintain one support program more than once before root publication.
3. 🟨 **Fine propagation representation.** The dependency queue is local by constraint graph, but touched constraints still recompute viable-key support from their current buckets/masks. Per-key support counters or a stronger derivative representation may reduce the constant factor.
4. 🟨 **Γ-QCN lifecycle/advisor.** Factors/support state remain explicit reconstructible families without unified create/share/retain/evict/rebuild and byte/RSS policy.
5. 🟨 **Structural/custom canonical laws.** Parallel R&D was interrupted before a reviewable bundle and remains production OPEN.
6. 🟨 **I64 Group/TopK constant-factor debt.** Unchanged by Pass52.

### Historical / active OPEN after Pass52 — 22

1. ⬜ Structural/custom-equivalence canonical indexing and typed production.
2. ⬜ General nested/multiway/bushy Join planning beyond the verified bounded 3–8-leaf builtin-primitive Γ-QCN fragment: adaptive/unbounded search, richer non-equality predicate graphs and fully general indexed/typed subset execution.
3. ⬜ First-class multi-family physical access advisor/lifecycle across specialized I64, generic semantic indexes, retained statistics, Γ-QCN factors/support state and future layouts.
4. ⬜ Autonomous workload statistics/telemetry, retention decay and lifecycle scheduling.
5. ⬜ Exact physical-index/statistics/quotient-factor/support-state byte/resident-memory budgeting and rebuild scheduling.
6. ⬜ Canonical-key encoding/version migration for long-lived physical caches/statistics/factors.
7. ⬜ Remaining physical layouts and explicit `OrderedView`/pagination.
8. ⬜ Secondary-index / alternate-layout rebuild performance after recovery.
9. ⬜ Clone-heavy runtime transaction candidates → COW/persistent roots.
10. ⬜ Durable revision DAG / branch+merge ancestry.
11. ⬜ General historical durable-format migration framework.
12. ⬜ Arbitrary/plugin semantic executable artifact packaging/signing/deployment.
13. ⬜ Transaction intent/outcome retention + GC.
14. ⬜ Streaming/chunked checkpoints/metadata.
15. ⬜ Real machine power-loss assurance and Windows/network-FS/FUSE durability semantics.
16. ⬜ General lock-poison/restart policy.
17. ⬜ Group commit / async durability.
18. ⬜ Replication / consensus and broader distribution architecture.
19. ⬜ Durable-store authentication/MAC; current CRC32C is corruption detection only.
20. ⬜ Formal power-loss proof for rename/fsync/GC protocol.
21. ⬜ Transaction repair runtime.
22. ⬜ Formal mechanization.

### New OPEN created in Pass52

**0.** Insertion/resurrection activation and per-revision coalescing are refinements of the already tracked Γ-QCN incremental-maintenance frontier.

## 9. External R&D status

The independent structural-canonical-key/fixpoint-adjacency investigation was interrupted before producing its final review bundle. Its reported speedups and design observations are not production claims of this branch. No R&D source was mechanically merged in Pass52.

## 10. Result / next direction

The current derivative stack is now:

```text
arbitrary query/change
        -> universal exact Replace

Γ-QCN affected by insert/mixed delta
        -> exact support Replace (Pass51)

Γ-QCN affected by pure deletion
        -> stable-handle remap
        -> local monotone quotient support Dq (Pass52)
```

The next mathematically meaningful question is the dual direction: whether support **activation** can be expressed as a sound local least/greatest-fixed-point derivative for insertions/resurrection without introducing bespoke invalidation semantics. A second useful target is revision-level coalescing so one Γ-QCN derivative is evaluated once per complete unpublished revision rather than once per constituent relation edit.
