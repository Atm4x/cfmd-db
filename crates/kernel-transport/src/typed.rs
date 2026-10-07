use std::collections::{BTreeMap, BTreeSet};

use kernel_model::{DatabaseState, FiniteModel, Value};
use kernel_query::{ExactQuery, QueryTypeError, RelExpr, RelQueryError, RelationDelta};
use kernel_schema::SemanticContext;
use kernel_semantics::SemanticRegistry;
use kernel_types::EntityId;

use crate::TransportError;

type FieldTransportChanges = (
    Vec<(kernel_types::SemanticId, EntityId, Option<Value>)>,
    BTreeSet<(kernel_types::SemanticId, EntityId)>,
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRewrite {
    pub source_field: kernel_types::SemanticId,
    pub target_field: kernel_types::SemanticId,
    pub transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationRewrite {
    pub target_relation: kernel_types::SemanticId,
    pub transform: RelExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedRelationRewrite {
    target_relation: kernel_types::SemanticId,
    transform: kernel_query::PreparedRelExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedRelationTransport {
    source: SemanticContext,
    target: SemanticContext,
    rewrites: Vec<PreparedRelationRewrite>,
    passthrough: BTreeSet<kernel_types::SemanticId>,
}

impl TypedRelationTransport {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
        rewrites: Vec<RelationRewrite>,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source
            .environment
            .definitionally_equivalent(&target.environment)
        {
            return Err(TransportError::SemanticEnvironmentChangeRequiresTransport);
        }
        if !source
            .schema
            .relation_transport_base_equivalent(&target.schema)
        {
            return Err(TransportError::UnsupportedStructuralChange);
        }

        let mut by_target = BTreeMap::new();
        for rewrite in rewrites {
            let target_relation = rewrite.target_relation;
            if by_target.insert(target_relation, rewrite).is_some() {
                return Err(TransportError::DuplicateTargetRelation(target_relation));
            }
        }

        let mut passthrough = BTreeSet::new();
        let mut prepared_rewrites = Vec::new();
        for target_relation in target.schema.relations() {
            let relation_definition_unchanged =
                source.schema.relation(target_relation.id) == Some(target_relation);
            let relation_column_identity_unchanged =
                source.schema.relation_column_ids(target_relation.id)
                    == target.schema.relation_column_ids(target_relation.id);
            if relation_definition_unchanged && relation_column_identity_unchanged {
                if by_target.contains_key(&target_relation.id) {
                    return Err(TransportError::RewriteTargetsPassthroughRelation(
                        target_relation.id,
                    ));
                }
                passthrough.insert(target_relation.id);
                continue;
            }
            let rewrite = by_target
                .get(&target_relation.id)
                .ok_or(TransportError::UnknownTargetRelation(target_relation.id))?;
            let prepared = rewrite
                .transform
                .prepare(source, registry)
                .map_err(TransportError::RelationTransformType)?;
            if prepared.result_type().columns != target_relation.columns
                || prepared.result_type().semantics != target_relation.semantics
            {
                return Err(TransportError::RelationTransformType(
                    RelQueryError::TypeMismatch,
                ));
            }
            prepared_rewrites.push(PreparedRelationRewrite {
                target_relation: target_relation.id,
                transform: prepared,
            });
        }

        let known_targets: BTreeSet<_> = target
            .schema
            .relations()
            .map(|relation| relation.id)
            .collect();
        if let Some(unknown) = by_target
            .keys()
            .find(|relation| !known_targets.contains(relation))
        {
            return Err(TransportError::UnknownTargetRelation(*unknown));
        }

        Ok(Self {
            source: source.clone(),
            target: target.clone(),
            rewrites: prepared_rewrites,
            passthrough,
        })
    }

    fn transport_state(
        &self,
        source_state: &DatabaseState,
        registry: &SemanticRegistry,
    ) -> Result<DatabaseState, TransportError> {
        let mut model = FiniteModel {
            carriers: source_state.model.carriers.clone(),
            fields: source_state.model.fields.clone(),
            relations: kernel_model::RelationStore::default(),
        };
        for relation in &self.passthrough {
            if let Some(rows) = source_state.model.relations.get(relation) {
                model.relations.insert_shared(*relation, rows.clone());
            }
        }
        for rewrite in &self.rewrites {
            let result = rewrite
                .transform
                .evaluate(&source_state.model, &self.source, registry)
                .map_err(TransportError::RelationTransformExecution)?;
            model
                .relations
                .insert(rewrite.target_relation, result.rows().to_vec());
        }
        let state = DatabaseState {
            model,
            lifecycle: source_state.lifecycle.clone(),
        };
        kernel_validation::validate_state(&self.target, registry, &state)
            .map_err(TransportError::InvalidTarget)?;
        Ok(state)
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        let state = self.transport_state(source.state(), registry)?;
        kernel_revision::Revision::build(target_id, &self.target, registry, state)
            .map_err(TransportError::InvalidRevision)
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedFieldRewrite {
    target_field: kernel_types::SemanticId,
    transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedFieldTransport {
    source: SemanticContext,
    target: SemanticContext,
    rewrites_by_source: BTreeMap<kernel_types::SemanticId, Vec<PreparedFieldRewrite>>,
    passthrough: BTreeSet<kernel_types::SemanticId>,
}

impl TypedFieldTransport {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
        rewrites: Vec<FieldRewrite>,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source
            .environment
            .definitionally_equivalent(&target.environment)
        {
            return Err(TransportError::SemanticEnvironmentChangeRequiresTransport);
        }
        if !source
            .schema
            .field_transport_base_equivalent(&target.schema)
        {
            return Err(TransportError::UnsupportedStructuralChange);
        }

        let mut by_target = BTreeMap::new();
        for rewrite in rewrites {
            let target_field = rewrite.target_field;
            if by_target.insert(target_field, rewrite).is_some() {
                return Err(TransportError::DuplicateTargetField(target_field));
            }
        }

        let mut passthrough = BTreeSet::new();
        let mut rewrites_by_source = BTreeMap::<_, Vec<_>>::new();
        for target_field in target.schema.fields() {
            if let Some(source_field) = source.schema.field(target_field.id)
                && source_field == target_field
            {
                if by_target.contains_key(&target_field.id) {
                    return Err(TransportError::RewriteTargetsPassthroughField(
                        target_field.id,
                    ));
                }
                passthrough.insert(target_field.id);
                continue;
            }
            let rewrite = by_target
                .get(&target_field.id)
                .ok_or(TransportError::UnknownTargetField(target_field.id))?;
            let source_field = source
                .schema
                .field(rewrite.source_field)
                .ok_or(TransportError::UnknownSourceField(rewrite.source_field))?;
            if source_field.owner != target_field.owner {
                return Err(TransportError::OwnerTypeMismatch(target_field.id));
            }
            let result = rewrite
                .transform
                .typecheck(&source_field.value)
                .map_err(TransportError::TransformType)?;
            if result != target_field.value {
                return Err(TransportError::TransformType(QueryTypeError::TypeMismatch));
            }
            rewrites_by_source
                .entry(rewrite.source_field)
                .or_default()
                .push(PreparedFieldRewrite {
                    target_field: target_field.id,
                    transform: rewrite.transform.clone(),
                });
        }

        let known_targets: BTreeSet<_> = target.schema.fields().map(|field| field.id).collect();
        if let Some(unknown) = by_target
            .keys()
            .find(|field| !known_targets.contains(field))
        {
            return Err(TransportError::UnknownTargetField(*unknown));
        }

        Ok(Self {
            source: source.clone(),
            target: target.clone(),
            rewrites_by_source,
            passthrough,
        })
    }

    fn transport_state(
        &self,
        source_state: &DatabaseState,
        registry: &SemanticRegistry,
    ) -> Result<DatabaseState, TransportError> {
        let mut model = FiniteModel {
            carriers: source_state.model.carriers.clone(),
            fields: BTreeMap::new().into(),
            relations: source_state.model.relations.clone(),
        };

        for (&(field, entity), value) in &source_state.model.fields {
            if self.passthrough.contains(&field) {
                model.fields.insert((field, entity), value.clone());
            }
            if let Some(rewrites) = self.rewrites_by_source.get(&field) {
                for rewrite in rewrites {
                    let transformed = rewrite
                        .transform
                        .evaluate(value)
                        .map_err(TransportError::TransformExecution)?;
                    model
                        .fields
                        .insert((rewrite.target_field, entity), transformed);
                }
            }
        }
        let state = DatabaseState {
            model,
            lifecycle: source_state.lifecycle.clone(),
        };
        kernel_validation::validate_state(&self.target, registry, &state)
            .map_err(TransportError::InvalidTarget)?;
        Ok(state)
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        let state = self.transport_state(source.state(), registry)?;
        kernel_revision::Revision::build(target_id, &self.target, registry, state)
            .map_err(TransportError::InvalidRevision)
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }
}

/// One target field in a schema migration.
///
/// `source_fields` are assembled into a product keyed by their semantic IDs and
/// passed to `transform`. An empty list supplies an empty product, which makes
/// constant/default field creation explicit. Multiple targets may read the same
/// source coordinates (split), and one target may read multiple source
/// coordinates (merge). Source fields not referenced by the target schema are
/// naturally dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFieldRewrite {
    pub source_fields: Vec<kernel_types::SemanticId>,
    pub target_field: kernel_types::SemanticId,
    pub transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedMigrationFieldRewrite {
    owner: kernel_types::SemanticId,
    source_fields: Vec<kernel_types::SemanticId>,
    target_field: kernel_types::SemanticId,
    transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationColumnRewrite {
    pub source_columns: Vec<kernel_types::SemanticId>,
    pub target_column: kernel_types::SemanticId,
    pub transform: ExactQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationRowRewrite {
    pub source_relation: kernel_types::SemanticId,
    pub target_relation: kernel_types::SemanticId,
    pub columns: Vec<MigrationColumnRewrite>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationRelationRewrite {
    Query(RelationRewrite),
    Rows(MigrationRowRewrite),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedMigrationColumnRewrite {
    target_column: kernel_types::SemanticId,
    source_columns: Vec<(kernel_types::SemanticId, usize)>,
    transform: ExactQuery,
}

/// Exact current-schema write coordinates induced by a source relation-column
/// footprint through one verified row-local migration step.
///
/// This transports *required authority*, not grants. Callers must authorize
/// every returned target coordinate in the current world. General relational
/// rewrites remain fail-closed because a local source write can have
/// non-local output effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationRelationWriteFootprint {
    pub target_relation: kernel_types::SemanticId,
    pub target_columns: BTreeSet<kernel_types::SemanticId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve inline state ownership without adding allocations."
)]
enum PreparedMigrationRelationRewrite {
    Query {
        rewrite: PreparedRelationRewrite,
        source_relations: BTreeSet<kernel_types::SemanticId>,
    },
    Rows {
        source_relation: kernel_types::SemanticId,
        target_relation: kernel_types::SemanticId,
        columns: Vec<PreparedMigrationColumnRewrite>,
    },
}

/// One independently materializable target-relation slice of a verified
/// schema migration.
///
/// The coordinate is derived from the verified forward program; it is not a
/// second migration-progress journal. A storage layer can therefore retain a
/// source-epoch physical slice until this target relation is materialized,
/// then advance only that coordinate to native target representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationRelationSlice {
    /// Source and target relation definitions are identical. The same logical
    /// rows are valid under the target schema without rewriting bytes/values.
    Passthrough { relation: kernel_types::SemanticId },
    /// Every target row depends only on one source row from one relation.
    /// This is the preferred streaming/background rewrite primitive.
    RowLocal {
        source_relation: kernel_types::SemanticId,
        target_relation: kernel_types::SemanticId,
    },
    /// General deterministic relational rewrite. The exact source dependency
    /// set is explicit so a mixed-representation scheduler never needs to
    /// route through an opaque generic fallback.
    Query {
        target_relation: kernel_types::SemanticId,
        source_relations: BTreeSet<kernel_types::SemanticId>,
    },
}

/// Exact structural class of one source relation as an observation crosses a
/// verified schema migration.
///
/// `RowIdentity` is the metadata-only theorem used by maintained watches: the
/// already-maintained row payload and its Γ support remain valid verbatim.
/// `RowLocalStateTransform` is deliberately distinct.  The migration has an
/// exact pointwise row transform, but an already-materialized observation
/// state must change value domain and therefore cannot be rebound as metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationRelationTransport {
    RowIdentity {
        target_relation: kernel_types::SemanticId,
    },
    RowLocalStateTransform {
        target_relation: kernel_types::SemanticId,
    },
}

impl MigrationRelationSlice {
    #[must_use]
    pub const fn target_relation(&self) -> kernel_types::SemanticId {
        match self {
            Self::Passthrough { relation } => *relation,
            Self::RowLocal {
                target_relation, ..
            }
            | Self::Query {
                target_relation, ..
            } => *target_relation,
        }
    }

    #[must_use]
    pub fn source_relations(&self) -> BTreeSet<kernel_types::SemanticId> {
        match self {
            Self::Passthrough { relation } => BTreeSet::from([*relation]),
            Self::RowLocal {
                source_relation, ..
            } => BTreeSet::from([*source_relation]),
            Self::Query {
                source_relations, ..
            } => source_relations.clone(),
        }
    }
}

#[must_use]
pub const fn migration_column_input_id(
    column: kernel_types::SemanticId,
) -> kernel_types::SemanticId {
    column
}

/// Canonical serializable description of one deterministic schema migration.
///
/// The program carries only target semantics and deterministic rewrites. The
/// source world is supplied by the causal source revision when the program is
/// verified/replayed, so durable migration identity never needs a second full
/// target-state snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessPolicyChangeKind {
    CapabilityAdded,
    CapabilityRemoved,
    CapabilityWidened,
    CapabilityNarrowed,
    CapabilityAuthorityShapeChanged,
    RoleAdded,
    RoleRemoved,
    RoleWidened,
    RoleNarrowed,
    RoleCompositionChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessPolicyChange {
    subject: crate::AccessPolicySubject,
    kind: AccessPolicyChangeKind,
    affected_roles: BTreeSet<kernel_types::SemanticId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccessObservationFlow {
    role: kernel_types::SemanticId,
    source: kernel_schema::PermissionCoordinate,
    target: kernel_schema::PermissionCoordinate,
}

impl AccessObservationFlow {
    #[must_use]
    pub const fn role(&self) -> kernel_types::SemanticId {
        self.role
    }

    #[must_use]
    pub const fn source(&self) -> kernel_schema::PermissionCoordinate {
        self.source
    }

    #[must_use]
    pub const fn target(&self) -> kernel_schema::PermissionCoordinate {
        self.target
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccessDeclassificationEdge {
    role: kernel_types::SemanticId,
    target: kernel_schema::PermissionCoordinate,
}

impl AccessDeclassificationEdge {
    #[must_use]
    pub const fn role(&self) -> kernel_types::SemanticId {
        self.role
    }

    #[must_use]
    pub const fn target(&self) -> kernel_schema::PermissionCoordinate {
        self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessNoninterferenceCertification {
    flows: Vec<AccessObservationFlow>,
    declassification_edges: Vec<AccessDeclassificationEdge>,
}

impl AccessNoninterferenceCertification {
    #[must_use]
    pub fn flows(&self) -> &[AccessObservationFlow] {
        &self.flows
    }

    #[must_use]
    pub fn declassification_edges(&self) -> &[AccessDeclassificationEdge] {
        &self.declassification_edges
    }

    #[must_use]
    pub fn is_noninterfering(&self) -> bool {
        self.declassification_edges.is_empty()
    }
}

impl AccessPolicyChange {
    #[must_use]
    pub const fn subject(&self) -> crate::AccessPolicySubject {
        self.subject
    }

    #[must_use]
    pub const fn kind(&self) -> AccessPolicyChangeKind {
        self.kind
    }

    #[must_use]
    pub const fn affected_roles(&self) -> &BTreeSet<kernel_types::SemanticId> {
        &self.affected_roles
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaMigrationProgram {
    target: SemanticContext,
    field_rewrites: Vec<MigrationFieldRewrite>,
    relation_rewrites: Vec<MigrationRelationRewrite>,
}

impl SchemaMigrationProgram {
    #[must_use]
    pub fn new(
        target: SemanticContext,
        field_rewrites: Vec<MigrationFieldRewrite>,
        relation_rewrites: Vec<MigrationRelationRewrite>,
    ) -> Self {
        Self {
            target,
            field_rewrites,
            relation_rewrites,
        }
    }

    #[must_use]
    pub const fn target(&self) -> &SemanticContext {
        &self.target
    }

    #[must_use]
    pub fn field_rewrites(&self) -> &[MigrationFieldRewrite] {
        &self.field_rewrites
    }

    #[must_use]
    pub fn relation_rewrites(&self) -> &[MigrationRelationRewrite] {
        &self.relation_rewrites
    }

    pub fn verify(
        &self,
        source: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<SchemaMigrationTransport, TransportError> {
        SchemaMigrationTransport::verify(
            source,
            &self.target,
            registry,
            self.field_rewrites.clone(),
            self.relation_rewrites.clone(),
        )
    }
}

/// Verified one-step structural data migration between two schema revisions.
///
/// Unlike `TypedFieldTransport`/`TypedRelationTransport`, this transport may
/// change fields and relations in the same atomic step. The migration is still
/// deliberately conservative about entity/type/lifecycle structure: those
/// coordinates must remain definitionally equal. Frontends can therefore
/// compile rename, conversion, split, merge, create/default and drop operations
/// into one deterministic kernel object without embedding host callbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaMigrationTransport {
    source: SemanticContext,
    target: SemanticContext,
    field_rewrites: Vec<PreparedMigrationFieldRewrite>,
    field_passthrough: BTreeSet<kernel_types::SemanticId>,
    relation_rewrites: Vec<PreparedMigrationRelationRewrite>,
    relation_passthrough: BTreeSet<kernel_types::SemanticId>,
    access_policy_changes: Vec<AccessPolicyChange>,
}

/// One verified current-world bridge from a source schema language into the
/// authoritative target schema. The bridge contains no source data and never
/// revives the source schema as a live database world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaBridge {
    transport: SchemaMigrationTransport,
}

fn access_effective_capabilities(
    access: &kernel_schema::SchemaAccess,
    role: kernel_types::SemanticId,
) -> BTreeSet<kernel_types::SemanticId> {
    fn collect(
        access: &kernel_schema::SchemaAccess,
        role: kernel_types::SemanticId,
        visiting: &mut BTreeSet<kernel_types::SemanticId>,
        out: &mut BTreeSet<kernel_types::SemanticId>,
    ) {
        if !visiting.insert(role) {
            return;
        }
        if let Some(def) = access.roles.get(&role) {
            out.extend(def.capabilities.iter().copied());
            for included in &def.includes {
                collect(access, *included, visiting, out);
            }
        }
        visiting.remove(&role);
    }
    let mut out = BTreeSet::new();
    collect(access, role, &mut BTreeSet::new(), &mut out);
    out
}

fn access_roles_using_capability(
    access: &kernel_schema::SchemaAccess,
    capability: kernel_types::SemanticId,
) -> BTreeSet<kernel_types::SemanticId> {
    access
        .roles
        .keys()
        .copied()
        .filter(|role| access_effective_capabilities(access, *role).contains(&capability))
        .collect()
}

fn access_roles_including_role(
    access: &kernel_schema::SchemaAccess,
    changed_role: kernel_types::SemanticId,
) -> BTreeSet<kernel_types::SemanticId> {
    fn includes(
        access: &kernel_schema::SchemaAccess,
        role: kernel_types::SemanticId,
        target: kernel_types::SemanticId,
        visiting: &mut BTreeSet<kernel_types::SemanticId>,
    ) -> bool {
        if role == target {
            return true;
        }
        if !visiting.insert(role) {
            return false;
        }
        let result = access.roles.get(&role).is_some_and(|def| {
            def.includes
                .iter()
                .copied()
                .any(|included| includes(access, included, target, visiting))
        });
        visiting.remove(&role);
        result
    }
    access
        .roles
        .keys()
        .copied()
        .filter(|role| includes(access, *role, changed_role, &mut BTreeSet::new()))
        .collect()
}

fn classify_access_capability_change(
    bridge: &SchemaBridge,
    source: &kernel_schema::SchemaAccess,
    target: &kernel_schema::SchemaAccess,
    id: kernel_types::SemanticId,
) -> Option<AccessPolicyChangeKind> {
    let (Some(source_capability), Some(target_capability)) =
        (source.capabilities.get(&id), target.capabilities.get(&id))
    else {
        return if source.capabilities.contains_key(&id) {
            Some(AccessPolicyChangeKind::CapabilityRemoved)
        } else {
            Some(AccessPolicyChangeKind::CapabilityAdded)
        };
    };
    let Ok(transported) = source_capability
        .permissions
        .iter()
        .copied()
        .map(|permission| bridge.transport_access_permission_exact(permission))
        .collect::<Result<BTreeSet<_>, _>>()
    else {
        return Some(AccessPolicyChangeKind::CapabilityAuthorityShapeChanged);
    };
    if transported == target_capability.permissions {
        None
    } else if transported.is_subset(&target_capability.permissions) {
        Some(AccessPolicyChangeKind::CapabilityWidened)
    } else if target_capability.permissions.is_subset(&transported) {
        Some(AccessPolicyChangeKind::CapabilityNarrowed)
    } else {
        Some(AccessPolicyChangeKind::CapabilityAuthorityShapeChanged)
    }
}

fn classify_access_role_change(
    source: &kernel_schema::SchemaAccess,
    target: &kernel_schema::SchemaAccess,
    id: kernel_types::SemanticId,
) -> Option<AccessPolicyChangeKind> {
    let (Some(source_role), Some(target_role)) = (source.roles.get(&id), target.roles.get(&id))
    else {
        return if source.roles.contains_key(&id) {
            Some(AccessPolicyChangeKind::RoleRemoved)
        } else {
            Some(AccessPolicyChangeKind::RoleAdded)
        };
    };
    if source_role == target_role {
        return None;
    }
    let source_effective = access_effective_capabilities(source, id);
    let target_effective = access_effective_capabilities(target, id);
    if source_effective != target_effective && source_effective.is_subset(&target_effective) {
        Some(AccessPolicyChangeKind::RoleWidened)
    } else if source_effective != target_effective && target_effective.is_subset(&source_effective)
    {
        Some(AccessPolicyChangeKind::RoleNarrowed)
    } else {
        Some(AccessPolicyChangeKind::RoleCompositionChanged)
    }
}

impl SchemaBridge {
    pub fn verify(
        program: &SchemaMigrationProgram,
        source: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<Self, TransportError> {
        Ok(Self {
            transport: program.verify(source, registry)?,
        })
    }

    #[must_use]
    pub const fn source(&self) -> &SemanticContext {
        &self.transport.source
    }

    #[must_use]
    pub const fn target(&self) -> &SemanticContext {
        &self.transport.target
    }

    /// Compiles one source-language relational read directly into the target
    /// semantic world when every scanned relation has exact row identity.
    /// Operator structure and result type are preserved; value-changing,
    /// fan-in/fan-out and global rewrites fail closed.
    pub fn compile_read_exact(
        &self,
        source_query: &RelExpr,
        registry: &SemanticRegistry,
    ) -> Result<RelExpr, TransportError> {
        let source_type = source_query
            .typecheck(&self.transport.source, registry)
            .map_err(TransportError::RelationTransformType)?;
        let mut relation_map = BTreeMap::new();
        let mut targets = BTreeSet::new();
        for source_relation in source_query.scan_relations() {
            let target_relation = match self
                .transport
                .classify_observation_relation_transport_exact(source_relation)?
            {
                ObservationRelationTransport::RowIdentity { target_relation } => target_relation,
                ObservationRelationTransport::RowLocalStateTransform { .. } => {
                    return Err(TransportError::UnrepresentableReadRelation(source_relation));
                }
            };
            if !targets.insert(target_relation) {
                return Err(TransportError::AliasedReadTarget(target_relation));
            }
            relation_map.insert(source_relation, target_relation);
        }
        let target_query = source_query
            .retarget_scan_relations_exact(&relation_map)
            .map_err(TransportError::RelationTransformType)?;
        let target_type = target_query
            .typecheck(&self.transport.target, registry)
            .map_err(TransportError::RelationTransformType)?;
        if target_type != source_type {
            return Err(TransportError::ReadResultTypeMismatch);
        }
        Ok(target_query)
    }

    pub fn transport_relation_delta_exact(
        &self,
        source_relation: kernel_types::SemanticId,
        delta: &RelationDelta,
        registry: &SemanticRegistry,
    ) -> Result<Vec<(kernel_types::SemanticId, RelationDelta)>, TransportError> {
        self.transport
            .transport_relation_delta_exact(source_relation, delta, registry)
    }

    pub fn transport_relation_write_footprint_exact(
        &self,
        source_relation: kernel_types::SemanticId,
        source_columns: &BTreeSet<kernel_types::SemanticId>,
    ) -> Result<Vec<MigrationRelationWriteFootprint>, TransportError> {
        self.transport
            .transport_relation_write_footprint_exact(source_relation, source_columns)
    }

    /// Returns the target relation coordinate only when the row representation
    /// itself survives this migration unchanged. This is the exact coordinate
    /// law used by bridged typed Context mutation lowering; row-local value
    /// transforms remain unrepresentable for direct old-language mutation.
    pub fn transport_relation_identity_exact(
        &self,
        source_relation: kernel_types::SemanticId,
    ) -> Result<kernel_types::SemanticId, TransportError> {
        self.transport
            .transport_observation_relation_identity_exact(source_relation)
    }

    /// Returns one target field coordinate only when this migration preserves
    /// the field value definitionally. Split/merge/value transforms fail closed.
    /// Transports one schema-owned exclusive-ownership contract only when
    /// relationship row identity, target object relation identity and orphan
    /// policy are all preserved by the verified migration.
    pub fn transport_owned_relationship_exact(
        &self,
        source_relation: kernel_types::SemanticId,
    ) -> Result<kernel_schema::OwnedRelationshipDef, TransportError> {
        let source = self
            .transport
            .source
            .schema
            .owned_relationship(source_relation)
            .ok_or(TransportError::UnrepresentableOwnedRelationship(
                source_relation,
            ))?;
        let target_relation = self.transport_relation_identity_exact(source.relation)?;
        let target_object_relation =
            self.transport_relation_identity_exact(source.target_relation)?;
        let target = self
            .transport
            .target
            .schema
            .owned_relationship(target_relation)
            .ok_or(TransportError::UnrepresentableOwnedRelationship(
                source_relation,
            ))?;
        if target.target_relation != target_object_relation
            || target.orphan_policy != source.orphan_policy
        {
            return Err(TransportError::UnrepresentableOwnedRelationship(
                source_relation,
            ));
        }
        Ok(target.clone())
    }

    fn transport_access_permission_exact(
        &self,
        permission: kernel_schema::PermissionCoordinate,
    ) -> Result<kernel_schema::PermissionCoordinate, TransportError> {
        transport_access_permission_exact(&self.transport, permission)
    }
}

fn transport_access_permission_exact(
    transport: &SchemaMigrationTransport,
    permission: kernel_schema::PermissionCoordinate,
) -> Result<kernel_schema::PermissionCoordinate, TransportError> {
    use kernel_schema::PermissionCoordinate as P;
    Ok(match permission {
        P::ReadRelation { relation } => P::ReadRelation {
            relation: SchemaBridge {
                transport: transport.clone(),
            }
            .transport_relation_identity_exact(relation)?,
        },
        P::WriteRelation { relation } => P::WriteRelation {
            relation: SchemaBridge {
                transport: transport.clone(),
            }
            .transport_relation_identity_exact(relation)?,
        },
        P::CreateObject { relation } => P::CreateObject {
            relation: SchemaBridge {
                transport: transport.clone(),
            }
            .transport_relation_identity_exact(relation)?,
        },
        P::DeleteObject { relation } => P::DeleteObject {
            relation: SchemaBridge {
                transport: transport.clone(),
            }
            .transport_relation_identity_exact(relation)?,
        },
        P::AttachRelationship { relation } => P::AttachRelationship {
            relation: SchemaBridge {
                transport: transport.clone(),
            }
            .transport_relation_identity_exact(relation)?,
        },
        P::DetachRelationship { relation } => P::DetachRelationship {
            relation: SchemaBridge {
                transport: transport.clone(),
            }
            .transport_relation_identity_exact(relation)?,
        },
        P::MoveRelationship { relation } => P::MoveRelationship {
            relation: SchemaBridge {
                transport: transport.clone(),
            }
            .transport_relation_identity_exact(relation)?,
        },
        P::ReadField { relation, column } => P::ReadField {
            relation: SchemaBridge {
                transport: transport.clone(),
            }
            .transport_relation_identity_exact(relation)?,
            column: SchemaBridge {
                transport: transport.clone(),
            }
            .transport_field_identity_exact(column)?,
        },
        P::WriteField { relation, column } => P::WriteField {
            relation: SchemaBridge {
                transport: transport.clone(),
            }
            .transport_relation_identity_exact(relation)?,
            column: SchemaBridge {
                transport: transport.clone(),
            }
            .transport_field_identity_exact(column)?,
        },
        other => other,
    })
}

impl SchemaBridge {
    fn schema_access_changes(&self) -> Vec<AccessPolicyChange> {
        let source = self.transport.source.schema.schema_access();
        let target = self.transport.target.schema.schema_access();
        let mut changes = Vec::new();
        let capability_ids: BTreeSet<_> = source
            .capabilities
            .keys()
            .chain(target.capabilities.keys())
            .copied()
            .collect();
        for id in capability_ids {
            if let Some(kind) = classify_access_capability_change(self, source, target, id) {
                let mut affected_roles = access_roles_using_capability(source, id);
                affected_roles.extend(access_roles_using_capability(target, id));
                changes.push(AccessPolicyChange {
                    subject: crate::AccessPolicySubject::Capability(id),
                    kind,
                    affected_roles,
                });
            }
        }
        let role_ids: BTreeSet<_> = source
            .roles
            .keys()
            .chain(target.roles.keys())
            .copied()
            .collect();
        for id in role_ids {
            if let Some(kind) = classify_access_role_change(source, target, id) {
                let mut affected_roles = access_roles_including_role(source, id);
                affected_roles.extend(access_roles_including_role(target, id));
                affected_roles.insert(id);
                changes.push(AccessPolicyChange {
                    subject: crate::AccessPolicySubject::Role(id),
                    kind,
                    affected_roles,
                });
            }
        }
        changes
    }

    pub fn transport_field_identity_exact(
        &self,
        source_field: kernel_types::SemanticId,
    ) -> Result<kernel_types::SemanticId, TransportError> {
        let source_def = self
            .transport
            .source
            .schema
            .field(source_field)
            .ok_or(TransportError::UnknownSourceField(source_field))?;
        if self.transport.field_passthrough.contains(&source_field) {
            return Ok(source_field);
        }

        let mut target = None;
        for rewrite in &self.transport.field_rewrites {
            if rewrite.source_fields.as_slice() != [source_field] {
                continue;
            }
            let exact_identity = rewrite.transform.root()
                == &(kernel_query::Expr::ProductField {
                    input: Box::new(kernel_query::Expr::Input),
                    field: source_field,
                });
            let target_def = self
                .transport
                .target
                .schema
                .field(rewrite.target_field)
                .ok_or(TransportError::UnknownTargetField(rewrite.target_field))?;
            if !exact_identity
                || source_def.owner != target_def.owner
                || source_def.value != target_def.value
                || target.replace(rewrite.target_field).is_some()
            {
                return Err(TransportError::UnrepresentableSourceFieldDependency(
                    source_field,
                ));
            }
        }
        target.ok_or(TransportError::UnrepresentableSourceFieldDependency(
            source_field,
        ))
    }
}

impl SchemaMigrationTransport {
    pub fn verify(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
        field_rewrites: Vec<MigrationFieldRewrite>,
        relation_rewrites: Vec<MigrationRelationRewrite>,
    ) -> Result<Self, TransportError> {
        Self::verify_transport(source, target, registry, field_rewrites, relation_rewrites)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    fn verify_transport(
        source: &SemanticContext,
        target: &SemanticContext,
        registry: &SemanticRegistry,
        field_rewrites: Vec<MigrationFieldRewrite>,
        relation_rewrites: Vec<MigrationRelationRewrite>,
    ) -> Result<Self, TransportError> {
        registry
            .validate_context(source)
            .map_err(TransportError::SourceSemantics)?;
        registry
            .validate_context(target)
            .map_err(TransportError::TargetSemantics)?;
        if !source.schema.migration_base_equivalent(&target.schema) {
            return Err(TransportError::UnsupportedStructuralChange);
        }

        let mut fields_by_target = BTreeMap::new();
        for rewrite in field_rewrites {
            let target = rewrite.target_field;
            if fields_by_target.insert(target, rewrite).is_some() {
                return Err(TransportError::DuplicateTargetField(target));
            }
        }

        let mut field_passthrough = BTreeSet::new();
        let mut prepared_fields = Vec::new();
        for target_field in target.schema.fields() {
            if source.schema.field(target_field.id) == Some(target_field) {
                if fields_by_target.contains_key(&target_field.id) {
                    return Err(TransportError::RewriteTargetsPassthroughField(
                        target_field.id,
                    ));
                }
                field_passthrough.insert(target_field.id);
                continue;
            }

            let rewrite = fields_by_target
                .get(&target_field.id)
                .ok_or(TransportError::UnknownTargetField(target_field.id))?;
            let mut source_types = BTreeMap::new();
            for source_field_id in &rewrite.source_fields {
                let source_field = source
                    .schema
                    .field(*source_field_id)
                    .ok_or(TransportError::UnknownSourceField(*source_field_id))?;
                if source_field.owner != target_field.owner {
                    return Err(TransportError::MigrationFieldOwnerMismatch(
                        *source_field_id,
                    ));
                }
                if source_types
                    .insert(*source_field_id, source_field.value.clone())
                    .is_some()
                {
                    return Err(TransportError::DuplicateMigrationSourceField(
                        *source_field_id,
                    ));
                }
            }
            let input = kernel_schema::TypeExpr::Product(source_types);
            let result = rewrite
                .transform
                .typecheck(&input)
                .map_err(TransportError::TransformType)?;
            if result != target_field.value {
                return Err(TransportError::TransformType(QueryTypeError::TypeMismatch));
            }
            prepared_fields.push(PreparedMigrationFieldRewrite {
                owner: target_field.owner,
                source_fields: rewrite.source_fields.clone(),
                target_field: target_field.id,
                transform: rewrite.transform.clone(),
            });
        }
        let known_target_fields: BTreeSet<_> =
            target.schema.fields().map(|field| field.id).collect();
        if let Some(unknown) = fields_by_target
            .keys()
            .find(|field| !known_target_fields.contains(field))
        {
            return Err(TransportError::UnknownTargetField(*unknown));
        }

        let mut relations_by_target = BTreeMap::new();
        for rewrite in relation_rewrites {
            let target = match &rewrite {
                MigrationRelationRewrite::Query(rewrite) => rewrite.target_relation,
                MigrationRelationRewrite::Rows(rewrite) => rewrite.target_relation,
            };
            if relations_by_target.insert(target, rewrite).is_some() {
                return Err(TransportError::DuplicateTargetRelation(target));
            }
        }
        let mut relation_passthrough = BTreeSet::new();
        let mut prepared_relations = Vec::new();
        for target_relation in target.schema.relations() {
            let relation_definition_unchanged =
                source.schema.relation(target_relation.id) == Some(target_relation);
            let relation_column_identity_unchanged =
                source.schema.relation_column_ids(target_relation.id)
                    == target.schema.relation_column_ids(target_relation.id);
            if relation_definition_unchanged && relation_column_identity_unchanged {
                if relations_by_target.contains_key(&target_relation.id) {
                    return Err(TransportError::RewriteTargetsPassthroughRelation(
                        target_relation.id,
                    ));
                }
                relation_passthrough.insert(target_relation.id);
                continue;
            }
            let rewrite = relations_by_target
                .get(&target_relation.id)
                .ok_or(TransportError::UnknownTargetRelation(target_relation.id))?;
            match rewrite {
                MigrationRelationRewrite::Query(rewrite) => {
                    let source_relations = rewrite.transform.scan_relations();
                    let prepared = rewrite
                        .transform
                        .prepare(source, registry)
                        .map_err(TransportError::RelationTransformType)?;
                    if prepared.result_type().columns != target_relation.columns
                        || prepared.result_type().semantics != target_relation.semantics
                    {
                        return Err(TransportError::RelationTransformType(
                            RelQueryError::TypeMismatch,
                        ));
                    }
                    prepared_relations.push(PreparedMigrationRelationRewrite::Query {
                        rewrite: PreparedRelationRewrite {
                            target_relation: target_relation.id,
                            transform: prepared,
                        },
                        source_relations,
                    });
                }
                MigrationRelationRewrite::Rows(rewrite) => {
                    let source_relation = source.schema.relation(rewrite.source_relation).ok_or(
                        TransportError::UnknownSourceRelation(rewrite.source_relation),
                    )?;
                    let target_column_ids =
                        target
                            .schema
                            .relation_column_ids(target_relation.id)
                            .ok_or(TransportError::UnknownTargetRelation(target_relation.id))?;
                    let mut by_target_column = BTreeMap::new();
                    for column in &rewrite.columns {
                        if target
                            .schema
                            .relation_column_ordinal(target_relation.id, column.target_column)
                            .is_none()
                        {
                            return Err(TransportError::MissingTargetMigrationColumn(
                                target_relation.id,
                                column.target_column,
                            ));
                        }
                        if by_target_column
                            .insert(column.target_column, column)
                            .is_some()
                        {
                            return Err(TransportError::DuplicateTargetMigrationColumn(
                                target_relation.id,
                                column.target_column,
                            ));
                        }
                    }
                    let mut prepared_columns = Vec::with_capacity(target_relation.columns.len());
                    for (target_ordinal, target_column_id) in
                        target_column_ids.iter().copied().enumerate()
                    {
                        let column = by_target_column.get(&target_column_id).ok_or(
                            TransportError::MissingTargetMigrationColumn(
                                target_relation.id,
                                target_column_id,
                            ),
                        )?;
                        let mut input_fields = BTreeMap::new();
                        let mut source_columns = Vec::with_capacity(column.source_columns.len());
                        for source_column_id in &column.source_columns {
                            let source_ordinal = source
                                .schema
                                .relation_column_ordinal(rewrite.source_relation, *source_column_id)
                                .ok_or(TransportError::UnknownSourceMigrationColumn(
                                    rewrite.source_relation,
                                    *source_column_id,
                                ))?;
                            let source_ty = &source_relation.columns[source_ordinal];
                            input_fields.insert(*source_column_id, source_ty.clone());
                            source_columns.push((*source_column_id, source_ordinal));
                        }
                        let result = column
                            .transform
                            .typecheck(&kernel_schema::TypeExpr::Product(input_fields))
                            .map_err(TransportError::TransformType)?;
                        if result != target_relation.columns[target_ordinal] {
                            return Err(TransportError::TransformType(
                                QueryTypeError::TypeMismatch,
                            ));
                        }
                        prepared_columns.push(PreparedMigrationColumnRewrite {
                            target_column: target_column_id,
                            source_columns,
                            transform: column.transform.clone(),
                        });
                    }
                    prepared_relations.push(PreparedMigrationRelationRewrite::Rows {
                        source_relation: rewrite.source_relation,
                        target_relation: rewrite.target_relation,
                        columns: prepared_columns,
                    });
                }
            }
        }
        let known_target_relations: BTreeSet<_> = target
            .schema
            .relations()
            .map(|relation| relation.id)
            .collect();
        if let Some(unknown) = relations_by_target
            .keys()
            .find(|relation| !known_target_relations.contains(relation))
        {
            return Err(TransportError::UnknownTargetRelation(*unknown));
        }

        let mut bridge = SchemaBridge {
            transport: Self {
                source: source.clone(),
                target: target.clone(),
                field_rewrites: prepared_fields,
                field_passthrough,
                relation_rewrites: prepared_relations,
                relation_passthrough,
                access_policy_changes: Vec::new(),
            },
        };
        bridge.transport.access_policy_changes = bridge.schema_access_changes();
        Ok(bridge.transport)
    }

    #[must_use]
    pub fn access_policy_changes(&self) -> &[AccessPolicyChange] {
        &self.access_policy_changes
    }

    /// Proves the client-role observation factorization
    /// `View_target(role) ∘ M = F_role ∘ View_source(role)` at the exact
    /// permission-coordinate boundary. Every target observation without an
    /// exact transported source observation is returned as a declassification
    /// edge instead of being hidden inside a coarse policy-change category.
    #[must_use]
    pub fn certify_access_noninterference(&self) -> AccessNoninterferenceCertification {
        fn is_observation(permission: kernel_schema::PermissionCoordinate) -> bool {
            use kernel_schema::PermissionCoordinate as P;
            matches!(
                permission,
                P::ModelRead
                    | P::ReadRelation { .. }
                    | P::ReadField { .. }
                    | P::HistoricalRead
                    | P::HistoryRead
                    | P::Watch
            )
        }

        let source_access = self.source.schema.schema_access();
        let target_access = self.target.schema.schema_access();
        let mut flows = BTreeSet::new();
        let mut declassification_edges = BTreeSet::new();

        for role in target_access.roles.keys().copied() {
            let target_permissions = self
                .target
                .schema
                .resolve_access_roles([role])
                .unwrap_or_default();
            let source_permissions = if source_access.roles.contains_key(&role) {
                self.source
                    .schema
                    .resolve_access_roles([role])
                    .unwrap_or_default()
            } else {
                BTreeSet::new()
            };

            let mut transported = BTreeMap::new();
            for source_permission in source_permissions
                .into_iter()
                .filter(|p| is_observation(*p))
            {
                if let Ok(target_permission) = (SchemaBridge {
                    transport: self.clone(),
                })
                .transport_access_permission_exact(source_permission)
                {
                    transported
                        .entry(target_permission)
                        .or_insert(source_permission);
                }
            }

            for target_permission in target_permissions
                .into_iter()
                .filter(|p| is_observation(*p))
            {
                if let Some(source_permission) = transported.get(&target_permission).copied() {
                    flows.insert(AccessObservationFlow {
                        role,
                        source: source_permission,
                        target: target_permission,
                    });
                } else {
                    declassification_edges.insert(AccessDeclassificationEdge {
                        role,
                        target: target_permission,
                    });
                }
            }
        }

        AccessNoninterferenceCertification {
            flows: flows.into_iter().collect(),
            declassification_edges: declassification_edges.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn source_field_dependencies(&self) -> BTreeSet<kernel_types::SemanticId> {
        self.field_rewrites
            .iter()
            .flat_map(|rewrite| rewrite.source_fields.iter().copied())
            .collect()
    }

    #[must_use]
    pub fn source_relation_dependencies(&self) -> BTreeSet<kernel_types::SemanticId> {
        let mut out = BTreeSet::new();
        for rewrite in &self.relation_rewrites {
            match rewrite {
                PreparedMigrationRelationRewrite::Query {
                    source_relations, ..
                } => {
                    out.extend(source_relations.iter().copied());
                }
                PreparedMigrationRelationRewrite::Rows {
                    source_relation, ..
                } => {
                    out.insert(*source_relation);
                }
            }
        }
        out
    }

    /// Returns the exact source-field footprint used to produce one target
    /// field. Definitionally preserved fields depend on themselves; new
    /// constants have an empty footprint; split/merge rewrites return every
    /// declared source input.
    #[must_use]
    pub fn target_field_source_dependencies(
        &self,
        target_field: kernel_types::SemanticId,
    ) -> Option<BTreeSet<kernel_types::SemanticId>> {
        if self.field_passthrough.contains(&target_field) {
            return Some(BTreeSet::from([target_field]));
        }
        self.field_rewrites
            .iter()
            .find(|rewrite| rewrite.target_field == target_field)
            .map(|rewrite| rewrite.source_fields.iter().copied().collect())
    }

    /// Returns the independently materializable physical coordinate for one
    /// target relation.
    ///
    /// The result is derived entirely from the verified migration program and
    /// therefore survives recovery/replay without storing mutable progress in
    /// a second journal. `None` means the requested relation is not part of the
    /// target schema.
    #[must_use]
    pub fn relation_slice(
        &self,
        target_relation: kernel_types::SemanticId,
    ) -> Option<MigrationRelationSlice> {
        if self.relation_passthrough.contains(&target_relation) {
            return Some(MigrationRelationSlice::Passthrough {
                relation: target_relation,
            });
        }
        self.relation_rewrites
            .iter()
            .find_map(|rewrite| match rewrite {
                PreparedMigrationRelationRewrite::Query {
                    rewrite,
                    source_relations,
                } if rewrite.target_relation == target_relation => {
                    Some(MigrationRelationSlice::Query {
                        target_relation,
                        source_relations: source_relations.clone(),
                    })
                }
                PreparedMigrationRelationRewrite::Rows {
                    source_relation,
                    target_relation: rewrite_target,
                    ..
                } if *rewrite_target == target_relation => Some(MigrationRelationSlice::RowLocal {
                    source_relation: *source_relation,
                    target_relation,
                }),
                _ => None,
            })
    }

    /// Classifies the exact row-local observation transport for one source
    /// relation without evaluating or materializing relation rows.
    ///
    /// This separates the zero-row-touch identity theorem from pointwise
    /// value/shape rewrites.  The latter have an exact row transform and exact
    /// future delta transport, but do not by themselves prove that an existing
    /// maintained query state can change value domain without touching that
    /// state.
    pub fn classify_observation_relation_transport_exact(
        &self,
        source_relation: kernel_types::SemanticId,
    ) -> Result<ObservationRelationTransport, TransportError> {
        let source_def = self
            .source
            .schema
            .relation(source_relation)
            .ok_or(TransportError::UnknownSourceRelation(source_relation))?;
        if self.relation_passthrough.contains(&source_relation) {
            return Ok(ObservationRelationTransport::RowIdentity {
                target_relation: source_relation,
            });
        }

        let mut transport = None;
        for rewrite in &self.relation_rewrites {
            let PreparedMigrationRelationRewrite::Rows {
                source_relation: rewrite_source,
                target_relation,
                columns,
            } = rewrite
            else {
                continue;
            };
            if *rewrite_source != source_relation {
                continue;
            }
            if transport.is_some() {
                return Err(TransportError::AmbiguousObservationRelation(
                    source_relation,
                ));
            }
            let target_def = self
                .target
                .schema
                .relation(*target_relation)
                .ok_or(TransportError::UnknownTargetRelation(*target_relation))?;
            let row_identity = source_def.columns == target_def.columns
                && source_def.semantics == target_def.semantics
                && columns.len() == source_def.columns.len()
                && columns.iter().enumerate().all(|(ordinal, column)| {
                    let [(source_column, source_ordinal)] = column.source_columns.as_slice() else {
                        return false;
                    };
                    *source_ordinal == ordinal
                        && column.transform.root()
                            == &(kernel_query::Expr::ProductField {
                                input: Box::new(kernel_query::Expr::Input),
                                field: *source_column,
                            })
                });
            transport = Some(if row_identity {
                ObservationRelationTransport::RowIdentity {
                    target_relation: *target_relation,
                }
            } else {
                ObservationRelationTransport::RowLocalStateTransform {
                    target_relation: *target_relation,
                }
            });
        }
        transport.ok_or(TransportError::UnrepresentableObservationRelation(
            source_relation,
        ))
    }

    /// Returns the target relation for a source observation whose exact row
    /// representation survives this migration unchanged.
    ///
    /// This is deliberately narrower than general migration transport. A
    /// row-local rewrite is observation-identity only when the target has the
    /// same relational type/semantics and every target ordinal is the exact
    /// projection of the source column at the same ordinal. Column semantic
    /// IDs may change; row values and ordinal meaning may not. General query
    /// rewrites, permutations, conversions, split/merge and fan-out remain
    /// fail-closed so maintained query state never needs a result rebuild.
    pub fn transport_observation_relation_identity_exact(
        &self,
        source_relation: kernel_types::SemanticId,
    ) -> Result<kernel_types::SemanticId, TransportError> {
        match self.classify_observation_relation_transport_exact(source_relation)? {
            ObservationRelationTransport::RowIdentity { target_relation } => Ok(target_relation),
            ObservationRelationTransport::RowLocalStateTransform { .. } => Err(
                TransportError::UnrepresentableObservationRelation(source_relation),
            ),
        }
    }

    /// Transports passive field-observation dependencies through the verified
    /// migration provenance. A source field maps to every target field whose
    /// deterministic rewrite reads it; definitionally unchanged fields map to
    /// themselves. Dropped/unobservable dependencies fail closed.
    pub fn transport_field_dependencies_exact(
        &self,
        source_fields: &BTreeSet<kernel_types::SemanticId>,
    ) -> Result<BTreeSet<kernel_types::SemanticId>, TransportError> {
        let mut target_fields = BTreeSet::new();
        for source_field in source_fields {
            if self.source.schema.field(*source_field).is_none() {
                return Err(TransportError::UnknownSourceField(*source_field));
            }
            let mut represented = false;
            if self.field_passthrough.contains(source_field) {
                target_fields.insert(*source_field);
                represented = true;
            }
            for rewrite in &self.field_rewrites {
                if rewrite.source_fields.contains(source_field) {
                    target_fields.insert(rewrite.target_field);
                    represented = true;
                }
            }
            if !represented {
                return Err(TransportError::UnrepresentableSourceFieldDependency(
                    *source_field,
                ));
            }
        }
        Ok(target_fields)
    }

    /// Coordinate-preserving form of `transport_field_dependencies_exact`.
    /// Entity identity is semantic identity, so a field dependency fan-out
    /// keeps the same owner while only the verified field coordinate changes.
    pub fn transport_field_coordinate_dependencies_exact(
        &self,
        source_coordinates: &BTreeSet<(kernel_types::SemanticId, EntityId)>,
    ) -> Result<BTreeSet<(kernel_types::SemanticId, EntityId)>, TransportError> {
        let mut target_coordinates = BTreeSet::new();
        for (source_field, owner) in source_coordinates {
            for target_field in
                self.transport_field_dependencies_exact(&BTreeSet::from([*source_field]))?
            {
                target_coordinates.insert((target_field, *owner));
            }
        }
        Ok(target_coordinates)
    }

    /// Transports one exact relation delta without materializing either source
    /// or target relation. Passthrough relations remain identical; row-local
    /// rewrites transform only inserted/removed rows. Any general relational
    /// rewrite depending on the source relation fails closed because its output
    /// delta may depend on the wider source world.
    pub fn transport_relation_delta_exact(
        &self,
        source_relation: kernel_types::SemanticId,
        delta: &RelationDelta,
        registry: &SemanticRegistry,
    ) -> Result<Vec<(kernel_types::SemanticId, RelationDelta)>, TransportError> {
        let source_type = RelExpr::Scan(source_relation)
            .typecheck(&self.source, registry)
            .map_err(TransportError::RelationTransformType)?;
        if source_type != delta.result_type {
            return Err(TransportError::SourceRelationDeltaTypeMismatch(
                source_relation,
            ));
        }
        if self.relation_passthrough.contains(&source_relation) {
            return Ok(vec![(source_relation, delta.clone())]);
        }

        for rewrite in &self.relation_rewrites {
            if let PreparedMigrationRelationRewrite::Query {
                rewrite,
                source_relations,
            } = rewrite
                && source_relations.contains(&source_relation)
            {
                return Err(TransportError::MigrationSliceNotRowLocal(
                    rewrite.target_relation,
                ));
            }
        }

        let mut out = Vec::new();
        for rewrite in &self.relation_rewrites {
            let PreparedMigrationRelationRewrite::Rows {
                source_relation: rewrite_source,
                target_relation,
                columns,
            } = rewrite
            else {
                continue;
            };
            if *rewrite_source != source_relation {
                continue;
            }
            let transform_row =
                |source_row: &kernel_query::Row| -> Result<kernel_query::Row, TransportError> {
                    let mut target_row = Vec::with_capacity(columns.len());
                    for column in columns {
                        let mut input = BTreeMap::new();
                        for (source_column_id, source_ordinal) in &column.source_columns {
                            let value = source_row.get(*source_ordinal).ok_or(
                                TransportError::UnknownSourceMigrationColumn(
                                    source_relation,
                                    *source_column_id,
                                ),
                            )?;
                            input.insert(*source_column_id, value.clone());
                        }
                        target_row.push(
                            column
                                .transform
                                .evaluate(&Value::Product(input))
                                .map_err(TransportError::TransformExecution)?,
                        );
                    }
                    Ok(target_row)
                };
            let inserted = delta
                .inserted
                .iter()
                .map(transform_row)
                .collect::<Result<Vec<_>, _>>()?;
            let removed = delta
                .removed
                .iter()
                .map(transform_row)
                .collect::<Result<Vec<_>, _>>()?;
            let result_type = RelExpr::Scan(*target_relation)
                .typecheck(&self.target, registry)
                .map_err(TransportError::RelationTransformType)?;
            out.push((
                *target_relation,
                RelationDelta {
                    inserted,
                    removed,
                    result_type,
                },
            ));
        }
        if out.is_empty() {
            return Err(TransportError::UnrepresentableSourceRelationEffect(
                source_relation,
            ));
        }
        Ok(out)
    }

    /// Transports an exact source relation-column write footprint to the
    /// current target schema without transporting or widening any grants.
    ///
    /// A target column is required iff its verified row-local transform reads
    /// at least one touched source column. Passthrough is identity. Query/global
    /// rewrites fail closed because their write footprint is not row-local.
    pub fn transport_relation_write_footprint_exact(
        &self,
        source_relation: kernel_types::SemanticId,
        source_columns: &BTreeSet<kernel_types::SemanticId>,
    ) -> Result<Vec<MigrationRelationWriteFootprint>, TransportError> {
        let source_column_ids = self
            .source
            .schema
            .relation_column_ids(source_relation)
            .ok_or(TransportError::UnknownSourceRelation(source_relation))?;
        if let Some(unknown) = source_columns
            .iter()
            .find(|column| !source_column_ids.contains(column))
        {
            return Err(TransportError::UnknownSourceMigrationColumn(
                source_relation,
                *unknown,
            ));
        }
        if self.relation_passthrough.contains(&source_relation) {
            return Ok(vec![MigrationRelationWriteFootprint {
                target_relation: source_relation,
                target_columns: source_columns.clone(),
            }]);
        }

        for rewrite in &self.relation_rewrites {
            if let PreparedMigrationRelationRewrite::Query {
                rewrite,
                source_relations,
            } = rewrite
                && source_relations.contains(&source_relation)
            {
                return Err(TransportError::MigrationSliceNotRowLocal(
                    rewrite.target_relation,
                ));
            }
        }

        let mut out = Vec::new();
        for rewrite in &self.relation_rewrites {
            let PreparedMigrationRelationRewrite::Rows {
                source_relation: rewrite_source,
                target_relation,
                columns,
            } = rewrite
            else {
                continue;
            };
            if *rewrite_source != source_relation {
                continue;
            }
            let target_columns = columns
                .iter()
                .filter(|column| {
                    column
                        .source_columns
                        .iter()
                        .any(|(source_column, _)| source_columns.contains(source_column))
                })
                .map(|column| column.target_column)
                .collect::<BTreeSet<_>>();
            if !target_columns.is_empty() {
                out.push(MigrationRelationWriteFootprint {
                    target_relation: *target_relation,
                    target_columns,
                });
            }
        }
        if out.is_empty() {
            return Err(TransportError::UnrepresentableSourceRelationEffect(
                source_relation,
            ));
        }
        Ok(out)
    }

    /// Exact target relations that a row-local source relation effect may
    /// mutate. This is the relation-level counterpart of
    /// `transport_relation_write_footprint_exact`.
    pub fn transport_relation_write_targets_exact(
        &self,
        source_relation: kernel_types::SemanticId,
    ) -> Result<BTreeSet<kernel_types::SemanticId>, TransportError> {
        if self.source.schema.relation(source_relation).is_none() {
            return Err(TransportError::UnknownSourceRelation(source_relation));
        }
        if self.relation_passthrough.contains(&source_relation) {
            return Ok(BTreeSet::from([source_relation]));
        }
        let mut targets = BTreeSet::new();
        for rewrite in &self.relation_rewrites {
            match rewrite {
                PreparedMigrationRelationRewrite::Query {
                    rewrite,
                    source_relations,
                } if source_relations.contains(&source_relation) => {
                    return Err(TransportError::MigrationSliceNotRowLocal(
                        rewrite.target_relation,
                    ));
                }
                PreparedMigrationRelationRewrite::Rows {
                    source_relation: rewrite_source,
                    target_relation,
                    ..
                } if *rewrite_source == source_relation => {
                    targets.insert(*target_relation);
                }
                _ => {}
            }
        }
        if targets.is_empty() {
            return Err(TransportError::UnrepresentableSourceRelationEffect(
                source_relation,
            ));
        }
        Ok(targets)
    }

    /// Transports exact field assignments with bounded work over only affected
    /// rewrite inputs. Returned dependencies are untouched source fields whose
    /// values were read to evaluate a merge/rewrite and therefore must remain
    /// observation-stable until the migration boundary.
    pub fn transport_field_updates_exact(
        &self,
        source_state: &DatabaseState,
        updates: &[(kernel_types::SemanticId, EntityId, Option<Value>)],
    ) -> Result<FieldTransportChanges, TransportError> {
        self.transport_field_updates_from_root_exact(&source_state.model.fields, updates)
    }

    /// Root-preserving counterpart of `transport_field_updates_exact`. The
    /// immutable COW field map is sufficient proof material for field transport;
    /// callers do not need to resurrect a full historical `DatabaseState`.
    pub fn transport_field_updates_from_root_exact(
        &self,
        source_fields: &kernel_model::CowMap<(kernel_types::SemanticId, EntityId), Value>,
        updates: &[(kernel_types::SemanticId, EntityId, Option<Value>)],
    ) -> Result<FieldTransportChanges, TransportError> {
        let updates_by_key = updates
            .iter()
            .map(|(field, owner, value)| ((*field, *owner), value.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut target = BTreeMap::new();
        let mut dependencies = BTreeSet::new();

        for ((field, owner), value) in &updates_by_key {
            if self.field_passthrough.contains(field) {
                target.insert((*field, *owner), value.clone());
            }
        }

        for rewrite in &self.field_rewrites {
            let owners = updates_by_key
                .keys()
                .filter_map(|(field, owner)| {
                    rewrite.source_fields.contains(field).then_some(*owner)
                })
                .collect::<BTreeSet<_>>();
            for owner in owners {
                let mut input = BTreeMap::new();
                for source_field in &rewrite.source_fields {
                    let value = if let Some(value) = updates_by_key.get(&(*source_field, owner)) {
                        value
                            .clone()
                            .ok_or(TransportError::MissingMigrationSourceValue(*source_field))?
                    } else {
                        dependencies.insert((*source_field, owner));
                        source_fields
                            .get(&(*source_field, owner))
                            .cloned()
                            .ok_or(TransportError::MissingMigrationSourceValue(*source_field))?
                    };
                    input.insert(*source_field, value);
                }
                let value = rewrite
                    .transform
                    .evaluate(&Value::Product(input))
                    .map_err(TransportError::TransformExecution)?;
                target.insert((rewrite.target_field, owner), Some(value));
            }
        }

        for (field, owner, _) in updates {
            let represented = self.field_passthrough.contains(field)
                || self
                    .field_rewrites
                    .iter()
                    .any(|rewrite| rewrite.source_fields.contains(field));
            if !represented {
                return Err(TransportError::UnrepresentableSourceFieldDependency(*field));
            }
            let _ = owner;
        }

        Ok((
            target
                .into_iter()
                .map(|((field, owner), value)| (field, owner, value))
                .collect(),
            dependencies,
        ))
    }

    pub fn required_source_relations(
        &self,
        native_target_relations: &BTreeSet<kernel_types::SemanticId>,
    ) -> Result<BTreeSet<kernel_types::SemanticId>, TransportError> {
        let target_relations = self
            .target
            .schema
            .relations()
            .map(|relation| relation.id)
            .collect::<BTreeSet<_>>();
        if let Some(unknown) = native_target_relations
            .iter()
            .find(|relation| !target_relations.contains(relation))
        {
            return Err(TransportError::UnknownTargetRelation(*unknown));
        }

        let mut required = BTreeSet::new();
        for target_relation in target_relations {
            if native_target_relations.contains(&target_relation) {
                continue;
            }
            let slice = self
                .relation_slice(target_relation)
                .ok_or(TransportError::UnknownTargetRelation(target_relation))?;
            required.extend(slice.source_relations());
        }
        Ok(required)
    }

    /// Deterministically materializes exactly one target relation from the
    /// source-epoch logical state.
    ///
    /// This is deliberately narrower than `transport_revision`: it does not
    /// claim that the whole target world has been materialized or globally
    /// validated. It is the physical A→B slice primitive used by mixed-epoch
    /// storage/background rewrite. Full target validation remains at the
    /// semantic cutover boundary.
    pub fn materialize_relation_slice(
        &self,
        source_state: &DatabaseState,
        target_relation: kernel_types::SemanticId,
        registry: &SemanticRegistry,
    ) -> Result<Vec<kernel_query::Row>, TransportError> {
        if self.relation_passthrough.contains(&target_relation) {
            return Ok(source_state
                .model
                .relations
                .materialize_owned(&target_relation)
                .unwrap_or_default());
        }

        let rewrite = self
            .relation_rewrites
            .iter()
            .find(|rewrite| match rewrite {
                PreparedMigrationRelationRewrite::Query { rewrite, .. } => {
                    rewrite.target_relation == target_relation
                }
                PreparedMigrationRelationRewrite::Rows {
                    target_relation: rewrite_target,
                    ..
                } => *rewrite_target == target_relation,
            })
            .ok_or(TransportError::UnknownTargetRelation(target_relation))?;

        match rewrite {
            PreparedMigrationRelationRewrite::Query { rewrite, .. } => rewrite
                .transform
                .evaluate(&source_state.model, &self.source, registry)
                .map(|value| value.rows().to_vec())
                .map_err(TransportError::RelationTransformExecution),
            PreparedMigrationRelationRewrite::Rows {
                source_relation,
                target_relation: _,
                columns,
            } => {
                let source_rows = source_state
                    .model
                    .relations
                    .materialize_owned(source_relation)
                    .unwrap_or_default();
                let mut target_rows = Vec::with_capacity(source_rows.len());
                for source_row in source_rows {
                    let mut target_row = Vec::with_capacity(columns.len());
                    for column in columns {
                        let mut input = BTreeMap::new();
                        for (source_column_id, source_ordinal) in &column.source_columns {
                            let value = source_row.get(*source_ordinal).ok_or(
                                TransportError::UnknownSourceMigrationColumn(
                                    *source_relation,
                                    *source_column_id,
                                ),
                            )?;
                            input.insert(*source_column_id, value.clone());
                        }
                        target_row.push(
                            column
                                .transform
                                .evaluate(&kernel_model::Value::Product(input))
                                .map_err(TransportError::TransformExecution)?,
                        );
                    }
                    target_rows.push(target_row);
                }
                Ok(target_rows)
            }
        }
    }

    /// Transforms one physical source row for a row-local migration slice.
    ///
    /// This is the bounded-memory primitive for background physical rewrite:
    /// callers may scan/chunk source storage and publish target-native rows
    /// without ever constructing the whole relation in memory. General query
    /// slices are rejected explicitly because their semantics may depend on
    /// multiple relations or global relational operators.
    pub fn transform_row_local_slice(
        &self,
        target_relation: kernel_types::SemanticId,
        source_row: &kernel_query::Row,
    ) -> Result<kernel_query::Row, TransportError> {
        let rewrite = self
            .relation_rewrites
            .iter()
            .find(|rewrite| match rewrite {
                PreparedMigrationRelationRewrite::Rows {
                    target_relation: rewrite_target,
                    ..
                } => *rewrite_target == target_relation,
                PreparedMigrationRelationRewrite::Query { rewrite, .. } => {
                    rewrite.target_relation == target_relation
                }
            })
            .ok_or(TransportError::UnknownTargetRelation(target_relation))?;

        let PreparedMigrationRelationRewrite::Rows {
            source_relation,
            columns,
            ..
        } = rewrite
        else {
            return Err(TransportError::MigrationSliceNotRowLocal(target_relation));
        };

        let mut target_row = Vec::with_capacity(columns.len());
        for column in columns {
            let mut input = BTreeMap::new();
            for (source_column_id, source_ordinal) in &column.source_columns {
                let value = source_row.get(*source_ordinal).ok_or(
                    TransportError::UnknownSourceMigrationColumn(
                        *source_relation,
                        *source_column_id,
                    ),
                )?;
                input.insert(*source_column_id, value.clone());
            }
            target_row.push(
                column
                    .transform
                    .evaluate(&kernel_model::Value::Product(input))
                    .map_err(TransportError::TransformExecution)?,
            );
        }
        Ok(target_row)
    }

    fn transport_state(
        &self,
        source_state: &DatabaseState,
        registry: &SemanticRegistry,
    ) -> Result<DatabaseState, TransportError> {
        let mut fields = BTreeMap::new();
        for (&(field, entity), value) in &source_state.model.fields {
            if self.field_passthrough.contains(&field) {
                fields.insert((field, entity), value.clone());
            }
        }
        for rewrite in &self.field_rewrites {
            let entities = source_state
                .model
                .carriers
                .get(&rewrite.owner)
                .cloned()
                .unwrap_or_default();
            for entity in entities {
                let mut input_fields = BTreeMap::new();
                for source_field in &rewrite.source_fields {
                    let value = source_state
                        .model
                        .fields
                        .get(&(*source_field, entity))
                        .ok_or(TransportError::MissingMigrationSourceValue(*source_field))?;
                    input_fields.insert(*source_field, value.clone());
                }
                let transformed = rewrite
                    .transform
                    .evaluate(&kernel_model::Value::Product(input_fields))
                    .map_err(TransportError::TransformExecution)?;
                fields.insert((rewrite.target_field, entity), transformed);
            }
        }

        let mut relations = kernel_model::RelationStore::default();
        for relation in &self.relation_passthrough {
            if let Some(rows) = source_state.model.relations.get(relation) {
                relations.insert_shared(*relation, rows.clone());
            }
        }
        for rewrite in &self.relation_rewrites {
            match rewrite {
                PreparedMigrationRelationRewrite::Query { rewrite, .. } => {
                    let result = rewrite
                        .transform
                        .evaluate(&source_state.model, &self.source, registry)
                        .map_err(TransportError::RelationTransformExecution)?;
                    relations.insert(rewrite.target_relation, result.rows().to_vec());
                }
                PreparedMigrationRelationRewrite::Rows {
                    source_relation,
                    target_relation,
                    columns,
                } => {
                    let source_rows = source_state
                        .model
                        .relations
                        .materialize_owned(source_relation)
                        .unwrap_or_default();
                    let mut target_rows = Vec::with_capacity(source_rows.len());
                    for source_row in source_rows {
                        let mut target_row = Vec::with_capacity(columns.len());
                        for column in columns {
                            let mut input = BTreeMap::new();
                            for (source_column_id, source_ordinal) in &column.source_columns {
                                let value = source_row.get(*source_ordinal).ok_or(
                                    TransportError::UnknownSourceMigrationColumn(
                                        *source_relation,
                                        *source_column_id,
                                    ),
                                )?;
                                input.insert(*source_column_id, value.clone());
                            }
                            target_row.push(
                                column
                                    .transform
                                    .evaluate(&kernel_model::Value::Product(input))
                                    .map_err(TransportError::TransformExecution)?,
                            );
                        }
                        target_rows.push(target_row);
                    }
                    relations.insert(*target_relation, target_rows);
                }
            }
        }

        let state = DatabaseState {
            model: FiniteModel {
                carriers: source_state.model.carriers.clone(),
                fields: fields.into(),
                relations,
            },
            lifecycle: source_state.lifecycle.clone(),
        };
        kernel_validation::validate_state(&self.target, registry, &state)
            .map_err(TransportError::InvalidTarget)?;
        Ok(state)
    }

    pub fn transport_revision(
        &self,
        source: &kernel_revision::Revision,
        target_id: kernel_types::RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<kernel_revision::Revision, TransportError> {
        if source.semantic_context() != &self.source {
            return Err(TransportError::SourceRevisionMismatch);
        }
        let state = self.transport_state(source.state(), registry)?;
        kernel_revision::Revision::build(target_id, &self.target, registry, state)
            .map_err(TransportError::InvalidRevision)
    }

    #[must_use]
    pub fn source(&self) -> &SemanticContext {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &SemanticContext {
        &self.target
    }
}
