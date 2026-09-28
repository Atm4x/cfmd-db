use std::collections::BTreeMap;

use crate::{EquivalenceId, FieldId, TypeId, VariantTagId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityRef {
    pub entity_type: TypeId,
    pub id: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Value {
    Unit,
    Bool(bool),
    I64(i64),
    F64Bits(u64),
    Text(String),
    LiveEntityRef(EntityRef),
    HistoricalEntityRef(EntityRef),
    Product(BTreeMap<FieldId, Self>),
    Option(Option<Box<Self>>),
    Variant {
        tag: VariantTagId,
        value: Box<Self>,
    },
    Seq(Vec<Self>),
    Set {
        equivalence: EquivalenceId,
        elements: Vec<Self>,
    },
    Bag {
        equivalence: EquivalenceId,
        entries: Vec<(Self, u64)>,
    },
    Map {
        key_equivalence: EquivalenceId,
        entries: Vec<(Self, Self)>,
    },
}

pub type Row = Vec<Value>;

impl From<Value> for kernel_model::Value {
    fn from(value: Value) -> Self {
        match value {
            Value::Unit => Self::Unit,
            Value::Bool(value) => Self::Bool(value),
            Value::I64(value) => Self::I64(value),
            Value::F64Bits(value) => Self::F64Bits(value),
            Value::Text(value) => Self::Text(value),
            Value::LiveEntityRef(value) => Self::LiveEntityRef {
                entity_type: value.entity_type.into(),
                id: kernel_types::EntityId::new(value.id),
            },
            Value::HistoricalEntityRef(value) => Self::HistoricalEntityId {
                entity_type: value.entity_type.into(),
                id: kernel_types::EntityId::new(value.id),
            },
            Value::Product(values) => Self::Product(
                values
                    .into_iter()
                    .map(|(key, value)| (kernel_types::SemanticId::new(key.raw()), value.into()))
                    .collect(),
            ),
            Value::Option(value) => Self::Option(value.map(|value| Box::new((*value).into()))),
            Value::Variant { tag, value } => Self::Variant {
                tag: kernel_types::SemanticId::new(tag.raw()),
                value: Box::new((*value).into()),
            },
            Value::Seq(values) => Self::Seq(values.into_iter().map(Into::into).collect()),
            Value::Set {
                equivalence,
                elements,
            } => Self::Set {
                equivalence: kernel_types::SemanticId::new(equivalence.raw()),
                elements: elements.into_iter().map(Into::into).collect(),
            },
            Value::Bag {
                equivalence,
                entries,
            } => Self::Bag {
                equivalence: kernel_types::SemanticId::new(equivalence.raw()),
                entries: entries
                    .into_iter()
                    .map(|(value, count)| (value.into(), count))
                    .collect(),
            },
            Value::Map {
                key_equivalence,
                entries,
            } => Self::Map {
                key_equivalence: kernel_types::SemanticId::new(key_equivalence.raw()),
                entries: entries
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
            },
        }
    }
}

impl From<kernel_model::Value> for Value {
    fn from(value: kernel_model::Value) -> Self {
        match value {
            kernel_model::Value::Unit => Self::Unit,
            kernel_model::Value::Bool(value) => Self::Bool(value),
            kernel_model::Value::I64(value) => Self::I64(value),
            kernel_model::Value::F64Bits(value) => Self::F64Bits(value),
            kernel_model::Value::Text(value) => Self::Text(value),
            kernel_model::Value::LiveEntityRef { entity_type, id } => {
                Self::LiveEntityRef(EntityRef {
                    entity_type: TypeId::new(entity_type.raw()),
                    id: id.raw(),
                })
            }
            kernel_model::Value::HistoricalEntityId { entity_type, id } => {
                Self::HistoricalEntityRef(EntityRef {
                    entity_type: TypeId::new(entity_type.raw()),
                    id: id.raw(),
                })
            }
            kernel_model::Value::Product(values) => Self::Product(
                values
                    .into_iter()
                    .map(|(key, value)| (FieldId::new(key.raw()), value.into()))
                    .collect(),
            ),
            kernel_model::Value::Option(value) => {
                Self::Option(value.map(|value| Box::new((*value).into())))
            }
            kernel_model::Value::Variant { tag, value } => Self::Variant {
                tag: VariantTagId::new(tag.raw()),
                value: Box::new((*value).into()),
            },
            kernel_model::Value::Seq(values) => {
                Self::Seq(values.into_iter().map(Into::into).collect())
            }
            kernel_model::Value::Set {
                equivalence,
                elements,
            } => Self::Set {
                equivalence: EquivalenceId::new(equivalence.raw()),
                elements: elements.into_iter().map(Into::into).collect(),
            },
            kernel_model::Value::Bag {
                equivalence,
                entries,
            } => Self::Bag {
                equivalence: EquivalenceId::new(equivalence.raw()),
                entries: entries
                    .into_iter()
                    .map(|(value, count)| (value.into(), count))
                    .collect(),
            },
            kernel_model::Value::Map {
                key_equivalence,
                entries,
            } => Self::Map {
                key_equivalence: EquivalenceId::new(key_equivalence.raw()),
                entries: entries
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
            },
        }
    }
}
