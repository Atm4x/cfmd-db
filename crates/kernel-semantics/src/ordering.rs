use std::cmp::Ordering as CmpOrdering;
use std::collections::{BTreeMap, BTreeSet};

use kernel_model::Value;
use kernel_schema::{
    ModuleDigest, SemanticContext, StructuralEquivalenceDef, StructuralOrderingDef,
};
use kernel_types::SemanticId;

use crate::canonical_key::{FiniteMeasure, finite_measure_from_atoms};
use crate::contracts::OrderingCompatibilitySpec;
use crate::equivalence::EquivalenceDomain;
use crate::error::SemanticError;
use crate::module_digest::typed_entity_digest;
use crate::registry::SemanticRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OrderingModule {
    UnitExact,
    BoolAscending,
    I64Ascending,
    F64Total,
    TextBinary,
    TextAsciiCaseInsensitive,
    TextAsciiCaseInsensitiveThenBinary,
    LiveEntityIdAscending(SemanticId),
    HistoricalEntityIdAscending(SemanticId),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CanonicalOrderKey {
    Unit,
    Bool(bool),
    I64(i64),
    F64Total(i64),
    TextBinary(String),
    TextAsciiCaseInsensitive(String),
    TextAsciiCaseInsensitiveThenBinary(String, String),
    LiveEntityId {
        entity_type: SemanticId,
        id: kernel_types::EntityId,
    },
    HistoricalEntityId {
        entity_type: SemanticId,
        id: kernel_types::EntityId,
    },
    Product(Vec<Self>),
    Option {
        rank: u8,
        value: Option<Box<Self>>,
    },
    Variant {
        rank: u32,
        value: Box<Self>,
    },
    Seq(Vec<Self>),
    Set(FiniteMeasure<Self>),
    Bag(FiniteMeasure<CanonicalOrderBagAtom>),
    Map(FiniteMeasure<CanonicalOrderMapAtom>),
}

pub type CanonicalOrderClassKey = CanonicalOrderKey;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalOrderBagAtom {
    pub value: CanonicalOrderKey,
    pub stored_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalOrderMapAtom {
    pub key: CanonicalOrderKey,
    pub value: CanonicalOrderKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPrimitiveOrdering {
    ordering: SemanticId,
    module_digest: ModuleDigest,
    contract: OrderingModule,
}

impl ResolvedPrimitiveOrdering {
    const fn new(
        ordering: SemanticId,
        module_digest: ModuleDigest,
        contract: OrderingModule,
    ) -> Self {
        Self {
            ordering,
            module_digest,
            contract,
        }
    }

    #[must_use]
    pub const fn ordering(&self) -> SemanticId {
        self.ordering
    }

    #[must_use]
    pub const fn module_digest(&self) -> ModuleDigest {
        self.module_digest
    }

    pub fn compare(&self, left: &Value, right: &Value) -> Result<CmpOrdering, SemanticError> {
        self.contract.compare(left, right, self.ordering)
    }

    pub fn canonical_key(&self, value: &Value) -> Result<CanonicalOrderKey, SemanticError> {
        let key = match (self.contract, value) {
            (OrderingModule::UnitExact, Value::Unit) => CanonicalOrderKey::Unit,
            (OrderingModule::BoolAscending, Value::Bool(value)) => CanonicalOrderKey::Bool(*value),
            (OrderingModule::I64Ascending, Value::I64(value)) => CanonicalOrderKey::I64(*value),
            (OrderingModule::F64Total, Value::F64Bits(bits)) => {
                let signed = bits.cast_signed();
                let sortable = signed ^ ((signed >> 63).cast_unsigned() >> 1).cast_signed();
                CanonicalOrderKey::F64Total(sortable)
            }
            (OrderingModule::TextBinary, Value::Text(value)) => {
                CanonicalOrderKey::TextBinary(value.clone())
            }
            (OrderingModule::TextAsciiCaseInsensitive, Value::Text(value)) => {
                CanonicalOrderKey::TextAsciiCaseInsensitive(value.to_ascii_lowercase())
            }
            (OrderingModule::TextAsciiCaseInsensitiveThenBinary, Value::Text(value)) => {
                CanonicalOrderKey::TextAsciiCaseInsensitiveThenBinary(
                    value.to_ascii_lowercase(),
                    value.clone(),
                )
            }
            (
                OrderingModule::LiveEntityIdAscending(expected),
                Value::LiveEntityRef { entity_type, id },
            ) if entity_type == &expected => CanonicalOrderKey::LiveEntityId {
                entity_type: expected,
                id: *id,
            },
            (
                OrderingModule::HistoricalEntityIdAscending(expected),
                Value::HistoricalEntityId { entity_type, id },
            ) if entity_type == &expected => CanonicalOrderKey::HistoricalEntityId {
                entity_type: expected,
                id: *id,
            },
            _ => return Err(SemanticError::TypeMismatch(self.ordering)),
        };
        Ok(key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderingDomain {
    Unit,
    Bool,
    I64,
    F64,
    Text,
    LiveEntityRef(SemanticId),
    HistoricalEntityId(SemanticId),
    Product(BTreeMap<SemanticId, Self>),
    Option(Box<Self>),
    Sum(BTreeMap<SemanticId, Self>),
    Set(Box<Self>),
    Bag(Box<Self>),
    Seq(Box<Self>),
    Map { key: Box<Self>, value: Box<Self> },
    Mu(Box<Self>),
    Var(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledOrdering {
    ordering: SemanticId,
    domain: OrderingDomain,
    root: usize,
    nodes: Vec<CompiledOrderingNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompiledOrderingNode {
    ordering: SemanticId,
    kind: CompiledOrderingKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompiledOrderingKind {
    Primitive(ResolvedPrimitiveOrdering),
    Mu { body: usize },
    Var { binder: usize },
    Product(Vec<(SemanticId, usize)>),
    Option { inner: usize, none_first: bool },
    Sum(BTreeMap<SemanticId, (u32, usize)>),
    Set { element: usize },
    Bag { element: usize },
    Seq { element: usize },
    Map { key: usize, value: usize },
}

struct OrderingCompileState<'a> {
    context: &'a SemanticContext,
    nodes: &'a mut Vec<CompiledOrderingNode>,
    stack: &'a mut BTreeSet<SemanticId>,
    binders: &'a mut Vec<(SemanticId, usize, usize)>,
}

impl CompiledOrdering {
    #[must_use]
    pub const fn ordering(&self) -> SemanticId {
        self.ordering
    }

    #[must_use]
    pub const fn domain(&self) -> &OrderingDomain {
        &self.domain
    }

    pub fn compare(&self, left: &Value, right: &Value) -> Result<CmpOrdering, SemanticError> {
        Ok(self.canonical_key(left)?.cmp(&self.canonical_key(right)?))
    }

    pub fn canonical_key(&self, value: &Value) -> Result<CanonicalOrderKey, SemanticError> {
        self.canonical_key_at(self.root, value)
    }

    fn canonical_key_at(
        &self,
        node_index: usize,
        value: &Value,
    ) -> Result<CanonicalOrderKey, SemanticError> {
        let node = &self.nodes[node_index];
        match (&node.kind, value) {
            (CompiledOrderingKind::Primitive(resolved), _) => resolved.canonical_key(value),
            (CompiledOrderingKind::Mu { body }, _) => self.canonical_key_at(*body, value),
            (CompiledOrderingKind::Var { binder }, _) => self.canonical_key_at(*binder, value),
            (CompiledOrderingKind::Product(fields), Value::Product(values)) => {
                if fields.len() != values.len() {
                    return Err(SemanticError::TypeMismatch(node.ordering));
                }
                fields
                    .iter()
                    .map(|(field, child)| {
                        let value = values
                            .get(field)
                            .ok_or(SemanticError::TypeMismatch(node.ordering))?;
                        self.canonical_key_at(*child, value)
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(CanonicalOrderKey::Product)
            }
            (CompiledOrderingKind::Option { inner, none_first }, Value::Option(value)) => {
                match value.as_deref() {
                    None => Ok(CanonicalOrderKey::Option {
                        rank: u8::from(!*none_first),
                        value: None,
                    }),
                    Some(value) => Ok(CanonicalOrderKey::Option {
                        rank: u8::from(*none_first),
                        value: Some(Box::new(self.canonical_key_at(*inner, value)?)),
                    }),
                }
            }
            (CompiledOrderingKind::Sum(variants), Value::Variant { tag, value }) => {
                let (rank, child) = variants
                    .get(tag)
                    .copied()
                    .ok_or(SemanticError::TypeMismatch(node.ordering))?;
                Ok(CanonicalOrderKey::Variant {
                    rank,
                    value: Box::new(self.canonical_key_at(child, value)?),
                })
            }
            (CompiledOrderingKind::Seq { element }, Value::Seq(values)) => values
                .iter()
                .map(|value| self.canonical_key_at(*element, value))
                .collect::<Result<Vec<_>, _>>()
                .map(CanonicalOrderKey::Seq),
            (CompiledOrderingKind::Set { element }, Value::Set { elements, .. }) => {
                let atoms = elements
                    .iter()
                    .map(|value| self.canonical_key_at(*element, value))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(CanonicalOrderKey::Set(finite_measure_from_atoms(atoms)))
            }
            (CompiledOrderingKind::Bag { element }, Value::Bag { entries, .. }) => {
                let atoms = entries
                    .iter()
                    .map(|(value, stored_count)| {
                        Ok(CanonicalOrderBagAtom {
                            value: self.canonical_key_at(*element, value)?,
                            stored_count: *stored_count,
                        })
                    })
                    .collect::<Result<Vec<_>, SemanticError>>()?;
                Ok(CanonicalOrderKey::Bag(finite_measure_from_atoms(atoms)))
            }
            (CompiledOrderingKind::Map { key, value }, Value::Map { entries, .. }) => {
                let atoms = entries
                    .iter()
                    .map(|(entry_key, entry_value)| {
                        Ok(CanonicalOrderMapAtom {
                            key: self.canonical_key_at(*key, entry_key)?,
                            value: self.canonical_key_at(*value, entry_value)?,
                        })
                    })
                    .collect::<Result<Vec<_>, SemanticError>>()?;
                Ok(CanonicalOrderKey::Map(finite_measure_from_atoms(atoms)))
            }
            _ => Err(SemanticError::TypeMismatch(node.ordering)),
        }
    }
}

impl From<EquivalenceDomain> for OrderingDomain {
    fn from(domain: EquivalenceDomain) -> Self {
        match domain {
            EquivalenceDomain::Unit => Self::Unit,
            EquivalenceDomain::Bool => Self::Bool,
            EquivalenceDomain::I64 => Self::I64,
            EquivalenceDomain::F64 => Self::F64,
            EquivalenceDomain::Text => Self::Text,
            EquivalenceDomain::LiveEntityRef(entity_type) => Self::LiveEntityRef(entity_type),
            EquivalenceDomain::HistoricalEntityId(entity_type) => {
                Self::HistoricalEntityId(entity_type)
            }
            EquivalenceDomain::Product(fields) => Self::Product(
                fields
                    .into_iter()
                    .map(|(field, child)| (field, Self::from(child)))
                    .collect(),
            ),
            EquivalenceDomain::Option(inner) => Self::Option(Box::new(Self::from(*inner))),
            EquivalenceDomain::Sum(variants) => Self::Sum(
                variants
                    .into_iter()
                    .map(|(tag, child)| (tag, Self::from(child)))
                    .collect(),
            ),
            EquivalenceDomain::Set(element) => Self::Set(Box::new(Self::from(*element))),
            EquivalenceDomain::Bag(element) => Self::Bag(Box::new(Self::from(*element))),
            EquivalenceDomain::Seq(element) => Self::Seq(Box::new(Self::from(*element))),
            EquivalenceDomain::Map { key, value } => Self::Map {
                key: Box::new(Self::from(*key)),
                value: Box::new(Self::from(*value)),
            },
            EquivalenceDomain::Mu(body) => Self::Mu(Box::new(Self::from(*body))),
            EquivalenceDomain::Var(distance) => Self::Var(distance),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OrderingImplementation {
    pub(super) contract: OrderingModule,
    pub(super) implementation_revision: u64,
}

impl OrderingModule {
    #[must_use]
    pub(super) fn implementation_digest(self, implementation_revision: u64) -> ModuleDigest {
        OrderingImplementation {
            contract: self,
            implementation_revision,
        }
        .digest()
    }

    #[must_use]
    pub fn domain(self) -> OrderingDomain {
        match self {
            Self::UnitExact => OrderingDomain::Unit,
            Self::BoolAscending => OrderingDomain::Bool,
            Self::I64Ascending => OrderingDomain::I64,
            Self::F64Total => OrderingDomain::F64,
            Self::TextBinary
            | Self::TextAsciiCaseInsensitive
            | Self::TextAsciiCaseInsensitiveThenBinary => OrderingDomain::Text,
            Self::LiveEntityIdAscending(entity_type) => OrderingDomain::LiveEntityRef(entity_type),
            Self::HistoricalEntityIdAscending(entity_type) => {
                OrderingDomain::HistoricalEntityId(entity_type)
            }
        }
    }

    #[must_use]
    pub fn digest(self) -> ModuleDigest {
        match self {
            Self::UnitExact => ModuleDigest([36; 32]),
            Self::BoolAscending => ModuleDigest([37; 32]),
            Self::I64Ascending => ModuleDigest([31; 32]),
            Self::F64Total => ModuleDigest([35; 32]),
            Self::TextBinary => ModuleDigest([32; 32]),
            Self::TextAsciiCaseInsensitiveThenBinary => ModuleDigest([33; 32]),
            Self::TextAsciiCaseInsensitive => ModuleDigest([34; 32]),
            Self::LiveEntityIdAscending(entity_type) => typed_entity_digest(38, entity_type),
            Self::HistoricalEntityIdAscending(entity_type) => typed_entity_digest(39, entity_type),
        }
    }

    pub(super) fn compare(
        self,
        left: &Value,
        right: &Value,
        ordering: SemanticId,
    ) -> Result<CmpOrdering, SemanticError> {
        match (self, left, right) {
            (Self::UnitExact, Value::Unit, Value::Unit) => Ok(CmpOrdering::Equal),
            (Self::BoolAscending, Value::Bool(left), Value::Bool(right)) => Ok(left.cmp(right)),
            (Self::I64Ascending, Value::I64(left), Value::I64(right)) => Ok(left.cmp(right)),
            (Self::F64Total, Value::F64Bits(left), Value::F64Bits(right)) => {
                Ok(f64::from_bits(*left).total_cmp(&f64::from_bits(*right)))
            }
            (Self::TextBinary, Value::Text(left), Value::Text(right)) => Ok(left.cmp(right)),
            (Self::TextAsciiCaseInsensitive, Value::Text(left), Value::Text(right)) => {
                Ok(left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase()))
            }
            (Self::TextAsciiCaseInsensitiveThenBinary, Value::Text(left), Value::Text(right)) => {
                let left_folded = left.to_ascii_lowercase();
                let right_folded = right.to_ascii_lowercase();
                Ok(left_folded.cmp(&right_folded).then_with(|| left.cmp(right)))
            }
            (
                Self::LiveEntityIdAscending(expected),
                Value::LiveEntityRef {
                    entity_type: left_type,
                    id: left,
                },
                Value::LiveEntityRef {
                    entity_type: right_type,
                    id: right,
                },
            ) if left_type == &expected && right_type == &expected => Ok(left.cmp(right)),
            (
                Self::HistoricalEntityIdAscending(expected),
                Value::HistoricalEntityId {
                    entity_type: left_type,
                    id: left,
                },
                Value::HistoricalEntityId {
                    entity_type: right_type,
                    id: right,
                },
            ) if left_type == &expected && right_type == &expected => Ok(left.cmp(right)),
            _ => Err(SemanticError::TypeMismatch(ordering)),
        }
    }
}

impl OrderingImplementation {
    #[must_use]
    pub(super) fn digest(self) -> ModuleDigest {
        let mut digest = self.contract.digest().0;
        let revision = self.implementation_revision.to_le_bytes();
        for (index, byte) in revision.into_iter().enumerate() {
            digest[24 + index] ^= byte;
        }
        ModuleDigest(digest)
    }
}

fn structural_ordering_children(definition: &StructuralOrderingDef) -> Vec<SemanticId> {
    match definition {
        StructuralOrderingDef::Mu { body } => vec![*body],
        StructuralOrderingDef::Var { .. } => Vec::new(),
        StructuralOrderingDef::Product { fields }
        | StructuralOrderingDef::Sum { variants: fields } => {
            fields.iter().map(|(_, child)| *child).collect()
        }
        StructuralOrderingDef::Option { inner, .. } => vec![*inner],
        StructuralOrderingDef::Set { element }
        | StructuralOrderingDef::Bag { element }
        | StructuralOrderingDef::Seq { element } => vec![*element],
        StructuralOrderingDef::Map { key, value } => vec![*key, *value],
    }
}

impl SemanticRegistry {
    pub(super) fn validate_structural_ordering_graph(
        &self,
        context: &SemanticContext,
    ) -> Result<(), SemanticError> {
        let definitions: BTreeMap<_, _> = context.schema.structural_orderings().collect();
        let mut referenced = BTreeSet::new();
        for definition in definitions.values() {
            referenced.extend(structural_ordering_children(definition));
        }
        let roots: Vec<_> = definitions
            .keys()
            .copied()
            .filter(|id| !referenced.contains(id))
            .collect();
        let mut reachable = BTreeSet::new();
        let mut pending = roots.clone();
        while let Some(id) = pending.pop() {
            if !reachable.insert(id) {
                continue;
            }
            if let Some(definition) = definitions.get(&id) {
                match definition {
                    StructuralOrderingDef::Var { binder } => pending.push(*binder),
                    _ => pending.extend(structural_ordering_children(definition)),
                }
            }
        }
        if let Some(unrooted) = definitions.keys().find(|id| !reachable.contains(id)) {
            return Err(SemanticError::CyclicStructuralOrdering(*unrooted));
        }
        for root in roots {
            self.ordering_domain(context, root)?;
        }
        Ok(())
    }

    pub fn compile_ordering(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
    ) -> Result<CompiledOrdering, SemanticError> {
        let domain = self.ordering_domain(context, ordering)?;
        let mut nodes = Vec::new();
        let mut stack = BTreeSet::new();
        let mut binders = Vec::new();
        let root = self.compile_ordering_node(
            ordering,
            &mut OrderingCompileState {
                context,
                nodes: &mut nodes,
                stack: &mut stack,
                binders: &mut binders,
            },
            0,
        )?;
        Ok(CompiledOrdering {
            ordering,
            domain,
            root,
            nodes,
        })
    }

    fn compile_ordering_node(
        &self,
        ordering: SemanticId,
        state: &mut OrderingCompileState<'_>,
        constructor_depth: usize,
    ) -> Result<usize, SemanticError> {
        let Some(definition) = state.context.schema.structural_ordering(ordering) else {
            let resolved = self.resolve_primitive_ordering(state.context, ordering)?;
            let index = state.nodes.len();
            state.nodes.push(CompiledOrderingNode {
                ordering,
                kind: CompiledOrderingKind::Primitive(resolved),
            });
            return Ok(index);
        };
        if let StructuralOrderingDef::Var { binder } = definition {
            let Some((_, binder_node, binder_depth)) = state
                .binders
                .iter()
                .rev()
                .find(|(candidate, _, _)| candidate == binder)
            else {
                return Err(SemanticError::FreeStructuralRecursion(*binder));
            };
            if constructor_depth <= *binder_depth {
                return Err(SemanticError::UnguardedStructuralRecursion(*binder));
            }
            let index = state.nodes.len();
            state.nodes.push(CompiledOrderingNode {
                ordering,
                kind: CompiledOrderingKind::Var {
                    binder: *binder_node,
                },
            });
            return Ok(index);
        }
        if !state.stack.insert(ordering) {
            return Err(SemanticError::CyclicStructuralOrdering(ordering));
        }
        let index = state.nodes.len();
        state.nodes.push(CompiledOrderingNode {
            ordering,
            kind: CompiledOrderingKind::Mu { body: usize::MAX },
        });
        let kind = match definition {
            StructuralOrderingDef::Mu { body } => {
                state.binders.push((ordering, index, constructor_depth));
                let body = self.compile_ordering_node(*body, state, constructor_depth)?;
                state.binders.pop();
                CompiledOrderingKind::Mu { body }
            }
            StructuralOrderingDef::Var { .. } => unreachable!(),
            StructuralOrderingDef::Product { fields } => {
                let mut compiled = Vec::with_capacity(fields.len());
                for (field, child) in fields {
                    compiled.push((
                        *field,
                        self.compile_ordering_node(*child, state, constructor_depth + 1)?,
                    ));
                }
                CompiledOrderingKind::Product(compiled)
            }
            StructuralOrderingDef::Option { inner, none_first } => CompiledOrderingKind::Option {
                inner: self.compile_ordering_node(*inner, state, constructor_depth + 1)?,
                none_first: *none_first,
            },
            StructuralOrderingDef::Sum { variants } => {
                let mut compiled = BTreeMap::new();
                for (rank, (tag, child)) in variants.iter().enumerate() {
                    let rank =
                        u32::try_from(rank).map_err(|_| SemanticError::TypeMismatch(ordering))?;
                    let child = self.compile_ordering_node(*child, state, constructor_depth + 1)?;
                    if compiled.insert(*tag, (rank, child)).is_some() {
                        return Err(SemanticError::TypeMismatch(*tag));
                    }
                }
                CompiledOrderingKind::Sum(compiled)
            }
            StructuralOrderingDef::Set { element } => CompiledOrderingKind::Set {
                element: self.compile_ordering_node(*element, state, constructor_depth + 1)?,
            },
            StructuralOrderingDef::Bag { element } => CompiledOrderingKind::Bag {
                element: self.compile_ordering_node(*element, state, constructor_depth + 1)?,
            },
            StructuralOrderingDef::Seq { element } => CompiledOrderingKind::Seq {
                element: self.compile_ordering_node(*element, state, constructor_depth + 1)?,
            },
            StructuralOrderingDef::Map { key, value } => CompiledOrderingKind::Map {
                key: self.compile_ordering_node(*key, state, constructor_depth + 1)?,
                value: self.compile_ordering_node(*value, state, constructor_depth + 1)?,
            },
        };
        state.nodes[index].kind = kind;
        state.stack.remove(&ordering);
        Ok(index)
    }

    pub fn compare(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
        left: &Value,
        right: &Value,
    ) -> Result<CmpOrdering, SemanticError> {
        if context.schema.structural_ordering(ordering).is_some() {
            return Ok(self
                .canonical_order_key(context, ordering, left)?
                .cmp(&self.canonical_order_key(context, ordering, right)?));
        }
        let digest = context
            .environment
            .module(ordering)
            .ok_or(SemanticError::WrongModuleKind(ordering))?;
        let implementation = self
            .ordering_implementation(digest)
            .ok_or(SemanticError::WrongModuleKind(ordering))?;
        implementation.contract.compare(left, right, ordering)
    }

    pub fn ordering_domain(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
    ) -> Result<OrderingDomain, SemanticError> {
        if context.schema.structural_ordering(ordering).is_some() {
            return self.ordering_domain_inner(
                context,
                ordering,
                &mut BTreeSet::new(),
                &mut Vec::new(),
                0,
            );
        }
        let digest = context
            .environment
            .module(ordering)
            .ok_or(SemanticError::WrongModuleKind(ordering))?;
        let implementation = self
            .ordering_implementation(digest)
            .ok_or(SemanticError::WrongModuleKind(ordering))?;
        Ok(implementation.contract.domain())
    }

    pub fn ordering_congruent_with_equivalence(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
        equivalence: SemanticId,
    ) -> Result<bool, SemanticError> {
        if context.schema.structural_ordering(ordering).is_some() {
            return self.structural_ordering_congruent_with_equivalence(
                context,
                ordering,
                equivalence,
                &mut BTreeSet::new(),
            );
        }
        let ordering = self.ordering_contract(context, ordering)?;
        let equivalence = self.equivalence_contract(context, equivalence)?;
        Ok(
            self.ordering_compatibility_installed(&OrderingCompatibilitySpec {
                ordering,
                equivalence,
            }),
        )
    }

    pub fn canonical_order_key(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
        value: &Value,
    ) -> Result<CanonicalOrderClassKey, SemanticError> {
        let Some(definition) = context.schema.structural_ordering(ordering) else {
            return self
                .resolve_primitive_ordering(context, ordering)?
                .canonical_key(value);
        };
        self.canonical_structural_order_key(context, ordering, definition, value)
    }

    fn canonical_structural_order_key(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
        definition: &StructuralOrderingDef,
        value: &Value,
    ) -> Result<CanonicalOrderClassKey, SemanticError> {
        match (definition, value) {
            (StructuralOrderingDef::Mu { body }, _) => {
                self.canonical_order_key(context, *body, value)
            }
            (StructuralOrderingDef::Var { binder }, _) => {
                self.canonical_order_key(context, *binder, value)
            }
            (StructuralOrderingDef::Product { fields }, Value::Product(values)) => {
                if values.len() != fields.len() {
                    return Err(SemanticError::TypeMismatch(ordering));
                }
                fields
                    .iter()
                    .map(|(field, child)| {
                        let value = values
                            .get(field)
                            .ok_or(SemanticError::TypeMismatch(ordering))?;
                        self.canonical_order_key(context, *child, value)
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(CanonicalOrderKey::Product)
            }
            (StructuralOrderingDef::Option { inner, none_first }, Value::Option(value)) => {
                match value.as_deref() {
                    None => Ok(CanonicalOrderKey::Option {
                        rank: u8::from(!*none_first),
                        value: None,
                    }),
                    Some(value) => Ok(CanonicalOrderKey::Option {
                        rank: u8::from(*none_first),
                        value: Some(Box::new(self.canonical_order_key(context, *inner, value)?)),
                    }),
                }
            }
            (StructuralOrderingDef::Sum { variants }, Value::Variant { tag, value }) => {
                let (rank, (_, child)) = variants
                    .iter()
                    .enumerate()
                    .find(|(_, (candidate, _))| candidate == tag)
                    .ok_or(SemanticError::TypeMismatch(ordering))?;
                let rank =
                    u32::try_from(rank).map_err(|_| SemanticError::TypeMismatch(ordering))?;
                Ok(CanonicalOrderKey::Variant {
                    rank,
                    value: Box::new(self.canonical_order_key(context, *child, value)?),
                })
            }
            (StructuralOrderingDef::Seq { element }, Value::Seq(values)) => values
                .iter()
                .map(|value| self.canonical_order_key(context, *element, value))
                .collect::<Result<Vec<_>, _>>()
                .map(CanonicalOrderKey::Seq),
            (StructuralOrderingDef::Set { element }, Value::Set { elements, .. }) => {
                let atoms = elements
                    .iter()
                    .map(|value| self.canonical_order_key(context, *element, value))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(CanonicalOrderKey::Set(finite_measure_from_atoms(atoms)))
            }
            (StructuralOrderingDef::Bag { element }, Value::Bag { entries, .. }) => {
                let atoms = entries
                    .iter()
                    .map(|(value, stored_count)| {
                        Ok(CanonicalOrderBagAtom {
                            value: self.canonical_order_key(context, *element, value)?,
                            stored_count: *stored_count,
                        })
                    })
                    .collect::<Result<Vec<_>, SemanticError>>()?;
                Ok(CanonicalOrderKey::Bag(finite_measure_from_atoms(atoms)))
            }
            (StructuralOrderingDef::Map { key, value }, Value::Map { entries, .. }) => {
                let atoms = entries
                    .iter()
                    .map(|(entry_key, entry_value)| {
                        Ok(CanonicalOrderMapAtom {
                            key: self.canonical_order_key(context, *key, entry_key)?,
                            value: self.canonical_order_key(context, *value, entry_value)?,
                        })
                    })
                    .collect::<Result<Vec<_>, SemanticError>>()?;
                Ok(CanonicalOrderKey::Map(finite_measure_from_atoms(atoms)))
            }
            _ => Err(SemanticError::TypeMismatch(ordering)),
        }
    }

    fn ordering_domain_inner(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
        stack: &mut BTreeSet<SemanticId>,
        binders: &mut Vec<(SemanticId, usize)>,
        constructor_depth: usize,
    ) -> Result<OrderingDomain, SemanticError> {
        if let Some(definition) = context.schema.structural_ordering(ordering) {
            return self.structural_ordering_domain(
                context,
                ordering,
                definition,
                stack,
                binders,
                constructor_depth,
            );
        }
        self.primitive_ordering_domain(context, ordering)
    }

    fn primitive_ordering_domain(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
    ) -> Result<OrderingDomain, SemanticError> {
        let digest = context
            .environment
            .module(ordering)
            .ok_or(SemanticError::WrongModuleKind(ordering))?;
        match self.ordering_implementation(digest) {
            Some(implementation) => Ok(implementation.contract.domain()),
            None if self.module_available(digest) => Err(SemanticError::WrongModuleKind(ordering)),
            None => Err(SemanticError::ModuleUnavailable(digest)),
        }
    }

    fn structural_ordering_domain(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
        definition: &StructuralOrderingDef,
        stack: &mut BTreeSet<SemanticId>,
        binders: &mut Vec<(SemanticId, usize)>,
        constructor_depth: usize,
    ) -> Result<OrderingDomain, SemanticError> {
        if let StructuralOrderingDef::Var { binder } = definition {
            let Some((distance, (_, binder_depth))) = binders
                .iter()
                .rev()
                .enumerate()
                .find(|(_, (candidate, _))| candidate == binder)
            else {
                return Err(SemanticError::FreeStructuralRecursion(*binder));
            };
            if constructor_depth <= *binder_depth {
                return Err(SemanticError::UnguardedStructuralRecursion(*binder));
            }
            return Ok(OrderingDomain::Var(distance));
        }
        if !stack.insert(ordering) {
            return Err(SemanticError::CyclicStructuralOrdering(ordering));
        }
        let child_depth = constructor_depth + 1;
        let result = match definition {
            StructuralOrderingDef::Mu { body } => {
                binders.push((ordering, constructor_depth));
                let body =
                    self.ordering_domain_inner(context, *body, stack, binders, constructor_depth)?;
                binders.pop();
                Ok(OrderingDomain::Mu(Box::new(body)))
            }
            StructuralOrderingDef::Var { .. } => unreachable!(),
            StructuralOrderingDef::Product { fields } => self
                .ordering_domain_named_children(context, fields, stack, binders, child_depth)
                .map(OrderingDomain::Product),
            StructuralOrderingDef::Option { inner, .. } => self
                .ordering_domain_inner(context, *inner, stack, binders, child_depth)
                .map(Box::new)
                .map(OrderingDomain::Option),
            StructuralOrderingDef::Sum { variants } => self
                .ordering_domain_named_children(context, variants, stack, binders, child_depth)
                .map(OrderingDomain::Sum),
            StructuralOrderingDef::Set { element } => self
                .ordering_domain_inner(context, *element, stack, binders, child_depth)
                .map(Box::new)
                .map(OrderingDomain::Set),
            StructuralOrderingDef::Bag { element } => self
                .ordering_domain_inner(context, *element, stack, binders, child_depth)
                .map(Box::new)
                .map(OrderingDomain::Bag),
            StructuralOrderingDef::Seq { element } => self
                .ordering_domain_inner(context, *element, stack, binders, child_depth)
                .map(Box::new)
                .map(OrderingDomain::Seq),
            StructuralOrderingDef::Map { key, value } => Ok(OrderingDomain::Map {
                key: Box::new(self.ordering_domain_inner(
                    context,
                    *key,
                    stack,
                    binders,
                    child_depth,
                )?),
                value: Box::new(self.ordering_domain_inner(
                    context,
                    *value,
                    stack,
                    binders,
                    child_depth,
                )?),
            }),
        };
        stack.remove(&ordering);
        result
    }

    fn ordering_domain_named_children(
        &self,
        context: &SemanticContext,
        children: &[(SemanticId, SemanticId)],
        stack: &mut BTreeSet<SemanticId>,
        binders: &mut Vec<(SemanticId, usize)>,
        child_depth: usize,
    ) -> Result<BTreeMap<SemanticId, OrderingDomain>, SemanticError> {
        let mut result = BTreeMap::new();
        for (name, child) in children {
            if result.contains_key(name) {
                return Err(SemanticError::TypeMismatch(*name));
            }
            result.insert(
                *name,
                self.ordering_domain_inner(context, *child, stack, binders, child_depth)?,
            );
        }
        Ok(result)
    }

    fn structural_ordering_congruent_with_equivalence(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
        equivalence: SemanticId,
        visited: &mut BTreeSet<(SemanticId, SemanticId)>,
    ) -> Result<bool, SemanticError> {
        if !visited.insert((ordering, equivalence)) {
            return Ok(true);
        }
        let Some(ordering_def) = context.schema.structural_ordering(ordering) else {
            if context.schema.structural_equivalence(equivalence).is_some() {
                return Ok(false);
            }
            let ordering = self.ordering_contract(context, ordering)?;
            let equivalence = self.equivalence_contract(context, equivalence)?;
            return Ok(
                self.ordering_compatibility_installed(&OrderingCompatibilitySpec {
                    ordering,
                    equivalence,
                }),
            );
        };
        let Some(equivalence_def) = context.schema.structural_equivalence(equivalence) else {
            return Ok(false);
        };
        self.structural_ordering_defs_congruent(context, ordering_def, equivalence_def, visited)
    }

    fn structural_ordering_defs_congruent(
        &self,
        context: &SemanticContext,
        ordering_def: &StructuralOrderingDef,
        equivalence_def: &StructuralEquivalenceDef,
        visited: &mut BTreeSet<(SemanticId, SemanticId)>,
    ) -> Result<bool, SemanticError> {
        match (ordering_def, equivalence_def) {
            (
                StructuralOrderingDef::Mu { body: order_body },
                StructuralEquivalenceDef::Mu { body: eq_body },
            )
            | (
                StructuralOrderingDef::Var { binder: order_body },
                StructuralEquivalenceDef::Var { binder: eq_body },
            ) => self.structural_ordering_congruent_with_equivalence(
                context,
                *order_body,
                *eq_body,
                visited,
            ),
            (
                StructuralOrderingDef::Product { fields },
                StructuralEquivalenceDef::Product { fields: eq_fields },
            ) => self.ordered_children_congruent(context, fields, eq_fields, visited),
            (
                StructuralOrderingDef::Option { inner, .. },
                StructuralEquivalenceDef::Option { inner: eq_inner },
            ) => self.structural_ordering_congruent_with_equivalence(
                context, *inner, *eq_inner, visited,
            ),
            (
                StructuralOrderingDef::Sum { variants },
                StructuralEquivalenceDef::Sum {
                    variants: eq_variants,
                },
            ) => self.ordered_children_congruent(context, variants, eq_variants, visited),
            (
                StructuralOrderingDef::Set { element },
                StructuralEquivalenceDef::Set {
                    element: eq_element,
                },
            )
            | (
                StructuralOrderingDef::Bag { element },
                StructuralEquivalenceDef::Bag {
                    element: eq_element,
                },
            )
            | (
                StructuralOrderingDef::Seq { element },
                StructuralEquivalenceDef::Seq {
                    element: eq_element,
                },
            ) => self.structural_ordering_congruent_with_equivalence(
                context,
                *element,
                *eq_element,
                visited,
            ),
            (
                StructuralOrderingDef::Map { key, value },
                StructuralEquivalenceDef::Map {
                    key: eq_key,
                    value: eq_value,
                },
            ) => Ok(self
                .structural_ordering_congruent_with_equivalence(context, *key, *eq_key, visited)?
                && self.structural_ordering_congruent_with_equivalence(
                    context, *value, *eq_value, visited,
                )?),
            _ => Ok(false),
        }
    }

    fn ordered_children_congruent(
        &self,
        context: &SemanticContext,
        ordered_children: &[(SemanticId, SemanticId)],
        equivalence_children: &BTreeMap<SemanticId, SemanticId>,
        visited: &mut BTreeSet<(SemanticId, SemanticId)>,
    ) -> Result<bool, SemanticError> {
        if ordered_children.len() != equivalence_children.len() {
            return Ok(false);
        }
        let mut seen = BTreeSet::new();
        for (name, child_ordering) in ordered_children {
            if !seen.insert(*name) {
                return Ok(false);
            }
            let Some(child_equivalence) = equivalence_children.get(name) else {
                return Ok(false);
            };
            if !self.structural_ordering_congruent_with_equivalence(
                context,
                *child_ordering,
                *child_equivalence,
                visited,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn ordering_contract(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
    ) -> Result<OrderingModule, SemanticError> {
        let digest = context
            .environment
            .module(ordering)
            .ok_or(SemanticError::WrongModuleKind(ordering))?;
        match self.ordering_implementation(digest) {
            Some(implementation) => Ok(implementation.contract),
            None if self.module_available(digest) => Err(SemanticError::WrongModuleKind(ordering)),
            None => Err(SemanticError::ModuleUnavailable(digest)),
        }
    }

    pub fn resolve_primitive_ordering(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
    ) -> Result<ResolvedPrimitiveOrdering, SemanticError> {
        let digest = context
            .environment
            .module(ordering)
            .ok_or(SemanticError::WrongModuleKind(ordering))?;
        let implementation = match self.ordering_implementation(digest) {
            Some(implementation) => implementation,
            None if self.module_available(digest) => {
                return Err(SemanticError::WrongModuleKind(ordering));
            }
            None => return Err(SemanticError::ModuleUnavailable(digest)),
        };
        Ok(ResolvedPrimitiveOrdering::new(
            ordering,
            digest,
            implementation.contract,
        ))
    }
}
