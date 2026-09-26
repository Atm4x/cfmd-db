use std::collections::BTreeMap;

use kernel_model::Value;
use kernel_schema::{ScalarType, TypeExpr, TypeVar};
use kernel_types::SemanticId;

use super::{NativeColumn, PersistentPhysicalVec, PhysicalExecutionError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlgebraicNativeColumn {
    ty: TypeExpr,
    storage: AlgebraicStorage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AlgebraicStorage {
    Empty,
    Scalar(NativeColumn),
    Product {
        fields: BTreeMap<SemanticId, AlgebraicNativeColumn>,
        row_count: usize,
    },
    Sum {
        tags: PersistentPhysicalVec<SemanticId>,
        payload_index: PersistentPhysicalVec<u32>,
        variants: BTreeMap<SemanticId, AlgebraicNativeColumn>,
    },
    Option {
        payload_index: PersistentPhysicalVec<Option<u32>>,
        payload: Box<AlgebraicNativeColumn>,
    },
    Seq {
        offsets: PersistentPhysicalVec<u32>,
        payload: Box<AlgebraicNativeColumn>,
    },
    Set {
        equivalence: SemanticId,
        offsets: PersistentPhysicalVec<u32>,
        payload: Box<AlgebraicNativeColumn>,
    },
    Bag {
        equivalence: SemanticId,
        offsets: PersistentPhysicalVec<u32>,
        payload: Box<AlgebraicNativeColumn>,
        counts: PersistentPhysicalVec<u64>,
    },
    Map {
        key_equivalence: SemanticId,
        offsets: PersistentPhysicalVec<u32>,
        keys: Box<AlgebraicNativeColumn>,
        values: Box<AlgebraicNativeColumn>,
    },
    Recursive(Box<AlgebraicNativeColumn>),
}

include!("algebraic_native/column_impl.rs");
include!("algebraic_native/builders.rs");
include!("algebraic_native/tests.rs");
