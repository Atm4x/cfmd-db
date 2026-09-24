# CFMD workspace

Executable research kernel for the Constructive Finite Model Database design.

This workspace intentionally has no third-party dependencies. The current vertical slice contains:

- `kernel-types`: stable nominal IDs and revision identities;
- `kernel-schema`: semantic symbols and versioned semantic environment `Γ`;
- `kernel-change`: universal `Change<T>` plus a fine-grained set delta;
- `kernel-query`: exact queries, universal recompute derivative, `Impact`, derivative-law checker;
- `kernel-lifecycle`: root/reachability lifecycle normalization and LCA intent merge;
- `kernel-model`: finite structural values, carriers, relations, revision state;
- `kernel-proof`: tiny PlanIR fragment and a structural proof checker for one optimizer rewrite;
- `storage-memory`: in-memory revision commit path.

The goal is falsification, not API polish. Each semantic claim added to the code should have an executable law test.
