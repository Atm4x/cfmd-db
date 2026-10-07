# CFMD Security R&D — migration / access / database-control authority separation

## Status

R&D / hostile security review only. No production authorization law was changed in this branch.

Baseline: PASS579 (`PUBLIC PROTECTION RECONFIGURATION ADMINISTRATION DX`).

The review started from the concern that `SchemaMigrate`, schema-owned roles/capabilities and whole-database administration all derive from mutable `Schema.Access`, even though PASS579 correctly models their individual permissions as orthogonal.

Architectural clarification after the hostile review: the intended product model is stricter than merely making these permissions orthogonal. CFMD has two authority planes. Schema roles are client/data-plane roles only. Database administration, including schema publication/migration, is control-plane authority authenticated by the database connection credential/key and is never granted through `Schema.Access`. Here `key` means an authentication/connection credential (JWT-claim-like authority carrier), not a storage-encryption key or KEK/DMK.

## Executive conclusion

The concern is valid.

PASS579's local operation gates are correct: `ProtectionReconfigure` is distinct from `SchemaMigrate` and authorization precedes provider execution. The deeper problem is authority provenance.

Current architecture permits a principal holding only `SchemaMigrate` to construct a `MigrationModel` containing `approve_access_change(...)`. That approval is copied into the kernel migration program and is treated as sufficient authorization for the exact access-policy subject. There is no independent runtime authority check for the approval itself.

Consequently the mutable schema policy can authorize a transition that changes the meaning of the same policy graph. This violates the desired root-of-trust separation.

A second independent leak exists: data-dependent migration preparation can be used as a read oracle over data the migration principal cannot read. A third issue is delayed revocation because schema roles are flattened into `Session::PermissionSet` and do not automatically follow later authoritative schema-policy narrowing.

## Confirmed finding 1 — CRITICAL: migration access-policy "approval" is self-asserted

Current chain:

```text
SessionDatabase::migrate / prepare / execute
    requires Permission::SchemaMigrate

MigrationModel::approve_access_change(subject)
    stores subject in the model

compile_migration_program(...)
    copies those subjects into SchemaMigrationProgram

SchemaMigrationTransport::verify_with_access_approvals(...)
    checks only that every detected changed subject appears in the supplied set
```

There is no second `RuntimeAuthority` permission proving that the caller may approve an access-policy change.

This is not merely naming debt. An executable R&D regression demonstrates:

1. source role has `SchemaMigrate` only;
2. `DatabaseAdministration::Export` and `ProtectionReconfigure` capabilities exist but are not assigned to that role;
3. migration target widens the same role;
4. the migration model self-approves the role change;
5. restricted migration publication succeeds with only `SchemaMigrate`;
6. after ordinary authoritative-role refresh, the same principal owns `Export` and `ProtectionReconfigure`;
7. the principal exports an encrypted source database under an attacker-controlled encryption key and can reopen that export with that key.

The persistence protection floor correctly rejects an encrypted -> plaintext downgrade; the exploit does not rely on such a downgrade. Export under a different equally strong caller-controlled key is sufficient because `Export` is intentionally whole-database data authority.

This also means self-granting `Restore`, `AuthorityTransfer` or `PersistenceTransition` is structurally possible if their well-known capabilities are admitted into the target role.

## Confirmed finding 2 — CRITICAL: self-approval defeats an otherwise useful access-transport theorem

The kernel transport does detect an important class of indirect declassification.

R&D attempted:

```text
secret relation S: unreadable by role
public relation P: readable by role
migration: P := Query::scan(S)
```

with `P` changing schema shape and `Schema.Access` source/target definitions otherwise authored to preserve the same role intent.

Without an access approval, verification fails with:

```text
AccessCapabilityContractChanged(...)
```

This is good: the transport recognizes that the effective meaning of `ReadRelation(P)` changed and does not silently call the policy preserved.

However, the same `SchemaMigrate` caller can add:

```text
approve_access_change(MigrationAccessSubject::Capability(...))
```

and publication succeeds. The already-authorized `ReadRelation(P)` then exposes rows copied from the previously unreadable source relation.

Therefore the lower theorem is useful; the broken boundary is the provenance of the approval.

## Confirmed finding 3 — CRITICAL/HIGH: target rules form a blind read oracle

This leak requires no access-policy change and no migration publication.

Executable regression:

- relation `secret` contains three rows;
- principal has `SchemaMigrate` and no data read entry;
- `SessionDatabase::snapshot()` correctly returns `PermissionDenied`;
- principal proposes a target schema containing:

```text
ModelRuleExpr::RelationExactCountRange {
    relation: secret,
    predicate: SemanticRuleExpr::True,
    min: k,
    max: Some(k),
}
```

- `prepare_migration(k = 2)` fails;
- `prepare_migration(k = 3)` succeeds.

The caller has learned exact information about unreadable data through migration success/failure.

The public rule language is substantially stronger than this minimal exploit: exact sums, grouped aggregates and ordered statistics/quantiles exist. Therefore data-dependent migration validation must be treated as an observation of its source footprint, not as harmless schema administration.

Sanitizing error text is insufficient because the success/failure bit itself is the oracle.

## Confirmed finding 4 — HIGH: schema role narrowing does not revoke already-created sessions

`SchemaView::session_for_roles(...)` flattens current role IDs into a `PermissionSet`. `Session` retains that set, but not the role assignment or authoritative schema-access epoch.

Executable regression:

1. source role grants `ReadRelation(secret)`;
2. session is created from that role and can read the secret relation;
3. an unrestricted administrator publishes a legitimate target schema narrowing the role to no read capability;
4. the already-created session continues reading the secret relation;
5. only explicit external `SchemaView::refresh_session_roles(...)` finally removes the authority.

The live-session machinery correctly propagates an explicit `refresh_permissions` or `revoke` through derived values, but authoritative schema-policy changes do not trigger either transition themselves.

For security-sensitive role revocation, this is a fail-open stale-authority window of unbounded duration unless the host performs a perfectly coordinated refresh.

## Database-key / protection review

No direct raw-key disclosure was found in the inspected key-memory path.

Positive properties observed:

- `EncryptionKey`, `StorageEncryptionKey`, `SecureBytes`, `SecureBox`, `SecretHandle` and `EncryptionKeyDestination` redact contents from `Debug`;
- provider initialization can write directly into the secure key destination;
- `StorageEncryptionKey` retains bytes in hardened `SecureBytes` pages;
- `SecureBytes::try_from_array` wraps the moved input in `Zeroizing` before copying into hardened memory;
- long-lived keys/AEAD state are handle-shared rather than byte-cloned;
- PASS579's `ProtectionReconfigure` permission gate is executed before provider resolution; its hostile panic-provider regression is green in this R&D environment;
- rewrap does not hand the database master key to the provider. The provider supplies the wrapping key.

Known/accepted limitation remains: direct `EncryptionKey::from_bytes([u8; 32])` necessarily begins from caller-owned bytes, and the storage-encryption protection profile already records `ConstructorTransientsMayExist`. This review found no new accidental formatting/error/debug leak of those bytes.

The security problem is therefore not primarily raw key handling. It is that the authority to reach key-management and export operations can currently be manufactured by a schema migration through the self-approved policy path.

## Root architectural issue

The intended authority model is a strict client/control-plane split, while the current trust graph is effectively self-hosting:

```text
Schema.Access
   |
   +-- data roles/capabilities
   +-- SchemaMigrate
   +-- DatabaseAdministration::{Export, Restore, AuthorityTransfer,
                                 PersistenceTransition, ProtectionReconfigure}

SchemaMigrate
   |
   +-- can publish the next Schema.Access
```

A mutable policy should not be the sole root that authorizes replacing the policy itself or acquiring database-root operations. More strongly, a schema/client role must not be a source of database-control authority at all.

`PrincipalId -> RoleId` assignment being external is not sufficient, because migration can change the meaning of an externally assigned stable `RoleId`. Likewise, merely renaming `SchemaMigrate` to a different schema capability would not fix the trust boundary.

## Required two-plane authority model

The product requirement is a strict separation between **Schema/client authority** and **Database/control authority**. They are not two kinds of roles inside one permission graph. They have different roots, different issuance paths and different mutation laws.

```text
DATA / CLIENT PLANE
connection identity -> external client role assignment -> Schema.Access -> model permissions

CONTROL / DATABASE PLANE
database admin credential/key -> database-control claims -> administration operations
```

There is no promotion edge from the first plane into the second. A schema migration may change `Schema.Access`, but it cannot create, widen, rotate or otherwise manufacture database-control claims.

### 1. Keep client/data authorization in the authoritative Schema

`Schema.Access` remains the natural owner of model-relative permissions:

```text
ReadRelation / ReadField
WriteRelation / WriteField
object / relationship actions
History / Watch
possibly ModelRead
```

This preserves the desired property that migrations can retarget stable semantic field/entity identities and access policy evolves with the model. Client roles may be widened, narrowed, renamed or semantically retargeted by an authorized database migration because they are part of the schema contract.

However, no client role is an administration role. In particular, no ordinary database user/client receives schema migration/publication authority through `Schema.Access`.

### 2. Database administration is credential-bound control authority

Introduce a database-root authority domain that is authenticated from the credential/key used for the administrative database connection and cannot be modified by schema migration:

```text
DatabaseControlPermission::SchemaPublish
DatabaseControlPermission::AccessPolicyAdmin
DatabaseControlPermission::MigrationDataInspect / Declassify
DatabaseControlPermission::Export
DatabaseControlPermission::Restore
DatabaseControlPermission::AuthorityTransfer
DatabaseControlPermission::PersistenceTransition
DatabaseControlPermission::ProtectionReconfigure
```

The credential behaves conceptually like a signed claim carrier: after authentication it resolves to an immutable/authoritative set of database-control permissions for that connection/session. The exact transport may be a key-backed credential, token, certificate or another authenticated handle; the architecture must not depend on JWT specifically.

This `key` is **not** an encryption key. Storage encryption keys, KEKs and the DMK remain cryptographic material; the administrative credential is an authentication/root-authority object that determines which database-control operations the connection may invoke.

Exact naming can change. The important laws are:

```text
Schema.Access cannot grant DatabaseControlPermission.
Schema migration cannot mint or widen an admin credential.
Admin credential rotation does not require a schema migration.
Client-role rotation does not alter database-control authority.
```

`SchemaMigrate` as a schema permission should therefore disappear entirely from `Schema.Access`. Publishing/migrating the authoritative schema is a database-control operation authorized by the administrative connection credential.

### 3. Make the connection/session types reflect the split

The runtime should make accidental cross-plane authority difficult to express. Conceptually:

```text
ClientSession
    principal / externally assigned client roles
    Schema.Access-derived model permissions
    NO database-control permissions

AdminSession (or equivalent control context)
    authenticated database administrative credential
    DatabaseControlPermission claims
    NO authority derived from Schema.Access
```

This does not require two network protocols or two database engines. It requires two authorization roots. A deployment may use one transport, but the authenticated credential class and permission namespace must make the distinction explicit.

If an operator also needs ordinary data access, that access should be granted explicitly through the client/data plane rather than inferred from administration authority. Conversely, possession of a client role must never imply any database-control permission.

### 4. Delete `MigrationModel::approve_access_change(...)` as an authorization mechanism

A migration model may describe a target schema. It must not carry its own security approval.

Instead static compilation computes an exact security-impact artifact:

```text
MigrationSecurityImpact {
    access_policy_changes,
    target_observation_flows,
    source_data_dependencies,
    data_dependent_validation_dependencies,
    integrity_policy_changes,
}
```

Approval is supplied separately by database-root authority and is bound to the exact prepared migration identity, source revision and target digest.

A token/certificate design is preferable if prepare and execute are separated:

```text
AccessPolicyApproval {
    source_revision,
    migration_digest,
    exact_change_digest,
    authority_generation,
}
```

Changing any part of the migration invalidates the approval.

### 5. Add a declassification/noninterference theorem

For each retained schema role `r`, let `View_A^r` and `View_B^r` be the data observations authorized before and after migration `M : A -> B`.

An ordinary schema migration that does not carry explicit declassification authority should satisfy:

```text
View_B^r o M = F_r o View_A^r
```

for some exact `F_r`.

In words: everything role `r` can observe after migration must be computable from what that role was already allowed to observe before migration.

If this factorization does not exist, the migration contains a declassification edge and requires an independently authorized declassification decision.

The existing access-transport machinery already catches at least some instances of this idea; the secret->public test failed closed before a self-approval was supplied. That machinery should be strengthened and its approval provenance fixed, not discarded.

### 6. Separate static migration compilation from data-sensitive certification

Current `prepare_migration` constructs/validates target data, so its observable result may depend on secrets.

Recommended split:

```text
plan / compile
    static schema + transform verification
    MUST NOT inspect protected database values

certify_data
    evaluates data-dependent transforms/rules
    exact source observation footprint is known
    requires MigrationDataInspect/Declassify authority where necessary

execute
    consumes the exact sealed certification
```

Alternatively, prove that a blind migration's caller-visible result is noninterfering with unauthorized source data. Returning generic errors alone is not sufficient.

### 7. Make schema-role sessions epoch-bound rather than flattened forever

A role-bound hosted session should retain external role assignments and the schema-access epoch, not only a flattened `PermissionSet`.

At every authority check (with caching allowed):

```text
if session.access_epoch != current_schema.access_epoch:
    re-resolve externally assigned RoleIds against current authoritative Schema.Access
    or fail closed pending reauthorization
```

A removed role fails closed. Narrowing must take effect before the next authorized operation. Widening may either become visible automatically or require explicit host confirmation, depending on product policy; revocation must not require a best-effort manual refresh.

Database-control grants are separate, are authenticated from the administrative connection credential/key, and are never recomputed from schema roles.

### 8. Optional high-assurance dual control

For especially dangerous transitions, policy may require different principals:

```text
SchemaPublish principal != AccessPolicyAdmin / Declassify principal
```

This is not required for the basic authority separation theorem, but it is a natural safety option for hosted/large installations.

Protection rotation / external authority transfer can independently require their own control principals.

## What should remain from PASS579

Keep:

- operation-specific `DatabaseAdministration` distinctions;
- authorization before provider/IO side effects;
- poison-boundary correction for invalid protection requests;
- structured recovery diagnostics;
- no implicit `SchemaMigrate => ProtectionReconfigure` implication in the permission enum.

Change the provenance/ownership of those authorities. PASS579 solved the local operation gate; this R&D finds that the current Schema-owned root is too mutable for those gates to be a complete security boundary. Database administration must be resolved from the administrative connection credential/key, while `Schema.Access` remains exclusively client/data-plane policy.

## R&D executable evidence

Added test-only file:

`crates/cfmd-runtime/tests/security_migration_rnd.rs`

Confirmed exploits/regressions:

1. `schema_migrate_can_self_widen_role_into_database_export_after_role_refresh`
2. `schema_migrate_can_self_approve_effective_acl_change_and_declassify_secret_rows`
3. `schema_migrate_can_use_target_model_rules_as_blind_oracle_over_unreadable_data`
4. `schema_role_narrowing_does_not_revoke_existing_session_until_external_refresh`

All four PASS in the R&D branch because they intentionally assert the current vulnerable/unsafe behavior exists.

Additional positive gate:

`security::tests::protection_reconfigure_permission_precedes_provider_resolution` PASS.

## Production recommendation

Do **not** continue projecting the PASS579 authority model into protocol/Python as final security semantics before this root split is resolved.

Recommended next production/R&D sequence:

1. introduce the strict two-plane model: `Schema.Access` for client/data permissions only, credential-bound `ControlAuthority` for database administration;
2. remove `SchemaMigrate`/database administration from the schema permission namespace and authorize schema publication from the administrative connection credential;
3. ensure there is no runtime or migration path that promotes a schema/client role into database-control claims;
4. remove self-contained access approvals from `MigrationModel`;
5. compile exact `MigrationSecurityImpact`;
6. add noninterference/declassification checks and data-sensitive prepare gating;
7. bind role-based client sessions to authoritative access epoch for monotone revocation;
8. define independent lifecycle/rotation/revocation for administrative credentials;
9. only then project the settled authority model to protocol/Python.

This is pre-release code, so preserving the current Schema-owned administration compatibility surface is not recommended. The intended invariant is strict: **no database client/user role can migrate or administer the database merely by virtue of `Schema.Access`; database-control authority exists only in the separately authenticated administrative credential plane.**
