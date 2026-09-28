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
mod typed;
mod value;
mod watch;

pub use candidate::{
    Candidate, CandidateDiagnostics, CandidateEffects, CandidateObjectQuery, CandidateObjectSet,
    CandidatePreview, CandidateProjectionQuery, CandidateReadiness, RelationChange,
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
    __row_shape_error, Object, ObjectEquivalence, ObjectFieldRole, ObjectFieldSchema,
    ObjectProjectionQuery, ObjectProxy, ObjectQuery, ObjectSet, ObjectValue,
};
pub use plan::{CommitOutcome, Plan};
pub use query::{OrderDirection, PreparedQuery, Query, RelationResult};
pub use runtime::{Database, DatabaseBuilder, ReadContext, Storage};
pub use schema::{
    PrimitiveEquivalence, PrimitiveOrdering, RelationSchema, RelationSemantics, ScalarType, Schema,
    SchemaBuilder, SchemaView, StructuralEquivalence, Type,
};
pub use security::{
    Permission, PermissionSet, PrincipalId, Session, SessionDatabase, SessionSnapshot,
};
pub use value::{EntityRef, Row, Value};
pub use watch::{
    ObjectWatch, ObjectWatchEvent, ProjectionWatch, ProjectionWatchEvent, QueryWatch,
    WatchCancellation, WatchEvent, WatchStatus,
};

pub use typed::{
    EqPredicate, Field, ObjectPredicate, PreparedTypedQuery, Projection, Relation, RelationQuery,
    RowCodec, TypedQuery, ValueCodec,
};
