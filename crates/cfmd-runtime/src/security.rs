use std::{
    collections::BTreeSet,
    fmt,
    sync::{Arc, RwLock},
};

use crate::{Database, Error, ErrorKind, Plan, ReadContext, Result, RevisionId, TransactionId};

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
    Read,
    HistoricalRead,
    HistoryRead,
    Watch,
    Write,
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
    pub fn contains(&self, permission: Permission) -> bool {
        self.0.contains(&permission)
    }

    pub fn iter(&self) -> impl Iterator<Item = Permission> + '_ {
        self.0.iter().copied()
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
            Err(Error::new(
                ErrorKind::PermissionDenied,
                format!(
                    "principal {} lacks required permission {permission:?}",
                    self.principal.raw()
                ),
            ))
        }
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
        self.authority.require(Permission::Read)?;
        self.database
            .snapshot_with_authority(self.authority.clone())
    }

    pub fn at(&self, revision: RevisionId) -> Result<ReadContext> {
        self.authority.require(Permission::Read)?;
        self.authority.require(Permission::HistoricalRead)?;
        self.database
            .at_with_authority(revision, self.authority.clone())
    }

    pub fn current_revision(&self) -> Result<RevisionId> {
        self.snapshot().map(|view| view.revision())
    }

    pub fn history(&self) -> Result<crate::History> {
        self.authority.require(Permission::HistoryRead)?;
        self.snapshot()?.history()
    }

    pub fn plan(&self) -> Result<Plan> {
        self.authority.require(Permission::Write)?;
        self.database.plan_with_authority(self.authority.clone())
    }

    pub fn commit(&self, plan: &Plan, transaction: TransactionId) -> Result<crate::CommitOutcome> {
        self.authority.require(Permission::Write)?;
        self.database.commit(plan, transaction)
    }
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
