//! Stable product/runtime boundary for CFMD Rust applications and language bindings.
//!
//! This crate deliberately owns its public identifiers, values, query IR and lifecycle types.
//! Internal `kernel-*` types are implementation details and never appear in public signatures.

mod candidate;
mod entity;
mod error;
mod history;
mod ids;
mod notification;
mod object;
mod plan;
mod query;
mod runtime;
mod schema;
mod security;
mod transaction;
mod typed;
mod value;
mod watch;

pub use candidate::{
    Candidate, CandidateDerivedEffects, CandidateDiagnostics, CandidateEffects,
    CandidateObjectQuery, CandidateObjectSet, CandidatePreview, CandidateProjectionQuery,
    CandidateReadiness, RelationChange,
};
pub use entity::{
    Id, ManyCount, ManyCountEq, ManyField, ManyPredicate, OptionalRefField, OptionalRefIsSome, Ref,
    RefField, RefPredicate,
};
pub use error::{Error, ErrorKind, Result};
pub use history::{
    History, HistoryEffectKind, HistoryEntry, HistoryRelationChange, HistoryReversibility,
    HistoryUndoReadiness,
};
pub use ids::{
    EquivalenceId, FieldId, OrderingId, RelationId, RevisionId, TransactionId, TypeId, VariantTagId,
};
pub use notification::{InProcessPublicationNotifier, PublicationNotifier};
pub use object::{
    __row_shape_error, Many, ManySelection, Object, ObjectEquivalence, ObjectFieldRole,
    ObjectFieldSchema, ObjectManyFieldSchema, ObjectProjectionQuery, ObjectProxy, ObjectQuery,
    ObjectRelationshipCardinality, ObjectSet, ObjectValue, OrderedObjectValue, OwnedMany,
    OwnedManySelection,
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
    PrimitiveEquivalence, PrimitiveOrdering, RelationSchema, RelationSemantics, ScalarType, Schema,
    SchemaBuilder, SchemaView, StructuralEquivalence, Type,
};
pub use security::{
    Permission, PermissionSet, PrincipalId, Session, SessionDatabase, SessionSnapshot,
};
pub use transaction::Transaction;
pub use value::{EntityRef, Row, Value};
pub use watch::{
    ObjectWatch, ObjectWatchEvent, ProjectionWatch, ProjectionWatchEvent, QueryWatch,
    WatchCancellation, WatchDrain, WatchEvent, WatchNext, WatchReadiness, WatchReadinessSourceId,
    WatchStatus, WatchSubscriptionId, WatchWake,
};

pub use object::{
    __append_flat_insert, __append_many_edge, __append_remove_many_edges,
    __identity_equivalence_id, __many_relation_id, __register_owned_many,
};
pub use typed::{
    BetweenPredicate, EqPredicate, Field, ObjectPredicate, OrderPredicate, PreparedTypedQuery,
    Projection, Relation, RelationQuery, RowCodec, TypedQuery, ValueCodec,
};
