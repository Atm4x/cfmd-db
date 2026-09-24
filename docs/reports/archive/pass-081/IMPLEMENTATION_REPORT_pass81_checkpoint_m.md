# CFMD Pass81 Checkpoint M implementation report

Status: **PROD PARTIAL / DISTRIBUTED DEBUG+CLIPPY VERIFIED**.

Checkpoint M adds the first production relational writable-view compiler substrate. It carries revision-local semantic column provenance, validates observable bindings against schema Γ-equivalence, chooses exactly one owner side, and makes `Project`/`Filter`/`Join` writability conditional on explicit complement, APNF determinant, DTC guard, and VMF invariant obligations. Ambiguous owner joins and operators without an explicit action policy fail closed.

Verification: final fmt/check for `kernel-lens`, Clippy `-D warnings`, and 14 kernel-lens tests pass; distributed all-crate checks/Clippy pass; distributed debug tests total **554 passed / 0 failed / 8 ignored**.

Not closed: executable view-change inversion into `RelationDelta`, dynamic obligation certificates, durable publication of the lifted relational Rewrite, REIC/cube coherence, or historical restore/GC enforcement.
