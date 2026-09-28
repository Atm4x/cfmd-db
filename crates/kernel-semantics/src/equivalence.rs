use std::collections::{BTreeMap, BTreeSet};

use kernel_model::Value;
use kernel_schema::{
    ModuleDigest, ScalarType, SemanticContext, StructuralEquivalenceDef, TypeExpr,
};
use kernel_types::SemanticId;

use crate::canonical_key::{
    CanonicalBagAtom, CanonicalEqKey, CanonicalMapAtom, finite_measure_from_atoms,
};
use crate::error::SemanticError;
use crate::module_digest::typed_entity_digest;
use crate::registry::SemanticRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EquivalenceDomain {
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

#[must_use]
pub fn domain_for_type(ty: &TypeExpr) -> Option<EquivalenceDomain> {
    ty.validate().ok()?;
    domain_for_type_inner(ty, &mut Vec::new())
}

fn domain_for_type_inner(
    ty: &TypeExpr,
    binders: &mut Vec<kernel_schema::TypeVar>,
) -> Option<EquivalenceDomain> {
    match ty {
        TypeExpr::Scalar(ScalarType::Unit) => Some(EquivalenceDomain::Unit),
        TypeExpr::Scalar(ScalarType::Bool) => Some(EquivalenceDomain::Bool),
        TypeExpr::Scalar(ScalarType::I64) => Some(EquivalenceDomain::I64),
        TypeExpr::Scalar(ScalarType::F64) => Some(EquivalenceDomain::F64),
        TypeExpr::Scalar(ScalarType::Text) => Some(EquivalenceDomain::Text),
        TypeExpr::Scalar(ScalarType::LiveEntityRef(entity_type)) => {
            Some(EquivalenceDomain::LiveEntityRef(*entity_type))
        }
        TypeExpr::Scalar(ScalarType::HistoricalEntityId(entity_type)) => {
            Some(EquivalenceDomain::HistoricalEntityId(*entity_type))
        }
        TypeExpr::Product(fields) => fields
            .iter()
            .map(|(field, ty)| domain_for_type_inner(ty, binders).map(|domain| (*field, domain)))
            .collect::<Option<BTreeMap<_, _>>>()
            .map(EquivalenceDomain::Product),
        TypeExpr::Option(inner) => domain_for_type_inner(inner, binders)
            .map(Box::new)
            .map(EquivalenceDomain::Option),
        TypeExpr::Sum(variants) => variants
            .iter()
            .map(|(tag, ty)| domain_for_type_inner(ty, binders).map(|domain| (*tag, domain)))
            .collect::<Option<BTreeMap<_, _>>>()
            .map(EquivalenceDomain::Sum),
        TypeExpr::Set { element, .. } => domain_for_type_inner(element, binders)
            .map(Box::new)
            .map(EquivalenceDomain::Set),
        TypeExpr::Bag { element, .. } => domain_for_type_inner(element, binders)
            .map(Box::new)
            .map(EquivalenceDomain::Bag),
        TypeExpr::Seq(element) => domain_for_type_inner(element, binders)
            .map(Box::new)
            .map(EquivalenceDomain::Seq),
        TypeExpr::Map { key, value, .. } => Some(EquivalenceDomain::Map {
            key: Box::new(domain_for_type_inner(key, binders)?),
            value: Box::new(domain_for_type_inner(value, binders)?),
        }),
        TypeExpr::Mu { binder, body } => {
            binders.push(*binder);
            let body = domain_for_type_inner(body, binders)?;
            binders.pop();
            Some(EquivalenceDomain::Mu(Box::new(body)))
        }
        TypeExpr::Var(var) => binders
            .iter()
            .rev()
            .position(|binder| binder == var)
            .map(EquivalenceDomain::Var),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EquivalenceModule {
    UnitExact,
    BoolExact,
    I64Exact,
    F64Bitwise,
    TextExact,
    TextAsciiCaseInsensitive,
    LiveEntityIdExact(SemanticId),
    HistoricalEntityIdExact(SemanticId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPrimitiveEquivalence {
    equivalence: SemanticId,
    module_digest: ModuleDigest,
    contract: EquivalenceModule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalEquivalenceDependency {
    pub semantic_id: SemanticId,
    pub module_digest: ModuleDigest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalStructuralEquivalenceDependency {
    pub semantic_id: SemanticId,
    pub definition: StructuralEquivalenceDef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledEquivalence {
    equivalence: SemanticId,
    domain: EquivalenceDomain,
    root: usize,
    nodes: Vec<CompiledEquivalenceNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompiledEquivalenceNode {
    equivalence: SemanticId,
    kind: CompiledEquivalenceKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompiledEquivalenceKind {
    Primitive(ResolvedPrimitiveEquivalence),
    Mu { body: usize },
    Var { binder: usize },
    Product(Vec<(SemanticId, usize)>),
    Option { inner: usize },
    Sum(BTreeMap<SemanticId, usize>),
    Set { element: usize },
    Bag { element: usize },
    Seq { element: usize },
    Map { key: usize, value: usize },
}

#[derive(Debug, Clone, Copy)]
pub enum CompiledEquivalenceNodeRef<'a> {
    Primitive(ResolvedPrimitiveEquivalence),
    Mu { body: usize },
    Var { binder: usize },
    Product(&'a [(SemanticId, usize)]),
    Option { inner: usize },
    Sum(&'a BTreeMap<SemanticId, usize>),
    Set { element: usize },
    Bag { element: usize },
    Seq { element: usize },
    Map { key: usize, value: usize },
}

struct EquivalenceCompileState<'a> {
    context: &'a SemanticContext,
    nodes: &'a mut Vec<CompiledEquivalenceNode>,
    stack: &'a mut BTreeSet<SemanticId>,
    binders: &'a mut Vec<(SemanticId, usize, usize)>,
}

impl CompiledEquivalence {
    fn from_compiled_nodes(
        equivalence: SemanticId,
        domain: EquivalenceDomain,
        root: usize,
        nodes: Vec<CompiledEquivalenceNode>,
    ) -> Self {
        Self {
            equivalence,
            domain,
            root,
            nodes,
        }
    }

    #[must_use]
    pub const fn equivalence(&self) -> SemanticId {
        self.equivalence
    }

    #[must_use]
    pub const fn domain(&self) -> &EquivalenceDomain {
        &self.domain
    }

    #[must_use]
    pub const fn root_node(&self) -> usize {
        self.root
    }

    #[must_use]
    pub fn node(&self, index: usize) -> Option<(SemanticId, CompiledEquivalenceNodeRef<'_>)> {
        let node = self.nodes.get(index)?;
        let kind = match &node.kind {
            CompiledEquivalenceKind::Primitive(resolved) => {
                CompiledEquivalenceNodeRef::Primitive(*resolved)
            }
            CompiledEquivalenceKind::Mu { body } => CompiledEquivalenceNodeRef::Mu { body: *body },
            CompiledEquivalenceKind::Var { binder } => {
                CompiledEquivalenceNodeRef::Var { binder: *binder }
            }
            CompiledEquivalenceKind::Product(fields) => CompiledEquivalenceNodeRef::Product(fields),
            CompiledEquivalenceKind::Option { inner } => {
                CompiledEquivalenceNodeRef::Option { inner: *inner }
            }
            CompiledEquivalenceKind::Sum(variants) => CompiledEquivalenceNodeRef::Sum(variants),
            CompiledEquivalenceKind::Set { element } => {
                CompiledEquivalenceNodeRef::Set { element: *element }
            }
            CompiledEquivalenceKind::Bag { element } => {
                CompiledEquivalenceNodeRef::Bag { element: *element }
            }
            CompiledEquivalenceKind::Seq { element } => {
                CompiledEquivalenceNodeRef::Seq { element: *element }
            }
            CompiledEquivalenceKind::Map { key, value } => CompiledEquivalenceNodeRef::Map {
                key: *key,
                value: *value,
            },
        };
        Some((node.equivalence, kind))
    }

    pub fn canonical_key(&self, value: &Value) -> Result<CanonicalEqKey, SemanticError> {
        self.canonical_key_at(self.root, value)
    }

    pub fn equivalent(&self, left: &Value, right: &Value) -> Result<bool, SemanticError> {
        Ok(self.canonical_key(left)? == self.canonical_key(right)?)
    }

    fn canonical_key_at(
        &self,
        node_index: usize,
        value: &Value,
    ) -> Result<CanonicalEqKey, SemanticError> {
        let node = &self.nodes[node_index];
        match (&node.kind, value) {
            (CompiledEquivalenceKind::Primitive(resolved), _) => resolved.canonical_key(value),
            (CompiledEquivalenceKind::Mu { body }, _) => self.canonical_key_at(*body, value),
            (CompiledEquivalenceKind::Var { binder }, _) => self.canonical_key_at(*binder, value),
            (CompiledEquivalenceKind::Product(fields), Value::Product(values)) => {
                if fields.len() != values.len() {
                    return Err(SemanticError::TypeMismatch(node.equivalence));
                }
                fields
                    .iter()
                    .map(|(field, child)| {
                        let value = values
                            .get(field)
                            .ok_or(SemanticError::TypeMismatch(node.equivalence))?;
                        Ok((*field, self.canonical_key_at(*child, value)?))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(CanonicalEqKey::Product)
            }
            (CompiledEquivalenceKind::Option { inner }, Value::Option(value)) => value
                .as_deref()
                .map_or(Ok(CanonicalEqKey::OptionNone), |value| {
                    self.canonical_key_at(*inner, value)
                        .map(Box::new)
                        .map(CanonicalEqKey::OptionSome)
                }),
            (CompiledEquivalenceKind::Sum(variants), Value::Variant { tag, value }) => {
                let child = variants
                    .get(tag)
                    .ok_or(SemanticError::TypeMismatch(node.equivalence))?;
                Ok(CanonicalEqKey::Variant {
                    tag: *tag,
                    value: Box::new(self.canonical_key_at(*child, value)?),
                })
            }
            (CompiledEquivalenceKind::Seq { element }, Value::Seq(values)) => values
                .iter()
                .map(|value| self.canonical_key_at(*element, value))
                .collect::<Result<Vec<_>, _>>()
                .map(CanonicalEqKey::Seq),
            (CompiledEquivalenceKind::Set { element }, Value::Set { elements, .. }) => {
                let atoms = elements
                    .iter()
                    .map(|value| self.canonical_key_at(*element, value))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(CanonicalEqKey::Set(finite_measure_from_atoms(atoms)))
            }
            (CompiledEquivalenceKind::Bag { element }, Value::Bag { entries, .. }) => {
                let atoms = entries
                    .iter()
                    .map(|(value, count)| {
                        Ok(CanonicalBagAtom {
                            value: self.canonical_key_at(*element, value)?,
                            stored_count: *count,
                        })
                    })
                    .collect::<Result<Vec<_>, SemanticError>>()?;
                Ok(CanonicalEqKey::Bag(finite_measure_from_atoms(atoms)))
            }
            (CompiledEquivalenceKind::Map { key, value }, Value::Map { entries, .. }) => {
                let atoms = entries
                    .iter()
                    .map(|(entry_key, entry_value)| {
                        Ok(CanonicalMapAtom {
                            key: self.canonical_key_at(*key, entry_key)?,
                            value: self.canonical_key_at(*value, entry_value)?,
                        })
                    })
                    .collect::<Result<Vec<_>, SemanticError>>()?;
                Ok(CanonicalEqKey::Map(finite_measure_from_atoms(atoms)))
            }
            _ => Err(SemanticError::TypeMismatch(node.equivalence)),
        }
    }
}

impl ResolvedPrimitiveEquivalence {
    pub(super) const fn new(
        equivalence: SemanticId,
        module_digest: ModuleDigest,
        contract: EquivalenceModule,
    ) -> Self {
        Self {
            equivalence,
            module_digest,
            contract,
        }
    }

    #[must_use]
    pub const fn equivalence(&self) -> SemanticId {
        self.equivalence
    }

    #[must_use]
    pub const fn module_digest(&self) -> ModuleDigest {
        self.module_digest
    }

    pub fn equivalent(&self, left: &Value, right: &Value) -> Result<bool, SemanticError> {
        self.contract.equivalent(left, right, self.equivalence)
    }

    pub fn canonical_key(&self, value: &Value) -> Result<CanonicalEqKey, SemanticError> {
        let key = match (self.contract, value) {
            (EquivalenceModule::UnitExact, Value::Unit) => CanonicalEqKey::Unit,
            (EquivalenceModule::BoolExact, Value::Bool(value)) => CanonicalEqKey::Bool(*value),
            (EquivalenceModule::I64Exact, Value::I64(value)) => CanonicalEqKey::I64(*value),
            (EquivalenceModule::F64Bitwise, Value::F64Bits(value)) => {
                CanonicalEqKey::F64Bits(*value)
            }
            (EquivalenceModule::TextExact, Value::Text(value)) => {
                CanonicalEqKey::TextExact(value.clone())
            }
            (EquivalenceModule::TextAsciiCaseInsensitive, Value::Text(value)) => {
                CanonicalEqKey::TextAsciiCaseInsensitive(value.to_ascii_lowercase())
            }
            (
                EquivalenceModule::LiveEntityIdExact(entity_type),
                Value::LiveEntityRef {
                    entity_type: actual,
                    id,
                },
            ) if actual == &entity_type => CanonicalEqKey::LiveEntityId {
                entity_type,
                id: *id,
            },
            (
                EquivalenceModule::HistoricalEntityIdExact(entity_type),
                Value::HistoricalEntityId {
                    entity_type: actual,
                    id,
                },
            ) if actual == &entity_type => CanonicalEqKey::HistoricalEntityId {
                entity_type,
                id: *id,
            },
            _ => return Err(SemanticError::TypeMismatch(self.equivalence)),
        };
        Ok(key)
    }

    pub fn bind_right(&self, right: &Value) -> Result<BoundPrimitivePredicate, SemanticError> {
        let predicate = match (self.contract, right) {
            (EquivalenceModule::UnitExact, Value::Unit) => BoundPrimitivePredicate::Unit,
            (EquivalenceModule::BoolExact, Value::Bool(value)) => {
                BoundPrimitivePredicate::Bool(*value)
            }
            (EquivalenceModule::I64Exact, Value::I64(value)) => {
                BoundPrimitivePredicate::I64(*value)
            }
            (EquivalenceModule::F64Bitwise, Value::F64Bits(value)) => {
                BoundPrimitivePredicate::F64Bits(*value)
            }
            (EquivalenceModule::TextExact, Value::Text(value)) => {
                BoundPrimitivePredicate::TextExact(value.clone())
            }
            (EquivalenceModule::TextAsciiCaseInsensitive, Value::Text(value)) => {
                BoundPrimitivePredicate::TextAsciiCaseInsensitive(value.clone())
            }
            (
                EquivalenceModule::LiveEntityIdExact(entity_type),
                Value::LiveEntityRef {
                    entity_type: actual,
                    id,
                },
            ) if actual == &entity_type => BoundPrimitivePredicate::LiveEntityId {
                entity_type,
                id: *id,
            },
            (
                EquivalenceModule::HistoricalEntityIdExact(entity_type),
                Value::HistoricalEntityId {
                    entity_type: actual,
                    id,
                },
            ) if actual == &entity_type => BoundPrimitivePredicate::HistoricalEntityId {
                entity_type,
                id: *id,
            },
            _ => return Err(SemanticError::TypeMismatch(self.equivalence)),
        };
        Ok(predicate)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundPrimitivePredicate {
    Unit,
    Bool(bool),
    I64(i64),
    F64Bits(u64),
    TextExact(String),
    TextAsciiCaseInsensitive(String),
    LiveEntityId {
        entity_type: SemanticId,
        id: kernel_types::EntityId,
    },
    HistoricalEntityId {
        entity_type: SemanticId,
        id: kernel_types::EntityId,
    },
}

impl BoundPrimitivePredicate {
    #[must_use]
    pub fn matches(&self, candidate: &Value) -> bool {
        match (self, candidate) {
            (Self::Unit, Value::Unit) => true,
            (Self::Bool(expected), Value::Bool(candidate)) => expected == candidate,
            (Self::I64(expected), Value::I64(candidate)) => expected == candidate,
            (Self::F64Bits(expected), Value::F64Bits(candidate)) => expected == candidate,
            (Self::TextExact(expected), Value::Text(candidate)) => expected == candidate,
            (Self::TextAsciiCaseInsensitive(expected), Value::Text(candidate)) => {
                expected.eq_ignore_ascii_case(candidate)
            }
            (
                Self::LiveEntityId {
                    entity_type: expected_type,
                    id: expected,
                },
                Value::LiveEntityRef {
                    entity_type: candidate_type,
                    id: candidate,
                },
            )
            | (
                Self::HistoricalEntityId {
                    entity_type: expected_type,
                    id: expected,
                },
                Value::HistoricalEntityId {
                    entity_type: candidate_type,
                    id: candidate,
                },
            ) => expected_type == candidate_type && expected == candidate,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EquivalenceImplementation {
    pub(super) contract: EquivalenceModule,
    pub(super) implementation_revision: u64,
}

impl EquivalenceModule {
    #[must_use]
    pub(super) fn implementation_digest(self, implementation_revision: u64) -> ModuleDigest {
        EquivalenceImplementation {
            contract: self,
            implementation_revision,
        }
        .digest()
    }

    #[must_use]
    pub const fn domain(self) -> EquivalenceDomain {
        match self {
            Self::UnitExact => EquivalenceDomain::Unit,
            Self::BoolExact => EquivalenceDomain::Bool,
            Self::I64Exact => EquivalenceDomain::I64,
            Self::F64Bitwise => EquivalenceDomain::F64,
            Self::TextExact | Self::TextAsciiCaseInsensitive => EquivalenceDomain::Text,
            Self::LiveEntityIdExact(entity_type) => EquivalenceDomain::LiveEntityRef(entity_type),
            Self::HistoricalEntityIdExact(entity_type) => {
                EquivalenceDomain::HistoricalEntityId(entity_type)
            }
        }
    }

    #[must_use]
    pub fn digest(self) -> ModuleDigest {
        match self {
            Self::I64Exact => ModuleDigest([1; 32]),
            Self::TextExact => ModuleDigest([2; 32]),
            Self::TextAsciiCaseInsensitive => ModuleDigest([3; 32]),
            Self::LiveEntityIdExact(entity_type) => typed_entity_digest(4, entity_type),
            Self::HistoricalEntityIdExact(entity_type) => typed_entity_digest(8, entity_type),
            Self::UnitExact => ModuleDigest([5; 32]),
            Self::BoolExact => ModuleDigest([6; 32]),
            Self::F64Bitwise => ModuleDigest([7; 32]),
        }
    }

    pub(super) fn equivalent(
        self,
        left: &Value,
        right: &Value,
        equivalence: SemanticId,
    ) -> Result<bool, SemanticError> {
        match (self, left, right) {
            (Self::UnitExact, Value::Unit, Value::Unit) => Ok(true),
            (Self::BoolExact, Value::Bool(left), Value::Bool(right)) => Ok(left == right),
            (Self::I64Exact, Value::I64(left), Value::I64(right)) => Ok(left == right),
            (Self::F64Bitwise, Value::F64Bits(left), Value::F64Bits(right)) => Ok(left == right),
            (Self::TextExact, Value::Text(left), Value::Text(right)) => Ok(left == right),
            (Self::TextAsciiCaseInsensitive, Value::Text(left), Value::Text(right)) => {
                Ok(left.eq_ignore_ascii_case(right))
            }
            (
                Self::LiveEntityIdExact(expected),
                Value::LiveEntityRef {
                    entity_type: left_type,
                    id: left,
                },
                Value::LiveEntityRef {
                    entity_type: right_type,
                    id: right,
                },
            ) if left_type == &expected && right_type == &expected => Ok(left == right),
            (
                Self::HistoricalEntityIdExact(expected),
                Value::HistoricalEntityId {
                    entity_type: left_type,
                    id: left,
                },
                Value::HistoricalEntityId {
                    entity_type: right_type,
                    id: right,
                },
            ) if left_type == &expected && right_type == &expected => Ok(left == right),
            _ => Err(SemanticError::TypeMismatch(equivalence)),
        }
    }
}

impl EquivalenceImplementation {
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

fn canonical_dependency_children(definition: &StructuralEquivalenceDef) -> Vec<SemanticId> {
    match definition {
        StructuralEquivalenceDef::Var { binder } => vec![*binder],
        _ => structural_equivalence_children(definition),
    }
}

fn structural_equivalence_children(definition: &StructuralEquivalenceDef) -> Vec<SemanticId> {
    match definition {
        StructuralEquivalenceDef::Mu { body } => vec![*body],
        StructuralEquivalenceDef::Var { .. } => Vec::new(),
        StructuralEquivalenceDef::Product { fields } => fields.values().copied().collect(),
        StructuralEquivalenceDef::Option { inner } => vec![*inner],
        StructuralEquivalenceDef::Sum { variants } => variants.values().copied().collect(),
        StructuralEquivalenceDef::Set { element }
        | StructuralEquivalenceDef::Bag { element }
        | StructuralEquivalenceDef::Seq { element } => vec![*element],
        StructuralEquivalenceDef::Map { key, value } => vec![*key, *value],
    }
}

impl SemanticRegistry {
    pub(super) fn validate_structural_equivalence_graph(
        &self,
        context: &SemanticContext,
    ) -> Result<(), SemanticError> {
        let definitions: BTreeMap<_, _> = context.schema.structural_equivalences().collect();
        let mut referenced = BTreeSet::new();
        for definition in definitions.values() {
            referenced.extend(structural_equivalence_children(definition));
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
                pending.extend(canonical_dependency_children(definition));
            }
        }
        if let Some(unrooted) = definitions.keys().find(|id| !reachable.contains(id)) {
            return Err(SemanticError::CyclicStructuralEquivalence(*unrooted));
        }
        for root in roots {
            self.equivalence_domain(context, root)?;
        }
        Ok(())
    }

    pub fn resolve_primitive_equivalence(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
    ) -> Result<Option<ResolvedPrimitiveEquivalence>, SemanticError> {
        if context.schema.structural_equivalence(equivalence).is_some() {
            return Ok(None);
        }
        let digest = context
            .environment
            .module(equivalence)
            .ok_or(SemanticError::WrongModuleKind(equivalence))?;
        let implementation = match self.equivalence_implementation(digest) {
            Some(implementation) => implementation,
            None if self.module_available(digest) => {
                return Err(SemanticError::WrongModuleKind(equivalence));
            }
            None => return Err(SemanticError::ModuleUnavailable(digest)),
        };
        Ok(Some(ResolvedPrimitiveEquivalence::new(
            equivalence,
            digest,
            implementation.contract,
        )))
    }

    pub fn compile_equivalence(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
    ) -> Result<CompiledEquivalence, SemanticError> {
        let domain = self.equivalence_domain(context, equivalence)?;
        let mut nodes = Vec::new();
        let mut stack = BTreeSet::new();
        let mut binders = Vec::new();
        let root = self.compile_equivalence_node(
            equivalence,
            &mut EquivalenceCompileState {
                context,
                nodes: &mut nodes,
                stack: &mut stack,
                binders: &mut binders,
            },
            0,
        )?;
        Ok(CompiledEquivalence::from_compiled_nodes(
            equivalence,
            domain,
            root,
            nodes,
        ))
    }

    fn compile_equivalence_node(
        &self,
        equivalence: SemanticId,
        state: &mut EquivalenceCompileState<'_>,
        constructor_depth: usize,
    ) -> Result<usize, SemanticError> {
        let Some(definition) = state.context.schema.structural_equivalence(equivalence) else {
            return self.compile_primitive_equivalence_node(equivalence, state);
        };
        if let StructuralEquivalenceDef::Var { binder } = definition {
            return Self::compile_variable_equivalence_node(
                equivalence,
                *binder,
                state,
                constructor_depth,
            );
        }
        if !state.stack.insert(equivalence) {
            return Err(SemanticError::CyclicStructuralEquivalence(equivalence));
        }
        let index = state.nodes.len();
        state.nodes.push(CompiledEquivalenceNode {
            equivalence,
            kind: CompiledEquivalenceKind::Mu { body: usize::MAX },
        });
        let kind = self.compile_structural_equivalence_kind(
            equivalence,
            definition,
            index,
            state,
            constructor_depth,
        )?;
        state.nodes[index].kind = kind;
        state.stack.remove(&equivalence);
        Ok(index)
    }

    fn compile_primitive_equivalence_node(
        &self,
        equivalence: SemanticId,
        state: &mut EquivalenceCompileState<'_>,
    ) -> Result<usize, SemanticError> {
        let resolved = self
            .resolve_primitive_equivalence(state.context, equivalence)?
            .ok_or(SemanticError::WrongModuleKind(equivalence))?;
        let index = state.nodes.len();
        state.nodes.push(CompiledEquivalenceNode {
            equivalence,
            kind: CompiledEquivalenceKind::Primitive(resolved),
        });
        Ok(index)
    }

    fn compile_variable_equivalence_node(
        equivalence: SemanticId,
        binder: SemanticId,
        state: &mut EquivalenceCompileState<'_>,
        constructor_depth: usize,
    ) -> Result<usize, SemanticError> {
        let Some((_, binder_node, binder_depth)) = state
            .binders
            .iter()
            .rev()
            .find(|(candidate, _, _)| candidate == &binder)
        else {
            return Err(SemanticError::FreeStructuralRecursion(binder));
        };
        if constructor_depth <= *binder_depth {
            return Err(SemanticError::UnguardedStructuralRecursion(binder));
        }
        let index = state.nodes.len();
        state.nodes.push(CompiledEquivalenceNode {
            equivalence,
            kind: CompiledEquivalenceKind::Var {
                binder: *binder_node,
            },
        });
        Ok(index)
    }

    fn compile_structural_equivalence_kind(
        &self,
        equivalence: SemanticId,
        definition: &StructuralEquivalenceDef,
        index: usize,
        state: &mut EquivalenceCompileState<'_>,
        constructor_depth: usize,
    ) -> Result<CompiledEquivalenceKind, SemanticError> {
        match definition {
            StructuralEquivalenceDef::Mu { body } => {
                state.binders.push((equivalence, index, constructor_depth));
                let body = self.compile_equivalence_node(*body, state, constructor_depth)?;
                state.binders.pop();
                Ok(CompiledEquivalenceKind::Mu { body })
            }
            StructuralEquivalenceDef::Var { .. } => unreachable!(),
            StructuralEquivalenceDef::Product { fields } => self
                .compile_named_children(fields, state, constructor_depth)
                .map(CompiledEquivalenceKind::Product),
            StructuralEquivalenceDef::Option { inner } => self
                .compile_equivalence_node(*inner, state, constructor_depth + 1)
                .map(|inner| CompiledEquivalenceKind::Option { inner }),
            StructuralEquivalenceDef::Sum { variants } => self
                .compile_tagged_children(variants, state, constructor_depth)
                .map(CompiledEquivalenceKind::Sum),
            StructuralEquivalenceDef::Set { element } => self
                .compile_equivalence_node(*element, state, constructor_depth + 1)
                .map(|element| CompiledEquivalenceKind::Set { element }),
            StructuralEquivalenceDef::Bag { element } => self
                .compile_equivalence_node(*element, state, constructor_depth + 1)
                .map(|element| CompiledEquivalenceKind::Bag { element }),
            StructuralEquivalenceDef::Seq { element } => self
                .compile_equivalence_node(*element, state, constructor_depth + 1)
                .map(|element| CompiledEquivalenceKind::Seq { element }),
            StructuralEquivalenceDef::Map { key, value } => {
                let key = self.compile_equivalence_node(*key, state, constructor_depth + 1)?;
                let value = self.compile_equivalence_node(*value, state, constructor_depth + 1)?;
                Ok(CompiledEquivalenceKind::Map { key, value })
            }
        }
    }

    fn compile_named_children(
        &self,
        children: &BTreeMap<SemanticId, SemanticId>,
        state: &mut EquivalenceCompileState<'_>,
        constructor_depth: usize,
    ) -> Result<Vec<(SemanticId, usize)>, SemanticError> {
        children
            .iter()
            .map(|(name, child)| {
                self.compile_equivalence_node(*child, state, constructor_depth + 1)
                    .map(|node| (*name, node))
            })
            .collect()
    }

    fn compile_tagged_children(
        &self,
        children: &BTreeMap<SemanticId, SemanticId>,
        state: &mut EquivalenceCompileState<'_>,
        constructor_depth: usize,
    ) -> Result<BTreeMap<SemanticId, usize>, SemanticError> {
        children
            .iter()
            .map(|(tag, child)| {
                self.compile_equivalence_node(*child, state, constructor_depth + 1)
                    .map(|node| (*tag, node))
            })
            .collect()
    }

    pub fn canonical_equivalence_key(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        value: &Value,
    ) -> Result<CanonicalEqKey, SemanticError> {
        if context.schema.structural_equivalence(equivalence).is_some() {
            self.equivalence_domain(context, equivalence)?;
        }
        self.canonical_equivalence_key_unchecked(context, equivalence, value)
    }

    pub fn canonical_equivalence_dependencies(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
    ) -> Result<Vec<CanonicalEquivalenceDependency>, SemanticError> {
        self.equivalence_domain(context, equivalence)?;
        let mut pending = vec![equivalence];
        let mut visited = BTreeSet::new();
        let mut dependencies = BTreeSet::new();
        while let Some(current) = pending.pop() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(definition) = context.schema.structural_equivalence(current) {
                pending.extend(canonical_dependency_children(definition));
                continue;
            }
            let resolved = self
                .resolve_primitive_equivalence(context, current)?
                .ok_or(SemanticError::WrongModuleKind(current))?;
            dependencies.insert(CanonicalEquivalenceDependency {
                semantic_id: current,
                module_digest: resolved.module_digest(),
            });
        }
        Ok(dependencies.into_iter().collect())
    }

    pub fn canonical_structural_equivalence_dependencies(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
    ) -> Result<Vec<CanonicalStructuralEquivalenceDependency>, SemanticError> {
        self.equivalence_domain(context, equivalence)?;
        let mut pending = vec![equivalence];
        let mut visited = BTreeSet::new();
        let mut dependencies = BTreeMap::new();
        while let Some(current) = pending.pop() {
            if !visited.insert(current) {
                continue;
            }
            let Some(definition) = context.schema.structural_equivalence(current) else {
                let _ = self
                    .resolve_primitive_equivalence(context, current)?
                    .ok_or(SemanticError::WrongModuleKind(current))?;
                continue;
            };
            dependencies.insert(current, definition.clone());
            pending.extend(canonical_dependency_children(definition));
        }
        Ok(dependencies
            .into_iter()
            .map(
                |(semantic_id, definition)| CanonicalStructuralEquivalenceDependency {
                    semantic_id,
                    definition,
                },
            )
            .collect())
    }

    fn canonical_equivalence_key_unchecked(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        value: &Value,
    ) -> Result<CanonicalEqKey, SemanticError> {
        let Some(definition) = context.schema.structural_equivalence(equivalence) else {
            return self
                .resolve_primitive_equivalence(context, equivalence)?
                .ok_or(SemanticError::WrongModuleKind(equivalence))?
                .canonical_key(value);
        };
        self.canonical_structural_equivalence_key(context, equivalence, definition, value)
    }

    fn canonical_structural_equivalence_key(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        definition: &StructuralEquivalenceDef,
        value: &Value,
    ) -> Result<CanonicalEqKey, SemanticError> {
        match (definition, value) {
            (StructuralEquivalenceDef::Mu { body }, _) => {
                self.canonical_equivalence_key_unchecked(context, *body, value)
            }
            (StructuralEquivalenceDef::Var { binder }, _) => {
                self.canonical_equivalence_key_unchecked(context, *binder, value)
            }
            (StructuralEquivalenceDef::Product { fields }, Value::Product(values)) => {
                self.canonical_product_key(context, equivalence, fields, values)
            }
            (StructuralEquivalenceDef::Option { inner }, Value::Option(value)) => value
                .as_deref()
                .map_or(Ok(CanonicalEqKey::OptionNone), |value| {
                    self.canonical_equivalence_key_unchecked(context, *inner, value)
                        .map(Box::new)
                        .map(CanonicalEqKey::OptionSome)
                }),
            (StructuralEquivalenceDef::Sum { variants }, Value::Variant { tag, value }) => {
                let child_equivalence = variants
                    .get(tag)
                    .ok_or(SemanticError::TypeMismatch(equivalence))?;
                Ok(CanonicalEqKey::Variant {
                    tag: *tag,
                    value: Box::new(self.canonical_equivalence_key_unchecked(
                        context,
                        *child_equivalence,
                        value,
                    )?),
                })
            }
            (StructuralEquivalenceDef::Seq { element }, Value::Seq(values)) => values
                .iter()
                .map(|value| self.canonical_equivalence_key_unchecked(context, *element, value))
                .collect::<Result<Vec<_>, _>>()
                .map(CanonicalEqKey::Seq),
            (StructuralEquivalenceDef::Set { element }, Value::Set { elements, .. }) => {
                let atoms = elements
                    .iter()
                    .map(|value| self.canonical_equivalence_key_unchecked(context, *element, value))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(CanonicalEqKey::Set(finite_measure_from_atoms(atoms)))
            }
            (StructuralEquivalenceDef::Bag { element }, Value::Bag { entries, .. }) => {
                let atoms = entries
                    .iter()
                    .map(|(value, count)| {
                        Ok(CanonicalBagAtom {
                            value: self
                                .canonical_equivalence_key_unchecked(context, *element, value)?,
                            stored_count: *count,
                        })
                    })
                    .collect::<Result<Vec<_>, SemanticError>>()?;
                Ok(CanonicalEqKey::Bag(finite_measure_from_atoms(atoms)))
            }
            (
                StructuralEquivalenceDef::Map {
                    key: key_equivalence,
                    value: value_equivalence,
                },
                Value::Map { entries, .. },
            ) => {
                let atoms = entries
                    .iter()
                    .map(|(entry_key, entry_value)| {
                        Ok(CanonicalMapAtom {
                            key: self.canonical_equivalence_key_unchecked(
                                context,
                                *key_equivalence,
                                entry_key,
                            )?,
                            value: self.canonical_equivalence_key_unchecked(
                                context,
                                *value_equivalence,
                                entry_value,
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, SemanticError>>()?;
                Ok(CanonicalEqKey::Map(finite_measure_from_atoms(atoms)))
            }
            _ => Err(SemanticError::TypeMismatch(equivalence)),
        }
    }

    fn canonical_product_key(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        fields: &BTreeMap<SemanticId, SemanticId>,
        values: &BTreeMap<SemanticId, Value>,
    ) -> Result<CanonicalEqKey, SemanticError> {
        if values.len() != fields.len() {
            return Err(SemanticError::TypeMismatch(equivalence));
        }
        fields
            .iter()
            .map(|(field, child_equivalence)| {
                let child = values
                    .get(field)
                    .ok_or(SemanticError::TypeMismatch(equivalence))?;
                Ok((
                    *field,
                    self.canonical_equivalence_key_unchecked(context, *child_equivalence, child)?,
                ))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(CanonicalEqKey::Product)
    }

    pub fn equivalence_domain(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
    ) -> Result<EquivalenceDomain, SemanticError> {
        self.equivalence_domain_inner(
            context,
            equivalence,
            &mut BTreeSet::new(),
            &mut Vec::new(),
            0,
        )
    }

    pub fn equivalence_refines(
        &self,
        context: &SemanticContext,
        finer: SemanticId,
        coarser: SemanticId,
    ) -> Result<bool, SemanticError> {
        self.equivalence_domain(context, finer)?;
        self.equivalence_domain(context, coarser)?;
        self.equivalence_refines_inner(
            context,
            finer,
            coarser,
            &mut BTreeSet::new(),
            &mut Vec::new(),
        )
    }

    fn equivalence_refines_inner(
        &self,
        context: &SemanticContext,
        finer: SemanticId,
        coarser: SemanticId,
        stack: &mut BTreeSet<(SemanticId, SemanticId)>,
        recursive_pairs: &mut Vec<(SemanticId, SemanticId)>,
    ) -> Result<bool, SemanticError> {
        if finer == coarser {
            return Ok(true);
        }
        if !stack.insert((finer, coarser)) {
            return Err(SemanticError::CyclicStructuralEquivalence(finer));
        }

        let result = match (
            context.schema.structural_equivalence(finer),
            context.schema.structural_equivalence(coarser),
        ) {
            (
                Some(StructuralEquivalenceDef::Mu { body: finer_body }),
                Some(StructuralEquivalenceDef::Mu { body: coarser_body }),
            ) => {
                recursive_pairs.push((finer, coarser));
                let result = self.equivalence_refines_inner(
                    context,
                    *finer_body,
                    *coarser_body,
                    stack,
                    recursive_pairs,
                );
                recursive_pairs.pop();
                result
            }
            (
                Some(StructuralEquivalenceDef::Var {
                    binder: finer_binder,
                }),
                Some(StructuralEquivalenceDef::Var {
                    binder: coarser_binder,
                }),
            ) => Ok(recursive_pairs
                .iter()
                .rev()
                .any(|pair| pair == &(*finer_binder, *coarser_binder))),
            (
                Some(StructuralEquivalenceDef::Product { fields: finer }),
                Some(StructuralEquivalenceDef::Product { fields: coarser }),
            )
            | (
                Some(StructuralEquivalenceDef::Sum { variants: finer }),
                Some(StructuralEquivalenceDef::Sum { variants: coarser }),
            ) => self.keyed_equivalences_refine(context, finer, coarser, stack, recursive_pairs),
            (
                Some(StructuralEquivalenceDef::Option { inner: finer }),
                Some(StructuralEquivalenceDef::Option { inner: coarser }),
            )
            | (
                Some(StructuralEquivalenceDef::Set { element: finer }),
                Some(StructuralEquivalenceDef::Set { element: coarser }),
            )
            | (
                Some(StructuralEquivalenceDef::Bag { element: finer }),
                Some(StructuralEquivalenceDef::Bag { element: coarser }),
            )
            | (
                Some(StructuralEquivalenceDef::Seq { element: finer }),
                Some(StructuralEquivalenceDef::Seq { element: coarser }),
            ) => self.equivalence_refines_inner(context, *finer, *coarser, stack, recursive_pairs),
            (
                Some(StructuralEquivalenceDef::Map {
                    key: finer_key,
                    value: finer_value,
                }),
                Some(StructuralEquivalenceDef::Map {
                    key: coarser_key,
                    value: coarser_value,
                }),
            ) => Ok(self.equivalence_refines_inner(
                context,
                *finer_key,
                *coarser_key,
                stack,
                recursive_pairs,
            )? && self.equivalence_refines_inner(
                context,
                *finer_value,
                *coarser_value,
                stack,
                recursive_pairs,
            )?),
            (None, None) => self.leaf_equivalence_refines(context, finer, coarser),
            _ => Ok(false),
        };
        stack.remove(&(finer, coarser));
        result
    }

    fn keyed_equivalences_refine(
        &self,
        context: &SemanticContext,
        finer: &BTreeMap<SemanticId, SemanticId>,
        coarser: &BTreeMap<SemanticId, SemanticId>,
        stack: &mut BTreeSet<(SemanticId, SemanticId)>,
        recursive_pairs: &mut Vec<(SemanticId, SemanticId)>,
    ) -> Result<bool, SemanticError> {
        if finer.len() == coarser.len() {
            for (key, finer_child) in finer {
                let Some(coarser_child) = coarser.get(key) else {
                    return Ok(false);
                };
                if !self.equivalence_refines_inner(
                    context,
                    *finer_child,
                    *coarser_child,
                    stack,
                    recursive_pairs,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn leaf_equivalence_refines(
        &self,
        context: &SemanticContext,
        finer: SemanticId,
        coarser: SemanticId,
    ) -> Result<bool, SemanticError> {
        let finer = self.equivalence_contract(context, finer)?;
        let coarser = self.equivalence_contract(context, coarser)?;
        Ok(finer == coarser
            || matches!(
                (finer, coarser),
                (
                    EquivalenceModule::TextExact,
                    EquivalenceModule::TextAsciiCaseInsensitive
                )
            ))
    }

    pub(super) fn equivalence_contract(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
    ) -> Result<EquivalenceModule, SemanticError> {
        let digest = context
            .environment
            .module(equivalence)
            .ok_or(SemanticError::WrongModuleKind(equivalence))?;
        match self.equivalence_implementation(digest) {
            Some(implementation) => Ok(implementation.contract),
            None if self.module_available(digest) => {
                Err(SemanticError::WrongModuleKind(equivalence))
            }
            None => Err(SemanticError::ModuleUnavailable(digest)),
        }
    }

    fn equivalence_domain_inner(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        stack: &mut BTreeSet<SemanticId>,
        binders: &mut Vec<(SemanticId, usize)>,
        constructor_depth: usize,
    ) -> Result<EquivalenceDomain, SemanticError> {
        if let Some(definition) = context.schema.structural_equivalence(equivalence) {
            if let StructuralEquivalenceDef::Var { binder } = definition {
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
                return Ok(EquivalenceDomain::Var(distance));
            }
            if !stack.insert(equivalence) {
                return Err(SemanticError::CyclicStructuralEquivalence(equivalence));
            }
            let child_depth = constructor_depth + 1;
            let result = match definition {
                StructuralEquivalenceDef::Mu { body } => {
                    binders.push((equivalence, constructor_depth));
                    let body = self.equivalence_domain_inner(
                        context,
                        *body,
                        stack,
                        binders,
                        constructor_depth,
                    )?;
                    binders.pop();
                    Ok(EquivalenceDomain::Mu(Box::new(body)))
                }
                StructuralEquivalenceDef::Var { .. } => unreachable!(),
                StructuralEquivalenceDef::Product { fields } => fields
                    .iter()
                    .map(|(field, child)| {
                        self.equivalence_domain_inner(context, *child, stack, binders, child_depth)
                            .map(|domain| (*field, domain))
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()
                    .map(EquivalenceDomain::Product),
                StructuralEquivalenceDef::Option { inner } => self
                    .equivalence_domain_inner(context, *inner, stack, binders, child_depth)
                    .map(Box::new)
                    .map(EquivalenceDomain::Option),
                StructuralEquivalenceDef::Sum { variants } => variants
                    .iter()
                    .map(|(tag, child)| {
                        self.equivalence_domain_inner(context, *child, stack, binders, child_depth)
                            .map(|domain| (*tag, domain))
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()
                    .map(EquivalenceDomain::Sum),
                StructuralEquivalenceDef::Set { element } => self
                    .equivalence_domain_inner(context, *element, stack, binders, child_depth)
                    .map(Box::new)
                    .map(EquivalenceDomain::Set),
                StructuralEquivalenceDef::Bag { element } => self
                    .equivalence_domain_inner(context, *element, stack, binders, child_depth)
                    .map(Box::new)
                    .map(EquivalenceDomain::Bag),
                StructuralEquivalenceDef::Seq { element } => self
                    .equivalence_domain_inner(context, *element, stack, binders, child_depth)
                    .map(Box::new)
                    .map(EquivalenceDomain::Seq),
                StructuralEquivalenceDef::Map { key, value } => Ok(EquivalenceDomain::Map {
                    key: Box::new(self.equivalence_domain_inner(
                        context,
                        *key,
                        stack,
                        binders,
                        child_depth,
                    )?),
                    value: Box::new(self.equivalence_domain_inner(
                        context,
                        *value,
                        stack,
                        binders,
                        child_depth,
                    )?),
                }),
            };
            stack.remove(&equivalence);
            return result;
        }
        let digest = context
            .environment
            .module(equivalence)
            .ok_or(SemanticError::WrongModuleKind(equivalence))?;
        match self.equivalence_implementation(digest) {
            Some(implementation) => Ok(implementation.contract.domain()),
            None if self.module_available(digest) => {
                Err(SemanticError::WrongModuleKind(equivalence))
            }
            None => Err(SemanticError::ModuleUnavailable(digest)),
        }
    }

    pub fn equivalent(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        left: &Value,
        right: &Value,
    ) -> Result<bool, SemanticError> {
        if context.schema.structural_equivalence(equivalence).is_some() {
            self.equivalence_domain(context, equivalence)?;
        }
        self.equivalent_unchecked(context, equivalence, left, right)
    }

    fn equivalent_unchecked(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        left: &Value,
        right: &Value,
    ) -> Result<bool, SemanticError> {
        if let Some(definition) = context.schema.structural_equivalence(equivalence) {
            return self.equivalent_structural(context, equivalence, definition, left, right);
        }
        let digest = context
            .environment
            .module(equivalence)
            .ok_or(SemanticError::WrongModuleKind(equivalence))?;
        let module = match self.equivalence_implementation(digest) {
            Some(module) => module,
            None if self.module_available(digest) => {
                return Err(SemanticError::WrongModuleKind(equivalence));
            }
            None => return Err(SemanticError::ModuleUnavailable(digest)),
        };
        module.contract.equivalent(left, right, equivalence)
    }

    fn equivalent_structural(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        definition: &StructuralEquivalenceDef,
        left: &Value,
        right: &Value,
    ) -> Result<bool, SemanticError> {
        match (definition, left, right) {
            (StructuralEquivalenceDef::Mu { body }, _, _) => {
                self.equivalent_unchecked(context, *body, left, right)
            }
            (StructuralEquivalenceDef::Var { binder }, _, _) => {
                self.equivalent_unchecked(context, *binder, left, right)
            }
            (
                StructuralEquivalenceDef::Product { fields },
                Value::Product(left),
                Value::Product(right),
            ) => self.product_values_equivalent(context, fields, left, right),
            (
                StructuralEquivalenceDef::Option { inner },
                Value::Option(left),
                Value::Option(right),
            ) => match (left.as_deref(), right.as_deref()) {
                (None, None) => Ok(true),
                (Some(left), Some(right)) => {
                    self.equivalent_unchecked(context, *inner, left, right)
                }
                _ => Ok(false),
            },
            (
                StructuralEquivalenceDef::Sum { variants },
                Value::Variant {
                    tag: left_tag,
                    value: left,
                },
                Value::Variant {
                    tag: right_tag,
                    value: right,
                },
            ) => {
                if left_tag != right_tag {
                    return Ok(false);
                }
                let child = variants
                    .get(left_tag)
                    .ok_or(SemanticError::TypeMismatch(equivalence))?;
                self.equivalent_unchecked(context, *child, left, right)
            }
            (StructuralEquivalenceDef::Seq { element }, Value::Seq(left), Value::Seq(right)) => {
                self.sequence_values_equivalent(context, *element, left, right)
            }
            (
                StructuralEquivalenceDef::Set { element },
                Value::Set { elements: left, .. },
                Value::Set {
                    elements: right, ..
                },
            ) => self.unordered_values_equivalent(context, *element, left, right),
            (
                StructuralEquivalenceDef::Bag { element },
                Value::Bag { entries: left, .. },
                Value::Bag { entries: right, .. },
            ) => self.bag_values_equivalent(context, *element, left, right),
            (
                StructuralEquivalenceDef::Map { key, value },
                Value::Map { entries: left, .. },
                Value::Map { entries: right, .. },
            ) => self.map_values_equivalent(context, *key, *value, left, right),
            _ => Err(SemanticError::TypeMismatch(equivalence)),
        }
    }

    fn product_values_equivalent(
        &self,
        context: &SemanticContext,
        fields: &BTreeMap<SemanticId, SemanticId>,
        left: &BTreeMap<SemanticId, Value>,
        right: &BTreeMap<SemanticId, Value>,
    ) -> Result<bool, SemanticError> {
        if left.len() != fields.len() || right.len() != fields.len() {
            return Ok(false);
        }
        for (field, child_equivalence) in fields {
            let (Some(left), Some(right)) = (left.get(field), right.get(field)) else {
                return Ok(false);
            };
            if !self.equivalent_unchecked(context, *child_equivalence, left, right)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn sequence_values_equivalent(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        left: &[Value],
        right: &[Value],
    ) -> Result<bool, SemanticError> {
        if left.len() != right.len() {
            return Ok(false);
        }
        for (left, right) in left.iter().zip(right) {
            if !self.equivalent_unchecked(context, equivalence, left, right)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn bag_values_equivalent(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        left: &[(Value, u64)],
        right: &[(Value, u64)],
    ) -> Result<bool, SemanticError> {
        let canonicalize = |entries: &[(Value, u64)]| {
            entries
                .iter()
                .map(|(value, count)| {
                    Ok(CanonicalBagAtom {
                        value: self.canonical_equivalence_key_unchecked(
                            context,
                            equivalence,
                            value,
                        )?,
                        stored_count: *count,
                    })
                })
                .collect::<Result<Vec<_>, SemanticError>>()
                .map(finite_measure_from_atoms)
        };
        Ok(canonicalize(left)? == canonicalize(right)?)
    }

    fn map_values_equivalent(
        &self,
        context: &SemanticContext,
        key_equivalence: SemanticId,
        value_equivalence: SemanticId,
        left: &[(Value, Value)],
        right: &[(Value, Value)],
    ) -> Result<bool, SemanticError> {
        let canonicalize = |entries: &[(Value, Value)]| {
            entries
                .iter()
                .map(|(key, value)| {
                    Ok(CanonicalMapAtom {
                        key: self.canonical_equivalence_key_unchecked(
                            context,
                            key_equivalence,
                            key,
                        )?,
                        value: self.canonical_equivalence_key_unchecked(
                            context,
                            value_equivalence,
                            value,
                        )?,
                    })
                })
                .collect::<Result<Vec<_>, SemanticError>>()
                .map(finite_measure_from_atoms)
        };
        Ok(canonicalize(left)? == canonicalize(right)?)
    }

    fn unordered_values_equivalent(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        left: &[Value],
        right: &[Value],
    ) -> Result<bool, SemanticError> {
        let canonicalize = |values: &[Value]| {
            values
                .iter()
                .map(|value| self.canonical_equivalence_key_unchecked(context, equivalence, value))
                .collect::<Result<Vec<_>, _>>()
                .map(finite_measure_from_atoms)
        };
        Ok(canonicalize(left)? == canonicalize(right)?)
    }
}
