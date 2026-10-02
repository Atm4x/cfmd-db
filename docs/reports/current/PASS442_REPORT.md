# PASS442 REPORT — ROLE COMPOSITION + MODEL / SCHEMA-MIGRATION AUTHORITY

Start: **2026-10-02 19:29:19 UTC**  
Functional freeze: **2026-10-02 19:44:59 UTC**  
Useful boundary: **2026-10-02 19:49:19 UTC**  
Hard boundary: **2026-10-02 19:53:19 UTC**

## CLOSED THIS PASS

- Added public immutable `Role` as named DX composition over the existing exact `PermissionSet` law.
- `Role` has no runtime hierarchy, inheritance, policy evaluator, durable role registry, or role-name enforcement. `Session::from_roles` / `PermissionSet::from_roles` flatten grants once into ordinary permissions.
- Added explicit `Permission::ModelRead`.
- Full `ReadContext::schema()` now returns `Result<SchemaView>` and requires `ModelRead`; ordinary `Read`, `ReadRelation`, and `ReadField` no longer disclose the complete authoritative model.
- Added narrow `ReadContext::schema_revision()` metadata. A reader can observe the current schema epoch without obtaining full model authority; this is intentional groundwork for the still-open remote-reader migration DX.
- Added explicit `Permission::SchemaMigrate` and `SessionDatabase::migrate`; generic `Write` does not grant schema migration.
- Schema migration performs early admission before deterministic preparation and then revalidates/holds current `SchemaMigrate` authority across the durable publication call. Expensive preparation therefore does not hold the shared session lock, while revocation/publication has one linearized boundary.
- P441 hosted-watch closeout was rerun first and passed.
- Full hostile host verification exposed a pre-existing terminal-error race: blocked watch cancellation after session revocation could surface as protocol `Internal`, and authorization refresh could race between `PermissionDenied` and `WatchClosed`.
- `SessionRevoked` now maps to hosted `SessionClosed`. A blocked watch awakened by cancellation revalidates current authorization before returning ordinary `WatchClosed`; exact grant loss therefore resolves to `PermissionDenied`, terminal session revocation to `SessionClosed`, ordinary user cancellation to `WatchClosed`.
- Full protocol verification exposed an obsolete P301 test assumption that output-equivalent revisions emit empty watch events. Current P348+ law intentionally quotients empty revisions; the protocol hostile was corrected to verify an observable event spanning the exact revision interval instead of waiting forever for a suppressed empty event.

## SELECTED AUTHORIZATION LAW

```text
Role(name, grants...)
        |
        | flatten once
        v
PermissionSet
        |
        v
shared RuntimeAuthority

DataRead != ModelRead
DataWrite != SchemaMigrate

migration:
    authorize SchemaMigrate
    -> deterministic preparation
    -> authorize/hold SchemaMigrate at durable publication
    -> publish
```

`Role` is therefore DX only. It cannot create authority absent from its constituent permissions and cannot diverge from field/relation/object/relationship/history/watch enforcement added in P439-P441.

## HOSTILE / REJECTED

1. **REJECTED: role-name or role-hierarchy authorization.** That would introduce a second semantics layer and make refresh/revocation depend on mutable role interpretation.
2. **REJECTED: `Read` implies model introspection.** A principal allowed one field must not enumerate hidden authoritative relations/columns/rules.
3. **REJECTED: hide schema epoch together with the model.** Remote-reader compatibility needs the narrow epoch/version metadata without requiring `ModelRead`.
4. **REJECTED: `Write` implies schema migration.** Data mutation and semantic-model publication are different capabilities.
5. **REJECTED: hold the session authority lock during migration compilation/preparation.** Only durable publication needs a linearization boundary.
6. **REJECTED: accept nondeterministic hosted termination codes after authorization change.** Current authority is rechecked on cancellation wake.
7. **REJECTED: restore empty watch events to satisfy an obsolete protocol test.** Output-equivalent revisions remain quotiented by the established watch law.

## VERIFICATION ON FROZEN FUNCTIONAL CODE

- P441 hosted exact-read revocation rerun: **PASS**.
- `cfmd-runtime`: **3 unit + 9 async-watch + 39 end-to-end = 51 passed / 0 failed**.
- new role/model hostile: **PASS**.
- new schema-migration authority hostile: **PASS**.
- public `cfmd`: **49 passed / 0 failed**.
- public API contract: **1 passed / 0 failed**.
- `cfmd-host`: **8 passed / 0 failed** after terminal-authority race fix.
- `cfmd-protocol`: **5 hosted + 4 wire = 9 passed / 0 failed** after obsolete empty-event hostile correction.
- No row/data-cardinality work was added to authorization; roles flatten only when constructing/refeshing authority, and model/schema checks are constant/permission-set work.

## LEDGER — CLOSED THROUGH P442

- Context partial scalar/reference/relationship mutation substrate and field-granular semantic coordinates: closed through P438.
- Exact query read footprints + field/relation authorization: closed P439.
- Exact create/delete/attach/detach/move + write-only mutation authority: closed P440.
- Durable history inverse/redo semantic authority + exact watch refresh/revocation: closed P441.
- Named role composition without a parallel engine: closed P442.
- Full model metadata separated from ordinary data read authority: closed P442.
- Schema migration separated from generic data write authority: closed P442.

## LEDGER — OPEN IMMEDIATE

1. Hostile-audit any **actually exposed** administrative/maintenance operations before assigning new permissions: backup/restore, key-management, retention/compaction, corruption recovery. Do not create speculative dead grants for operations not reachable through `SessionDatabase`/hosted product surface.
2. P441 same-relation history-coverage precomposition remains fail-closed. Generalize to an interval/span calculus only if a real transaction-composition case requires it; do not build unused abstraction.
3. If the administrative audit finds no live bypass, freeze the authorization architecture and move to deterministic Semantic Rules / common serialized expression substrate.

## LEDGER — R&D / REMOTE-READER SCHEMA-EVOLUTION DX

Syntax is deliberately **not selected**. `bind/rebind` is not an accepted design.

Required properties carried forward:

- reader service may contain no authoritative schema definition;
- DB/server has no reader-specific contract markers or configuration;
- no dedicated compatibility handshake;
- ordinary read metadata exposes schema epoch/version; P442's `schema_revision()` now provides the local runtime half of this without `ModelRead`;
- reader may select compatibility deterministically from the schema epoch;
- same source/persisted field spelling may have different semantics after migration, therefore existence/name fallback is invalid;
- resolution happens once at Context/ReadContext binding for an epoch, never per query/row;
- uncovered/incompatible epochs fail closed;
- authoritative schema remains current-only and legacy-free;
- compatibility history belongs to the reader/consumer side;
- target remains the smallest clean DX for predeploying a reader before A→B semantic migration without a cutover/restart race.

## LEDGER — OPEN DEFERRED / MANDATORY CARRY

- deterministic `Matches`/regex and richer serialized Semantic Rules;
- entity/model invariants;
- transaction `require` / semantic preconditions on the common expression substrate;
- final Context/Database creation/open DX cleanup after remote-reader compatibility design is selected;
- migration frontend Rust/Python/TMD/CLI + diagnostics;
- final Python/.NET/Studio surfaces;
- backup/restore/corruption UX and explicit admin authority when those product surfaces are exposed;
- public performance/binary-size budgets;
- native Windows secure-memory expansion;
- transient/wide persistent-tree sorted bulk-builder R&D remains performance-only.

## SUPERSEDED / DO NOT EXTEND

- Context shape as authorization;
- frontend/host-specific ACL engines;
- ordinal/source-name security coordinates;
- role-name / role-hierarchy enforcement separate from `PermissionSet`;
- generic `Write` as schema/admin authority;
- full model disclosure as a side effect of data read;
- per-query reader schema-version routing;
- current-world mixed A/B schema routing and other previously superseded migration fallbacks.

## PERFORMANCE BASELINES TO PRESERVE

- authorization work stays structural in finite semantic footprints/grants, independent of row count and physical atom count;
- role composition is O(total grants in selected roles) at authority construction/refresh, not on hot query execution;
- migration preparation carries no session-lock duration; only the final durable publication is protected by current migration authority;
- preserve P397-P400 native-cost realization class, P435 persistent-sharing behavior, and no O(data) semantic migration cutover regression.

## NEXT RECOMMENDED PASS

**PASS443 — authorization closeout + Semantic Rules transition.** Audit only live administrative surfaces for capability leaks; if no reachable seam remains, explicitly freeze authorization architecture and start deterministic `Matches` / common semantic-expression law for field rules, entity/model invariants, and transaction `require` rather than creating another validator engine.
