# IMPLEMENTATION REPORT — PASS84

Pass84 rebases the verified historical #7 semantic-core recovery contract onto the current Pass83 production architecture.

## Production changes

`kernel-durability` now exposes `DurableArtifactCore` and metadata codec v11 persists ObservableAtom canonical-key tuples by durable relation occurrence ordinal. `DurableRevisionStore` owns the core set alongside physical recipes and exposes core-aware create/checkpoint rotation adapters while retaining the previous public APIs as empty-core compatibility paths.

`kernel-semantics::observable` exposes the checked canonical-equivalence-key intern operation needed to reconstruct revision-local observable classes from a persisted canonical key without treating a physical handle as semantic identity.

`kernel-plan` derives cores from live ObservableAtom states on durable create/checkpoint/config publication, replays checkpoint cores across committed relation WAL mutations under pinned Γ first-match semantics, and rehydrates compatible cores only after authoritative relations have been rebuilt with fresh row handles. Invalid/stale cores are discarded and exact artifact rebuild remains the fallback. Recovery reporting distinguishes `rehydrated` from `rebuilt`.

## Authority / dependency boundary

No second model state was introduced. Core contents are reconstructible acceleration data; `Revision=(S,Γ,M)` remains logical authority. `PhysicalRowId` is not durable. Process-local Pass83 advisor telemetry is not persisted into recovery semantics. Schema/Γ/full-revision transitions invalidate rather than speculatively transport an ObservableAtom core.

## Frozen source delta

Exactly five Rust files differ from Pass83:

- `crates/kernel-durability/src/lib.rs`
- `crates/kernel-durability/src/metadata.rs`
- `crates/kernel-durability/src/store.rs`
- `crates/kernel-plan/src/lib.rs`
- `crates/kernel-semantics/src/observable.rs`

Approximate textual delta versus Pass83: durability lib `+16/-0`, metadata `+98/-6`, store `+51/-6`, plan `+854/-98`, observable `+9/-0`.

## Verification

Rust 1.98.1; external target directory. Workspace fmt/check/strict Clippy/full tests all pass. Test discovery reports **628** tests with **8 ignored**. Targeted final reruns: `kernel-plan --lib` 235/0/4 and `kernel-durability --lib` 50/0/0. Source freeze was 2026-09-22 19:58:42 UTC and no Rust source was edited afterwards.

Historical #7 is therefore **PROD CLOSED**. Pass85 starts from #2 PWRC; Pass84 contains audit/preparation only for #2 and makes no production-closure claim for it.
