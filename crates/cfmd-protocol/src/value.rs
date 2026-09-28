use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityRef {
    pub entity_type: u128,
    pub id: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolValue {
    Unit,
    Bool(bool),
    I64(i64),
    F64Bits(u64),
    Text(String),
    LiveEntityRef(EntityRef),
    HistoricalEntityRef(EntityRef),
    Product(BTreeMap<u128, Self>),
    Option(Option<Box<Self>>),
    Variant {
        tag: u128,
        value: Box<Self>,
    },
    Seq(Vec<Self>),
    Set {
        equivalence: u128,
        elements: Vec<Self>,
    },
    Bag {
        equivalence: u128,
        entries: Vec<(Self, u64)>,
    },
    Map {
        key_equivalence: u128,
        entries: Vec<(Self, Self)>,
    },
}

pub type Row = Vec<ProtocolValue>;

impl From<ProtocolValue> for cfmd_runtime::Value {
    fn from(value: ProtocolValue) -> Self {
        match value {
            ProtocolValue::Unit => Self::Unit,
            ProtocolValue::Bool(value) => Self::Bool(value),
            ProtocolValue::I64(value) => Self::I64(value),
            ProtocolValue::F64Bits(value) => Self::F64Bits(value),
            ProtocolValue::Text(value) => Self::Text(value),
            ProtocolValue::LiveEntityRef(value) => Self::LiveEntityRef(cfmd_runtime::EntityRef {
                entity_type: cfmd_runtime::TypeId::new(value.entity_type),
                id: value.id,
            }),
            ProtocolValue::HistoricalEntityRef(value) => {
                Self::HistoricalEntityRef(cfmd_runtime::EntityRef {
                    entity_type: cfmd_runtime::TypeId::new(value.entity_type),
                    id: value.id,
                })
            }
            ProtocolValue::Product(values) => Self::Product(
                values
                    .into_iter()
                    .map(|(field, value)| (cfmd_runtime::FieldId::new(field), value.into()))
                    .collect(),
            ),
            ProtocolValue::Option(value) => {
                Self::Option(value.map(|value| Box::new((*value).into())))
            }
            ProtocolValue::Variant { tag, value } => Self::Variant {
                tag: cfmd_runtime::VariantTagId::new(tag),
                value: Box::new((*value).into()),
            },
            ProtocolValue::Seq(values) => Self::Seq(values.into_iter().map(Into::into).collect()),
            ProtocolValue::Set {
                equivalence,
                elements,
            } => Self::Set {
                equivalence: cfmd_runtime::EquivalenceId::new(equivalence),
                elements: elements.into_iter().map(Into::into).collect(),
            },
            ProtocolValue::Bag {
                equivalence,
                entries,
            } => Self::Bag {
                equivalence: cfmd_runtime::EquivalenceId::new(equivalence),
                entries: entries
                    .into_iter()
                    .map(|(value, count)| (value.into(), count))
                    .collect(),
            },
            ProtocolValue::Map {
                key_equivalence,
                entries,
            } => Self::Map {
                key_equivalence: cfmd_runtime::EquivalenceId::new(key_equivalence),
                entries: entries
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
            },
        }
    }
}

impl From<cfmd_runtime::Value> for ProtocolValue {
    fn from(value: cfmd_runtime::Value) -> Self {
        match value {
            cfmd_runtime::Value::Unit => Self::Unit,
            cfmd_runtime::Value::Bool(value) => Self::Bool(value),
            cfmd_runtime::Value::I64(value) => Self::I64(value),
            cfmd_runtime::Value::F64Bits(value) => Self::F64Bits(value),
            cfmd_runtime::Value::Text(value) => Self::Text(value),
            cfmd_runtime::Value::LiveEntityRef(value) => Self::LiveEntityRef(EntityRef {
                entity_type: value.entity_type.raw(),
                id: value.id,
            }),
            cfmd_runtime::Value::HistoricalEntityRef(value) => {
                Self::HistoricalEntityRef(EntityRef {
                    entity_type: value.entity_type.raw(),
                    id: value.id,
                })
            }
            cfmd_runtime::Value::Product(values) => Self::Product(
                values
                    .into_iter()
                    .map(|(field, value)| (field.raw(), value.into()))
                    .collect(),
            ),
            cfmd_runtime::Value::Option(value) => {
                Self::Option(value.map(|value| Box::new((*value).into())))
            }
            cfmd_runtime::Value::Variant { tag, value } => Self::Variant {
                tag: tag.raw(),
                value: Box::new((*value).into()),
            },
            cfmd_runtime::Value::Seq(values) => {
                Self::Seq(values.into_iter().map(Into::into).collect())
            }
            cfmd_runtime::Value::Set {
                equivalence,
                elements,
            } => Self::Set {
                equivalence: equivalence.raw(),
                elements: elements.into_iter().map(Into::into).collect(),
            },
            cfmd_runtime::Value::Bag {
                equivalence,
                entries,
            } => Self::Bag {
                equivalence: equivalence.raw(),
                entries: entries
                    .into_iter()
                    .map(|(value, count)| (value.into(), count))
                    .collect(),
            },
            cfmd_runtime::Value::Map {
                key_equivalence,
                entries,
            } => Self::Map {
                key_equivalence: key_equivalence.raw(),
                entries: entries
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
            },
            _ => unreachable!("non-exhaustive runtime value variant requires protocol update"),
        }
    }
}
