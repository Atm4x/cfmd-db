# PASS85 HISTORICAL PROBLEM LEDGER

Authoritative production baseline: frozen Pass85 over Pass84. Whole-row status only.

| # | Historical problem | Production status after Pass85 | Note |
|---:|---|---|---|
| 1 | structural/custom semantic physical persistence and ordering | **PROD CLOSED** | Pass82. |
| 2 | general nested/multiway/bushy + positive recursion | **PROD CLOSED** | Pass85 PWRC: grounded support + exact compact N∞ multiplicity + Γ-pinned prepared execution. |
| 3 | unified cross-family physical lifecycle/advisor ontology | **PROD CLOSED** | Pass82. |
| 4 | autonomous telemetry/correlation/decay/hysteresis/scheduling | **PROD CLOSED** | Pass83. |
| 5 | shared resource/memory pressure | **PROD CLOSED** | Pass83. |
| 6 | layouts / OrderedView / pagination | OPEN | Deferred. |
| 7 | recovery rebuild economics | **PROD CLOSED** | Pass83+84. |
| 8 | durable revision DAG / effect history | OPEN | Existing Γ-REIC substrate is relevant to #11 but this whole row remains open. |
| 9 | general historical durable-format migration | OPEN | Deferred. |
| 10 | executable semantic plugin packaging/deployment | OPEN | Deferred. |
| 11 | transaction retry retention/GC | **OPEN — NEW INTEGRATION BLOCKER** | Old portable reference conflicts with post-Pass81 Γ-REIC under epoch-qualified raw-id reuse; all attempted Pass85 changes reverted. |
| 12 | streaming/chunked checkpoints | OPEN | Deferred. |
| 13 | real platform durability assurance | OPEN | Deferred. |
| 14 | poison/restart policy | OPEN | Portable R&D closed; production rebase pending. |
| 15 | group commit / async durability | OPEN | Portable R&D closed; production rebase pending. |
| 16 | replication / consensus / distribution | OPEN | Deferred. |
| 17 | durable-store authentication | OPEN | Deferred. |
| 18 | formal rename/fsync/GC theorem | OPEN | Deferred. |
| 19 | bounded transaction repair runtime | OPEN | Portable R&D closed; production rebase pending. |
| 20 | remaining formal mechanization | OPEN | Deferred. |
| 21 | maintained Group constant-factor debt | OPEN | Benchmark/lowering gate. |
| 22 | maintained TopK constant-factor debt | OPEN | Benchmark/lowering gate. |

Whole-row accounting after Pass85: **6 PROD CLOSED** (#1/#2/#3/#4/#5/#7); **16 not fully closed**.

## #11 blocker discovered in Pass85

The portable #11 contract remains correct, but its old reference implementation is insufficient after later causal-history work:

- retry identity must be composite `(epoch, tx-id)`, not a raw-id map plus a side epoch map;
- Γ-REIC effect identity must not equal raw tx-id if raw ids can be reused in a later epoch;
- effect allocation must replay deterministically from checkpoint + WAL after a crash;
- causal validation cannot require an exact retry intent that the retention policy is allowed to erase; durable causal records must retain their own canonical causal information;
- hostile closure must include crash after new-epoch reuse but before the next checkpoint.

## Next order

1. **#11 retry retention/GC redesigned against current Γ-REIC — Pass86.**
2. #14 poison/restart policy.
3. #15 group commit / async durability.
4. #19 bounded transaction repair runtime.
