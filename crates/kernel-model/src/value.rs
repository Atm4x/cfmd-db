use std::collections::{BTreeMap, BTreeSet};

use kernel_types::{EntityId, SemanticId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Value {
    Unit,
    Bool(bool),
    I64(i64),
    F64Bits(u64),
    Text(String),
    LiveEntityRef {
        entity_type: SemanticId,
        id: EntityId,
    },
    HistoricalEntityId {
        entity_type: SemanticId,
        id: EntityId,
    },
    Product(BTreeMap<SemanticId, Self>),
    Option(Option<Box<Self>>),
    Variant {
        tag: SemanticId,
        value: Box<Self>,
    },
    Seq(Vec<Self>),
    Set {
        equivalence: SemanticId,
        elements: Vec<Self>,
    },
    Bag {
        equivalence: SemanticId,
        entries: Vec<(Self, u64)>,
    },
    Map {
        key_equivalence: SemanticId,
        entries: Vec<(Self, Self)>,
    },
}

impl Value {
    #[must_use]
    pub fn contains_live_ref(&self) -> bool {
        match self {
            Self::LiveEntityRef { .. } => true,
            Self::HistoricalEntityId { .. }
            | Self::Unit
            | Self::Bool(_)
            | Self::I64(_)
            | Self::F64Bits(_)
            | Self::Text(_) => false,
            Self::Product(values) => values.values().any(Self::contains_live_ref),
            Self::Seq(values)
            | Self::Set {
                elements: values, ..
            } => values.iter().any(Self::contains_live_ref),
            Self::Variant { value, .. } => value.contains_live_ref(),
            Self::Option(value) => value.as_deref().is_some_and(Self::contains_live_ref),
            Self::Bag { entries, .. } => entries.iter().any(|(value, _)| value.contains_live_ref()),
            Self::Map { entries, .. } => entries
                .iter()
                .any(|(key, value)| key.contains_live_ref() || value.contains_live_ref()),
        }
    }

    #[must_use]
    pub fn first_dangling_live_ref(&self, live: &BTreeSet<EntityId>) -> Option<EntityId> {
        match self {
            Self::LiveEntityRef { id, .. } => (!live.contains(id)).then_some(*id),
            Self::HistoricalEntityId { .. }
            | Self::Unit
            | Self::Bool(_)
            | Self::I64(_)
            | Self::F64Bits(_)
            | Self::Text(_) => None,
            Self::Product(values) => values
                .values()
                .find_map(|value| value.first_dangling_live_ref(live)),
            Self::Seq(values) => values
                .iter()
                .find_map(|value| value.first_dangling_live_ref(live)),
            Self::Variant { value, .. } => value.first_dangling_live_ref(live),
            Self::Option(value) => value
                .as_deref()
                .and_then(|value| value.first_dangling_live_ref(live)),
            Self::Set { elements, .. } => elements
                .iter()
                .find_map(|value| value.first_dangling_live_ref(live)),
            Self::Bag { entries, .. } => entries
                .iter()
                .find_map(|(value, _)| value.first_dangling_live_ref(live)),
            Self::Map { entries, .. } => entries.iter().find_map(|(key, value)| {
                key.first_dangling_live_ref(live)
                    .or_else(|| value.first_dangling_live_ref(live))
            }),
        }
    }

    pub(super) fn collect_live_refs(&self, output: &mut Vec<EntityId>) {
        match self {
            Self::LiveEntityRef { id, .. } => output.push(*id),
            Self::HistoricalEntityId { .. }
            | Self::Unit
            | Self::Bool(_)
            | Self::I64(_)
            | Self::F64Bits(_)
            | Self::Text(_) => {}
            Self::Product(values) => {
                for value in values.values() {
                    value.collect_live_refs(output);
                }
            }
            Self::Seq(values)
            | Self::Set {
                elements: values, ..
            } => {
                for value in values {
                    value.collect_live_refs(output);
                }
            }
            Self::Variant { value, .. } => value.collect_live_refs(output),
            Self::Option(value) => {
                if let Some(value) = value.as_deref() {
                    value.collect_live_refs(output);
                }
            }
            Self::Bag { entries, .. } => {
                for (value, _) in entries {
                    value.collect_live_refs(output);
                }
            }
            Self::Map { entries, .. } => {
                for (key, value) in entries {
                    key.collect_live_refs(output);
                    value.collect_live_refs(output);
                }
            }
        }
    }
}
