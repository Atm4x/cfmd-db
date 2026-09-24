# PASS112 formal proof map — historical #18

Lean artifact SHA-256: `50ac70af7dc7557de18c8790c8f12ec117d0f48cba8aa7a5d99ffbc754f7857f`

| Obligation | Lean theorem / artifact |
|---|---|
| P18.1 Unique authority | `P18_1_unique_authority` |
| P18.2 No premature authority | `P18_2_no_premature_authority` |
| P18.3 Old-authority safety | `P18_3_old_authority_before_rename` |
| P18.4 Rename uncertainty | `authority_uncertainty_exact`, `P18_4_rename_uncertainty` |
| P18.5 Publication closure | `required_components_durable_before_rename`, `P18_5_publication_closure` |
| P18.6 Recovery closure | `P18_6_recovery_closure` |
| P18.7 GC non-interference | `P18_7_gc_non_interference` |
| P18.8 GC crash safety | `P18_8_gc_crash_safety` |
| P18.9 Generation monotonicity | `P18_9_generation_monotonicity` |
| P18.10 Production refinement | `P18_10_fault_point_refinement`, `P18_10_event_mapping_exact`, `check_refinement.py` |

The formal model is deliberately a durability proof, not a claim about every filesystem implementation. Its filesystem assumptions are explicit and the remaining real-platform validation obligation is tracked by historical #13.

The artifact also defines the explicit finite `PublishTransition` and `Reachable` relations and proves every reachable publication phase crash-safe.
