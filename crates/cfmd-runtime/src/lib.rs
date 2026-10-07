//! Stable product/runtime boundary for CFMD Rust applications and language bindings.
//!
//! This crate deliberately owns its public identifiers, values, query IR and lifecycle types.
//! Internal `kernel-*` types are implementation details and never appear in public signatures.

mod candidate;
mod control;
mod entity;
mod error;
mod history;
mod ids;
mod intent_journal;
mod migration;
mod notification;
mod object;
mod plan;
mod query;
mod retention;
mod runtime;
mod schema;
mod schema_model;
mod security;
mod typed;
mod value;
mod watch;

pub use candidate::{
    Candidate, CandidateDerivedEffects, CandidateDiagnostics, CandidateEffects,
    CandidateGroupedAggregateQuery, CandidateObjectGroupQuery, CandidateObjectQuery,
    CandidateObjectSet, CandidatePreview, CandidateProjectionQuery, CandidateReadiness,
    RelationChange,
};
pub use control::{
    AdminDatabase, DatabaseControlCredential, DatabaseControlPermission,
    DatabaseControlPermissionSet, DatabaseControlSession, DatabaseControlSnapshot,
    MigrationSecurityApproval,
};
pub use entity::{
    Id, ManyCount, ManyCountPredicate, ManyField, ManyPredicate, OptionalRefField,
    OptionalRefIsSome, PathField, PathPredicate, Ref, RefField, RefPath, RefPredicate,
};
pub use error::{
    Error, ErrorKind, RecoveryAuthority, RecoveryDiagnostic, RecoveryOperation, RecoveryReason,
    Result,
};
pub use history::{
    History, HistoryBoundaryAuthority, HistoryEffectKind, HistoryEntry, HistoryRelationChange,
    HistoryReversibility, HistorySemanticChange, HistoryUndoReadiness,
};
pub use ids::{
    AccessCapabilityId, EquivalenceId, FieldId, ModelEntityId, ModelSemanticId, OrderingId,
    RelationColumnId, RelationId, RevisionId, RoleId, TransactionId, TypeId, VariantTagId,
};
#[doc(hidden)]
pub use intent_journal::{IntentJournal, IntentReadiness};
pub use migration::{
    MigrationAccessChange, MigrationAccessChangeKind, MigrationAccessSubject, MigrationColumnRule,
    MigrationCoordinate, MigrationCostClass, MigrationCutover, MigrationDataDependency,
    MigrationDeclassificationEdge, MigrationDiagnostic, MigrationDiagnosticCode,
    MigrationDiagnosticDomain, MigrationDiagnosticReason, MigrationDiagnosticSeverity,
    MigrationFieldRule, MigrationHistoryPolicy, MigrationIntegrityPolicyCoordinate, MigrationModel,
    MigrationObservation, MigrationObservationFlow, MigrationObservationState, MigrationPlan,
    MigrationPreview, MigrationRelationRule, MigrationSecurityImpact,
    MigrationSecurityImpactDigest, MigrationValidation, MigrationValueExpr, MigrationWorkflowStage,
    PreparedMigration,
};
pub use notification::{InProcessPublicationNotifier, PublicationNotifier};
pub use object::{
    __row_shape_error, GroupedAggregateQuery, Many, ManySelection, Object, ObjectEquivalence,
    ObjectFieldRole, ObjectFieldSchema, ObjectGroupQuery, ObjectManyFieldSchema, ObjectPatchField,
    ObjectProjectionQuery, ObjectProxy, ObjectQuery, ObjectRelationshipCardinality,
    ObjectRuleField, ObjectSet, ObjectValue, OrderedObjectValue, OwnedMany, OwnedManySelection,
    ScopedRelationship,
};
pub use plan::{CommitOutcome, OrphanPolicy, Plan};
pub use query::{
    OrderComparison, OrderDirection, PreparedQuery, Query, QueryNode, QueryNodeId, QueryNodeKind,
    QuerySource, RelationResult,
};
pub use retention::{HistoryRetentionPin, HistoryRetentionReason};
#[doc(hidden)]
pub use runtime::ExactRelationMutation;
pub use runtime::{
    BackupVerification, Database, DatabaseBuilder, Encryption, EncryptionKey,
    EncryptionKeyAcknowledgement, EncryptionKeyDestination, EncryptionKeyId,
    EncryptionKeyOperation, EncryptionKeyProvider, EncryptionProviderKeyMetadata,
    ExternalFreshness, ReadContext, Storage,
};
pub use schema::{
    ExactAggregateMeasureExpr, FieldRule, FiniteF64, ModelRuleExpr, OrderedExtremumKind,
    OrderedStatisticBound, OrderedStatisticSelector, PrimitiveEquivalence, PrimitiveOrdering,
    RelationSchema, RelationSemantics, RuleOrderComparison, RuleValueExpr, ScalarType, Schema,
    SchemaBuilder, SchemaView, SemanticRuleExpr, StructuralEquivalence, TextPattern, Type,
};
#[doc(hidden)]
pub use schema_model::{
    AuthoritativeObject, CompleteSchemaAuthority, SchemaAuthorityEmpty, SchemaAuthorityLeaf,
    SchemaAuthorityPair,
};
pub use schema_model::{
    CfmdSchema, Context, ContextAdmission, ContextSource, DatabaseDefinition, EntitySet, Snapshot,
};
pub use security::{
    AccessCapability, Permission, PermissionCoordinate, PermissionSet, PrincipalId, Role,
    SchemaAccess, Session, SessionDatabase, SessionSnapshot,
};
pub use value::{EntityRef, Row, Value};
pub use watch::{
    GroupedAggregateWatch, GroupedAggregateWatchEvent, MigratableQueryWatch, MigratableWatchEvent,
    ObjectWatch, ObjectWatchEvent, ProjectionWatch, ProjectionWatchEvent, QueryWatch,
    WatchAuthorization, WatchCancellation, WatchDrain, WatchEvent, WatchNext, WatchReadiness,
    WatchReadinessSourceId, WatchStatus, WatchSubscriptionId, WatchWake,
};

pub use object::{
    __append_flat_insert, __append_many_edge, __append_remove_many_edges,
    __identity_equivalence_id, __many_relation_id, __register_owned_many,
};
pub use typed::{
    AndPredicate, BetweenPredicate, EqOperand, EqPredicate, Field, GroupKey, NotPredicate,
    ObjectPredicate, OrPredicate, OrderPredicate, PreparedTypedQuery, Projection, Relation,
    RelationQuery, RowCodec, TypedQuery, ValueCodec,
};
