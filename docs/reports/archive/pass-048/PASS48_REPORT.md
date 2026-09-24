# CFMD Pass48 Report — Order-Preserving Semijoin Masks / Bounded N-Way Selectivity Search

**Status:** VERIFIED on Rust 1.98.1.

**Production source windows:**

- initial: 2026-09-20 17:00:59 → 17:13:58 +03:00 (**12m59s**), then source freeze;
- hostile correction after review of the Pass47-style final-sort design: 17:17:27 → 17:24:19 +03:00 (**6m52s**).

Total production-edit time: **19m51s**. The intervening 3m29s was frozen verification/review, not production editing. Final production source was frozen at 17:24:19; the final gate found no correctness defect, so that freeze was not lifted.

**Scope:** physical multiway Join planning/execution only. Pass48 does not change semantic `Revision=(S,Γ,M)`, exact query semantics, transaction authority, or durable authority.

## 1. Problem

Pass47 proved that non-contiguous leaf permutation can preserve exact reference Bag output by carrying per-leaf physical provenance and sorting completed tuples back into original scan-ordinal order. That closed a correctness prerequisite, but hostile review exposed an architectural defect in the physical realization:

1. reordered execution deliberately destroyed the reference enumeration order and then paid a global final sort to reconstruct it;
2. `ProvenanceJoinRow` became an increasingly rich transient carrier even though the real requirement was only to exploit selective cross-leaf constraints without changing output enumeration;
3. extending that representation to N-way subset planning would have amplified tuple materialization, provenance copying and `O(output log output)` restoration work.

The stronger design target is therefore: **use reordering/selectivity information for pruning, but enumerate final tuples directly in the original leaf order.**

## 2. Pass48 architecture — pruning without order destruction

Pass48 removes the current production `ProvenanceJoinRow` / `ProvenanceJoinFragment` / `restore_provenance_join_order` path entirely.

For the admitted primitive equality fragment, physical execution now builds Γ-bound compatibility structures:

```text
predicate equality
    -> canonical keys under pinned Γ
    -> key -> ordinal bit-mask buckets
    -> semijoin support masks
    -> original-leaf-order DFS with early mask intersection / forward viability
```

The final tuple enumeration is always leaf `0,1,...,N-1`, and each leaf's row ordinals are visited monotonically in authoritative logical scan order. Therefore reference left-major/right-minor Bag order is produced **constructively**; no final sort exists.

The masks are reconstructible execution-local physical state. They are not logical values, semantic authority, or durable state. Completed tuples are still revalidated through the exact pinned-Γ equality predicates before output.

## 3. Bounded N-way subset/selectivity search

The statistics layer now supports a bounded subset DP for **3–8 leaves**. It estimates whether non-contiguous/selective predicate structure justifies the semijoin-mask path relative to the best current contiguous plan.

Important distinction:

- subset DP is optimizer evidence/admission;
- the executor does **not** materialize the selected bushy tree and then sort it;
- the executor uses compatibility masks to prune while preserving original enumeration order.

Existing persisted I64/semantic indexes retain priority when the unified `JoinAccessDecision` says an already-built singleton-side access path is preferable. Execution-local semijoin masks are themselves a transient access structure, so they are not rejected merely because an equivalent ephemeral index family could also be constructed.

This is a bounded N-way physical optimization, not arbitrary unbounded/general WCOJ closure.

## 4. Hostile correction of the initial Pass48 route

The first Pass48 implementation generalized Pass47 into stable-handle `ProvenanceJoinRow` subset execution plus final order restoration. It was semantically correct and passed tests, but hostile review identified the final sort as the wrong architectural boundary.

That design was **retired before checkpoint packaging**. Production source no longer contains:

- `ProvenanceJoinRow`;
- `ProvenanceJoinFragment`;
- `join_provenance_rows`;
- `restore_provenance_join_order`;
- restoration-sort work accounting.

The replacement preserves the useful idea — physical non-contiguous selectivity exploitation — while removing the post-hoc order repair mechanism.

## 5. Hostile / regression evidence

Pass48 verifies:

- the previous three-way non-contiguous hostile query exactly matches logical reference Bag rows/order through order-preserving semijoin enumeration;
- a four-way fixture with selective non-contiguous predicate structure exactly matches logical reference rows, multiplicity, column order and row order;
- equality compatibility keys come only from resolved primitive pinned-Γ modules;
- completed tuples are exact-Γ revalidated before output;
- duplicate physical representatives remain distinct ordinal choices, preserving Bag multiplicity;
- existing persisted-index-preferred singleton access is not stolen by the mask path;
- the old provenance/final-sort symbols are absent from production source;
- no new `#[allow(...)]` site was introduced relative to Pass47 (19 existing sites remain);
- no `unsafe`, TODO/FIXME, `todo!` or `unimplemented!` appears in `crates/`;
- debug/release/overflow behavior agrees.

Diagnostic release benchmark on the pre-existing adversarial three-way fixture after the hostile rewrite:

```text
baseline median:   5,395,958 ns
optimized median:     36,284 ns
ratio_milli:          148,714   (~148.7x)
```

This is workload-specific evidence that removing the final sort did not destroy the prior optimization; it is not a universal Join-speed claim.

Freeze result in `kernel-plan`: **120 passed, 0 failed, 1 ignored diagnostic benchmark**.

## 6. Verification

Final Rust 1.98.1 gate after the hostile rewrite and final source freeze:

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

Workspace metrics at freeze:

- 349 declared Rust tests;
- 121 `kernel-plan` tests (120 normal + 1 ignored diagnostic benchmark);
- 21 crates;
- 47,692 Rust LOC under `crates/`;
- 0 external Cargo registry/git sources;
- 0 `unsafe` in `crates/`;
- 0 TODO/FIXME/todo!/unimplemented! markers in `crates/`.

Evidence is retained under `evidence/pass48/` inside the workspace.

## 7. Problem ledger

### CLOSED exactly in Pass48 — 2

1. ✅ **The first permutation design preserved correctness by destroying output order and repairing it with `ProvenanceJoinRow` + final global sort.** Pass48 removes that production mechanism. Γ-canonical semijoin compatibility masks prune candidates while final tuples are enumerated directly in original leaf/scan order, eliminating post-hoc order restoration and tuple-provenance sorting.
2. ✅ **Non-contiguous selectivity optimization was effectively a three-way special case.** Pass48 adds bounded subset selectivity search for 3–8 leaves and verifies a real four-way query. The bounded planner can admit an order-preserving semijoin-mask execution when it beats the current contiguous estimate, while respecting persisted-index-preferred access.

### Historical OPEN from Pass47 fully closed this pass

**0 / 22.** The broad general multiway/bushy item remains OPEN because the current bounded search is capped at eight leaves, mask execution currently targets the builtin primitive canonical equality fragment, arbitrary indexed/typed subset execution is not unified with the mask representation, and richer cardinality/correlation modeling remains absent.

### Advanced but still OPEN

1. 🟨 **General nested/multiway/bushy Join planning.** 3–8 leaf subset selectivity search and order-preserving non-contiguous execution are verified. Unbounded/general search, cyclic/wider physical algorithms, richer predicate graphs and adaptive search-space control remain OPEN.
2. 🟨 **Semijoin mask representation.** Current masks are execution-local and rebuilt per admitted execution. Persisted/shared mask caches, typed-native key extraction, SIMD/roaring/dense-vs-sparse representations and memory-aware selection remain OPEN physical work.
3. 🟨 **Indexed access inside general permuted/subset execution.** Existing persisted singleton-side access is respected, but an arbitrary subset node is not yet a first-class consumer/producer of every `JoinAccessDecision` family.
4. 🟨 **Statistics ecosystem and multi-family lifecycle.** Histograms/correlation/autonomous telemetry and lifecycle ownership across specialized I64/statistics/masks/future layouts remain OPEN.
5. 🟨 **I64 Group/TopK constant-factor debt.** Unchanged by Pass48.

### Historical / active OPEN after Pass48 — 22

1. ⬜ Structural/custom-equivalence canonical indexing and typed production.
2. ⬜ General nested/multiway/bushy Join planning beyond the verified bounded 3–8-leaf primitive equality semijoin-mask fragment: unbounded/adaptive search, richer predicate graphs and fully general indexed/typed subset execution.
3. ⬜ First-class multi-family physical access advisor/lifecycle across specialized I64, generic semantic indexes, retained statistics, semijoin masks and future layouts.
4. ⬜ Autonomous workload statistics/telemetry, retention decay and lifecycle scheduling.
5. ⬜ Exact physical-index/statistics/mask byte/resident-memory budgeting and rebuild scheduling.
6. ⬜ Canonical-key encoding/version migration for long-lived physical caches/statistics.
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

### New OPEN created in Pass48

**0.** Mask representation/lifecycle/performance requirements refine the existing general multiway and multi-family physical-state frontiers rather than creating a new independent architectural subsystem.

## 8. Result / next direction

Pass48 changes the physical architecture materially: non-contiguous Join optimization no longer means "execute in a convenient order and sort the answer back." It means "derive selective compatibility/pruning structures, then enumerate the answer in the required order from the beginning."

The next coherent planner step is to make the compatibility layer itself adaptive: dense bitsets vs sparse ordinal lists / existing persisted indexes, multi-predicate composite masks, richer selectivity/correlation statistics, and a general subset-node access interface. Only then should the 8-leaf search cap be relaxed; removing the cap before controlling search/mask memory would create exponential planner cost without a clean physical policy.
