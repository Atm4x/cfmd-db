# CFMD PASS490 — Canonical Multi-Root RelObservationForest

Status: production kernel primitive / executable equivalence boundary.

PASS490 turns the PASS489 shared-subtree theorem into one multi-root maintained Γ-DTC execution owner.

## Selected law

`RelObservationForest` interns canonical-identical `RelExpr` subtrees once per semantic world. A canonical state cell is distinct from an operator/root occurrence. Parent fanout preserves occurrence multiplicity: a self-join has one shared Scan state cell but two edges into the Join left/right input slots.

The forest executes exact source deltas through one sparse fanout scheduler. Every influenced canonical cell is transitioned at most once per forest transition; its exact differential output is then cloned only onto fanout edges. It reuses the existing node-local Γ-DTC transition and patch kernels; there is no second relational executor.

## Executable equivalence

`p490_multi_root_forest_shares_state_cells_and_matches_independent_gamma_dtc` compares forest roots against independent `MaterializedRelPlanState` execution across shared Scan/Filter plus Project, Distinct/Set support, Difference/blocker, Join, AntiJoin, Group, TopKWithTies and Union before and after exact source deltas.

`p490_self_join_shares_one_scan_cell_but_preserves_two_occurrence_edges` proves the occurrence/state split directly: one Scan cell + one Join cell, two input edges, and each unique cell transitions once.

## Frontier correction

One global capsule revision is invalid for a multi-root forest: roots can depend on different relation subsets. The forest therefore separates:

- a world/sweep anchor used to order shared reconstruction transitions;
- per-root dependency-frontier revisions used by `RelCausalCapsule` identity.

A root frontier moves only when a source relation of that root changes, preserving the PASS485 unrelated-churn law while allowing cells to be shared across roots.

## Runtime carrier

`RelCausalCapsule` can now reference either a single maintained plan or one root of a shared forest. This is derived runtime representation only. Durable identity remains effect + observation id + observed frontier + exact `RelExpr`; no forest/node id is persisted.

## Complexity

Transition work is output-sensitive to influenced unique semantic cells plus fanout edges. Structural node count is `N`, not total per-root occurrence count. This establishes the prerequisite for replacing PASS488 per-lineage reopen with one forest rewind.

## Deferred integration

PASS490 freezes after kernel equivalence. PASS491 should replace the PASS488 reopen lineage array with one `RelObservationForest`, emit root capsules that share one forest Arc per reconstructed frontier, and measure history visits / unique nodes / actual node transitions / root routes. No recovery-only alternate semantics are permitted.
