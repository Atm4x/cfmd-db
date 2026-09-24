# Implementation report — Pass108

## Production change

`crates/kernel-query/src/lib.rs`

- `MaterializedRelPlanState` now owns reusable V4 scheduler scratch as reconstructible runtime-only state.
- Manual `Clone` resets scratch instead of copying transient scheduler contents.
- Manual `PartialEq/Eq` excludes scratch from semantic/revision identity.
- `plan_relation_deltas_execgraph` reuses and restores scratch across transitions; failure paths reset it.

No operator semantics, Γ contracts, source-version semantics, `GraphPatchSet` semantics, public delta representation, or revision publication protocol were changed.

## Rejected experiment

A specialized I64 Group singleton dense-move commit was prototyped and benchmarked. It did not materially improve the corrected Group benchmark and was removed before freeze. Pass108 therefore contains no unsupported micro-specialization from that experiment.

## Evidence

See `PASS108_STAGE6_MATRIX/` for raw five-process outputs and explicit closure-test output. See `PASS108_REPORT.md` for interpreted results and closure decision.
