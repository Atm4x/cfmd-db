use std::{
    collections::BTreeSet,
    fmt,
    sync::{Arc, RwLock},
};

use crate::{
    CommitOutcome, Database, Error, ErrorKind, MigrationHistoryPolicy, MigrationModel,
    MigrationObservation, MigrationPlan, MigrationPreview, MigrationSecurityImpact,
    MigrationSecurityImpactDigest, MigrationValidation, PreparedMigration, PrincipalId, Result,
    RevisionId, TransactionId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum DatabaseControlPermission {
    SchemaPublish,
    AccessPolicyAdmin,
    MigrationDataInspect,
    Declassify,
    Export,
    Restore,
    Fork,
    AuthorityTransfer,
    PersistenceTransition,
    ProtectionReconfigure,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DatabaseControlPermissionSet(BTreeSet<DatabaseControlPermission>);

impl DatabaseControlPermissionSet {
    #[must_use]
    pub const fn new() -> Self {
        Self(BTreeSet::new())
    }

    #[must_use]
    pub fn with(mut self, permission: DatabaseControlPermission) -> Self {
        self.0.insert(permission);
        self
    }

    #[must_use]
    pub fn contains(&self, permission: DatabaseControlPermission) -> bool {
        self.0.contains(&permission)
    }

    pub fn iter(&self) -> impl Iterator<Item = DatabaseControlPermission> + '_ {
        self.0.iter().copied()
    }
}

impl FromIterator<DatabaseControlPermission> for DatabaseControlPermissionSet {
    fn from_iter<T: IntoIterator<Item = DatabaseControlPermission>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<const N: usize> From<[DatabaseControlPermission; N]> for DatabaseControlPermissionSet {
    fn from(value: [DatabaseControlPermission; N]) -> Self {
        Self(value.into_iter().collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseControlSnapshot {
    generation: u64,
    permissions: DatabaseControlPermissionSet,
    revoked: bool,
}

impl DatabaseControlSnapshot {
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn permissions(&self) -> &DatabaseControlPermissionSet {
        &self.permissions
    }

    #[must_use]
    pub const fn is_revoked(&self) -> bool {
        self.revoked
    }
}

#[derive(Debug)]
struct DatabaseControlState {
    generation: u64,
    permissions: DatabaseControlPermissionSet,
    revoked: bool,
}

#[derive(Clone)]
pub struct DatabaseControlCredential {
    principal: PrincipalId,
    state: Arc<RwLock<DatabaseControlState>>,
}

impl fmt::Debug for DatabaseControlCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DatabaseControlCredential")
            .field("principal", &self.principal)
            .field("snapshot", &self.snapshot().ok())
            .finish_non_exhaustive()
    }
}

impl DatabaseControlCredential {
    /// Constructs the mutable control-plane root after an external authenticator has
    /// verified one administrative credential. Only this root may rotate/revoke claims;
    /// derived `DatabaseControlSession`s are use-only live views.
    #[must_use]
    pub fn authenticated(
        principal: PrincipalId,
        permissions: DatabaseControlPermissionSet,
    ) -> Self {
        Self {
            principal,
            state: Arc::new(RwLock::new(DatabaseControlState {
                generation: 0,
                permissions,
                revoked: false,
            })),
        }
    }

    #[must_use]
    pub const fn principal(&self) -> PrincipalId {
        self.principal
    }

    #[must_use]
    pub fn session(&self) -> DatabaseControlSession {
        DatabaseControlSession {
            principal: self.principal,
            state: Arc::clone(&self.state),
        }
    }

    pub fn snapshot(&self) -> Result<DatabaseControlSnapshot> {
        snapshot_control_state(&self.state)
    }

    pub fn rotate_permissions(&self, permissions: DatabaseControlPermissionSet) -> Result<u64> {
        let mut state = self.state.write().map_err(|_| control_state_poisoned())?;
        if state.revoked {
            return Err(control_revoked(self.principal));
        }
        state.generation = state.generation.checked_add(1).ok_or_else(|| {
            Error::new(
                ErrorKind::Internal,
                "database-control generation overflowed",
            )
        })?;
        state.permissions = permissions;
        Ok(state.generation)
    }

    pub fn revoke(&self) -> Result<bool> {
        let mut state = self.state.write().map_err(|_| control_state_poisoned())?;
        if state.revoked {
            return Ok(false);
        }
        state.generation = state.generation.checked_add(1).ok_or_else(|| {
            Error::new(
                ErrorKind::Internal,
                "database-control generation overflowed",
            )
        })?;
        state.revoked = true;
        Ok(true)
    }
}

#[derive(Clone)]
pub struct DatabaseControlSession {
    principal: PrincipalId,
    state: Arc<RwLock<DatabaseControlState>>,
}

impl fmt::Debug for DatabaseControlSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DatabaseControlSession")
            .field("principal", &self.principal)
            .field("snapshot", &self.snapshot().ok())
            .finish_non_exhaustive()
    }
}

impl PartialEq for DatabaseControlSession {
    fn eq(&self, other: &Self) -> bool {
        self.principal == other.principal && Arc::ptr_eq(&self.state, &other.state)
    }
}

impl Eq for DatabaseControlSession {}

impl DatabaseControlSession {
    #[must_use]
    pub const fn principal(&self) -> PrincipalId {
        self.principal
    }

    pub fn snapshot(&self) -> Result<DatabaseControlSnapshot> {
        snapshot_control_state(&self.state)
    }

    fn approve_migration_for_database(
        &self,
        database_identity: u64,
        impact: &MigrationSecurityImpact,
    ) -> Result<MigrationSecurityApproval> {
        let state = self.state.read().map_err(|_| control_state_poisoned())?;
        if state.revoked {
            return Err(control_revoked(self.principal));
        }
        require_impact_approval_permissions(self.principal, &state.permissions, impact)?;
        Ok(MigrationSecurityApproval {
            database_identity,
            source_revision: impact.source_revision(),
            migration_id: impact.migration_id(),
            impact_digest: impact.digest(),
            impact: impact.clone(),
            authority_principal: self.principal,
            authority_generation: state.generation,
            authority: self.clone(),
        })
    }

    pub fn require(&self, permission: DatabaseControlPermission) -> Result<()> {
        let state = self.state.read().map_err(|_| control_state_poisoned())?;
        if state.revoked {
            return Err(control_revoked(self.principal));
        }
        if state.permissions.contains(permission) {
            Ok(())
        } else {
            Err(control_denied(self.principal, permission))
        }
    }

    fn with_permission<T>(
        &self,
        permission: DatabaseControlPermission,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let state = self.state.read().map_err(|_| control_state_poisoned())?;
        if state.revoked {
            return Err(control_revoked(self.principal));
        }
        if !state.permissions.contains(permission) {
            return Err(control_denied(self.principal, permission));
        }
        operation()
    }
}

fn snapshot_control_state(
    state: &Arc<RwLock<DatabaseControlState>>,
) -> Result<DatabaseControlSnapshot> {
    let state = state.read().map_err(|_| control_state_poisoned())?;
    Ok(DatabaseControlSnapshot {
        generation: state.generation,
        permissions: state.permissions.clone(),
        revoked: state.revoked,
    })
}

#[derive(Clone)]
pub struct MigrationSecurityApproval {
    database_identity: u64,
    source_revision: RevisionId,
    migration_id: u128,
    impact_digest: MigrationSecurityImpactDigest,
    impact: MigrationSecurityImpact,
    authority_principal: PrincipalId,
    authority_generation: u64,
    authority: DatabaseControlSession,
}

impl fmt::Debug for MigrationSecurityApproval {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MigrationSecurityApproval")
            .field("database_identity", &"<runtime-bound>")
            .field("source_revision", &self.source_revision)
            .field("migration_id", &self.migration_id)
            .field("impact_digest", &self.impact_digest)
            .field("authority_principal", &self.authority_principal)
            .field("authority_generation", &self.authority_generation)
            .finish_non_exhaustive()
    }
}

impl MigrationSecurityApproval {
    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.source_revision
    }

    #[must_use]
    pub const fn migration_id(&self) -> u128 {
        self.migration_id
    }

    #[must_use]
    pub const fn impact_digest(&self) -> MigrationSecurityImpactDigest {
        self.impact_digest
    }

    #[must_use]
    pub const fn authority_principal(&self) -> PrincipalId {
        self.authority_principal
    }

    #[must_use]
    pub const fn authority_generation(&self) -> u64 {
        self.authority_generation
    }

    pub(crate) fn verify_for(
        &self,
        database_identity: u64,
        impact: &MigrationSecurityImpact,
    ) -> Result<()> {
        if self.database_identity != database_identity
            || self.source_revision != impact.source_revision()
            || self.migration_id != impact.migration_id()
            || self.impact_digest != impact.digest()
            || self.impact != *impact
        {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "migration security approval does not match the exact sealed migration impact",
            ));
        }

        let state = self
            .authority
            .state
            .read()
            .map_err(|_| control_state_poisoned())?;
        if state.revoked {
            return Err(control_revoked(self.authority_principal));
        }
        if state.generation != self.authority_generation {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                format!(
                    "migration security approval from principal {} was issued at control generation {}, current generation is {}",
                    self.authority_principal.raw(),
                    self.authority_generation,
                    state.generation
                ),
            ));
        }
        require_impact_approval_permissions(self.authority_principal, &state.permissions, impact)
    }
}

fn require_impact_approval_permissions(
    principal: PrincipalId,
    permissions: &DatabaseControlPermissionSet,
    impact: &MigrationSecurityImpact,
) -> Result<()> {
    if !impact.access_policy_changes().is_empty()
        && !permissions.contains(DatabaseControlPermission::AccessPolicyAdmin)
    {
        return Err(control_denied(
            principal,
            DatabaseControlPermission::AccessPolicyAdmin,
        ));
    }
    if !impact.declassification_edges().is_empty()
        && !permissions.contains(DatabaseControlPermission::Declassify)
    {
        return Err(control_denied(
            principal,
            DatabaseControlPermission::Declassify,
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ControlAuthority {
    Unrestricted,
    Session(DatabaseControlSession),
}

impl ControlAuthority {
    pub(crate) fn require(&self, permission: DatabaseControlPermission) -> Result<()> {
        match self {
            Self::Unrestricted => Ok(()),
            Self::Session(session) => session.require(permission),
        }
    }

    pub(crate) fn with_permission<T>(
        &self,
        permission: DatabaseControlPermission,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        match self {
            Self::Unrestricted => operation(),
            Self::Session(session) => session.with_permission(permission, operation),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AdminDatabase {
    database: Database,
    authority: ControlAuthority,
}

impl AdminDatabase {
    pub(crate) fn new(database: Database, session: DatabaseControlSession) -> Self {
        Self {
            database,
            authority: ControlAuthority::Session(session),
        }
    }

    #[must_use]
    pub fn control_session(&self) -> &DatabaseControlSession {
        match &self.authority {
            ControlAuthority::Session(session) => session,
            ControlAuthority::Unrestricted => unreachable!("AdminDatabase is always restricted"),
        }
    }

    pub fn backup_to(
        &self,
        path: impl Into<std::path::PathBuf>,
        encryption: &crate::Encryption,
    ) -> Result<crate::BackupVerification> {
        self.authority.require(DatabaseControlPermission::Export)?;
        self.database.backup_to(path, encryption)
    }

    pub fn fork_to(
        &self,
        path: impl Into<std::path::PathBuf>,
        encryption: &crate::Encryption,
    ) -> Result<Database> {
        self.authority.require(DatabaseControlPermission::Fork)?;
        self.database.fork_to(path, encryption)
    }

    pub fn fork_to_with_external_freshness(
        &self,
        path: impl Into<std::path::PathBuf>,
        encryption: &crate::Encryption,
        target_freshness: &crate::ExternalFreshness,
    ) -> Result<Database> {
        self.authority.require(DatabaseControlPermission::Fork)?;
        self.database
            .fork_to_with_external_freshness(path, encryption, target_freshness)
    }

    pub fn persist(&self, path: impl Into<std::path::PathBuf>) -> Result<()> {
        self.authority
            .require(DatabaseControlPermission::PersistenceTransition)?;
        self.database.persist(path)
    }

    pub fn persist_with_encryption(
        &self,
        path: impl Into<std::path::PathBuf>,
        encryption: &crate::Encryption,
    ) -> Result<()> {
        self.authority
            .require(DatabaseControlPermission::PersistenceTransition)?;
        self.database.persist_with_encryption(path, encryption)
    }

    pub fn make_volatile(&self) -> Result<()> {
        self.authority
            .require(DatabaseControlPermission::PersistenceTransition)?;
        self.database.make_volatile()
    }

    pub fn reconfigure_protection(&self, next: &crate::Encryption) -> Result<u64> {
        self.authority
            .require(DatabaseControlPermission::ProtectionReconfigure)?;
        self.database.reconfigure_protection(next)
    }

    pub fn transfer_authority_to(
        &mut self,
        target_path: impl Into<std::path::PathBuf>,
        target_encryption: &crate::Encryption,
        target_freshness: &crate::ExternalFreshness,
    ) -> Result<()> {
        self.authority
            .require(DatabaseControlPermission::AuthorityTransfer)?;
        self.database
            .transfer_authority_to(target_path, target_encryption, target_freshness)
    }

    pub fn plan_migration(&self, model: &MigrationModel) -> Result<MigrationPlan> {
        self.authority
            .require(DatabaseControlPermission::SchemaPublish)?;
        self.database
            .plan_migration_with_control(model, &self.authority)
    }

    pub fn validate_migration(&self, model: &MigrationModel) -> Result<MigrationValidation> {
        self.authority
            .require(DatabaseControlPermission::SchemaPublish)?;
        self.database
            .validate_migration_with_control(model, &self.authority)
    }

    pub fn approve_migration(
        &self,
        validation: &MigrationValidation,
    ) -> Result<MigrationSecurityApproval> {
        self.control_session().approve_migration_for_database(
            self.database.runtime_identity(),
            validation.security_impact(),
        )
    }

    pub fn prepare_migration(&self, model: &MigrationModel) -> Result<PreparedMigration> {
        self.authority
            .require(DatabaseControlPermission::SchemaPublish)?;
        self.database
            .prepare_migration_with_control(model, &self.authority, None)
    }

    pub fn prepare_migration_approved(
        &self,
        model: &MigrationModel,
        approval: &MigrationSecurityApproval,
    ) -> Result<PreparedMigration> {
        self.authority
            .require(DatabaseControlPermission::SchemaPublish)?;
        self.database
            .prepare_migration_with_control(model, &self.authority, Some(approval))
    }

    pub fn preview_migration(&self, model: &MigrationModel) -> Result<MigrationPreview> {
        Ok(self.prepare_migration(model)?.preview())
    }

    pub fn execute_migration(
        &self,
        prepared: &PreparedMigration,
        transaction: TransactionId,
        history: MigrationHistoryPolicy,
    ) -> Result<CommitOutcome> {
        self.authority
            .require(DatabaseControlPermission::SchemaPublish)?;
        self.database.execute_migration_with_control(
            prepared,
            transaction,
            history,
            &self.authority,
        )
    }

    pub fn migrate(
        &self,
        model: &MigrationModel,
        transaction: TransactionId,
        history: MigrationHistoryPolicy,
    ) -> Result<CommitOutcome> {
        let prepared = self.prepare_migration(model)?;
        self.execute_migration(&prepared, transaction, history)
    }

    pub fn migrate_approved(
        &self,
        model: &MigrationModel,
        approval: &MigrationSecurityApproval,
        transaction: TransactionId,
        history: MigrationHistoryPolicy,
    ) -> Result<CommitOutcome> {
        let prepared = self.prepare_migration_approved(model, approval)?;
        self.execute_migration(&prepared, transaction, history)
    }

    pub fn observe_migration(&self, prepared: &PreparedMigration) -> Result<MigrationObservation> {
        self.authority
            .require(DatabaseControlPermission::SchemaPublish)?;
        self.database.observe_migration(prepared)
    }
}

fn control_denied(principal: PrincipalId, permission: DatabaseControlPermission) -> Error {
    Error::new(
        ErrorKind::PermissionDenied,
        format!(
            "database-control principal {} lacks {permission:?}",
            principal.raw()
        ),
    )
}

fn control_revoked(principal: PrincipalId) -> Error {
    Error::new(
        ErrorKind::PermissionDenied,
        format!("database-control principal {} is revoked", principal.raw()),
    )
}

fn control_state_poisoned() -> Error {
    Error::new(ErrorKind::Internal, "database-control state lock poisoned")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AccessCapability, FieldId, MigrationFieldRule, MigrationHistoryPolicy, MigrationValueExpr,
        Role, Schema, SchemaAccess, Type, TypeId,
    };
    use std::fs;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("cfmd-pass582-{name}-{}", std::process::id()))
    }

    #[test]
    fn live_fork_requires_dedicated_database_control_permission_before_target_creation() {
        let path = temp_path("fork-control-source");
        let target = temp_path("fork-control-target");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&target);
        let database = Database::create(
            &path,
            Schema::builder().revisions(585_010, 1).build().unwrap(),
        )
        .unwrap();
        let credential = DatabaseControlCredential::authenticated(
            PrincipalId::new(585_101),
            DatabaseControlPermissionSet::from([DatabaseControlPermission::Export]),
        );
        let admin = database.admin_session(credential.session());
        let error = admin
            .fork_to(&target, &crate::Encryption::None)
            .expect_err("Export authority must not imply live-fork authority");
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert!(!target.exists(), "denied fork must not create a target");

        let freshness = crate::ExternalFreshness::tcp(
            [0x58; 32],
            1,
            vec![[0x59; 32]],
            1,
            std::net::SocketAddr::from(([127, 0, 0, 1], 9)),
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(1),
        );
        let error = admin
            .fork_to_with_external_freshness(&target, &crate::Encryption::None, &freshness)
            .expect_err("Export authority must not reach freshness resolution for live fork");
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert!(
            !target.exists(),
            "denied anchored fork must not create a target"
        );

        drop(admin);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn policy_change_requires_independent_control_authority() {
        let path = temp_path("policy-control");
        let _ = fs::remove_file(&path);
        let source_capability = AccessCapability::new("client.observe").watch();
        let source_role = Role::new("client.observer").capability(&source_capability);
        let source = Schema::builder()
            .revisions(582_010, 1)
            .access(
                SchemaAccess::new()
                    .capability(source_capability)
                    .role(source_role),
            )
            .build()
            .unwrap();
        let database = Database::create(&path, source).unwrap();

        let widened = AccessCapability::new("client.observe")
            .watch()
            .history_read();
        let target_role = Role::new("client.observer").capability(&widened);
        let target = Schema::builder()
            .revisions(582_011, 1)
            .access(SchemaAccess::new().capability(widened).role(target_role))
            .build()
            .unwrap();
        let migration = MigrationModel::new(582_010_011, target);

        let publisher = DatabaseControlCredential::authenticated(
            PrincipalId::new(582_101),
            DatabaseControlPermissionSet::from([
                DatabaseControlPermission::SchemaPublish,
                DatabaseControlPermission::MigrationDataInspect,
            ]),
        );
        let publisher_admin = database.admin_session(publisher.session());
        let validation = publisher_admin
            .validate_migration(&migration)
            .expect("publisher may compute the sealed impact without approving it");
        let error = publisher_admin
            .prepare_migration(&migration)
            .expect_err("schema publication must not self-approve access policy changes");
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);

        let policy_admin = DatabaseControlCredential::authenticated(
            PrincipalId::new(582_102),
            DatabaseControlPermissionSet::from([DatabaseControlPermission::AccessPolicyAdmin]),
        );
        let policy_admin = database.admin_session(policy_admin.session());
        let error = policy_admin
            .approve_migration(&validation)
            .expect_err("access-policy administration alone must not imply declassification");
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);

        let declassifier = DatabaseControlCredential::authenticated(
            PrincipalId::new(582_103),
            DatabaseControlPermissionSet::from([
                DatabaseControlPermission::AccessPolicyAdmin,
                DatabaseControlPermission::Declassify,
            ]),
        );
        let declassifier = database.admin_session(declassifier.session());
        let approval = declassifier
            .approve_migration(&validation)
            .expect("independent security credential may approve the exact impact");
        publisher_admin
            .prepare_migration_approved(&migration, &approval)
            .expect("publisher may consume independently authorized policy widening");

        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn static_validation_does_not_grant_data_sensitive_prepare_oracle() {
        let path = temp_path("inspect-gate");
        let _ = fs::remove_file(&path);
        let owner = TypeId::new(583_020_001);
        let source_field = FieldId::new(583_020_002);
        let target_field = FieldId::new(583_021_002);
        let database = Database::create(
            &path,
            Schema::builder()
                .revisions(583_020, 1)
                .__entity_type(owner)
                .__entity_field(source_field, owner, Type::i64())
                .build()
                .unwrap(),
        )
        .unwrap();
        let target = Schema::builder()
            .revisions(583_021, 1)
            .__entity_type(owner)
            .__entity_field(target_field, owner, Type::i64())
            .build()
            .unwrap();
        let migration = MigrationModel::new(583_020_021, target).field(MigrationFieldRule {
            target: target_field,
            value: MigrationValueExpr::Field(source_field),
        });
        let publisher = DatabaseControlCredential::authenticated(
            PrincipalId::new(583_201),
            DatabaseControlPermissionSet::from([DatabaseControlPermission::SchemaPublish]),
        );
        let admin = database.admin_session(publisher.session());
        let validation = admin
            .validate_migration(&migration)
            .expect("static validation must not inspect protected relation values");
        assert_eq!(
            validation.security_impact().source_data_dependencies(),
            &[crate::MigrationDataDependency::Field(source_field)]
        );
        let error = admin
            .prepare_migration(&migration)
            .expect_err("data-sensitive preparation requires independent inspect authority");
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);

        drop(admin);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn policy_recomposition_without_new_observation_needs_no_declassification_claim() {
        let path = temp_path("noninterfering-policy-change");
        let _ = fs::remove_file(&path);
        let old_capability = AccessCapability::new("client.observe.old").watch();
        let role = Role::new("client.observer").capability(&old_capability);
        let source = Schema::builder()
            .revisions(583_030, 1)
            .access(SchemaAccess::new().capability(old_capability).role(role))
            .build()
            .unwrap();
        let database = Database::create(&path, source).unwrap();

        let new_capability = AccessCapability::new("client.observe.new").watch();
        let target_role = Role::new("client.observer").capability(&new_capability);
        let target = Schema::builder()
            .revisions(583_031, 1)
            .access(
                SchemaAccess::new()
                    .capability(new_capability)
                    .role(target_role),
            )
            .build()
            .unwrap();
        let same_target = target.clone();
        let migration = MigrationModel::new(583_030_031, target);
        let control = DatabaseControlCredential::authenticated(
            PrincipalId::new(583_301),
            DatabaseControlPermissionSet::from([
                DatabaseControlPermission::SchemaPublish,
                DatabaseControlPermission::AccessPolicyAdmin,
            ]),
        );
        let admin = database.admin_session(control.session());
        let validation = admin.validate_migration(&migration).unwrap();
        assert!(validation.security_impact().is_noninterfering());
        assert!(
            !validation
                .security_impact()
                .access_policy_changes()
                .is_empty()
        );
        assert_eq!(
            validation.security_impact().digest(),
            admin
                .validate_migration(&migration)
                .unwrap()
                .security_impact()
                .digest(),
            "impact identity must be stable for the exact source+program"
        );
        let different_identity = MigrationModel::new(583_030_032, same_target);
        assert_ne!(
            validation.security_impact().digest(),
            admin
                .validate_migration(&different_identity)
                .unwrap()
                .security_impact()
                .digest(),
            "changing migration identity must invalidate a sealed approval identity"
        );
        let approval = admin
            .approve_migration(&validation)
            .expect("noninterfering policy administration needs no declassification claim");
        let error = admin
            .prepare_migration_approved(&different_identity, &approval)
            .expect_err("approval must be bound to the exact migration identity and digest");
        assert_eq!(error.kind(), ErrorKind::InvalidPlan);
        admin
            .prepare_migration_approved(&migration, &approval)
            .expect("policy administration without observation widening is not declassification");

        drop(admin);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn migration_security_approval_is_invalidated_by_control_rotation_and_revocation() {
        let path = temp_path("approval-generation");
        let _ = fs::remove_file(&path);
        let old_capability = AccessCapability::new("client.old").watch();
        let old_role = Role::new("client.role").capability(&old_capability);
        let source = Schema::builder()
            .revisions(584_040, 1)
            .access(
                SchemaAccess::new()
                    .capability(old_capability)
                    .role(old_role),
            )
            .build()
            .unwrap();
        let database = Database::create(&path, source).unwrap();

        let new_capability = AccessCapability::new("client.new").watch();
        let new_role = Role::new("client.role").capability(&new_capability);
        let target = Schema::builder()
            .revisions(584_041, 1)
            .access(
                SchemaAccess::new()
                    .capability(new_capability)
                    .role(new_role),
            )
            .build()
            .unwrap();
        let migration = MigrationModel::new(584_040_041, target);

        let publisher = DatabaseControlCredential::authenticated(
            PrincipalId::new(584_401),
            DatabaseControlPermissionSet::from([DatabaseControlPermission::SchemaPublish]),
        );
        let approver = DatabaseControlCredential::authenticated(
            PrincipalId::new(584_402),
            DatabaseControlPermissionSet::from([DatabaseControlPermission::AccessPolicyAdmin]),
        );
        let admin = database.admin_session(publisher.session());
        let approver_admin = database.admin_session(approver.session());
        let validation = admin.validate_migration(&migration).unwrap();
        let approval = approver_admin.approve_migration(&validation).unwrap();
        let prepared = admin
            .prepare_migration_approved(&migration, &approval)
            .unwrap();

        assert_eq!(approval.authority_generation(), 0);
        assert_eq!(
            approver
                .rotate_permissions(DatabaseControlPermissionSet::from([
                    DatabaseControlPermission::AccessPolicyAdmin,
                ]))
                .unwrap(),
            1
        );
        let error = admin
            .execute_migration(
                &prepared,
                TransactionId::new(584_040_041),
                MigrationHistoryPolicy::Forget,
            )
            .expect_err("credential rotation must invalidate an already sealed approval");
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(database.snapshot().unwrap().schema_revision(), 584_040);

        let approval = approver_admin.approve_migration(&validation).unwrap();
        let prepared = admin
            .prepare_migration_approved(&migration, &approval)
            .unwrap();
        assert!(approver.revoke().unwrap());
        let error = admin
            .execute_migration(
                &prepared,
                TransactionId::new(584_040_042),
                MigrationHistoryPolicy::Forget,
            )
            .expect_err("credential revocation must invalidate an already sealed approval");
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(database.snapshot().unwrap().schema_revision(), 584_040);

        drop(admin);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn migration_security_approval_is_bound_to_one_live_database_identity() {
        let left_path = temp_path("approval-db-left");
        let right_path = temp_path("approval-db-right");
        let _ = fs::remove_file(&left_path);
        let _ = fs::remove_file(&right_path);

        let old_capability = AccessCapability::new("client.old").watch();
        let old_role = Role::new("client.role").capability(&old_capability);
        let source = Schema::builder()
            .revisions(584_050, 1)
            .access(
                SchemaAccess::new()
                    .capability(old_capability)
                    .role(old_role),
            )
            .build()
            .unwrap();
        let left = Database::create(&left_path, source.clone()).unwrap();
        let right = Database::create(&right_path, source).unwrap();

        let new_capability = AccessCapability::new("client.new").watch();
        let new_role = Role::new("client.role").capability(&new_capability);
        let target = Schema::builder()
            .revisions(584_051, 1)
            .access(
                SchemaAccess::new()
                    .capability(new_capability)
                    .role(new_role),
            )
            .build()
            .unwrap();
        let migration = MigrationModel::new(584_050_051, target);

        let left_publisher = DatabaseControlCredential::authenticated(
            PrincipalId::new(584_501),
            DatabaseControlPermissionSet::from([DatabaseControlPermission::SchemaPublish]),
        );
        let left_approver = DatabaseControlCredential::authenticated(
            PrincipalId::new(584_502),
            DatabaseControlPermissionSet::from([DatabaseControlPermission::AccessPolicyAdmin]),
        );
        let left_admin = left.admin_session(left_publisher.session());
        let left_validation = left_admin.validate_migration(&migration).unwrap();
        let approval = left
            .admin_session(left_approver.session())
            .approve_migration(&left_validation)
            .unwrap();

        let right_publisher = DatabaseControlCredential::authenticated(
            PrincipalId::new(584_503),
            DatabaseControlPermissionSet::from([DatabaseControlPermission::SchemaPublish]),
        );
        let right_admin = right.admin_session(right_publisher.session());
        assert_eq!(
            left_validation.security_impact().digest(),
            right_admin
                .validate_migration(&migration)
                .unwrap()
                .security_impact()
                .digest(),
            "fixture requires the same source/program impact on two live databases"
        );
        let error = right_admin
            .prepare_migration_approved(&migration, &approval)
            .expect_err("approval issued for one live database must not replay on another");
        assert_eq!(error.kind(), ErrorKind::InvalidPlan);

        drop(right_admin);
        drop(left_admin);
        drop(right);
        drop(left);
        let _ = fs::remove_file(right_path);
        let _ = fs::remove_file(left_path);
    }

    #[test]
    fn schema_role_narrowing_refreshes_live_role_bound_session() {
        let path = temp_path("role-epoch");
        let _ = fs::remove_file(&path);
        let capability = AccessCapability::new("client.watch").watch();
        let role = Role::new("client.role").capability(&capability);
        let role_id = role.id();
        let source = Schema::builder()
            .revisions(582_030, 1)
            .access(SchemaAccess::new().capability(capability).role(role))
            .build()
            .unwrap();
        let database = Database::create(&path, source).unwrap();
        let client = database
            .session_for_roles(PrincipalId::new(582_301), [role_id])
            .unwrap();
        assert!(
            client
                .session()
                .snapshot()
                .unwrap()
                .permissions()
                .contains(crate::Permission::Watch)
        );

        let narrowed = AccessCapability::new("client.watch");
        let narrowed_role = Role::new("client.role").capability(&narrowed);
        let target = Schema::builder()
            .revisions(582_031, 1)
            .access(SchemaAccess::new().capability(narrowed).role(narrowed_role))
            .build()
            .unwrap();
        database
            .migrate(
                &MigrationModel::new(582_030_031, target),
                TransactionId::new(582_030_031),
                MigrationHistoryPolicy::Forget,
            )
            .unwrap();

        let refreshed = client.session().snapshot().unwrap();
        assert_eq!(refreshed.generation(), 1);
        assert!(!refreshed.permissions().contains(crate::Permission::Watch));

        drop(client);
        drop(database);
        let _ = fs::remove_file(path);
    }
}
