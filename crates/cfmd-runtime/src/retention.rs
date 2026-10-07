use crate::RevisionId;

/// Why CFMD is retaining an exact historical epoch authority.
///
/// FORMAT V1 creates these pins at semantic schema-migration boundaries. They
/// are durable authority records, not RAII reader handles and not a second
/// history log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryRetentionReason {
    SchemaMigration,
}

/// One durable historical-retention pin.
///
/// Pins are created by CFMD when a schema migration publishes a historical
/// source epoch. A released pin is irreversible: a stale copy of this value
/// cannot recreate historical authority after the underlying closure is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryRetentionPin {
    pub(crate) database_identity: u64,
    pub(crate) effect_id: u128,
    pub(crate) source_revision: RevisionId,
    pub(crate) source_schema_revision: u64,
    pub(crate) reason: HistoryRetentionReason,
}

impl HistoryRetentionPin {
    #[must_use]
    pub const fn effect_id(self) -> u128 {
        self.effect_id
    }

    #[must_use]
    pub const fn source_revision(self) -> RevisionId {
        self.source_revision
    }

    #[must_use]
    pub const fn source_schema_revision(self) -> u64 {
        self.source_schema_revision
    }

    #[must_use]
    pub const fn reason(self) -> HistoryRetentionReason {
        self.reason
    }
}
