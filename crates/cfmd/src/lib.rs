//! Public Rust application facade for CFMD.
//!
//! Application code should depend on this crate rather than `cfmd-runtime` or any `kernel-*` crate.
//! The root surface is object-first. Low-level relation/query/value primitives remain available
//! through [`dynamic`] as an explicit escape hatch for tooling and generated bindings.
//!
//! Exact watch authority remains runtime-neutral. P345 exposes shared readiness identity and bounded
//! drains; P346/P347 add race-free Waker registration. P348 collapses executor-neutral async
//! ergonomics directly into each watch: `watch.next().await` needs no wrapper crate or runtime adapter.
//! Tokio is dev-only compatibility coverage, not a semantic dependency.

mod diagnostic;

pub use diagnostic::{Diagnostic, DiagnosticCode, ErrorDiagnosticExt, Severity};

pub use cfmd_derive::{CfmdEntity, CfmdSchema};
pub use cfmd_runtime::{cfmd_entity, cfmd_object};

pub use cfmd_runtime::{
    AccessCapability, AccessCapabilityId, AdminDatabase, AndPredicate, BackupVerification,
    BetweenPredicate, Candidate, CandidateDerivedEffects, CandidateDiagnostics, CandidateEffects,
    CandidateGroupedAggregateQuery, CandidateObjectQuery, CandidateObjectSet, CandidatePreview,
    CandidateProjectionQuery, CandidateReadiness, CfmdSchema, CommitOutcome, Context,
    ContextAdmission, Database, DatabaseBuilder, DatabaseControlCredential,
    DatabaseControlPermission, DatabaseControlPermissionSet, DatabaseControlSession,
    DatabaseControlSnapshot, DatabaseDefinition, Encryption, EncryptionKey,
    EncryptionKeyAcknowledgement, EncryptionKeyDestination, EncryptionKeyId,
    EncryptionKeyOperation, EncryptionKeyProvider, EncryptionProviderKeyMetadata, EntitySet,
    EqOperand, EqPredicate, Error, ErrorKind, ExactAggregateMeasureExpr, ExternalFreshness, Field,
    FieldRule, FiniteF64, GroupKey, GroupedAggregateQuery, GroupedAggregateWatch,
    GroupedAggregateWatchEvent, History, HistoryEffectKind, HistoryEntry, HistoryRelationChange,
    HistoryRetentionPin, HistoryRetentionReason, HistoryReversibility, HistoryUndoReadiness, Id,
    InProcessPublicationNotifier, Many, ManyCount, ManyCountPredicate, ManyField, ManyPredicate,
    ManySelection, MigratableQueryWatch, MigratableWatchEvent, MigrationAccessChange,
    MigrationAccessChangeKind, MigrationAccessSubject, MigrationCoordinate, MigrationCostClass,
    MigrationCutover, MigrationDataDependency, MigrationDeclassificationEdge, MigrationDiagnostic,
    MigrationDiagnosticCode, MigrationDiagnosticDomain, MigrationDiagnosticReason,
    MigrationDiagnosticSeverity, MigrationIntegrityPolicyCoordinate, MigrationObservation,
    MigrationObservationFlow, MigrationObservationState, MigrationPlan, MigrationPreview,
    MigrationSecurityApproval, MigrationSecurityImpact, MigrationSecurityImpactDigest,
    MigrationValidation, MigrationWorkflowStage, ModelEntityId, ModelRuleExpr, ModelSemanticId,
    NotPredicate, Object, ObjectEquivalence, ObjectFieldRole, ObjectFieldSchema,
    ObjectManyFieldSchema, ObjectPatchField, ObjectPredicate, ObjectProjectionQuery, ObjectProxy,
    ObjectQuery, ObjectRelationshipCardinality, ObjectRuleField, ObjectSet, ObjectValue,
    ObjectWatch, ObjectWatchEvent, OptionalRefField, OptionalRefIsSome, OrPredicate,
    OrderPredicate, OrderedExtremumKind, OrderedObjectValue, OrderedStatisticBound,
    OrderedStatisticSelector, OrphanPolicy, OwnedMany, OwnedManySelection, PathField,
    PathPredicate, Permission, PermissionCoordinate, PermissionSet, PreparedMigration, PrincipalId,
    Projection, ProjectionWatch, ProjectionWatchEvent, PublicationNotifier, QueryNodeId,
    QuerySource, RecoveryAuthority, RecoveryDiagnostic, RecoveryOperation, RecoveryReason, Ref,
    RefField, RefPath, RefPredicate, RelationChange, Result, RevisionId, Role, RoleId,
    RuleOrderComparison, RuleValueExpr, Schema, SchemaAccess, ScopedRelationship, SemanticRuleExpr,
    Session, SessionDatabase, SessionSnapshot, Snapshot, Storage, TextPattern, ValueCodec,
    WatchCancellation, WatchDrain, WatchNext, WatchReadiness, WatchReadinessSourceId, WatchStatus,
    WatchSubscriptionId, WatchWake,
};

#[doc(hidden)]
pub mod __private {
    pub use cfmd_runtime::{
        __append_flat_insert as append_flat_insert, __append_many_edge as append_many_edge,
        __append_remove_many_edges as append_remove_many_edges,
        __identity_equivalence_id as identity_equivalence_id,
        __many_relation_id as many_relation_id, __register_owned_many as register_owned_many,
        __row_shape_error as row_shape_error, AuthoritativeObject, ContextSource, IntentJournal,
        IntentReadiness, Plan, ReadContext, Relation, Row, RowCodec, SchemaAuthorityEmpty,
        SchemaAuthorityLeaf, SchemaAuthorityPair, TransactionId, Type, ValueCodec,
    };
}

/// Explicit low-level relation/value/query escape hatch.
///
/// This is the stable binding/tooling vocabulary. Normal Rust application code should prefer the
/// object-first root surface and symbolic object proxies.
pub mod dynamic {
    pub use cfmd_runtime::{
        BetweenPredicate, EntityRef, EqPredicate, EquivalenceId, Field, FieldId, FieldRule,
        FiniteF64, MigrationAccessSubject, MigrationColumnRule, MigrationFieldRule,
        MigrationHistoryPolicy, MigrationModel, MigrationRelationRule, MigrationValueExpr,
        ModelRuleExpr, OrderComparison, OrderDirection, OrderPredicate, OrderingId, Plan,
        PreparedQuery, PreparedTypedQuery, PrimitiveEquivalence, PrimitiveOrdering, Query,
        QueryNode, QueryNodeKind, QueryWatch, Relation, RelationId, RelationQuery, RelationResult,
        RelationSchema, RelationSemantics, Row, RowCodec, ScalarType, SchemaBuilder, SchemaView,
        StructuralEquivalence, Type, TypeId, TypedQuery, Value, VariantTagId, WatchEvent,
    };
}

/// Common object-first imports for Rust applications.
pub mod prelude {
    pub use crate::{
        CfmdEntity, CfmdSchema, Database, EntitySet, ErrorDiagnosticExt, Id, Many, Object,
        ObjectPredicate, OrphanPolicy, OwnedMany, PrincipalId, Ref, Result, RevisionId, Schema,
        Storage, cfmd_entity, cfmd_object,
    };
}
