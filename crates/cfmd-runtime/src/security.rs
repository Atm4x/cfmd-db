use std::{
    collections::BTreeSet,
    fmt,
    sync::{Arc, RwLock},
};

use crate::{
    CandidatePreview, Database, Error, ErrorKind, IntentJournal, Plan, ReadContext,
    RelationColumnId, RelationId, Result, RevisionId, TransactionId,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PublicationAuthorityFootprint {
    pub(crate) relation_writes: BTreeSet<RelationId>,
    pub(crate) field_writes: BTreeSet<(RelationId, RelationColumnId)>,
    pub(crate) actions: BTreeSet<(RelationId, crate::plan::MutationAction)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrincipalId(u128);

impl PrincipalId {
    #[must_use]
    pub const fn new(raw: u128) -> Self {
        Self(raw)
    }
    #[must_use]
    pub const fn raw(self) -> u128 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Permission {
    ModelRead,
    SchemaMigrate,
    Read,
    ReadRelation(RelationId),
    ReadField {
        relation: RelationId,
        field: RelationColumnId,
    },
    HistoricalRead,
    HistoryRead,
    Watch,
    Write,
    WriteRelation(RelationId),
    WriteField {
        relation: RelationId,
        field: RelationColumnId,
    },
    CreateObject(RelationId),
    DeleteObject(RelationId),
    AttachRelationship(RelationId),
    DetachRelationship(RelationId),
    MoveRelationship(RelationId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    name: String,
    permissions: PermissionSet,
}

impl Role {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            permissions: PermissionSet::new(),
        }
    }

    #[must_use]
    pub fn grant(mut self, permission: Permission) -> Self {
        self.permissions.0.insert(permission);
        self
    }

    #[must_use]
    pub fn grants(mut self, permissions: impl IntoIterator<Item = Permission>) -> Self {
        self.permissions.0.extend(permissions);
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn permissions(&self) -> &PermissionSet {
        &self.permissions
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PermissionSet(BTreeSet<Permission>);

impl PermissionSet {
    #[must_use]
    pub const fn new() -> Self {
        Self(BTreeSet::new())
    }

    #[must_use]
    pub fn with(mut self, permission: Permission) -> Self {
        self.0.insert(permission);
        self
    }

    #[must_use]
    pub fn with_role(mut self, role: &Role) -> Self {
        self.0.extend(role.permissions.iter());
        self
    }

    #[must_use]
    pub fn from_roles<'a>(roles: impl IntoIterator<Item = &'a Role>) -> Self {
        let mut permissions = Self::new();
        for role in roles {
            permissions.0.extend(role.permissions.iter());
        }
        permissions
    }

    #[must_use]
    pub fn contains(&self, permission: Permission) -> bool {
        self.0.contains(&permission)
    }

    pub fn iter(&self) -> impl Iterator<Item = Permission> + '_ {
        self.0.iter().copied()
    }

    fn has_read_entry(&self) -> bool {
        self.0.iter().any(|permission| {
            matches!(
                permission,
                Permission::Read | Permission::ReadRelation(_) | Permission::ReadField { .. }
            )
        })
    }

    fn has_write_entry(&self) -> bool {
        self.0.iter().any(|permission| {
            matches!(
                permission,
                Permission::Write
                    | Permission::WriteRelation(_)
                    | Permission::WriteField { .. }
                    | Permission::CreateObject(_)
                    | Permission::DeleteObject(_)
                    | Permission::AttachRelationship(_)
                    | Permission::DetachRelationship(_)
                    | Permission::MoveRelationship(_)
            )
        })
    }

    fn can_read_relation(&self, relation: RelationId) -> bool {
        self.contains(Permission::Read)
            || self.contains(Permission::ReadRelation(relation))
            || self.0.iter().any(|permission| {
                matches!(permission, Permission::ReadField { relation: candidate, .. } if *candidate == relation)
            })
    }

    fn can_read_field(&self, relation: RelationId, field: RelationColumnId) -> bool {
        self.contains(Permission::Read)
            || self.contains(Permission::ReadRelation(relation))
            || self.contains(Permission::ReadField { relation, field })
    }

    fn can_write_relation(&self, relation: RelationId) -> bool {
        self.contains(Permission::Write) || self.contains(Permission::WriteRelation(relation))
    }

    fn can_write_field(&self, relation: RelationId, field: RelationColumnId) -> bool {
        self.can_write_relation(relation)
            || self.contains(Permission::WriteField { relation, field })
    }

    fn can_mutate_action(&self, relation: RelationId, action: crate::plan::MutationAction) -> bool {
        if self.can_write_relation(relation) {
            return true;
        }
        let permission = match action {
            crate::plan::MutationAction::ObjectCreate => Permission::CreateObject(relation),
            crate::plan::MutationAction::ObjectDelete => Permission::DeleteObject(relation),
            crate::plan::MutationAction::RelationshipAttach => {
                Permission::AttachRelationship(relation)
            }
            crate::plan::MutationAction::RelationshipDetach => {
                Permission::DetachRelationship(relation)
            }
            crate::plan::MutationAction::RelationshipMove => Permission::MoveRelationship(relation),
        };
        self.contains(permission)
    }

    fn require_publication_footprint(
        &self,
        principal: PrincipalId,
        footprint: &PublicationAuthorityFootprint,
    ) -> Result<()> {
        if !self.has_write_entry() {
            return Err(permission_denied(principal, "write authority"));
        }
        for relation in &footprint.relation_writes {
            if !self.can_write_relation(*relation) {
                return Err(permission_denied(
                    principal,
                    format!("write relation {}", relation.raw()),
                ));
            }
        }
        for (relation, field) in &footprint.field_writes {
            if !self.can_write_field(*relation, *field) {
                return Err(permission_denied(
                    principal,
                    format!("write field {}:{}", relation.raw(), field.raw()),
                ));
            }
        }
        for (relation, action) in &footprint.actions {
            if !self.can_mutate_action(*relation, *action) {
                return Err(permission_denied(
                    principal,
                    format!("{action:?} on relation {}", relation.raw()),
                ));
            }
        }
        Ok(())
    }
}

impl<const N: usize> From<[Permission; N]> for PermissionSet {
    fn from(value: [Permission; N]) -> Self {
        Self(value.into_iter().collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSnapshot {
    generation: u64,
    permissions: PermissionSet,
    revoked: bool,
}

impl SessionSnapshot {
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn permissions(&self) -> &PermissionSet {
        &self.permissions
    }

    #[must_use]
    pub const fn is_revoked(&self) -> bool {
        self.revoked
    }
}

#[derive(Debug)]
struct SessionState {
    generation: u64,
    permissions: PermissionSet,
    revoked: bool,
}

#[derive(Clone)]
pub struct Session {
    principal: PrincipalId,
    state: Arc<RwLock<SessionState>>,
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Session")
            .field("principal", &self.principal)
            .field("snapshot", &self.snapshot().ok())
            .finish_non_exhaustive()
    }
}

impl PartialEq for Session {
    fn eq(&self, other: &Self) -> bool {
        self.principal == other.principal && Arc::ptr_eq(&self.state, &other.state)
    }
}

impl Eq for Session {}

impl Session {
    #[must_use]
    pub fn new(principal: PrincipalId, permissions: PermissionSet) -> Self {
        Self {
            principal,
            state: Arc::new(RwLock::new(SessionState {
                generation: 0,
                permissions,
                revoked: false,
            })),
        }
    }

    #[must_use]
    pub fn from_roles<'a>(
        principal: PrincipalId,
        roles: impl IntoIterator<Item = &'a Role>,
    ) -> Self {
        Self::new(principal, PermissionSet::from_roles(roles))
    }

    #[must_use]
    pub const fn principal(&self) -> PrincipalId {
        self.principal
    }

    pub fn snapshot(&self) -> Result<SessionSnapshot> {
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        Ok(SessionSnapshot {
            generation: state.generation,
            permissions: state.permissions.clone(),
            revoked: state.revoked,
        })
    }

    pub fn refresh_permissions(&self, permissions: PermissionSet) -> Result<u64> {
        let mut state = self.state.write().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        state.generation = state.generation.checked_add(1).ok_or_else(|| {
            Error::new(
                ErrorKind::Internal,
                "session authority generation overflowed",
            )
        })?;
        state.permissions = permissions;
        Ok(state.generation)
    }

    pub fn revoke(&self) -> Result<bool> {
        let mut state = self.state.write().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Ok(false);
        }
        state.generation = state.generation.checked_add(1).ok_or_else(|| {
            Error::new(
                ErrorKind::Internal,
                "session authority generation overflowed",
            )
        })?;
        state.revoked = true;
        Ok(true)
    }

    pub fn require(&self, permission: Permission) -> Result<()> {
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        if state.permissions.contains(permission) {
            Ok(())
        } else {
            Err(permission_denied(self.principal, format!("{permission:?}")))
        }
    }

    fn with_permission<T>(
        &self,
        permission: Permission,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        if !state.permissions.contains(permission) {
            return Err(permission_denied(self.principal, format!("{permission:?}")));
        }
        operation()
    }

    fn require_read_entry(&self) -> Result<()> {
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        if state.permissions.has_read_entry() {
            Ok(())
        } else {
            Err(permission_denied(self.principal, "read authority"))
        }
    }

    fn require_write_entry(&self) -> Result<()> {
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        if state.permissions.has_write_entry() {
            Ok(())
        } else {
            Err(permission_denied(self.principal, "write authority"))
        }
    }

    fn require_read_footprint(&self, footprint: &kernel_query::RelReadFootprint) -> Result<()> {
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        for relation in footprint.relations() {
            let relation = RelationId::new(relation.raw());
            if !state.permissions.can_read_relation(relation) {
                return Err(permission_denied(
                    self.principal,
                    format!("read relation {}", relation.raw()),
                ));
            }
        }
        for (relation, field) in footprint.columns() {
            let relation = RelationId::new(relation.raw());
            let field = RelationColumnId::new(field.raw());
            if !state.permissions.can_read_field(relation, field) {
                return Err(permission_denied(
                    self.principal,
                    format!("read field {}:{}", relation.raw(), field.raw()),
                ));
            }
        }
        Ok(())
    }

    fn require_read_relation(&self, relation: RelationId) -> Result<()> {
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        if state.permissions.can_read_relation(relation) {
            Ok(())
        } else {
            Err(permission_denied(
                self.principal,
                format!("read relation {}", relation.raw()),
            ))
        }
    }

    fn require_read_field(&self, relation: RelationId, field: RelationColumnId) -> Result<()> {
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        if state.permissions.can_read_field(relation, field) {
            Ok(())
        } else {
            Err(permission_denied(
                self.principal,
                format!("read field {}:{}", relation.raw(), field.raw()),
            ))
        }
    }

    fn require_write_relation(&self, relation: RelationId) -> Result<()> {
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        if state.permissions.can_write_relation(relation) {
            Ok(())
        } else {
            Err(permission_denied(
                self.principal,
                format!("write relation {}", relation.raw()),
            ))
        }
    }

    fn require_publication_footprint(
        &self,
        footprint: &PublicationAuthorityFootprint,
    ) -> Result<()> {
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        state
            .permissions
            .require_publication_footprint(self.principal, footprint)
    }

    fn with_publication_authority<T>(
        &self,
        footprint: &PublicationAuthorityFootprint,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        state
            .permissions
            .require_publication_footprint(self.principal, footprint)?;
        operation()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeAuthority {
    Unrestricted,
    Session(Session),
}

impl RuntimeAuthority {
    pub(crate) fn require(&self, permission: Permission) -> Result<()> {
        match self {
            Self::Unrestricted => Ok(()),
            Self::Session(session) => session.require(permission),
        }
    }

    pub(crate) fn with_permission<T>(
        &self,
        permission: Permission,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        match self {
            Self::Unrestricted => operation(),
            Self::Session(session) => session.with_permission(permission, operation),
        }
    }

    pub(crate) fn require_read_entry(&self) -> Result<()> {
        match self {
            Self::Unrestricted => Ok(()),
            Self::Session(session) => session.require_read_entry(),
        }
    }

    pub(crate) fn require_write_entry(&self) -> Result<()> {
        match self {
            Self::Unrestricted => Ok(()),
            Self::Session(session) => session.require_write_entry(),
        }
    }

    pub(crate) fn require_read_footprint(
        &self,
        footprint: &kernel_query::RelReadFootprint,
    ) -> Result<()> {
        match self {
            Self::Unrestricted => Ok(()),
            Self::Session(session) => session.require_read_footprint(footprint),
        }
    }

    pub(crate) fn require_read_relation(&self, relation: RelationId) -> Result<()> {
        match self {
            Self::Unrestricted => Ok(()),
            Self::Session(session) => session.require_read_relation(relation),
        }
    }

    pub(crate) fn require_read_field(
        &self,
        relation: RelationId,
        field: RelationColumnId,
    ) -> Result<()> {
        match self {
            Self::Unrestricted => Ok(()),
            Self::Session(session) => session.require_read_field(relation, field),
        }
    }

    pub(crate) fn require_write_relation(&self, relation: RelationId) -> Result<()> {
        match self {
            Self::Unrestricted => Ok(()),
            Self::Session(session) => session.require_write_relation(relation),
        }
    }

    pub(crate) fn require_publication_footprint(
        &self,
        footprint: &PublicationAuthorityFootprint,
    ) -> Result<()> {
        match self {
            Self::Unrestricted => Ok(()),
            Self::Session(session) => session.require_publication_footprint(footprint),
        }
    }

    pub(crate) fn with_publication_authority<T>(
        &self,
        footprint: &PublicationAuthorityFootprint,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        match self {
            Self::Unrestricted => operation(),
            Self::Session(session) => session.with_publication_authority(footprint, operation),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionDatabase {
    database: Database,
    authority: RuntimeAuthority,
}

impl SessionDatabase {
    pub(crate) fn new(database: Database, session: Session) -> Self {
        Self {
            database,
            authority: RuntimeAuthority::Session(session),
        }
    }

    #[must_use]
    pub fn session(&self) -> &Session {
        match &self.authority {
            RuntimeAuthority::Session(session) => session,
            RuntimeAuthority::Unrestricted => unreachable!("SessionDatabase is always restricted"),
        }
    }

    pub fn snapshot(&self) -> Result<ReadContext> {
        self.authority.require_read_entry()?;
        self.database
            .snapshot_with_authority(self.authority.clone())
    }

    pub fn at(&self, revision: RevisionId) -> Result<ReadContext> {
        self.authority.require_read_entry()?;
        self.authority.require(Permission::HistoricalRead)?;
        self.database
            .at_with_authority(revision, self.authority.clone())
    }

    pub fn current_revision(&self) -> Result<RevisionId> {
        self.snapshot().map(|view| view.revision())
    }

    /// Atomically admits one bounded typed Context carrying this session's live authority.
    ///
    /// Admission itself does not require read permission: write-only principals may form a
    /// Context and stage authorized writes. Individual reads, history, watches and publication
    /// remain checked against the exact semantic coordinates they touch.
    pub fn begin_context(&self) -> Result<crate::ContextAdmission> {
        self.database
            .begin_context_with_authority(self.authority.clone())
    }

    /// Binds a typed consumer contract to one session-authorized bounded working world.
    pub fn context<S: crate::CfmdSchema>(&self) -> Result<crate::Context<S>> {
        self.begin_context()?.context::<S>()
    }

    fn operation_context(&self) -> Result<ReadContext> {
        self.database
            .snapshot_with_authority(self.authority.clone())
    }

    pub fn history(&self) -> Result<crate::History> {
        self.authority.require(Permission::HistoryRead)?;
        self.snapshot()?.history()
    }

    pub fn migrate(
        &self,
        model: &crate::MigrationModel,
        transaction: TransactionId,
        history: crate::MigrationHistoryPolicy,
    ) -> Result<crate::CommitOutcome> {
        self.database
            .migrate_with_authority(model, transaction, history, &self.authority)
    }

    /// Adds a historical inverse under this session's write authority.
    pub fn undo(&self, transaction: &mut IntentJournal, entry: &crate::HistoryEntry) -> Result<()> {
        self.authority.require_write_entry()?;
        let plan = entry.undo_plan()?;
        if plan.authority != self.authority {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "history entry belongs to a different session authority",
            ));
        }
        transaction.add_plan(plan)
    }

    /// Adds the current history head's inverse under this session's write authority.
    pub fn undo_latest(&self, transaction: &mut IntentJournal) -> Result<()> {
        self.authority.require(Permission::HistoryRead)?;
        self.authority.require_write_entry()?;
        let history = self.history()?;
        let entry = history.latest().ok_or_else(|| {
            Error::new(ErrorKind::NotFound, "history has no transition at its head")
        })?;
        self.undo(transaction, entry)
    }

    /// Returns the current object collection under this session authority.
    pub fn objects<E: crate::Object>(&self) -> Result<crate::ObjectSet<E>> {
        self.operation_context()?.objects::<E>()
    }

    pub fn plan(&self) -> Result<Plan> {
        self.authority.require_write_entry()?;
        self.database.plan_with_authority(self.authority.clone())
    }

    pub fn intent_readiness(&self, transaction: &IntentJournal) -> Result<crate::IntentReadiness> {
        self.authority.require_write_entry()?;
        if transaction.authority() != Some(&self.authority) {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "transaction belongs to a different session authority",
            ));
        }
        self.database.intent_readiness(transaction)
    }

    pub fn preview(&self, transaction: &IntentJournal) -> Result<CandidatePreview> {
        self.authority.require_write_entry()?;
        if transaction.authority() != Some(&self.authority) {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "transaction belongs to a different session authority",
            ));
        }
        self.database.preview(transaction)
    }

    pub fn commit(&self, transaction: &IntentJournal) -> Result<crate::CommitOutcome> {
        self.authority.require_write_entry()?;
        if transaction.authority() != Some(&self.authority) {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "transaction belongs to a different session authority",
            ));
        }
        self.database.commit(transaction)
    }

    #[doc(hidden)]
    pub fn commit_exact_relation_intent(
        &self,
        formation_revision: crate::RevisionId,
        formation_schema_revision: u64,
        formation_environment_revision: u64,
        transaction: TransactionId,
        mutations: &[crate::ExactRelationMutation],
    ) -> Result<crate::CommitOutcome> {
        self.database.commit_exact_relation_intent(
            formation_revision,
            formation_schema_revision,
            formation_environment_revision,
            transaction,
            mutations,
            &self.authority,
        )
    }

    pub fn commit_plan(
        &self,
        plan: &Plan,
        transaction: TransactionId,
    ) -> Result<crate::CommitOutcome> {
        self.authority.require_write_entry()?;
        if plan.authority != self.authority {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "plan belongs to a different session authority",
            ));
        }
        self.database.commit_plan(plan, transaction)
    }
}

fn permission_denied(principal: PrincipalId, required: impl fmt::Display) -> Error {
    Error::new(
        ErrorKind::PermissionDenied,
        format!("principal {} lacks required {required}", principal.raw()),
    )
}

fn session_state_poisoned() -> Error {
    Error::new(
        ErrorKind::Internal,
        "session authority state is unavailable",
    )
}

fn session_revoked(principal: PrincipalId) -> Error {
    Error::new(
        ErrorKind::SessionRevoked,
        format!("session for principal {} has been revoked", principal.raw()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{TryLockError, mpsc};

    #[test]
    fn publication_authority_holds_generation_stable_until_publish_finishes() {
        let relation = RelationId::new(91_001);
        let session = Session::new(
            PrincipalId::new(91_002),
            PermissionSet::from([Permission::WriteRelation(relation)]),
        );
        let authority = RuntimeAuthority::Session(session.clone());
        let footprint = PublicationAuthorityFootprint {
            relation_writes: BTreeSet::from([relation]),
            ..PublicationAuthorityFootprint::default()
        };
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let worker = std::thread::spawn(move || {
            authority
                .with_publication_authority(&footprint, || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap();
        });

        entered_rx.recv().unwrap();
        assert!(matches!(
            session.state.try_write(),
            Err(TryLockError::WouldBlock)
        ));
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(
            session
                .refresh_permissions(PermissionSet::from([Permission::Read]))
                .unwrap(),
            1
        );
    }
}
