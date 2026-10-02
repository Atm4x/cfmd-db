//! Stable product/runtime boundary for CFMD Rust applications and language bindings.
//!
//! This crate deliberately owns its public identifiers, values, query IR and lifecycle types.
//! Internal `kernel-*` types are implementation details and never appear in public signatures.

mod candidate;
mod entity;
mod error;
mod history;
mod ids;
mod migration;
mod notification;
mod object;
mod plan;
mod query;
mod runtime;
mod schema;
mod schema_model;
mod security;
mod transaction;
mod typed;
mod value;
mod watch;

pub use candidate::{
    Candidate, CandidateDerivedEffects, CandidateDiagnostics, CandidateEffects,
    CandidateGroupedAggregateQuery, CandidateObjectGroupQuery, CandidateObjectQuery,
    CandidateObjectSet, CandidatePreview, CandidateProjectionQuery, CandidateReadiness,
    RelationChange,
};
pub use entity::{
    Id, ManyCount, ManyCountPredicate, ManyField, ManyPredicate, OptionalRefField,
    OptionalRefIsSome, PathField, PathPredicate, Ref, RefField, RefPath, RefPredicate,
};
pub use error::{Error, ErrorKind, Result};
pub use history::{
    History, HistoryBoundaryAuthority, HistoryEffectKind, HistoryEntry, HistoryRelationChange,
    HistoryReversibility, HistorySemanticChange, HistoryUndoReadiness,
};
pub use ids::{
    EquivalenceId, FieldId, OrderingId, RelationColumnId, RelationId, RevisionId, TransactionId,
    TypeId, VariantTagId,
};
pub use migration::{
    MigrationColumnRule, MigrationFieldRule, MigrationHistoryPolicy, MigrationModel,
    MigrationRelationRule, MigrationValueExpr,
};
pub use notification::{InProcessPublicationNotifier, PublicationNotifier};
pub use object::{
    __row_shape_error, GroupedAggregateQuery, Many, ManySelection, Object, ObjectEquivalence,
    ObjectFieldRole, ObjectFieldSchema, ObjectGroupQuery, ObjectManyFieldSchema, ObjectPatchField,
    ObjectProjectionQuery, ObjectProxy, ObjectQuery, ObjectRelationshipCardinality, ObjectSet,
    ObjectValue, OrderedObjectValue, OwnedMany, OwnedManySelection,
};
pub use plan::{CommitOutcome, OrphanPolicy, Plan};
pub use query::{
    OrderComparison, OrderDirection, PreparedQuery, Query, QueryNode, QueryNodeId, QueryNodeKind,
    QuerySource, RelationResult,
};
pub use runtime::{
    Database, DatabaseBuilder, Encryption, EncryptionKey, EncryptionKeyAcknowledgement,
    EncryptionKeyDestination, EncryptionKeyId, EncryptionKeyOperation, EncryptionKeyProvider,
    EncryptionProviderKeyMetadata, ReadContext, Storage,
};
pub use schema::{
    FieldRule, PrimitiveEquivalence, PrimitiveOrdering, RelationSchema, RelationSemantics,
    RuleValueExpr, ScalarType, Schema, SchemaBuilder, SchemaView, SemanticRuleExpr,
    StructuralEquivalence, TextPattern, Type,
};
#[doc(hidden)]
pub use schema_model::{
    AuthoritativeObject, CompleteSchemaAuthority, SchemaAuthorityEmpty, SchemaAuthorityLeaf,
    SchemaAuthorityPair,
};
pub use schema_model::{
    CfmdSchema, Context, ContextSource, DatabaseContext, DatabaseDefinition, EntitySet,
    SchemaDatabase, SchemaDatabaseBuilder, Snapshot,
};
pub use security::{
    Permission, PermissionSet, PrincipalId, Role, Session, SessionDatabase, SessionSnapshot,
};
pub use transaction::{Transaction, TransactionReadiness};
pub use value::{EntityRef, Row, Value};
pub use watch::{
    GroupedAggregateWatch, GroupedAggregateWatchEvent, ObjectWatch, ObjectWatchEvent,
    ProjectionWatch, ProjectionWatchEvent, QueryWatch, WatchAuthorization, WatchCancellation,
    WatchDrain, WatchEvent, WatchNext, WatchReadiness, WatchReadinessSourceId, WatchStatus,
    WatchSubscriptionId, WatchWake,
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
