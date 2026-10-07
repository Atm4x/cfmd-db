use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AccessCapabilityId, FieldId, PermissionCoordinate, Query, RelationColumnId, RelationId, RoleId,
    Schema, Type, TypeId, Value, VariantTagId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationWorkflowStage {
    Plan,
    Validate,
    Preview,
    Execute,
    Observe,
    CutOver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationCostClass {
    MetadataOnly,
    RowLocalData,
    GlobalData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MigrationAccessSubject {
    Capability(AccessCapabilityId),
    Role(RoleId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationAccessChangeKind {
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
pub struct MigrationAccessChange {
    subject: MigrationAccessSubject,
    kind: MigrationAccessChangeKind,
    affected_roles: Vec<RoleId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MigrationSecurityImpactDigest([u8; 32]);

impl MigrationSecurityImpactDigest {
    pub(crate) const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MigrationDataDependency {
    Field(FieldId),
    Relation(RelationId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MigrationObservationFlow {
    role: RoleId,
    source: PermissionCoordinate,
    target: PermissionCoordinate,
}

impl MigrationObservationFlow {
    #[must_use]
    pub const fn role(&self) -> RoleId {
        self.role
    }
    #[must_use]
    pub const fn source(&self) -> PermissionCoordinate {
        self.source
    }
    #[must_use]
    pub const fn target(&self) -> PermissionCoordinate {
        self.target
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MigrationDeclassificationEdge {
    role: RoleId,
    target: PermissionCoordinate,
}

impl MigrationDeclassificationEdge {
    #[must_use]
    pub const fn role(&self) -> RoleId {
        self.role
    }
    #[must_use]
    pub const fn target(&self) -> PermissionCoordinate {
        self.target
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MigrationIntegrityPolicyCoordinate {
    Field(FieldId),
    Entity(TypeId),
    RelationColumn {
        relation: RelationId,
        column: RelationColumnId,
    },
    Model,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationSecurityImpact {
    source_revision: crate::RevisionId,
    migration_id: u128,
    digest: MigrationSecurityImpactDigest,
    access_policy_changes: Vec<MigrationAccessChange>,
    observation_flows: Vec<MigrationObservationFlow>,
    declassification_edges: Vec<MigrationDeclassificationEdge>,
    source_data_dependencies: Vec<MigrationDataDependency>,
    data_dependent_validation_dependencies: Vec<MigrationDataDependency>,
    integrity_policy_changes: Vec<MigrationIntegrityPolicyCoordinate>,
}

impl MigrationSecurityImpact {
    #[must_use]
    pub fn requires_security_approval(&self) -> bool {
        !self.access_policy_changes.is_empty() || !self.declassification_edges.is_empty()
    }
}

pub(crate) struct MigrationSecurityImpactParts {
    pub(crate) source_data_dependencies: Vec<MigrationDataDependency>,
    pub(crate) data_dependent_validation_dependencies: Vec<MigrationDataDependency>,
    pub(crate) integrity_policy_changes: Vec<MigrationIntegrityPolicyCoordinate>,
}

impl MigrationSecurityImpact {
    pub(crate) fn from_kernel(
        source_revision: crate::RevisionId,
        migration_id: u128,
        digest: MigrationSecurityImpactDigest,
        access_policy_changes: &[kernel_transport::AccessPolicyChange],
        certification: &kernel_transport::AccessNoninterferenceCertification,
        parts: MigrationSecurityImpactParts,
    ) -> Self {
        Self {
            source_revision,
            migration_id,
            digest,
            access_policy_changes: access_policy_changes
                .iter()
                .map(MigrationAccessChange::from_kernel)
                .collect(),
            observation_flows: certification
                .flows()
                .iter()
                .map(|flow| MigrationObservationFlow {
                    role: RoleId::new(flow.role().raw()),
                    source: crate::security::permission_coordinate_from_kernel(flow.source()),
                    target: crate::security::permission_coordinate_from_kernel(flow.target()),
                })
                .collect(),
            declassification_edges: certification
                .declassification_edges()
                .iter()
                .map(|edge| MigrationDeclassificationEdge {
                    role: RoleId::new(edge.role().raw()),
                    target: crate::security::permission_coordinate_from_kernel(edge.target()),
                })
                .collect(),
            source_data_dependencies: parts.source_data_dependencies,
            data_dependent_validation_dependencies: parts.data_dependent_validation_dependencies,
            integrity_policy_changes: parts.integrity_policy_changes,
        }
    }

    #[must_use]
    pub const fn source_revision(&self) -> crate::RevisionId {
        self.source_revision
    }
    #[must_use]
    pub const fn migration_id(&self) -> u128 {
        self.migration_id
    }
    #[must_use]
    pub const fn digest(&self) -> MigrationSecurityImpactDigest {
        self.digest
    }
    #[must_use]
    pub fn access_policy_changes(&self) -> &[MigrationAccessChange] {
        &self.access_policy_changes
    }
    #[must_use]
    pub fn observation_flows(&self) -> &[MigrationObservationFlow] {
        &self.observation_flows
    }
    #[must_use]
    pub fn declassification_edges(&self) -> &[MigrationDeclassificationEdge] {
        &self.declassification_edges
    }
    #[must_use]
    pub fn source_data_dependencies(&self) -> &[MigrationDataDependency] {
        &self.source_data_dependencies
    }
    #[must_use]
    pub fn data_dependent_validation_dependencies(&self) -> &[MigrationDataDependency] {
        &self.data_dependent_validation_dependencies
    }
    #[must_use]
    pub fn integrity_policy_changes(&self) -> &[MigrationIntegrityPolicyCoordinate] {
        &self.integrity_policy_changes
    }
    #[must_use]
    pub fn is_noninterfering(&self) -> bool {
        self.declassification_edges.is_empty()
    }
    #[must_use]
    pub fn requires_data_inspection(&self) -> bool {
        !self.source_data_dependencies.is_empty()
    }
}

impl MigrationAccessChange {
    #[must_use]
    pub const fn subject(&self) -> MigrationAccessSubject {
        self.subject
    }

    #[must_use]
    pub const fn kind(&self) -> MigrationAccessChangeKind {
        self.kind
    }

    #[must_use]
    pub fn affected_roles(&self) -> &[RoleId] {
        &self.affected_roles
    }

    fn from_kernel(change: &kernel_transport::AccessPolicyChange) -> Self {
        let subject = match change.subject() {
            kernel_transport::AccessPolicySubject::Capability(id) => {
                MigrationAccessSubject::Capability(AccessCapabilityId::new(id.raw()))
            }
            kernel_transport::AccessPolicySubject::Role(id) => {
                MigrationAccessSubject::Role(RoleId::new(id.raw()))
            }
        };
        let kind = match change.kind() {
            kernel_transport::AccessPolicyChangeKind::CapabilityAdded => Self::capability_added(),
            kernel_transport::AccessPolicyChangeKind::CapabilityRemoved => {
                MigrationAccessChangeKind::CapabilityRemoved
            }
            kernel_transport::AccessPolicyChangeKind::CapabilityWidened => {
                MigrationAccessChangeKind::CapabilityWidened
            }
            kernel_transport::AccessPolicyChangeKind::CapabilityNarrowed => {
                MigrationAccessChangeKind::CapabilityNarrowed
            }
            kernel_transport::AccessPolicyChangeKind::CapabilityAuthorityShapeChanged => {
                MigrationAccessChangeKind::CapabilityAuthorityShapeChanged
            }
            kernel_transport::AccessPolicyChangeKind::RoleAdded => {
                MigrationAccessChangeKind::RoleAdded
            }
            kernel_transport::AccessPolicyChangeKind::RoleRemoved => {
                MigrationAccessChangeKind::RoleRemoved
            }
            kernel_transport::AccessPolicyChangeKind::RoleWidened => {
                MigrationAccessChangeKind::RoleWidened
            }
            kernel_transport::AccessPolicyChangeKind::RoleNarrowed => {
                MigrationAccessChangeKind::RoleNarrowed
            }
            kernel_transport::AccessPolicyChangeKind::RoleCompositionChanged => {
                MigrationAccessChangeKind::RoleCompositionChanged
            }
        };
        Self {
            subject,
            kind,
            affected_roles: change
                .affected_roles()
                .iter()
                .map(|id| RoleId::new(id.raw()))
                .collect(),
        }
    }

    const fn capability_added() -> MigrationAccessChangeKind {
        MigrationAccessChangeKind::CapabilityAdded
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// Semantic subsystem that owns a migration/bridge diagnostic. This is intentionally frontend
/// neutral: Rust, Python, remote and CLI surfaces should project the same runtime classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationDiagnosticDomain {
    Planning,
    DataTransport,
    ReadBridge,
    WriteBridge,
    Lifecycle,
    Access,
    Preparation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationDiagnosticReason {
    Exact,
    RowLocalData,
    GlobalData,
    UnknownSourceCoordinate,
    UnknownTargetCoordinate,
    DuplicateTargetCoordinate,
    StructuralChangeUnsupported,
    DefinitionallyChanged,
    SemanticEnvironmentChanged,
    TransformTypeMismatch,
    TransformExecutionFailed,
    TargetStateRejected,
    ReadNotRepresentable,
    WriteNotRepresentable,
    AliasedTarget,
    ResultTypeChanged,
    OwnershipContractChanged,
    SourceRevisionMismatch,
    AccessPolicyChangeDetected,
    PreparationRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationCoordinate {
    Field(FieldId),
    Relation(RelationId),
    RelationColumn {
        relation: RelationId,
        column: RelationColumnId,
    },
    Semantic(u128),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationDiagnosticCode {
    ExactTransport,
    RowLocalDataTransform,
    GlobalRelationRewrite,
    HistoryAuthorityForgotten,
    ModelNotRepresentable,
    TargetStateRejected,
    ReadNotRepresentable,
    WriteNotRepresentable,
    LifecycleNotRepresentable,
    AccessPolicyChange,
    AwaitingPublication,
    PreparedArtifactStale,
    SemanticCutoverPublished,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationDiagnostic {
    stage: MigrationWorkflowStage,
    severity: MigrationDiagnosticSeverity,
    code: MigrationDiagnosticCode,
    domain: MigrationDiagnosticDomain,
    reason: MigrationDiagnosticReason,
    coordinates: Vec<MigrationCoordinate>,
    message: String,
}

impl MigrationDiagnostic {
    #[must_use]
    pub const fn stage(&self) -> MigrationWorkflowStage {
        self.stage
    }

    #[must_use]
    pub const fn severity(&self) -> MigrationDiagnosticSeverity {
        self.severity
    }

    #[must_use]
    pub const fn code(&self) -> MigrationDiagnosticCode {
        self.code
    }

    #[must_use]
    pub const fn domain(&self) -> MigrationDiagnosticDomain {
        self.domain
    }

    #[must_use]
    pub const fn reason(&self) -> MigrationDiagnosticReason {
        self.reason
    }

    #[must_use]
    pub fn coordinates(&self) -> &[MigrationCoordinate] {
        &self.coordinates
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn failure(
        stage: MigrationWorkflowStage,
        domain: MigrationDiagnosticDomain,
        reason: MigrationDiagnosticReason,
        coordinates: Vec<MigrationCoordinate>,
        message: impl Into<String>,
    ) -> Self {
        let code = match domain {
            MigrationDiagnosticDomain::ReadBridge => MigrationDiagnosticCode::ReadNotRepresentable,
            MigrationDiagnosticDomain::WriteBridge => {
                MigrationDiagnosticCode::WriteNotRepresentable
            }
            MigrationDiagnosticDomain::Lifecycle => {
                MigrationDiagnosticCode::LifecycleNotRepresentable
            }
            MigrationDiagnosticDomain::Access => MigrationDiagnosticCode::AccessPolicyChange,
            _ if matches!(reason, MigrationDiagnosticReason::TargetStateRejected) => {
                MigrationDiagnosticCode::TargetStateRejected
            }
            _ => MigrationDiagnosticCode::ModelNotRepresentable,
        };
        Self {
            stage,
            severity: MigrationDiagnosticSeverity::Error,
            code,
            domain,
            reason,
            coordinates,
            message: message.into(),
        }
    }

    pub(crate) fn transport_failure(
        stage: MigrationWorkflowStage,
        domain: MigrationDiagnosticDomain,
        error: &kernel_transport::TransportError,
    ) -> Self {
        let (reason, coordinates) = classify_transport_failure(error);
        Self::failure(stage, domain, reason, coordinates, format!("{error:?}"))
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete kernel transport diagnostic classification exhaustive in one place."
)]
fn classify_transport_failure(
    error: &kernel_transport::TransportError,
) -> (MigrationDiagnosticReason, Vec<MigrationCoordinate>) {
    use kernel_transport::TransportError as T;

    match error {
        T::UnknownSourceField(id)
        | T::MissingMigrationSourceValue(id)
        | T::UnrepresentableSourceFieldDependency(id) => (
            MigrationDiagnosticReason::UnknownSourceCoordinate,
            vec![migration_field_coordinate(*id)],
        ),
        T::UnknownTargetField(id) => (
            MigrationDiagnosticReason::UnknownTargetCoordinate,
            vec![migration_field_coordinate(*id)],
        ),
        T::DuplicateTargetField(id) | T::RewriteTargetsPassthroughField(id) => (
            MigrationDiagnosticReason::DuplicateTargetCoordinate,
            vec![migration_field_coordinate(*id)],
        ),
        T::UnknownSourceRelation(id) | T::UnrepresentableSourceRelationEffect(id) => (
            MigrationDiagnosticReason::UnknownSourceCoordinate,
            vec![migration_relation_coordinate(*id)],
        ),
        T::UnknownTargetRelation(id) => (
            MigrationDiagnosticReason::UnknownTargetCoordinate,
            vec![migration_relation_coordinate(*id)],
        ),
        T::DuplicateTargetRelation(id) | T::RewriteTargetsPassthroughRelation(id) => (
            MigrationDiagnosticReason::DuplicateTargetCoordinate,
            vec![migration_relation_coordinate(*id)],
        ),
        T::DuplicateTargetMigrationColumn(relation_id, column_id)
        | T::MissingTargetMigrationColumn(relation_id, column_id) => (
            MigrationDiagnosticReason::DuplicateTargetCoordinate,
            vec![migration_relation_column_coordinate(
                *relation_id,
                *column_id,
            )],
        ),
        T::UnknownSourceMigrationColumn(relation_id, column_id) => (
            MigrationDiagnosticReason::UnknownSourceCoordinate,
            vec![migration_relation_column_coordinate(
                *relation_id,
                *column_id,
            )],
        ),
        T::UnrepresentableReadRelation(id)
        | T::UnrepresentableObservationRelation(id)
        | T::AmbiguousObservationRelation(id) => (
            MigrationDiagnosticReason::ReadNotRepresentable,
            vec![migration_relation_coordinate(*id)],
        ),
        T::AliasedReadTarget(id) => (
            MigrationDiagnosticReason::AliasedTarget,
            vec![migration_relation_coordinate(*id)],
        ),
        T::ReadResultTypeMismatch => (MigrationDiagnosticReason::ResultTypeChanged, vec![]),
        T::UnrepresentableOwnedRelationship(id) => (
            MigrationDiagnosticReason::OwnershipContractChanged,
            vec![migration_relation_coordinate(*id)],
        ),
        T::SourceRelationDeltaTypeMismatch(id) | T::MigrationSliceNotRowLocal(id) => (
            MigrationDiagnosticReason::WriteNotRepresentable,
            vec![migration_relation_coordinate(*id)],
        ),
        T::OwnerTypeMismatch(id)
        | T::MigrationFieldOwnerMismatch(id)
        | T::SemanticContractChanged(id) => (
            MigrationDiagnosticReason::DefinitionallyChanged,
            vec![MigrationCoordinate::Semantic(id.raw())],
        ),
        T::SourceRevisionMismatch => (MigrationDiagnosticReason::SourceRevisionMismatch, vec![]),
        T::SemanticEnvironmentChangeRequiresTransport => (
            MigrationDiagnosticReason::SemanticEnvironmentChanged,
            vec![],
        ),
        T::UnsupportedStructuralChange
        | T::NotConservativeSemanticExtension
        | T::IdentitySourceCoverageMismatch
        | T::SourceSemantics(_)
        | T::TargetSemantics(_)
        | T::DuplicateMigrationSourceField(_) => (
            MigrationDiagnosticReason::StructuralChangeUnsupported,
            vec![],
        ),
        T::NotDefinitionallyEquivalent | T::NoSemanticLawChange => {
            (MigrationDiagnosticReason::DefinitionallyChanged, vec![])
        }
        T::TransformType(_) | T::RelationTransformType(_) => {
            (MigrationDiagnosticReason::TransformTypeMismatch, vec![])
        }
        T::TransformExecution(_) | T::RelationTransformExecution(_) => {
            (MigrationDiagnosticReason::TransformExecutionFailed, vec![])
        }
        T::InvalidTarget(_) | T::InvalidRevision(_) => {
            (MigrationDiagnosticReason::TargetStateRejected, vec![])
        }
    }
}

fn migration_field_coordinate(id: kernel_types::SemanticId) -> MigrationCoordinate {
    MigrationCoordinate::Field(FieldId::new(id.raw()))
}

fn migration_relation_coordinate(id: kernel_types::SemanticId) -> MigrationCoordinate {
    MigrationCoordinate::Relation(RelationId::new(id.raw()))
}

fn migration_relation_column_coordinate(
    relation: kernel_types::SemanticId,
    column: kernel_types::SemanticId,
) -> MigrationCoordinate {
    MigrationCoordinate::RelationColumn {
        relation: RelationId::new(relation.raw()),
        column: RelationColumnId::new(column.raw()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    source_revision: crate::RevisionId,
    source_schema_revision: u64,
    target_schema_revision: u64,
    field_rewrites: usize,
    row_local_relation_rewrites: usize,
    global_relation_rewrites: usize,
    cost_class: MigrationCostClass,
    diagnostics: Vec<MigrationDiagnostic>,
}

impl MigrationPlan {
    pub(crate) fn from_model(
        source_revision: crate::RevisionId,
        source_schema_revision: u64,
        model: &MigrationModel,
    ) -> Self {
        let row_local_relation_rewrites = model
            .relations
            .iter()
            .filter(|rule| matches!(rule, MigrationRelationRule::Rows { .. }))
            .count();
        let global_relation_rewrites = model
            .relations
            .iter()
            .filter(|rule| matches!(rule, MigrationRelationRule::Query { .. }))
            .count();
        let cost_class = if global_relation_rewrites > 0 {
            MigrationCostClass::GlobalData
        } else if !model.fields.is_empty() || row_local_relation_rewrites > 0 {
            MigrationCostClass::RowLocalData
        } else {
            MigrationCostClass::MetadataOnly
        };
        let mut diagnostics = Vec::new();
        match cost_class {
            MigrationCostClass::MetadataOnly => diagnostics.push(MigrationDiagnostic {
                stage: MigrationWorkflowStage::Plan,
                severity: MigrationDiagnosticSeverity::Info,
                code: MigrationDiagnosticCode::ExactTransport,
                domain: MigrationDiagnosticDomain::Planning,
                reason: MigrationDiagnosticReason::Exact,
                coordinates: Vec::new(),
                message: "migration has no declared data rewrite".to_owned(),
            }),
            MigrationCostClass::RowLocalData => diagnostics.push(MigrationDiagnostic {
                stage: MigrationWorkflowStage::Plan,
                severity: MigrationDiagnosticSeverity::Info,
                code: MigrationDiagnosticCode::RowLocalDataTransform,
                domain: MigrationDiagnosticDomain::Planning,
                reason: MigrationDiagnosticReason::RowLocalData,
                coordinates: Vec::new(),
                message: "migration contains only deterministic row-local data rewrites".to_owned(),
            }),
            MigrationCostClass::GlobalData => diagnostics.push(MigrationDiagnostic {
                stage: MigrationWorkflowStage::Plan,
                severity: MigrationDiagnosticSeverity::Warning,
                code: MigrationDiagnosticCode::GlobalRelationRewrite,
                domain: MigrationDiagnosticDomain::Planning,
                reason: MigrationDiagnosticReason::GlobalData,
                coordinates: Vec::new(),
                message:
                    "migration contains at least one relation-query rewrite with global data cost"
                        .to_owned(),
            }),
        }
        Self {
            source_revision,
            source_schema_revision,
            target_schema_revision: model.target.revision,
            field_rewrites: model.fields.len(),
            row_local_relation_rewrites,
            global_relation_rewrites,
            cost_class,
            diagnostics,
        }
    }

    #[must_use]
    pub const fn source_revision(&self) -> crate::RevisionId {
        self.source_revision
    }

    #[must_use]
    pub const fn source_schema_revision(&self) -> u64 {
        self.source_schema_revision
    }

    #[must_use]
    pub const fn target_schema_revision(&self) -> u64 {
        self.target_schema_revision
    }

    #[must_use]
    pub const fn field_rewrites(&self) -> usize {
        self.field_rewrites
    }

    #[must_use]
    pub const fn row_local_relation_rewrites(&self) -> usize {
        self.row_local_relation_rewrites
    }

    #[must_use]
    pub const fn global_relation_rewrites(&self) -> usize {
        self.global_relation_rewrites
    }

    #[must_use]
    pub const fn cost_class(&self) -> MigrationCostClass {
        self.cost_class
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[MigrationDiagnostic] {
        &self.diagnostics
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationValidation {
    plan: MigrationPlan,
    diagnostics: Vec<MigrationDiagnostic>,
    security_impact: MigrationSecurityImpact,
}

impl MigrationValidation {
    pub(crate) fn verified(plan: MigrationPlan, security_impact: MigrationSecurityImpact) -> Self {
        let mut diagnostics = vec![MigrationDiagnostic {
            stage: MigrationWorkflowStage::Validate,
            severity: MigrationDiagnosticSeverity::Info,
            code: MigrationDiagnosticCode::ExactTransport,
            domain: MigrationDiagnosticDomain::DataTransport,
            reason: MigrationDiagnosticReason::Exact,
            coordinates: Vec::new(),
            message: "migration program is exactly representable from the current schema"
                .to_owned(),
        }];
        diagnostics.extend(security_impact.access_policy_changes.iter().map(|change| {
            MigrationDiagnostic {
                stage: MigrationWorkflowStage::Validate,
                severity: MigrationDiagnosticSeverity::Warning,
                code: MigrationDiagnosticCode::AccessPolicyChange,
                domain: MigrationDiagnosticDomain::Access,
                reason: MigrationDiagnosticReason::AccessPolicyChangeDetected,
                coordinates: vec![match change.subject {
                    MigrationAccessSubject::Capability(id) => {
                        MigrationCoordinate::Semantic(id.raw())
                    }
                    MigrationAccessSubject::Role(id) => MigrationCoordinate::Semantic(id.raw()),
                }],
                message: format!(
                    "detected schema access change: {:?}; affected roles: {}",
                    change.kind,
                    change.affected_roles.len()
                ),
            }
        }));
        Self {
            plan,
            diagnostics,
            security_impact,
        }
    }

    #[must_use]
    pub const fn plan(&self) -> &MigrationPlan {
        &self.plan
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[MigrationDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn access_changes(&self) -> &[MigrationAccessChange] {
        self.security_impact.access_policy_changes()
    }

    #[must_use]
    pub const fn security_impact(&self) -> &MigrationSecurityImpact {
        &self.security_impact
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPreview {
    validation: MigrationValidation,
    target_revision: crate::RevisionId,
}

/// One migration workflow artifact prepared against an exact immutable source revision.
///
/// Kernel transport/program details stay private. Frontends inspect the stable plan/validation/
/// preview surfaces while execution reuses this exact prepared meaning and fails stale if HEAD
/// moved before publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationObservationState {
    /// The exact prepared source revision is still current and publication may proceed.
    Prepared,
    /// HEAD advanced without publishing this exact migration artifact.
    StaleUnpublished,
    /// The exact migration effect is present in authoritative causal history.
    CutOver,
}

/// Durable proof that one prepared migration crossed its semantic publication boundary.
///
/// This is a projection of authoritative history/current HEAD, not mutable migration state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationCutover {
    effect_id: u128,
    published_revision: crate::RevisionId,
    current_revision: crate::RevisionId,
    current_schema_revision: u64,
    target_schema_revision: u64,
}

impl MigrationCutover {
    pub(crate) const fn new(
        effect_id: u128,
        published_revision: crate::RevisionId,
        current_revision: crate::RevisionId,
        current_schema_revision: u64,
        target_schema_revision: u64,
    ) -> Self {
        Self {
            effect_id,
            published_revision,
            current_revision,
            current_schema_revision,
            target_schema_revision,
        }
    }

    #[must_use]
    pub const fn effect_id(&self) -> u128 {
        self.effect_id
    }

    #[must_use]
    pub const fn published_revision(&self) -> crate::RevisionId {
        self.published_revision
    }

    #[must_use]
    pub const fn current_revision(&self) -> crate::RevisionId {
        self.current_revision
    }

    #[must_use]
    pub const fn current_schema_revision(&self) -> u64 {
        self.current_schema_revision
    }

    #[must_use]
    pub const fn target_schema_revision(&self) -> u64 {
        self.target_schema_revision
    }

    #[must_use]
    pub const fn target_schema_is_current(&self) -> bool {
        self.current_schema_revision == self.target_schema_revision
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationObservation {
    state: MigrationObservationState,
    current_revision: crate::RevisionId,
    current_schema_revision: u64,
    cutover: Option<MigrationCutover>,
    diagnostics: Vec<MigrationDiagnostic>,
}

impl MigrationObservation {
    pub(crate) fn prepared(
        current_revision: crate::RevisionId,
        current_schema_revision: u64,
    ) -> Self {
        Self {
            state: MigrationObservationState::Prepared,
            current_revision,
            current_schema_revision,
            cutover: None,
            diagnostics: vec![MigrationDiagnostic {
                stage: MigrationWorkflowStage::Observe,
                severity: MigrationDiagnosticSeverity::Info,
                code: MigrationDiagnosticCode::AwaitingPublication,
                domain: MigrationDiagnosticDomain::Planning,
                reason: MigrationDiagnosticReason::Exact,
                coordinates: Vec::new(),
                message: "prepared migration source revision is still current; no semantic cutover has been published".to_owned(),
            }],
        }
    }

    pub(crate) fn stale(current_revision: crate::RevisionId, current_schema_revision: u64) -> Self {
        Self {
            state: MigrationObservationState::StaleUnpublished,
            current_revision,
            current_schema_revision,
            cutover: None,
            diagnostics: vec![MigrationDiagnostic {
                stage: MigrationWorkflowStage::Observe,
                severity: MigrationDiagnosticSeverity::Error,
                code: MigrationDiagnosticCode::PreparedArtifactStale,
                domain: MigrationDiagnosticDomain::Planning,
                reason: MigrationDiagnosticReason::SourceRevisionMismatch,
                coordinates: Vec::new(),
                message: "HEAD advanced without publishing this exact prepared migration"
                    .to_owned(),
            }],
        }
    }

    pub(crate) fn cut_over(cutover: MigrationCutover) -> Self {
        Self {
            state: MigrationObservationState::CutOver,
            current_revision: cutover.current_revision,
            current_schema_revision: cutover.current_schema_revision,
            cutover: Some(cutover),
            diagnostics: vec![MigrationDiagnostic {
                stage: MigrationWorkflowStage::CutOver,
                severity: MigrationDiagnosticSeverity::Info,
                code: MigrationDiagnosticCode::SemanticCutoverPublished,
                domain: MigrationDiagnosticDomain::DataTransport,
                reason: MigrationDiagnosticReason::Exact,
                coordinates: Vec::new(),
                message: "authoritative causal history proves semantic migration cutover"
                    .to_owned(),
            }],
        }
    }

    #[must_use]
    pub const fn state(&self) -> MigrationObservationState {
        self.state
    }

    #[must_use]
    pub const fn current_revision(&self) -> crate::RevisionId {
        self.current_revision
    }

    #[must_use]
    pub const fn current_schema_revision(&self) -> u64 {
        self.current_schema_revision
    }

    /// Returns the durable semantic cutover proof when publication is already in causal history.
    /// This never performs publication or physical materialization.
    #[must_use]
    pub const fn cutover(&self) -> Option<&MigrationCutover> {
        self.cutover.as_ref()
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[MigrationDiagnostic] {
        &self.diagnostics
    }
}

#[derive(Debug, Clone)]
pub struct PreparedMigration {
    pub(crate) database_identity: u64,
    pub(crate) migration_id: u128,
    pub(crate) program: kernel_transport::SchemaMigrationProgram,
    pub(crate) target: kernel_revision::Revision,
    validation: MigrationValidation,
    pub(crate) security_approval: Option<crate::MigrationSecurityApproval>,
}

impl PreparedMigration {
    pub(crate) const fn new(
        database_identity: u64,
        migration_id: u128,
        program: kernel_transport::SchemaMigrationProgram,
        target: kernel_revision::Revision,
        validation: MigrationValidation,
        security_approval: Option<crate::MigrationSecurityApproval>,
    ) -> Self {
        Self {
            database_identity,
            migration_id,
            program,
            target,
            validation,
            security_approval,
        }
    }

    #[must_use]
    pub const fn plan(&self) -> &MigrationPlan {
        self.validation.plan()
    }

    #[must_use]
    pub const fn validation(&self) -> &MigrationValidation {
        &self.validation
    }

    #[must_use]
    pub const fn security_approval(&self) -> Option<&crate::MigrationSecurityApproval> {
        self.security_approval.as_ref()
    }

    #[must_use]
    pub fn preview(&self) -> MigrationPreview {
        MigrationPreview::new(
            self.validation.clone(),
            crate::RevisionId::new(self.target.id().raw()),
        )
    }
}

impl MigrationPreview {
    pub(crate) const fn new(
        validation: MigrationValidation,
        target_revision: crate::RevisionId,
    ) -> Self {
        Self {
            validation,
            target_revision,
        }
    }

    #[must_use]
    pub const fn validation(&self) -> &MigrationValidation {
        &self.validation
    }

    #[must_use]
    pub const fn target_revision(&self) -> crate::RevisionId {
        self.target_revision
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationValueExpr {
    Field(FieldId),
    Column(usize),
    Constant {
        value: Value,
        ty: Type,
    },
    AddI64(Box<Self>, Box<Self>),
    I64ToF64(Box<Self>),
    SeqLength(Box<Self>),
    SeqSumI64(Box<Self>),
    /// Canonical semantic-sum injection. Existing source variant IDs and
    /// payload types must occur unchanged in `target_variants`; extra target
    /// variants are allowed and require no rewrite of existing values.
    WidenSum {
        source: Box<Self>,
        target_variants: BTreeMap<VariantTagId, Type>,
    },
    If {
        condition: Box<Self>,
        when_true: Box<Self>,
        when_false: Box<Self>,
    },
}

impl MigrationValueExpr {
    pub(crate) fn source_fields(&self, output: &mut BTreeSet<FieldId>) {
        match self {
            Self::Field(field) => {
                output.insert(*field);
            }
            Self::Column(_) | Self::Constant { .. } => {}
            Self::AddI64(left, right) => {
                left.source_fields(output);
                right.source_fields(output);
            }
            Self::I64ToF64(source)
            | Self::SeqLength(source)
            | Self::SeqSumI64(source)
            | Self::WidenSum { source, .. } => {
                source.source_fields(output);
            }
            Self::If {
                condition,
                when_true,
                when_false,
            } => {
                condition.source_fields(output);
                when_true.source_fields(output);
                when_false.source_fields(output);
            }
        }
    }

    pub(crate) fn to_kernel(&self) -> kernel_query::Expr {
        self.to_kernel_with_columns(None)
    }

    pub(crate) fn to_kernel_for_relation(
        &self,
        column_ids: &[kernel_types::SemanticId],
    ) -> kernel_query::Expr {
        self.to_kernel_with_columns(Some(column_ids))
    }

    fn to_kernel_with_columns(
        &self,
        column_ids: Option<&[kernel_types::SemanticId]>,
    ) -> kernel_query::Expr {
        match self {
            Self::Field(field) => kernel_query::Expr::ProductField {
                input: Box::new(kernel_query::Expr::Input),
                field: (*field).into(),
            },
            Self::Column(column) => {
                let field = column_ids
                    .and_then(|ids| ids.get(*column).copied())
                    .unwrap_or_else(|| kernel_types::SemanticId::new((*column as u128) + 1));
                kernel_query::Expr::ProductField {
                    input: Box::new(kernel_query::Expr::Input),
                    field,
                }
            }
            Self::Constant { value, ty } => kernel_query::Expr::TypedConst {
                value: value.clone().into(),
                ty: crate::schema::type_to_kernel(ty),
            },
            Self::AddI64(left, right) => kernel_query::Expr::AddI64(
                Box::new(left.to_kernel_with_columns(column_ids)),
                Box::new(right.to_kernel_with_columns(column_ids)),
            ),
            Self::I64ToF64(source) => {
                kernel_query::Expr::I64ToF64(Box::new(source.to_kernel_with_columns(column_ids)))
            }
            Self::SeqLength(source) => {
                kernel_query::Expr::SeqLength(Box::new(source.to_kernel_with_columns(column_ids)))
            }
            Self::SeqSumI64(source) => {
                kernel_query::Expr::SeqSumI64(Box::new(source.to_kernel_with_columns(column_ids)))
            }
            Self::WidenSum {
                source,
                target_variants,
            } => kernel_query::Expr::WidenSum {
                input: Box::new(source.to_kernel_with_columns(column_ids)),
                target_variants: target_variants
                    .iter()
                    .map(|(tag, ty)| ((*tag).into(), crate::schema::type_to_kernel(ty)))
                    .collect(),
            },
            Self::If {
                condition,
                when_true,
                when_false,
            } => kernel_query::Expr::If {
                condition: Box::new(condition.to_kernel_with_columns(column_ids)),
                when_true: Box::new(when_true.to_kernel_with_columns(column_ids)),
                when_false: Box::new(when_false.to_kernel_with_columns(column_ids)),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationFieldRule {
    pub target: FieldId,
    pub value: MigrationValueExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationColumnRule {
    pub source_columns: Vec<usize>,
    pub target_column: usize,
    pub value: MigrationValueExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationRelationRule {
    Query {
        target: RelationId,
        query: Query,
    },
    Rows {
        source: RelationId,
        target: RelationId,
        columns: Vec<MigrationColumnRule>,
    },
}

#[derive(Debug, Clone)]
pub struct MigrationModel {
    id: u128,
    target: Schema,
    fields: Vec<MigrationFieldRule>,
    relations: Vec<MigrationRelationRule>,
}

impl MigrationModel {
    #[must_use]
    pub fn new(id: u128, target: Schema) -> Self {
        Self {
            id,
            target,
            fields: Vec::new(),
            relations: Vec::new(),
        }
    }

    #[must_use]
    pub fn field(mut self, rule: MigrationFieldRule) -> Self {
        self.fields.push(rule);
        self
    }

    #[must_use]
    pub fn relation(mut self, rule: MigrationRelationRule) -> Self {
        self.relations.push(rule);
        self
    }

    pub(crate) const fn id(&self) -> u128 {
        self.id
    }
    pub(crate) fn target(&self) -> Schema {
        self.target.clone()
    }
    pub(crate) fn field_rules(&self) -> &[MigrationFieldRule] {
        &self.fields
    }
    pub(crate) fn relation_rules(&self) -> &[MigrationRelationRule] {
        &self.relations
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationHistoryPolicy {
    /// Explicitly accepts that this migration cannot be inverted from local history.
    Forget,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Database, Schema, Storage, TransactionId, TypeId, VariantTagId};
    use std::collections::BTreeMap;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn sum_widening_migration_is_durable_and_reopens_with_stable_variant_ids() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-sum-widening-{}-{}.cfmd",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let owner = TypeId::new(548_001);
        let field = FieldId::new(548_002);
        let idle = VariantTagId::new(548_003);
        let running = VariantTagId::new(548_004);
        let source_variants = BTreeMap::from([(idle, Type::unit())]);
        let target_variants = BTreeMap::from([(idle, Type::unit()), (running, Type::unit())]);

        let source = Schema::builder()
            .revisions(548, 1)
            .__entity_type(owner)
            .__entity_field(field, owner, Type::Sum(source_variants))
            .build()
            .unwrap();
        let db = Database::builder(&path).schema(source).create().unwrap();
        let target = Schema::builder()
            .revisions(549, 1)
            .__entity_type(owner)
            .__entity_field(field, owner, Type::Sum(target_variants.clone()))
            .build()
            .unwrap();
        let migration = MigrationModel::new(548_549, target).field(MigrationFieldRule {
            target: field,
            value: MigrationValueExpr::WidenSum {
                source: Box::new(MigrationValueExpr::Field(field)),
                target_variants,
            },
        });

        db.migrate(
            &migration,
            TransactionId::new(548_549),
            MigrationHistoryPolicy::Forget,
        )
        .unwrap();
        assert_eq!(db.snapshot().unwrap().schema_revision(), 549);
        drop(db);

        let reopened = Database::open(&path).unwrap();
        assert_eq!(reopened.snapshot().unwrap().schema_revision(), 549);
        drop(reopened);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn runtime_publishes_kernel_verified_migration_model() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-migration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = Schema::builder().revisions(380, 1).build().unwrap();
        let db = Database::create(&path, source).unwrap();
        let target = Schema::builder().revisions(381, 1).build().unwrap();
        let model = MigrationModel::new(380_381, target);
        let outcome = db
            .migrate(
                &model,
                TransactionId::new(380_001),
                MigrationHistoryPolicy::Forget,
            )
            .unwrap();
        assert!(matches!(outcome, crate::CommitOutcome::Committed { .. }));
        assert_eq!(db.snapshot().unwrap().schema_revision(), 381);
        let history = db.history().unwrap();
        let migration = history.latest().expect("migration history event");
        assert_eq!(migration.kind(), crate::HistoryEffectKind::SchemaMigration);
        assert_eq!(
            migration.reversibility(),
            crate::HistoryReversibility::NonPlanTransition
        );
        let semantic_change = migration
            .semantic_change()
            .expect("semantic migration boundary");
        assert_eq!(semantic_change.source_schema(), 380);
        assert_eq!(semantic_change.target_schema(), 381);
        assert_eq!(semantic_change.migration_spec(), 380_381);
        assert_eq!(
            semantic_change.historical_authority(),
            crate::HistoryBoundaryAuthority::ExplicitlyForgotten
        );
        assert!(
            !semantic_change
                .historical_authority()
                .retains_history_authority()
        );
        drop(db);
        let reopened = Database::open(&path).unwrap();
        assert_eq!(reopened.snapshot().unwrap().schema_revision(), 381);
        drop(reopened);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn validation_preserves_structured_transport_coordinate_diagnostic() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-migration-diagnostic-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = Schema::builder().revisions(544, 1).build().unwrap();
        let db = Database::create(&path, source).unwrap();
        let target = Schema::builder().revisions(545, 1).build().unwrap();
        let missing_target = FieldId::new(545_999);
        let model = MigrationModel::new(544_545, target).field(MigrationFieldRule {
            target: missing_target,
            value: MigrationValueExpr::Constant {
                value: Value::I64(1),
                ty: Type::i64(),
            },
        });

        let error = db
            .validate_migration(&model)
            .expect_err("unknown target field must fail exact migration validation");
        assert_eq!(error.kind(), crate::ErrorKind::InvalidSchema);
        let diagnostic = error
            .migration_diagnostic()
            .expect("transport failure must retain a structured diagnostic");
        assert_eq!(diagnostic.stage(), MigrationWorkflowStage::Validate);
        assert_eq!(
            diagnostic.domain(),
            MigrationDiagnosticDomain::DataTransport
        );
        assert_eq!(
            diagnostic.reason(),
            MigrationDiagnosticReason::UnknownTargetCoordinate
        );
        assert_eq!(
            diagnostic.coordinates(),
            &[MigrationCoordinate::Field(missing_target)]
        );

        drop(db);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn validation_classifies_schema_owned_access_change_as_access_domain() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-access-migration-diagnostic-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source_capability = crate::AccessCapability::new("access.live.observe").watch();
        let source_role = crate::Role::new("observer").capability(&source_capability);
        let role_id = source_role.id();
        let source = Schema::builder()
            .revisions(546, 1)
            .access(
                crate::SchemaAccess::new()
                    .capability(source_capability)
                    .role(source_role),
            )
            .build()
            .unwrap();
        let db = Database::create(&path, source).unwrap();

        let widened_capability = crate::AccessCapability::new("access.live.observe")
            .watch()
            .history_read();
        let widened_role = crate::Role::new("observer").capability(&widened_capability);
        let target = Schema::builder()
            .revisions(547, 1)
            .access(
                crate::SchemaAccess::new()
                    .capability(widened_capability)
                    .role(widened_role),
            )
            .build()
            .unwrap();
        let model = MigrationModel::new(546_547, target.clone());

        let validation = db.validate_migration(&model).unwrap();
        assert_eq!(validation.access_changes().len(), 1);
        assert_eq!(
            validation.access_changes()[0].kind(),
            MigrationAccessChangeKind::CapabilityWidened
        );
        assert_eq!(validation.access_changes()[0].affected_roles(), &[role_id]);
        assert!(validation.diagnostics().iter().any(|diagnostic| {
            diagnostic.domain() == MigrationDiagnosticDomain::Access
                && diagnostic.reason() == MigrationDiagnosticReason::AccessPolicyChangeDetected
        }));
        db.migrate(
            &model,
            TransactionId::new(547_001),
            MigrationHistoryPolicy::Forget,
        )
        .unwrap();
        drop(db);
        let reopened = Database::open(&path).unwrap();
        assert_eq!(reopened.snapshot().unwrap().schema_revision(), 547);
        drop(reopened);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn prepared_migration_exposes_one_exact_plan_validate_preview_execute_artifact() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-prepared-migration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = Schema::builder().revisions(543, 1).build().unwrap();
        let db = Database::create(&path, source).unwrap();
        let source_revision = db.current_revision().unwrap();
        let target = Schema::builder().revisions(544, 1).build().unwrap();
        let model = MigrationModel::new(543_544, target);

        let plan = db.plan_migration(&model).unwrap();
        assert_eq!(plan.source_revision(), source_revision);
        assert_eq!(plan.source_schema_revision(), 543);
        assert_eq!(plan.target_schema_revision(), 544);
        assert_eq!(plan.cost_class(), MigrationCostClass::MetadataOnly);

        let validation = db.validate_migration(&model).unwrap();
        assert_eq!(validation.plan(), &plan);
        assert_eq!(
            validation.diagnostics()[0].stage(),
            MigrationWorkflowStage::Validate
        );

        let prepared = db.prepare_migration(&model).unwrap();
        assert_eq!(prepared.plan(), &plan);
        let preview = prepared.preview();
        assert_eq!(preview.validation(), prepared.validation());
        assert_eq!(preview.target_revision().raw(), source_revision.raw() + 1);

        let outcome = db
            .execute_migration(
                &prepared,
                TransactionId::new(543_001),
                MigrationHistoryPolicy::Forget,
            )
            .unwrap();
        assert!(matches!(outcome, crate::CommitOutcome::Committed { .. }));
        assert_eq!(db.snapshot().unwrap().schema_revision(), 544);

        drop(db);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn migration_observation_projects_durable_cutover_without_progress_state() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-migration-observe-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let db =
            Database::create(&path, Schema::builder().revisions(543, 1).build().unwrap()).unwrap();
        let prepared = db
            .prepare_migration(&MigrationModel::new(
                543_544,
                Schema::builder().revisions(544, 1).build().unwrap(),
            ))
            .unwrap();

        let before = db.observe_migration(&prepared).unwrap();
        assert_eq!(before.state(), MigrationObservationState::Prepared);
        assert!(before.cutover().is_none());
        assert_eq!(before.current_schema_revision(), 543);

        db.execute_migration(
            &prepared,
            TransactionId::new(545_001),
            MigrationHistoryPolicy::Forget,
        )
        .unwrap();
        let after = db.observe_migration(&prepared).unwrap();
        assert_eq!(after.state(), MigrationObservationState::CutOver);
        let cutover = after.cutover().expect("durable migration cutover");
        assert_eq!(
            cutover.published_revision(),
            prepared.preview().target_revision()
        );
        assert_eq!(cutover.current_schema_revision(), 544);
        assert!(cutover.target_schema_is_current());
        let effect_id = cutover.effect_id();

        db.migrate(
            &MigrationModel::new(
                544_545,
                Schema::builder().revisions(545, 1).build().unwrap(),
            ),
            TransactionId::new(545_002),
            MigrationHistoryPolicy::Forget,
        )
        .unwrap();
        let advanced = db.observe_migration(&prepared).unwrap();
        assert_eq!(advanced.state(), MigrationObservationState::CutOver);
        let cutover = advanced.cutover().expect("retained causal cutover");
        assert_eq!(cutover.effect_id(), effect_id);
        assert_eq!(cutover.current_schema_revision(), 545);
        assert!(!cutover.target_schema_is_current());

        drop(db);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn prepared_migration_fails_closed_when_head_moves() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-stale-prepared-migration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = Schema::builder().revisions(543, 1).build().unwrap();
        let db = Database::create(&path, source).unwrap();
        let prepared = db
            .prepare_migration(&MigrationModel::new(
                543_544,
                Schema::builder().revisions(544, 1).build().unwrap(),
            ))
            .unwrap();
        db.migrate(
            &MigrationModel::new(
                543_545,
                Schema::builder().revisions(545, 1).build().unwrap(),
            ),
            TransactionId::new(543_002),
            MigrationHistoryPolicy::Forget,
        )
        .unwrap();

        let error = db
            .execute_migration(
                &prepared,
                TransactionId::new(543_003),
                MigrationHistoryPolicy::Forget,
            )
            .expect_err("prepared migration must be source-revision exact");
        assert_eq!(error.kind(), crate::ErrorKind::StaleRevision);
        let observation = db.observe_migration(&prepared).unwrap();
        assert_eq!(
            observation.state(),
            MigrationObservationState::StaleUnpublished
        );
        assert!(observation.cutover().is_none());

        drop(db);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn historical_at_crosses_migration_in_active_single_file_epoch() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-single-file-migration-history-{}-{}.cfmd",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = Schema::builder().revisions(380, 1).build().unwrap();
        let db = Database::builder(&path).schema(source).create().unwrap();
        let source_revision = db.current_revision().unwrap();
        let target = Schema::builder().revisions(381, 1).build().unwrap();
        db.migrate(
            &MigrationModel::new(380_382, target),
            TransactionId::new(380_003),
            MigrationHistoryPolicy::Forget,
        )
        .unwrap();
        assert_eq!(db.snapshot().unwrap().schema_revision(), 381);
        let historical = db.at(source_revision).unwrap();
        assert_eq!(historical.revision(), source_revision);
        assert_eq!(historical.schema_revision(), 380);
        drop(historical);
        drop(db);

        let reopened = Database::open(&path).unwrap();
        let historical = reopened.at(source_revision).unwrap();
        assert_eq!(historical.revision(), source_revision);
        assert_eq!(historical.schema_revision(), 380);
        drop(historical);
        drop(reopened);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn historical_at_crosses_migration_through_source_epoch_anchor() {
        let path = std::env::temp_dir().join(format!(
            "cfmd-runtime-migration-history-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = Schema::builder().revisions(380, 1).build().unwrap();
        let db = Database::builder(&path)
            .storage(Storage::Directory)
            .schema(source)
            .create()
            .unwrap();
        let source_revision = db.current_revision().unwrap();
        let target = Schema::builder().revisions(381, 1).build().unwrap();
        db.migrate(
            &MigrationModel::new(380_381, target),
            TransactionId::new(380_002),
            MigrationHistoryPolicy::Forget,
        )
        .unwrap();
        assert_eq!(db.snapshot().unwrap().schema_revision(), 381);
        let historical = db.at(source_revision).unwrap();
        assert_eq!(historical.revision(), source_revision);
        assert_eq!(historical.schema_revision(), 380);
        drop(historical);
        drop(db);

        let reopened = Database::builder(&path)
            .storage(Storage::Directory)
            .open()
            .unwrap();
        let historical = reopened.at(source_revision).unwrap();
        assert_eq!(historical.revision(), source_revision);
        assert_eq!(historical.schema_revision(), 380);
        drop(reopened);
        let _ = fs::remove_dir_all(&path);
    }
}
