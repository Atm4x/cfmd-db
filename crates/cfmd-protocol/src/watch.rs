use crate::{QueryResponse, Row};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubscriptionId(u64);

impl SubscriptionId {
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWatchRequest {
    pub target: crate::SnapshotTarget,
    pub query: crate::ProtocolQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWatchResponse {
    pub subscription: SubscriptionId,
    pub initial: QueryResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEventDto {
    pub subscription: SubscriptionId,
    pub source_revision: u64,
    pub target_revision: u64,
    pub inserted: Vec<Row>,
    pub removed: Vec<Row>,
}

impl WatchEventDto {
    pub(crate) fn from_runtime(
        subscription: SubscriptionId,
        event: &cfmd_runtime::WatchEvent,
    ) -> Self {
        Self {
            subscription,
            source_revision: event.source_revision().raw(),
            target_revision: event.target_revision().raw(),
            inserted: event
                .inserted()
                .iter()
                .cloned()
                .map(|row| row.into_iter().map(Into::into).collect())
                .collect(),
            removed: event
                .removed()
                .iter()
                .cloned()
                .map(|row| row.into_iter().map(Into::into).collect())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStatusDto {
    Current {
        revision: u64,
    },
    Lagging {
        anchor_revision: u64,
        head_revision: u64,
        pending_transitions: usize,
    },
    Cancelled {
        revision: u64,
    },
    RuntimeClosed {
        revision: u64,
    },
    Unavailable {
        anchor_revision: u64,
        head_revision: u64,
    },
}

impl From<cfmd_runtime::WatchStatus> for WatchStatusDto {
    fn from(value: cfmd_runtime::WatchStatus) -> Self {
        match value {
            cfmd_runtime::WatchStatus::Current { revision } => Self::Current {
                revision: revision.raw(),
            },
            cfmd_runtime::WatchStatus::Lagging {
                anchor_revision,
                head_revision,
                pending_transitions,
            } => Self::Lagging {
                anchor_revision: anchor_revision.raw(),
                head_revision: head_revision.raw(),
                pending_transitions,
            },
            cfmd_runtime::WatchStatus::Cancelled { revision } => Self::Cancelled {
                revision: revision.raw(),
            },
            cfmd_runtime::WatchStatus::RuntimeClosed { revision } => Self::RuntimeClosed {
                revision: revision.raw(),
            },
            cfmd_runtime::WatchStatus::Unavailable {
                anchor_revision,
                head_revision,
            } => Self::Unavailable {
                anchor_revision: anchor_revision.raw(),
                head_revision: head_revision.raw(),
            },
        }
    }
}
