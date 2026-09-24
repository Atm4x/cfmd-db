# CFMD Pass53 Report — R&D Integration: Structural Γ Keys + Relation→Adjacency Lowering

**Status:** VERIFIED on Rust 1.98.1.

**Production source window:** 2026-09-20 19:59:41 → 20:07:04 +03:00 (**7m23s**). Production source was frozen after that point; final verification found no correctness defect, so the freeze was not removed.

## 1. Scope

Pass53 integrates and hostile-reviews the four production candidates from `CFMD_RND_NONJOIN_PASS1_PASS2_PASS51_2026-09-20(1).zip` against authoritative Pass52. The rejected SemanticBucketIndex experiment is deliberately not integrated.

Integrated production files:

- `crates/kernel-semantics/src/lib.rs`
- `crates/kernel-query/src/lib.rs`
- `crates/kernel-fixpoint/src/lib.rs`
- `crates/kernel-schema/src/lib.rs`

No Join / Γ-QCN planner code was changed in Pass53.

## 2. Structural Γ-canonical keys

`CanonicalEqKey` now composes through structural equivalence definitions admitted by pinned Γ:

- Product
- Option
- Sum
- Seq
- Set
- Bag
- Map
- guarded Mu/Var recursion through the existing structural-equivalence checker

Primitive semantic modules remain the base authority. Host `Eq/Hash/Ord` is not substituted for Γ equality. Unordered Set/Bag/Map representations are canonicalized independently of physical entry order.

`ensure_unique` now uses exact canonical quotient keys rather than O(n²) pairwise semantic comparison.

### Independent hostile law check

Pass53 added an oracle test independent of the canonical implementation:

```text
canonical_E(x) == canonical_E(y)  <=>  registry.equivalent(E, x, y)
```

across hostile Product / Option / Sum / Set / Bag / Seq / Map samples, including order permutations, ASCII-CI representatives, multiplicity changes and value swaps. The existing guarded recursive structural test also checks equal/different recursive values against the new canonical law.

This passed under debug, release and overflow-check release gates.

### Durable boundary retained

Pass53 does **not** admit structural keys into persisted semantic-index encoding. `KEY_ENCODING_REVISION=1` is unchanged. Workspace search confirms durable semantic-index paths remain on their existing primitive/binding contracts. Recursive structural key encoding/version migration therefore remains OPEN rather than being silently introduced.

## 3. MaterializedSetSupportState lowering

`MaterializedSetSupportState` now maintains:

```text
CanonicalRowKey -> support slot
```

instead of semantic vector scans for build/probe/delta maintenance.

Delta application first groups removals/insertions by canonical class, validates every underflow, then commits changes. This removes the previous whole-support clone used for atomicity while preserving all-or-nothing behavior and representative retention semantics.

Existing atomic-underflow/context-binding tests remain green, and the R&D structural Product consumer test now runs on authoritative Pass53.

## 4. Fixpoint relation → adjacency lowering

`kernel-fixpoint::solve` now deterministically compiles the edge relation once:

```text
BTreeSet<(source,target)> -> BTreeMap<source, Vec<target>>
```

before BFS.

Pass53 added a hostile reference solver using the old whole-relation scan and compared **the entire certificate**, not only the reachable set, over 32 deterministic graph fixtures. Reachable nodes, rank and parent witness choice are byte-for-byte equal.

The checker/certificate boundary is unchanged.

## 5. Schema inclusion → adjacency lowering

`Schema` retains `inclusions` as the exact direct-inclusion relation and maintains private `inclusion_parents` as a deterministic derivative adjacency. `include` updates it only when a new direct pair is admitted; `is_subtype` traverses adjacency instead of rescanning the complete relation per frontier node.

Pass53 hostile-computed transitive closure from the authoritative `inclusions` relation and compared every subtype pair on a deterministic DAG. Duplicate `include` calls remain idempotent. Full durability checkpoint round-trip tests also pass; decode reconstructs adjacency only through `Schema::include`.

Thus adjacency is redundant derived state, not a second schema authority.

## 6. R&D performance evidence retained

The original bundle raw measurements are copied under `evidence/pass53/rnd_bundle/`. They were **not rerun as Pass53 benchmark claims**; they are retained as provenance for why the integration was worth reviewing.

Median R&D evidence:

- structural support build: 1,769,998 µs → 25,972 µs (~68.15x);
- structural support probe: 874,916 µs → 3,633 µs (~240.82x);
- reachability N=8000: 273,228 µs → 16,328 µs (~16.73x);
- schema chain build/cycle checks: 11,431,034 ns → 1,100,687 ns (~10.39x);
- 20 subtype queries: 456,506,683 ns → 9,230,385 ns (~49.46x).

These are fixture-specific R&D measurements, not universal performance guarantees.

## 7. Rejected bucket experiment remains rejected

The R&D chunked/stable-slot SemanticBucketIndex alternatives are not present in production Pass53. Their deletion improvement came with material build/read regressions. The confirmed skew-deletion defect remains a multi-family/advisor problem rather than a global container replacement.

## 8. Verification

Final Rust 1.98.1 gate on frozen production source:

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

Initial combined release and overflow invocations that hit external compile timeouts were not counted; each unfinished command was rerun on warmed targets to an actual PASS.

Workspace metrics:

- 362 declared Rust tests;
- 130 `kernel-plan` tests;
- 21 crates;
- 49,911 Rust LOC under `crates/`;
- 19 pre-existing `#[allow(...)]` sites, no new suppression;
- 0 external Cargo registry/git sources;
- 0 `unsafe` in `crates/`.

## 9. Problem ledger

### CLOSED exactly in Pass53 — 4

1. ✅ Structural Γ equivalence had no compositional exact in-memory canonical key across the admitted structural constructors.
2. ✅ `MaterializedSetSupportState` still paid semantic vector scans/full-state clone despite exact structural quotient information being available.
3. ✅ Reachability solving rescanned the complete edge relation for every frontier vertex instead of lowering Relation→adjacency once.
4. ✅ Schema subtype/cycle reachability rescanned the complete inclusion relation per frontier node instead of using an exact maintained derivative adjacency.

### Historical OPEN from Pass52 fully closed this pass

**0 / 22.** Historical item #1 is materially advanced, but it includes custom/plugin canonical laws, persisted structural index families and typed structural Join/Group execution, none of which Pass53 claims closed.

### Historical / active OPEN after Pass53 — 22

The count remains **22**. In particular:

- structural builtin/compositional canonicalization is now production, but custom/plugin canonicalization and persisted structural index encoding remain OPEN;
- maintained Join structural keys remain on exact GenericScan fallback;
- Γ-QCN structural/custom factors remain OPEN;
- SemanticBucketIndex skew mutation remains an advisor-controlled multi-family problem;
- Pass52 insertion/resurrection Γ-QCN activation remains OPEN;
- all prior lifecycle/memory/durability/distribution/formal items remain unchanged.

### New OPEN created in Pass53

**0.** Durable structural key encoding and adaptive bucket families were already represented by historical canonical-key migration / multi-family advisor frontiers.

## 10. Result

Pass53 confirms the R&D thesis on authoritative code: several hot paths were paying a **generalization tax** because the physical layer ignored exact mathematics already admitted by Γ or by the finite relation model.

The common physical-compilation pattern is now explicit:

```text
semantic law / finite relation
        -> exact canonical or adjacency derivative
        -> specialist physical operation
        -> original semantic checker/authority unchanged
```

The next structural integration should not be a blind sweep. The highest-value follow-up is to route the newly proved structural canonical law into selected maintained Group/Join/index families under an explicit non-durable/persisted boundary, while Pass52 Γ-QCN insertion activation remains a separate mathematical frontier.
