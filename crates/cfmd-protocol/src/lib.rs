//! Transport-neutral hosted protocol boundary for CFMD.
//!
//! This crate owns language-neutral request/value/query DTOs and dispatches
//! them only through restricted `cfmd_runtime::SessionDatabase` values. It
//! deliberately owns no listener, socket, authentication mechanism or server
//! lifecycle.

mod error;
mod history;
mod query;
mod session;
mod value;
mod watch;
pub mod wire;

pub use error::{ProtocolError, ProtocolErrorCode, Result};
pub use history::{
    HistoryEntryDto, HistoryKind, HistoryRelationChangeDto, HistoryReversibilityDto,
};
pub use query::{OrderDirection, ProtocolQuery, QueryRequest, QueryResponse, SnapshotTarget};
pub use session::{
    CommitRequest, CommitResponse, HostedRequest, HostedResponse, HostedSession, IdempotencyKey,
    PROTOCOL_VERSION, ProtocolLimits, RelationMutation, SemanticRevision,
};
pub use value::{EntityRef, ProtocolValue, Row};
pub use watch::{
    OpenWatchRequest, OpenWatchResponse, SubscriptionId, WatchEventDto, WatchStatusDto,
};
