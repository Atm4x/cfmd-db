# PASS119 HISTORICAL PROBLEMS LEDGER

Authoritative production status after Historical #20 formal surface-to-kernel mechanization closure.

## PROD CLOSED — 21 / 22

- #1 structural/custom semantic physical persistence and ordering — CLOSED Pass82.
- #2 PWRC positive recursive Bag execution — CLOSED Pass85.
- #3 unified physical lifecycle/capability/convergence — CLOSED Pass82.
- #4 autonomous telemetry/controller — CLOSED Pass83.
- #5 resource accounting / pressure separation — CLOSED Pass83.
- #6 Revision/Γ-bound OrderedView/pagination — CLOSED Pass90.
- #7 recovery rebuild economics + durable semantic-core rehydrate/WAL replay — CLOSED Pass84.
- #8 durable causal effect ledger / REIC branch lifecycle — CLOSED Pass92.
- #9 canonical durable-format migration registry — CLOSED Pass88.
- #10 semantic implementation package/auth/deployment — PROD CLOSED Pass116.
- #11 idempotency epochs / bounded exact retry history / payload GC — CLOSED Pass86.
- #12 streaming/chunked checkpoint + PreparedCutCapsule + exact shadow WAL — CLOSED Pass89.
- #14 authority-uncertainty restart / poison policy — CLOSED Pass87.
- #15 barrier-safe group commit / non-authoritative async batching — CLOSED Pass87.
- #16 replication/consensus runtime — PROD CLOSED Pass115.
- #17 authenticated durable store + external freshness/anti-rollback anchor — PROD CLOSED Pass118.
- #18 formal immutable-generation publication/rename/fsync/GC proof — PROD CLOSED Pass112.
- #19 bounded repair / VMF-OFC / verified observation transport — CLOSED Pass88.
- #20 formal surface-to-kernel mechanization — **PROD CLOSED Pass119**.
- #21 maintained I64 Group constant-factor debt — PROD CLOSED Pass108.
- #22 maintained TopK constant-factor debt — PROD CLOSED Pass108.

## OPEN — 1 / 22

- #13 supported-platform real durability assurance — OPEN. Closure requires named supported platform/filesystem/device profiles and destructive or equivalent real power-loss/fault evidence for the filesystem axioms consumed by #18 and the external-authority persistence used by #17. Process-kill tests or mocked filesystems are insufficient.

## #20 closure evidence

1. Lean 4.34 core-only `SurfaceKernel.lean` theorem artifact.
2. complete normative surface-feature mapping proved bijective.
3. structural type elaboration and guarded recursion/free-variable admission preservation.
4. full current relational query vocabulary mechanized.
5. exact physical-plan erasure and no-hidden-node theorem.
6. certificate-soundness theorem matching production `LoweringChecker` premises and independent of physical algorithm choice.
7. fail-closed source binding to the normative spec table, `TypeExpr`, `RelExpr`, `Plan`, guarded-recursion validator, `LoweringChecker`, capability/inclusion, retention, violation and rewrite witnesses.
8. all Pass119 Lean/refinement gates PASS.
9. all 85 production Cargo/crate files are byte-identical to the fully gated Pass118 production snapshot.
