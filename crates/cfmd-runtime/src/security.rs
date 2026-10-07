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
    pub(crate) carrier_presence: BTreeSet<crate::ModelSemanticId>,
    pub(crate) carrier_members: BTreeSet<(crate::ModelSemanticId, crate::ModelEntityId)>,
    pub(crate) lifecycle_entities: BTreeSet<crate::ModelEntityId>,
    pub(crate) lifecycle_roots: BTreeSet<crate::ModelEntityId>,
    pub(crate) keeps_alive_presence: BTreeSet<crate::ModelEntityId>,
    pub(crate) keeps_alive_edges: BTreeSet<(crate::ModelEntityId, crate::ModelEntityId)>,
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
    WriteCarrierPresence(crate::ModelSemanticId),
    WriteCarrierMember {
        carrier: crate::ModelSemanticId,
        member: crate::ModelEntityId,
    },
    WriteLifecycleEntity(crate::ModelEntityId),
    WriteLifecycleRoot(crate::ModelEntityId),
    WriteKeepsAlivePresence(crate::ModelEntityId),
    WriteKeepsAliveEdge {
        parent: crate::ModelEntityId,
        child: crate::ModelEntityId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PermissionCoordinate {
    ModelRead,
    ReadRelation(RelationId),
    ReadField {
        relation: RelationId,
        field: RelationColumnId,
    },
    HistoricalRead,
    HistoryRead,
    Watch,
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
    WriteCarrierPresence(crate::ModelSemanticId),
    WriteCarrierMember {
        carrier: crate::ModelSemanticId,
        member: crate::ModelEntityId,
    },
    WriteLifecycleEntity(crate::ModelEntityId),
    WriteLifecycleRoot(crate::ModelEntityId),
    WriteKeepsAlivePresence(crate::ModelEntityId),
    WriteKeepsAliveEdge {
        parent: crate::ModelEntityId,
        child: crate::ModelEntityId,
    },
}

impl PermissionCoordinate {
    #[must_use]
    pub const fn into_permission(self) -> Permission {
        match self {
            Self::ModelRead => Permission::ModelRead,
            Self::ReadRelation(relation) => Permission::ReadRelation(relation),
            Self::ReadField { relation, field } => Permission::ReadField { relation, field },
            Self::HistoricalRead => Permission::HistoricalRead,
            Self::HistoryRead => Permission::HistoryRead,
            Self::Watch => Permission::Watch,
            Self::WriteRelation(relation) => Permission::WriteRelation(relation),
            Self::WriteField { relation, field } => Permission::WriteField { relation, field },
            Self::CreateObject(relation) => Permission::CreateObject(relation),
            Self::DeleteObject(relation) => Permission::DeleteObject(relation),
            Self::AttachRelationship(relation) => Permission::AttachRelationship(relation),
            Self::DetachRelationship(relation) => Permission::DetachRelationship(relation),
            Self::MoveRelationship(relation) => Permission::MoveRelationship(relation),
            Self::WriteCarrierPresence(carrier) => Permission::WriteCarrierPresence(carrier),
            Self::WriteCarrierMember { carrier, member } => {
                Permission::WriteCarrierMember { carrier, member }
            }
            Self::WriteLifecycleEntity(entity) => Permission::WriteLifecycleEntity(entity),
            Self::WriteLifecycleRoot(entity) => Permission::WriteLifecycleRoot(entity),
            Self::WriteKeepsAlivePresence(parent) => Permission::WriteKeepsAlivePresence(parent),
            Self::WriteKeepsAliveEdge { parent, child } => {
                Permission::WriteKeepsAliveEdge { parent, child }
            }
        }
    }
}

impl crate::AccessCapabilityId {
    #[must_use]
    pub const fn from_key(key: &str) -> Self {
        Self::new(crate::object::__semantic_id(
            "cfmd.authorization.capability.v1",
            key,
            "",
        ))
    }
}

impl crate::RoleId {
    #[must_use]
    pub const fn from_key(key: &str) -> Self {
        Self::new(crate::object::__semantic_id(
            "cfmd.authorization.role.v1",
            key,
            "",
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessCapability {
    id: crate::AccessCapabilityId,
    permissions: BTreeSet<PermissionCoordinate>,
}

impl AccessCapability {
    #[must_use]
    pub fn new(key: &str) -> Self {
        Self {
            id: crate::AccessCapabilityId::from_key(key),
            permissions: BTreeSet::new(),
        }
    }

    #[must_use]
    pub const fn id(&self) -> crate::AccessCapabilityId {
        self.id
    }

    #[must_use]
    pub fn grant(mut self, permission: PermissionCoordinate) -> Self {
        self.permissions.insert(permission);
        self
    }

    /// Grants exact authority to inspect the authoritative semantic model.
    #[must_use]
    pub fn model_read(self) -> Self {
        self.grant(PermissionCoordinate::ModelRead)
    }

    /// Grants exact historical-revision materialization authority.
    #[must_use]
    pub fn historical_read(self) -> Self {
        self.grant(PermissionCoordinate::HistoricalRead)
    }

    /// Grants exact history-log read authority.
    #[must_use]
    pub fn history_read(self) -> Self {
        self.grant(PermissionCoordinate::HistoryRead)
    }

    /// Grants exact subscription/watch authority.
    #[must_use]
    pub fn watch(self) -> Self {
        self.grant(PermissionCoordinate::Watch)
    }

    pub fn permissions(&self) -> impl Iterator<Item = PermissionCoordinate> + '_ {
        self.permissions.iter().copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemaAccess {
    capabilities: Vec<AccessCapability>,
    roles: Vec<Role>,
}

impl SchemaAccess {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            capabilities: Vec::new(),
            roles: Vec::new(),
        }
    }

    #[must_use]
    pub fn capability(mut self, capability: AccessCapability) -> Self {
        self.capabilities.push(capability);
        self
    }

    #[must_use]
    pub fn role(mut self, role: Role) -> Self {
        self.roles.push(role);
        self
    }

    pub(crate) fn into_parts(self) -> (Vec<AccessCapability>, Vec<Role>) {
        (self.capabilities, self.roles)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    id: crate::RoleId,
    capabilities: BTreeSet<crate::AccessCapabilityId>,
    includes: BTreeSet<crate::RoleId>,
}

impl Role {
    #[must_use]
    pub fn new(key: &str) -> Self {
        Self {
            id: crate::RoleId::from_key(key),
            capabilities: BTreeSet::new(),
            includes: BTreeSet::new(),
        }
    }

    #[must_use]
    pub const fn id(&self) -> crate::RoleId {
        self.id
    }

    #[must_use]
    pub fn capability(mut self, capability: &AccessCapability) -> Self {
        self.capabilities.insert(capability.id());
        self
    }

    #[must_use]
    pub fn include(mut self, role: &Self) -> Self {
        self.includes.insert(role.id());
        self
    }

    pub fn capabilities(&self) -> impl Iterator<Item = crate::AccessCapabilityId> + '_ {
        self.capabilities.iter().copied()
    }

    pub fn includes(&self) -> impl Iterator<Item = crate::RoleId> + '_ {
        self.includes.iter().copied()
    }
}

pub(crate) fn permission_coordinate_to_kernel(
    permission: PermissionCoordinate,
) -> kernel_schema::PermissionCoordinate {
    match permission {
        PermissionCoordinate::ModelRead => kernel_schema::PermissionCoordinate::ModelRead,
        PermissionCoordinate::ReadRelation(relation) => {
            kernel_schema::PermissionCoordinate::ReadRelation {
                relation: relation.into(),
            }
        }
        PermissionCoordinate::ReadField { relation, field } => {
            kernel_schema::PermissionCoordinate::ReadField {
                relation: relation.into(),
                column: field.into(),
            }
        }
        PermissionCoordinate::HistoricalRead => kernel_schema::PermissionCoordinate::HistoricalRead,
        PermissionCoordinate::HistoryRead => kernel_schema::PermissionCoordinate::HistoryRead,
        PermissionCoordinate::Watch => kernel_schema::PermissionCoordinate::Watch,
        PermissionCoordinate::WriteRelation(relation) => {
            kernel_schema::PermissionCoordinate::WriteRelation {
                relation: relation.into(),
            }
        }
        PermissionCoordinate::WriteField { relation, field } => {
            kernel_schema::PermissionCoordinate::WriteField {
                relation: relation.into(),
                column: field.into(),
            }
        }
        PermissionCoordinate::CreateObject(relation) => {
            kernel_schema::PermissionCoordinate::CreateObject {
                relation: relation.into(),
            }
        }
        PermissionCoordinate::DeleteObject(relation) => {
            kernel_schema::PermissionCoordinate::DeleteObject {
                relation: relation.into(),
            }
        }
        PermissionCoordinate::AttachRelationship(relation) => {
            kernel_schema::PermissionCoordinate::AttachRelationship {
                relation: relation.into(),
            }
        }
        PermissionCoordinate::DetachRelationship(relation) => {
            kernel_schema::PermissionCoordinate::DetachRelationship {
                relation: relation.into(),
            }
        }
        PermissionCoordinate::MoveRelationship(relation) => {
            kernel_schema::PermissionCoordinate::MoveRelationship {
                relation: relation.into(),
            }
        }
        PermissionCoordinate::WriteCarrierPresence(carrier) => {
            kernel_schema::PermissionCoordinate::WriteCarrierPresence {
                carrier: kernel_types::SemanticId::new(carrier.raw()),
            }
        }
        PermissionCoordinate::WriteCarrierMember { carrier, member } => {
            kernel_schema::PermissionCoordinate::WriteCarrierMember {
                carrier: kernel_types::SemanticId::new(carrier.raw()),
                member: kernel_types::SemanticId::new(member.raw()),
            }
        }
        PermissionCoordinate::WriteLifecycleEntity(entity) => {
            kernel_schema::PermissionCoordinate::WriteLifecycleEntity {
                entity: kernel_types::SemanticId::new(entity.raw()),
            }
        }
        PermissionCoordinate::WriteLifecycleRoot(entity) => {
            kernel_schema::PermissionCoordinate::WriteLifecycleRoot {
                entity: kernel_types::SemanticId::new(entity.raw()),
            }
        }
        PermissionCoordinate::WriteKeepsAlivePresence(parent) => {
            kernel_schema::PermissionCoordinate::WriteKeepsAlivePresence {
                parent: kernel_types::SemanticId::new(parent.raw()),
            }
        }
        PermissionCoordinate::WriteKeepsAliveEdge { parent, child } => {
            kernel_schema::PermissionCoordinate::WriteKeepsAliveEdge {
                parent: kernel_types::SemanticId::new(parent.raw()),
                child: kernel_types::SemanticId::new(child.raw()),
            }
        }
    }
}

pub(crate) fn permission_from_kernel(
    permission: kernel_schema::PermissionCoordinate,
) -> Permission {
    match permission {
        kernel_schema::PermissionCoordinate::ModelRead => Permission::ModelRead,
        kernel_schema::PermissionCoordinate::ReadRelation { relation } => {
            Permission::ReadRelation(RelationId::new(relation.raw()))
        }
        kernel_schema::PermissionCoordinate::ReadField { relation, column } => {
            Permission::ReadField {
                relation: RelationId::new(relation.raw()),
                field: RelationColumnId::new(column.raw()),
            }
        }
        kernel_schema::PermissionCoordinate::HistoricalRead => Permission::HistoricalRead,
        kernel_schema::PermissionCoordinate::HistoryRead => Permission::HistoryRead,
        kernel_schema::PermissionCoordinate::Watch => Permission::Watch,
        kernel_schema::PermissionCoordinate::WriteRelation { relation } => {
            Permission::WriteRelation(RelationId::new(relation.raw()))
        }
        kernel_schema::PermissionCoordinate::WriteField { relation, column } => {
            Permission::WriteField {
                relation: RelationId::new(relation.raw()),
                field: RelationColumnId::new(column.raw()),
            }
        }
        kernel_schema::PermissionCoordinate::CreateObject { relation } => {
            Permission::CreateObject(RelationId::new(relation.raw()))
        }
        kernel_schema::PermissionCoordinate::DeleteObject { relation } => {
            Permission::DeleteObject(RelationId::new(relation.raw()))
        }
        kernel_schema::PermissionCoordinate::AttachRelationship { relation } => {
            Permission::AttachRelationship(RelationId::new(relation.raw()))
        }
        kernel_schema::PermissionCoordinate::DetachRelationship { relation } => {
            Permission::DetachRelationship(RelationId::new(relation.raw()))
        }
        kernel_schema::PermissionCoordinate::MoveRelationship { relation } => {
            Permission::MoveRelationship(RelationId::new(relation.raw()))
        }
        kernel_schema::PermissionCoordinate::WriteCarrierPresence { carrier } => {
            Permission::WriteCarrierPresence(crate::ModelSemanticId::new(carrier.raw()))
        }
        kernel_schema::PermissionCoordinate::WriteCarrierMember { carrier, member } => {
            Permission::WriteCarrierMember {
                carrier: crate::ModelSemanticId::new(carrier.raw()),
                member: crate::ModelEntityId::new(member.raw()),
            }
        }
        kernel_schema::PermissionCoordinate::WriteLifecycleEntity { entity } => {
            Permission::WriteLifecycleEntity(crate::ModelEntityId::new(entity.raw()))
        }
        kernel_schema::PermissionCoordinate::WriteLifecycleRoot { entity } => {
            Permission::WriteLifecycleRoot(crate::ModelEntityId::new(entity.raw()))
        }
        kernel_schema::PermissionCoordinate::WriteKeepsAlivePresence { parent } => {
            Permission::WriteKeepsAlivePresence(crate::ModelEntityId::new(parent.raw()))
        }
        kernel_schema::PermissionCoordinate::WriteKeepsAliveEdge { parent, child } => {
            Permission::WriteKeepsAliveEdge {
                parent: crate::ModelEntityId::new(parent.raw()),
                child: crate::ModelEntityId::new(child.raw()),
            }
        }
    }
}

pub(crate) fn permission_coordinate_from_kernel(
    permission: kernel_schema::PermissionCoordinate,
) -> PermissionCoordinate {
    match permission_from_kernel(permission) {
        Permission::ModelRead => PermissionCoordinate::ModelRead,
        Permission::ReadRelation(relation) => PermissionCoordinate::ReadRelation(relation),
        Permission::ReadField { relation, field } => {
            PermissionCoordinate::ReadField { relation, field }
        }
        Permission::HistoricalRead => PermissionCoordinate::HistoricalRead,
        Permission::HistoryRead => PermissionCoordinate::HistoryRead,
        Permission::Watch => PermissionCoordinate::Watch,
        Permission::WriteRelation(relation) => PermissionCoordinate::WriteRelation(relation),
        Permission::WriteField { relation, field } => {
            PermissionCoordinate::WriteField { relation, field }
        }
        Permission::CreateObject(relation) => PermissionCoordinate::CreateObject(relation),
        Permission::DeleteObject(relation) => PermissionCoordinate::DeleteObject(relation),
        Permission::AttachRelationship(relation) => {
            PermissionCoordinate::AttachRelationship(relation)
        }
        Permission::DetachRelationship(relation) => {
            PermissionCoordinate::DetachRelationship(relation)
        }
        Permission::MoveRelationship(relation) => PermissionCoordinate::MoveRelationship(relation),
        Permission::WriteCarrierPresence(carrier) => {
            PermissionCoordinate::WriteCarrierPresence(carrier)
        }
        Permission::WriteCarrierMember { carrier, member } => {
            PermissionCoordinate::WriteCarrierMember { carrier, member }
        }
        Permission::WriteLifecycleEntity(entity) => {
            PermissionCoordinate::WriteLifecycleEntity(entity)
        }
        Permission::WriteLifecycleRoot(entity) => PermissionCoordinate::WriteLifecycleRoot(entity),
        Permission::WriteKeepsAlivePresence(parent) => {
            PermissionCoordinate::WriteKeepsAlivePresence(parent)
        }
        Permission::WriteKeepsAliveEdge { parent, child } => {
            PermissionCoordinate::WriteKeepsAliveEdge { parent, child }
        }
        Permission::Read | Permission::Write => {
            unreachable!("kernel permissions are always exact coordinates")
        }
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
                    | Permission::WriteCarrierPresence(_)
                    | Permission::WriteCarrierMember { .. }
                    | Permission::WriteLifecycleEntity(_)
                    | Permission::WriteLifecycleRoot(_)
                    | Permission::WriteKeepsAlivePresence(_)
                    | Permission::WriteKeepsAliveEdge { .. }
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
        for carrier in &footprint.carrier_presence {
            if !self.contains(Permission::WriteCarrierPresence(*carrier)) {
                return Err(permission_denied(
                    principal,
                    format!("write carrier presence {}", carrier.raw()),
                ));
            }
        }
        for (carrier, member) in &footprint.carrier_members {
            if !self.contains(Permission::WriteCarrierMember {
                carrier: *carrier,
                member: *member,
            }) {
                return Err(permission_denied(
                    principal,
                    format!("write carrier member {}:{}", carrier.raw(), member.raw()),
                ));
            }
        }
        for entity in &footprint.lifecycle_entities {
            if !self.contains(Permission::WriteLifecycleEntity(*entity)) {
                return Err(permission_denied(
                    principal,
                    format!("write lifecycle entity {}", entity.raw()),
                ));
            }
        }
        for entity in &footprint.lifecycle_roots {
            if !self.contains(Permission::WriteLifecycleRoot(*entity)) {
                return Err(permission_denied(
                    principal,
                    format!("write lifecycle root {}", entity.raw()),
                ));
            }
        }
        for parent in &footprint.keeps_alive_presence {
            if !self.contains(Permission::WriteKeepsAlivePresence(*parent)) {
                return Err(permission_denied(
                    principal,
                    format!("write keeps-alive presence {}", parent.raw()),
                ));
            }
        }
        for (parent, child) in &footprint.keeps_alive_edges {
            if !self.contains(Permission::WriteKeepsAliveEdge {
                parent: *parent,
                child: *child,
            }) {
                return Err(permission_denied(
                    principal,
                    format!("write keeps-alive edge {}:{}", parent.raw(), child.raw()),
                ));
            }
        }
        Ok(())
    }
}

impl FromIterator<Permission> for PermissionSet {
    fn from_iter<T: IntoIterator<Item = Permission>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
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
    schema_roles: Option<BTreeSet<crate::RoleId>>,
    access_schema_revision: Option<u64>,
}

#[derive(Clone)]
pub struct Session {
    principal: PrincipalId,
    state: Arc<RwLock<SessionState>>,
    role_runtime: Option<Arc<kernel_plan::DurableRuntime>>,
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
                schema_roles: None,
                access_schema_revision: None,
            })),
            role_runtime: None,
        }
    }

    pub(crate) fn for_schema_roles(
        principal: PrincipalId,
        runtime: Arc<kernel_plan::DurableRuntime>,
        roles: BTreeSet<crate::RoleId>,
    ) -> Result<Self> {
        let (schema_revision, permissions) = resolve_schema_roles(&runtime, &roles, principal)?;
        Ok(Self {
            principal,
            state: Arc::new(RwLock::new(SessionState {
                generation: 0,
                permissions,
                revoked: false,
                schema_roles: Some(roles),
                access_schema_revision: Some(schema_revision),
            })),
            role_runtime: Some(runtime),
        })
    }

    fn refresh_schema_roles_if_needed(&self) -> Result<()> {
        let Some(runtime) = &self.role_runtime else {
            return Ok(());
        };
        let snapshot = runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("schema-role authority snapshot failed: {error:?}"),
            )
        })?;
        let schema_revision = snapshot.revision().semantic_revision().schema.raw();
        let roles = {
            let state = self.state.read().map_err(|_| session_state_poisoned())?;
            if state.revoked {
                return Err(session_revoked(self.principal));
            }
            if state.access_schema_revision == Some(schema_revision) {
                return Ok(());
            }
            state
                .schema_roles
                .clone()
                .expect("role-bound session retains external role assignments")
        };
        let permissions =
            resolve_schema_roles_from_revision(snapshot.revision(), &roles, self.principal)?;
        let mut state = self.state.write().map_err(|_| session_state_poisoned())?;
        if state.revoked {
            return Err(session_revoked(self.principal));
        }
        if state.access_schema_revision != Some(schema_revision) {
            state.generation = state.generation.checked_add(1).ok_or_else(|| {
                Error::new(
                    ErrorKind::Internal,
                    "session authority generation overflowed",
                )
            })?;
            state.permissions = permissions;
            state.access_schema_revision = Some(schema_revision);
        }
        Ok(())
    }

    pub fn refresh_role_assignments(
        &self,
        roles: impl IntoIterator<Item = crate::RoleId>,
    ) -> Result<u64> {
        let Some(runtime) = &self.role_runtime else {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "explicit-permission session has no schema-role assignment",
            ));
        };
        let roles: BTreeSet<_> = roles.into_iter().collect();
        let (schema_revision, permissions) = resolve_schema_roles(runtime, &roles, self.principal)?;
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
        state.schema_roles = Some(roles);
        state.access_schema_revision = Some(schema_revision);
        Ok(state.generation)
    }

    #[must_use]
    pub const fn principal(&self) -> PrincipalId {
        self.principal
    }

    pub fn snapshot(&self) -> Result<SessionSnapshot> {
        self.refresh_schema_roles_if_needed()?;
        let state = self.state.read().map_err(|_| session_state_poisoned())?;
        Ok(SessionSnapshot {
            generation: state.generation,
            permissions: state.permissions.clone(),
            revoked: state.revoked,
        })
    }

    pub fn refresh_permissions(&self, permissions: PermissionSet) -> Result<u64> {
        if self.role_runtime.is_some() {
            return Err(Error::new(
                ErrorKind::InvalidPlan,
                "schema-role sessions must refresh external role assignments, not flattened permissions",
            ));
        }
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
        self.refresh_schema_roles_if_needed()?;
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

    fn require_read_entry(&self) -> Result<()> {
        self.refresh_schema_roles_if_needed()?;
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
        self.refresh_schema_roles_if_needed()?;
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
        self.refresh_schema_roles_if_needed()?;
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
        self.refresh_schema_roles_if_needed()?;
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
        self.refresh_schema_roles_if_needed()?;
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
        self.refresh_schema_roles_if_needed()?;
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
        self.refresh_schema_roles_if_needed()?;
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
        self.refresh_schema_roles_if_needed()?;
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

fn resolve_schema_roles(
    runtime: &Arc<kernel_plan::DurableRuntime>,
    roles: &BTreeSet<crate::RoleId>,
    principal: PrincipalId,
) -> Result<(u64, PermissionSet)> {
    let snapshot = runtime.snapshot().map_err(|error| {
        Error::new(
            ErrorKind::Internal,
            format!("schema-role authority snapshot failed: {error:?}"),
        )
    })?;
    let schema_revision = snapshot.revision().semantic_revision().schema.raw();
    let permissions = resolve_schema_roles_from_revision(snapshot.revision(), roles, principal)?;
    Ok((schema_revision, permissions))
}

fn resolve_schema_roles_from_revision(
    revision: &kernel_revision::Revision,
    roles: &BTreeSet<crate::RoleId>,
    principal: PrincipalId,
) -> Result<PermissionSet> {
    revision
        .semantic_context()
        .schema
        .resolve_access_roles(
            roles
                .iter()
                .map(|role| kernel_types::SemanticId::new(role.raw())),
        )
        .map_err(|_| {
            permission_denied(
                principal,
                "current authoritative Schema.Access role assignment",
            )
        })
        .map(|permissions| {
            permissions
                .into_iter()
                .map(permission_from_kernel)
                .collect()
        })
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

    /// Opens current authoritative HEAD while interpreting incoming queries in one retained
    /// contract schema language. The returned context contains only a verified semantic bridge;
    /// no historical source-world state is materialized.
    pub fn snapshot_for_contract(&self, source_schema_revision: u64) -> Result<ReadContext> {
        self.authority.require_read_entry()?;
        let current = self
            .database
            .snapshot_with_authority(self.authority.clone())?;
        if current.schema_revision() == source_schema_revision {
            return Ok(current);
        }
        let bridge = self
            .database
            .current_schema_bridge(source_schema_revision)?
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::ContractNotRepresentable,
                    format!(
                        "no retained exact current-world bridge from schema revision {source_schema_revision} to current schema revision {}",
                        current.schema_revision(),
                    ),
                )
            })?;
        current.with_current_schema_bridge(bridge)
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
    use std::{
        fs,
        path::Path,
        sync::{Arc, TryLockError, mpsc},
    };

    #[derive(Debug)]
    struct PanicEncryptionProvider;

    #[derive(Debug)]
    struct StaticEncryptionProvider;

    impl crate::EncryptionKeyProvider for StaticEncryptionProvider {
        fn provide_key(
            &self,
            _path: &Path,
            _operation: crate::EncryptionKeyOperation,
            destination: &mut crate::EncryptionKeyDestination<'_>,
        ) -> Result<crate::EncryptionProviderKeyMetadata> {
            destination.write(&[0x79; 32]);
            Ok(crate::EncryptionProviderKeyMetadata::new(
                crate::EncryptionKeyId::from_bytes([0x79; 16]),
                1,
            ))
        }
    }

    impl crate::EncryptionKeyProvider for PanicEncryptionProvider {
        fn provide_key(
            &self,
            _path: &Path,
            _operation: crate::EncryptionKeyOperation,
            _destination: &mut crate::EncryptionKeyDestination<'_>,
        ) -> Result<crate::EncryptionProviderKeyMetadata> {
            panic!("encryption provider must not run before administration authorization")
        }
    }

    #[test]
    fn schema_access_cannot_mint_database_control_authority() {
        let fake_admin = AccessCapability::new("cfmd.database.admin.export").model_read();
        let operator = Role::new("database.operator").capability(&fake_admin);
        let operator_id = operator.id();
        let schema = crate::Schema::builder()
            .revisions(582, 1)
            .access(SchemaAccess::new().capability(fake_admin).role(operator))
            .build()
            .unwrap();
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-two-plane-schema-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let database = Database::create(&path, schema).unwrap();
        let restricted = database
            .session_for_roles(PrincipalId::new(582_001), [operator_id])
            .unwrap();
        let permissions = restricted.session().snapshot().unwrap();
        assert!(permissions.permissions().contains(Permission::ModelRead));

        let backup = path.with_extension("denied-backup");
        let control = crate::DatabaseControlCredential::authenticated(
            PrincipalId::new(582_002),
            crate::DatabaseControlPermissionSet::new(),
        );
        let admin = database.admin_session(control.session());
        let error = admin
            .backup_to(&backup, &crate::Encryption::None)
            .expect_err("Schema.Access must not imply control-plane export");
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert!(!backup.exists());
        drop(admin);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn protection_reconfigure_control_gate_precedes_provider_resolution() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-control-protection-denied-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let database = Database::create(
            &path,
            crate::Schema::builder().revisions(582, 2).build().unwrap(),
        )
        .unwrap();
        let control = crate::DatabaseControlCredential::authenticated(
            PrincipalId::new(582_003),
            crate::DatabaseControlPermissionSet::new(),
        );
        let admin = database.admin_session(control.session());
        let encryption =
            crate::Encryption::aes256_gcm_siv_with_provider(Arc::new(PanicEncryptionProvider));
        let error = admin
            .reconfigure_protection(&encryption)
            .expect_err("control denial must precede provider resolution");
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        drop(admin);
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn authorized_control_reaches_existing_protection_authority() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-control-protection-authorized-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let database = Database::create(
            &path,
            crate::Schema::builder().revisions(582, 3).build().unwrap(),
        )
        .unwrap();
        let control = crate::DatabaseControlCredential::authenticated(
            PrincipalId::new(582_004),
            crate::DatabaseControlPermissionSet::from([
                crate::DatabaseControlPermission::ProtectionReconfigure,
            ]),
        );
        let admin = database.admin_session(control.session());
        let encryption =
            crate::Encryption::aes256_gcm_siv_with_provider(Arc::new(StaticEncryptionProvider));
        let error = admin
            .reconfigure_protection(&encryption)
            .expect_err("plaintext store has no wrapped-key authority");
        assert_eq!(error.kind(), ErrorKind::Recovery);
        assert_eq!(
            error.recovery_diagnostic().unwrap().operation(),
            crate::RecoveryOperation::ProtectionReconfigure
        );
        drop(admin);
        database
            .snapshot()
            .expect("invalid request must not poison source");
        drop(database);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn model_coordinate_authority_is_exact_and_generic_write_does_not_cover_it() {
        let carrier = crate::ModelSemanticId::new(91_100);
        let member = crate::ModelEntityId::new(91_101);
        let parent = crate::ModelEntityId::new(91_102);
        let child = crate::ModelEntityId::new(91_103);
        let footprint = PublicationAuthorityFootprint {
            carrier_presence: BTreeSet::from([carrier]),
            carrier_members: BTreeSet::from([(carrier, member)]),
            lifecycle_entities: BTreeSet::from([member]),
            lifecycle_roots: BTreeSet::from([member]),
            keeps_alive_presence: BTreeSet::from([parent]),
            keeps_alive_edges: BTreeSet::from([(parent, child)]),
            ..PublicationAuthorityFootprint::default()
        };
        let principal = PrincipalId::new(91_104);
        assert_eq!(
            PermissionSet::from([Permission::Write])
                .require_publication_footprint(principal, &footprint)
                .unwrap_err()
                .kind(),
            ErrorKind::PermissionDenied
        );
        let exact = PermissionSet::from([
            Permission::WriteCarrierPresence(carrier),
            Permission::WriteCarrierMember { carrier, member },
            Permission::WriteLifecycleEntity(member),
            Permission::WriteLifecycleRoot(member),
            Permission::WriteKeepsAlivePresence(parent),
            Permission::WriteKeepsAliveEdge { parent, child },
        ]);
        exact
            .require_publication_footprint(principal, &footprint)
            .expect("exact model-coordinate authority");
    }

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
