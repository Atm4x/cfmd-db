# PASS57 REPORT — R&D Program-2 COW/path-copy integration

Status: **VERIFIED**

Source window: **2026-09-20 21:18:24 → 21:27:26 +03:00 = 9:02**.
Production source was frozen at 21:27:26. All later changes are reports/spec/evidence/package only.

## R&D bundle review

Reviewed bundle:
`CFMD_RND_GENERIC_PROGRAMS_PASS4_PASS56_REVIEW_2026-09-20.zip`
SHA-256: `1e8b95cab88eb26344bb64fbfd0a749c119e0f99ac2fa4102e0e6fcdc7e9ed58`.

The archive checksum manifest passed. Its Pass56 rebase note was correct: the only mechanical conflict in the full Pass54 patch is the relation-multiset region already superseded by Pass55/56.

Disposition:

- **MERGED:** Program 2 maintained-query path-copy / heavy physical-root COW / source-root freshness / affected-only logical validation.
- **NOT MERGED:** Program-1 `EqClassId` / `TransientEqClassInterner` and validation rewrite. Pass55 already supplies canonical row multisets; the R&D validation patch also assumes canonicalization is always present and therefore weakens the explicit exact fallback boundary for future custom/plugin equality.
- **NOT MERGED:** no replay of the old Program-1 relation multiset hunk; Pass55 remains authoritative.

## Closed exactly in Pass57

### 1. Maintained query-state clone tax

`MaterializedRelPlanState` previously deep-cloned the recursive `MaintainedRelPlanNode` tree. The root is now `Arc<MaintainedRelPlanNode>`; mutation uses `Arc::make_mut` only along touched recursive paths.

Hostile test `maintained_plan_clone_uses_cow_and_isolates_mutation` proves:
- a fresh clone shares the maintained node root;
- mutating the clone detaches the path;
- original output remains unchanged;
- candidate output reflects the delta.

The R&D 100k-row Scan clone fixture measured median proxy about `21.663 ms/clone` before path-copy and `383 ns/clone` after. This is clone-dominated fixture evidence only.

### 2. Heavy `PhysicalStore` derivative deep-copy tax

Heavy reconstructible physical families are now stored behind shared COW roots:
- installed relations;
- persisted I64 indexes;
- generic semantic indexes;
- Γ-QCN quotient factors;
- Γ-QCN support state;
- semantic statistics.

Mutation uses `Arc::make_mut` only for affected roots. The outer `BTreeMap` metadata remains ordinary map metadata; therefore this is **not** claimed as a fully persistent map/root implementation.

New hostile test `physical_store_clone_cow_isolates_relation_mutation` proves on the authoritative Pass57 baseline:
- relation and I64-index roots are shared after clone;
- one normal resolved relation delta detaches both affected roots;
- the original relation/index remain unchanged;
- the candidate relation/index agree with the mutation.

Pass57 release diagnostic on 200k-row relation:
- relation-only clone: **279 ns**;
- store with persisted I64 index: **206 ns**.

The difference between the two nanosecond values is noise; the evidence is that payload size/index payload no longer produces millisecond-scale store clone cost.

### 3. Whole logical-state clone during transition validation

`RuntimeRevisionBundle::validate_target_logical_state` no longer begins by cloning the complete `DatabaseState`.

It now:
1. compares lifecycle/carriers/fields directly;
2. compares untouched relation maps directly;
3. reconstructs only relations named by the transition;
4. compares reconstructed affected rows with the target revision.

Prepared physical freshness no longer retains a deep source `PhysicalStore`; it carries an in-process `Arc<()>` root identity witness plus epoch/revision checks. Every physical mutation/publication path that advances the physical root rotates that identity.

Existing stale-transition and competing-transition hostile tests remain green, plus the R&D affected-only transition test is integrated.

## Historical OPEN accounting

Pass56 historical OPEN count: **22**.

- Historical OPEN fully closed this pass: **0 / 22**.
- Historical OPEN remaining: **22**.
- Genuinely new OPEN created: **0**.

Historical item #9, `Clone-heavy runtime transaction candidates → COW/persistent roots`, is substantially advanced but intentionally remains OPEN because `PhysicalStore` still owns ordinary `BTreeMap<K, Arc<State>>`. Cloning the store therefore still copies map metadata in `O(number of artifacts)`. A true persistent map/HAMT/B-tree-like root remains the final closure for very large artifact catalogs.

## R&D candidates explicitly not promoted

`EqClassId` remains R&D until a repeated runtime consumer demonstrates a real benefit over Pass55 canonical row keys without creating a second classification representation.

The R&D validation Set-uniqueness rewrite was not promoted because future custom/plugin equality may remain exact without a canonical key; production must preserve the pairwise exact fallback unless that contract changes explicitly.

## Verification

Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`.

PASS:
- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo test --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets --release`
- `cargo build --workspace --release`
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`
- `RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release`

The first overflow-check release invocation hit the external compilation timeout and was not counted; the warmed retry completed PASS.

Static snapshot:
- 369 declared tests;
- 133 `kernel-plan` tests;
- 73 `kernel-query` tests;
- 21 crates;
- 50,812 Rust LOC;
- 0 `unsafe` hits;
- 19 existing `#[allow(...)]`, no new suppression;
- 0 external registry/git Cargo sources.

## Next frontier

The clean continuation is now either:

1. finish historical #9 with a persistent artifact-map/root representation, or
2. return to Γ-QCN insertion/resurrection activation Dq from Pass52.

They should remain separate passes: persistent-root ownership and semantic fixed-point derivatives have independent falsification surfaces.
