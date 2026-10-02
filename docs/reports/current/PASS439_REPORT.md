# PASS439 REPORT — SEMANTIC GRANULAR AUTHORIZATION SUBSTRATE

Start: **2026-10-02 17:33:32 UTC**  
Functional freeze: **2026-10-02 17:49:30 UTC**  
Useful boundary: **2026-10-02 17:53:32 UTC**  
Hard boundary: **2026-10-02 17:57:32 UTC**

## CLOSED THIS PASS

- Introduced one `kernel-query::RelReadFootprint` calculus over the existing `RelExpr` IR. It derives relation + stable field dependencies structurally, with no row scan, SQL/table fallback, frontend-specific policy engine, or physical-realization routing.
- Footprint semantics are information-flow aware: projection pulls only observed source columns; predicates/order/top-k add controlling columns; joins add key coordinates; anti-join adds the right key; Group adds group/aggregate inputs; Difference/Union/Distinct include full row-equality dependencies where hidden values can alter membership/deduplication.
- `ReadContext::prepare`, `PreparedQuery::execute`, Candidate query execution, and Watch creation enforce the same footprint through shared runtime authority. Historical factorized reads therefore inherit the same law rather than receiving a historical ACL engine.
- Added granular `Permission::{ReadRelation, ReadField, WriteRelation, WriteField}` while preserving coarse `Read`/`Write` as explicit whole-database grants.
- Object relation columns no longer use ordinal `1..N` identities. Stored object fields use the existing P438 `cfmd.object.kernel-field.v1` semantic identity as their relation-column ID; relationship carrier columns receive stable named semantic IDs.
- `commit_bound_plan` rechecks current session authority at publication. Whole relation mutations require relation-write authority; P438 object field patches require the exact field-write coordinate. Existing plans do not freeze stale grants across refresh/revocation.
- Hostile regression proves: a one-field read grant admits `Scan -> Project(field)` but rejects full-row `Scan`; a one-field write grant admits the matching object field patch but rejects raw whole-relation insert.

## R&D LAW SELECTED

```text
Authorize(observation Q)
    = authorize(semantic_read_footprint(Q))

semantic_read_footprint(Q)
    : RelExpr -> finite set of
        Relation(r)
      + Field(r, stable_semantic_column_id)

Authorize(change C)
    = authorize(exact semantic write coordinates already carried by C)
```

The authorization footprint is a semantic dependency law, not merely output projection. A coordinate belongs to the footprint whenever changing it can alter the observable result through filtering, ordering, grouping, equality, deduplication, join membership, or set/bag behavior.

## HOSTILE FINDINGS / REJECTED

- **REJECTED:** `scan_relations()` as authorization. It cannot express field secrecy.
- **REJECTED:** authorize all columns of every scanned relation as the universal path. That is safe but destroys legitimate field-granular projection and is not the semantic dependency law.
- **REJECTED:** Context omission as security.
- **REJECTED:** Rust/Python/host-specific ACL engines.
- **REJECTED:** ordinal object-column identity for policy; authorization must survive source rename/reorder/physical layout evolution.
- **KEPT FAIL-CLOSED:** dedicated create/delete/relationship grants are not exposed yet because exact mutation-action classification is not closed. Current granular whole-relation mutation authority remains `WriteRelation`; field patches remain `WriteField`.

## TOOLCHAIN / CRYPTO PROBE

The supplied Rust 1.98.1 distribution was reduced to `rustc + cargo + host rust-std`; `rustc 1.98.1` / `cargo 1.98.1` compiled and executed a standalone probe. Repository compilation then successfully built the actual crypto dependency path including `aes 0.9.3` and `aes-gcm-siv 0.12.1`. The supplied Rust tar archive was deleted after validation as requested.

## VERIFICATION ON FROZEN SOURCE

- `cfmd-runtime`: **48 passed / 0 failed** across unit + async-watch + end-to-end; includes new granular authorization hostile.
- `kernel-query --lib`: **143 passed / 10 ignored**.
- `kernel-plan --lib`: **297 passed / 5 ignored**.
- `cfmd` public surface: **47 passed / 0 failed**.
- `cfmd` public API contract: **1 passed / 0 failed**.
- `cfmd-host`: **8 passed / 0 failed**.
- `cargo check -p cfmd -p cfmd-host`: PASS.
- `cargo check -p kernel-query -p cfmd-runtime`: PASS; real AES/AES-GCM-SIV dependency path compiled.

## PERFORMANCE BASELINES TO PRESERVE

- Authorization footprint construction is O(query IR + referenced semantic coordinates), independent of row count and physical atom count.
- Runtime grant checking scales with the finite footprint/grant set, never per row.
- Preserve P397-P400 native-cost realization baselines and P435 persistent-sharing baselines; authorization stays above physical execution and must not become a per-cell/per-row router.

## OPEN — IMMEDIATE

1. Exact mutation-action classifier for object create/delete and relationship attach/detach/move. Reuse lifecycle/relationship/change authority; do not flatten these to generic relation writes.
2. Introduce dedicated create/delete/relationship grants only after the classifier and commit enforcement are exact.
3. History/undo/redo authorization against the exact coordinates of the historical effect being materialized, while `HistoryRead` remains distinct metadata/history visibility authority.
4. Hosted/watch hostile for granular grants under refresh/revocation: Watch must require both lifecycle `Watch` authority and the same read footprint after grant changes.
5. Close write-only DX: a principal allowed to mutate one field should not need read authority over unrelated hidden fields merely because patch formation internally inspects the authoritative row. Internal DB reads must remain non-observable and not become external read grants.

## OPEN — DEFERRED / MANDATORY LEDGER CARRY

- safe create/delete and final Database/Context DX cleanup after authorization action closure;
- deterministic `Matches`/regex, richer Semantic Rules, entity/model invariants, transaction `require` on the common serialized expression substrate;
- migration frontend Rust/Python/TMD/CLI DSL + diagnostics;
- final Python/.NET/Studio surfaces, backup/restore/corruption UX, public perf/binary budgets, native Windows secure-memory expansion;
- transient/wide persistent-tree + sorted bulk builder R&D remains performance-only;
- storage/history/Physical Realization line remains closed absent new hostile evidence.

## SUPERSEDED / DO NOT EXTEND

- Context-shape-as-security;
- frontend-specific authorization engines;
- ordinal relation-column identity for object security policy;
- blanket all-columns authorization as replacement for semantic dependency analysis;
- SQL/table ACL fallback beneath semantic query/change kernels.

## NEXT RECOMMENDED PASS

**PASS440 — exact change-action authorization closure:** classify create/delete/relationship/field/relation mutations through the existing object/lifecycle/change semantics, carry exact authorization into history undo/redo and hosted/watch refresh, and remove the field-writer dependency on unrelated read grants without exposing hidden values.
