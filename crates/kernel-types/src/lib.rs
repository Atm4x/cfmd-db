use core::fmt;

macro_rules! semantic_id {
    ($name:ident, $inner:ty) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(pub $inner);

        impl $name {
            #[must_use]
            pub const fn new(raw: $inner) -> Self {
                Self(raw)
            }

            #[must_use]
            pub const fn raw(self) -> $inner {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }
    };
}

semantic_id!(EntityId, u128);
semantic_id!(SemanticId, u128);
semantic_id!(SchemaRevisionId, u64);
semantic_id!(SemanticEnvId, u64);
semantic_id!(RevisionId, u64);
semantic_id!(RevisionObservableId, u128);
semantic_id!(EqClassId, u128);
semantic_id!(MaterializationId, u128);
semantic_id!(ClientTransactionId, u128);

/// Stable storage identity for one physical row slot.
///
/// The slot may be reused after deletion, while `generation` prevents stale
/// handles from aliasing a later occupant of the same slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StableRowHandle {
    pub slot: usize,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticRevision {
    pub schema: SchemaRevisionId,
    pub environment: SemanticEnvId,
}

impl SemanticRevision {
    #[must_use]
    pub const fn new(schema: SchemaRevisionId, environment: SemanticEnvId) -> Self {
        Self {
            schema,
            environment,
        }
    }
}
