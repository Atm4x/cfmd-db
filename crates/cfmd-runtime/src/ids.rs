macro_rules! id_type {
    ($name:ident, $inner:ty) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($inner);

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
    };
}

id_type!(RelationId, u128);
id_type!(RelationColumnId, u128);
id_type!(EquivalenceId, u128);
id_type!(OrderingId, u128);
id_type!(TypeId, u128);
id_type!(FieldId, u128);
id_type!(VariantTagId, u128);
id_type!(RevisionId, u64);
id_type!(TransactionId, u128);

impl From<RelationColumnId> for kernel_types::SemanticId {
    fn from(value: RelationColumnId) -> Self {
        Self::new(value.raw())
    }
}

impl From<RelationId> for kernel_types::SemanticId {
    fn from(value: RelationId) -> Self {
        Self::new(value.raw())
    }
}

impl From<EquivalenceId> for kernel_types::SemanticId {
    fn from(value: EquivalenceId) -> Self {
        Self::new(value.raw())
    }
}

impl From<OrderingId> for kernel_types::SemanticId {
    fn from(value: OrderingId) -> Self {
        Self::new(value.raw())
    }
}

impl From<TypeId> for kernel_types::SemanticId {
    fn from(value: TypeId) -> Self {
        Self::new(value.raw())
    }
}

impl From<kernel_types::RevisionId> for RevisionId {
    fn from(value: kernel_types::RevisionId) -> Self {
        Self::new(value.raw())
    }
}

impl From<FieldId> for kernel_types::SemanticId {
    fn from(value: FieldId) -> Self {
        Self::new(value.raw())
    }
}

impl From<VariantTagId> for kernel_types::SemanticId {
    fn from(value: VariantTagId) -> Self {
        Self::new(value.raw())
    }
}
