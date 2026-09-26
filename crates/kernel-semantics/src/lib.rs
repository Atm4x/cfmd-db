use std::cmp::Ordering as CmpOrdering;
use std::collections::{BTreeMap, BTreeSet};

use kernel_model::{FiniteModel, Value};
use kernel_schema::{
    ModuleDigest, ScalarType, SemanticContext, StructuralEquivalenceDef, StructuralOrderingDef,
    TypeExpr,
};
use kernel_types::SemanticId;

pub mod anchor_pullback;
pub mod observable;
pub mod support_atom;

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

fn canonical_dependency_children(definition: &StructuralEquivalenceDef) -> Vec<SemanticId> {
    match definition {
        StructuralEquivalenceDef::Var { binder } => vec![*binder],
        _ => structural_equivalence_children(definition),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FiniteMeasureEntry<T> {
    pub atom: T,
    pub multiplicity: u64,
}

pub type FiniteMeasure<T> = Vec<FiniteMeasureEntry<T>>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalBagAtom {
    pub value: CanonicalEqKey,
    pub stored_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalMapAtom {
    pub key: CanonicalEqKey,
    pub value: CanonicalEqKey,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CanonicalEqKey {
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
    Product(Vec<(SemanticId, Self)>),
    OptionNone,
    OptionSome(Box<Self>),
    Variant {
        tag: SemanticId,
        value: Box<Self>,
    },
    Seq(Vec<Self>),
    Set(FiniteMeasure<Self>),
    Bag(FiniteMeasure<CanonicalBagAtom>),
    Map(FiniteMeasure<CanonicalMapAtom>),
}

/// Allocator-independent structural estimate for semantic key work.
///
/// This is intentionally not a CPU-time claim. `value_nodes` counts logical
/// constructor/scalar visits and `payload_bytes` counts stable user payload
/// bytes (currently text). The estimate is deterministic across physical
/// layouts and is suitable for admission/scheduling heuristics that must not
/// treat a deeply nested or very large text value as equivalent to one scalar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemanticWorkEstimate {
    pub value_nodes: usize,
    pub payload_bytes: usize,
}

impl SemanticWorkEstimate {
    #[must_use]
    pub const fn scalar() -> Self {
        Self {
            value_nodes: 1,
            payload_bytes: 0,
        }
    }

    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self {
            value_nodes: self.value_nodes.saturating_add(other.value_nodes),
            payload_bytes: self.payload_bytes.saturating_add(other.payload_bytes),
        }
    }

    #[must_use]
    pub const fn total_units(self) -> usize {
        self.value_nodes.saturating_add(self.payload_bytes)
    }
}

/// Estimates the structural work required to inspect a logical value while
/// producing an exact semantic key. The metric is independent of Rust heap
/// capacity and therefore stable across row/column/algebraic layouts.
#[must_use]
pub fn semantic_value_work_estimate(value: &Value) -> SemanticWorkEstimate {
    fn add_children<'a>(children: impl IntoIterator<Item = &'a Value>) -> SemanticWorkEstimate {
        children
            .into_iter()
            .fold(SemanticWorkEstimate::scalar(), |work, child| {
                work.saturating_add(semantic_value_work_estimate(child))
            })
    }

    match value {
        Value::Unit
        | Value::Bool(_)
        | Value::I64(_)
        | Value::F64Bits(_)
        | Value::LiveEntityRef { .. }
        | Value::HistoricalEntityId { .. }
        | Value::Option(None) => SemanticWorkEstimate::scalar(),
        Value::Text(text) => SemanticWorkEstimate {
            value_nodes: 1,
            payload_bytes: text.len(),
        },
        Value::Product(fields) => add_children(fields.values()),
        Value::Option(Some(value)) | Value::Variant { value, .. } => {
            SemanticWorkEstimate::scalar().saturating_add(semantic_value_work_estimate(value))
        }
        Value::Seq(values) => add_children(values),
        Value::Set { elements, .. } => add_children(elements),
        Value::Bag { entries, .. } => add_children(entries.iter().map(|(value, _)| value)),
        Value::Map { entries, .. } => {
            entries
                .iter()
                .fold(SemanticWorkEstimate::scalar(), |work, (key, value)| {
                    work.saturating_add(semantic_value_work_estimate(key))
                        .saturating_add(semantic_value_work_estimate(value))
                })
        }
    }
}

/// Same deterministic metric after canonicalization. This is useful for
/// materialization/resource accounting without encoding or allocating bytes.
#[must_use]
pub fn canonical_eq_key_work_estimate(key: &CanonicalEqKey) -> SemanticWorkEstimate {
    fn add_keys<'a>(keys: impl IntoIterator<Item = &'a CanonicalEqKey>) -> SemanticWorkEstimate {
        keys.into_iter()
            .fold(SemanticWorkEstimate::scalar(), |work, key| {
                work.saturating_add(canonical_eq_key_work_estimate(key))
            })
    }

    match key {
        CanonicalEqKey::Unit
        | CanonicalEqKey::Bool(_)
        | CanonicalEqKey::I64(_)
        | CanonicalEqKey::F64Bits(_)
        | CanonicalEqKey::LiveEntityId { .. }
        | CanonicalEqKey::HistoricalEntityId { .. }
        | CanonicalEqKey::OptionNone => SemanticWorkEstimate::scalar(),
        CanonicalEqKey::TextExact(text) | CanonicalEqKey::TextAsciiCaseInsensitive(text) => {
            SemanticWorkEstimate {
                value_nodes: 1,
                payload_bytes: text.len(),
            }
        }
        CanonicalEqKey::Product(fields) => add_keys(fields.iter().map(|(_, value)| value)),
        CanonicalEqKey::OptionSome(value) | CanonicalEqKey::Variant { value, .. } => {
            SemanticWorkEstimate::scalar().saturating_add(canonical_eq_key_work_estimate(value))
        }
        CanonicalEqKey::Seq(values) => add_keys(values),
        CanonicalEqKey::Set(entries) => add_keys(entries.iter().map(|entry| &entry.atom)),
        CanonicalEqKey::Bag(entries) => add_keys(entries.iter().map(|entry| &entry.atom.value)),
        CanonicalEqKey::Map(entries) => {
            entries
                .iter()
                .fold(SemanticWorkEstimate::scalar(), |work, entry| {
                    work.saturating_add(canonical_eq_key_work_estimate(&entry.atom.key))
                        .saturating_add(canonical_eq_key_work_estimate(&entry.atom.value))
                })
        }
    }
}

/// Canonical finite counting measure over an ordered atom domain.
#[must_use]
pub fn finite_measure_from_atoms<T: Ord>(atoms: impl IntoIterator<Item = T>) -> FiniteMeasure<T> {
    let mut counts = BTreeMap::<T, u64>::new();
    for atom in atoms {
        let count = counts.entry(atom).or_insert(0);
        *count = count
            .checked_add(1)
            .expect("an in-memory collection cannot contain more than u64::MAX entries");
    }
    counts
        .into_iter()
        .map(|(atom, multiplicity)| FiniteMeasureEntry { atom, multiplicity })
        .collect()
}

/// Stable durable encoding revision for [`CanonicalEqKey`].
///
/// This is deliberately independent from Rust enum layout and `Ord`
/// implementation details. Any change to the byte contract must introduce a
/// new decoder revision rather than silently reinterpreting persisted keys.
pub const CANONICAL_EQ_KEY_ENCODING_VERSION: u32 = 2;

const CANONICAL_EQ_KEY_MAGIC: [u8; 4] = *b"CEK\0";
const CANONICAL_EQ_KEY_TUPLE_MAGIC: [u8; 4] = *b"CKT\0";
const MAX_CANONICAL_EQ_KEY_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalEqKeyCodecError {
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u32),
    UnknownTag(u8),
    InvalidUtf8,
    LengthOverflow,
    NonCanonicalShape,
    TrailingBytes,
    DepthLimitExceeded,
}

/// Encodes one canonical equality key using the explicit durable format.
#[must_use]
pub fn encode_canonical_eq_key(key: &CanonicalEqKey) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&CANONICAL_EQ_KEY_MAGIC);
    bytes.extend_from_slice(&CANONICAL_EQ_KEY_ENCODING_VERSION.to_be_bytes());
    encode_canonical_eq_key_payload(key, &mut bytes);
    bytes
}

/// Decodes one canonical equality key and rejects unknown format revisions.
pub fn decode_canonical_eq_key(bytes: &[u8]) -> Result<CanonicalEqKey, CanonicalEqKeyCodecError> {
    let mut cursor = CanonicalKeyCursor::new(bytes);
    if cursor.take_array::<4>()? != CANONICAL_EQ_KEY_MAGIC {
        return Err(CanonicalEqKeyCodecError::InvalidMagic);
    }
    let version = u32::from_be_bytes(cursor.take_array()?);
    if version != CANONICAL_EQ_KEY_ENCODING_VERSION {
        return Err(CanonicalEqKeyCodecError::UnsupportedVersion(version));
    }
    let key = decode_canonical_eq_key_payload(&mut cursor, 0)?;
    if !cursor.is_empty() {
        return Err(CanonicalEqKeyCodecError::TrailingBytes);
    }
    if !canonical_eq_key_shape_is_canonical(&key) {
        return Err(CanonicalEqKeyCodecError::NonCanonicalShape);
    }
    Ok(key)
}

/// Encodes an ordered composite semantic-index key without relying on Rust
/// `Vec` layout. Individual tuple components use the same recursive payload
/// grammar as [`encode_canonical_eq_key`].
#[must_use]
pub fn encode_canonical_eq_key_tuple(keys: &[CanonicalEqKey]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&CANONICAL_EQ_KEY_TUPLE_MAGIC);
    bytes.extend_from_slice(&CANONICAL_EQ_KEY_ENCODING_VERSION.to_be_bytes());
    encode_len(keys.len(), &mut bytes);
    for key in keys {
        encode_canonical_eq_key_payload(key, &mut bytes);
    }
    bytes
}

/// Decodes one ordered composite semantic-index key.
pub fn decode_canonical_eq_key_tuple(
    bytes: &[u8],
) -> Result<Vec<CanonicalEqKey>, CanonicalEqKeyCodecError> {
    let mut cursor = CanonicalKeyCursor::new(bytes);
    if cursor.take_array::<4>()? != CANONICAL_EQ_KEY_TUPLE_MAGIC {
        return Err(CanonicalEqKeyCodecError::InvalidMagic);
    }
    let version = u32::from_be_bytes(cursor.take_array()?);
    if version != CANONICAL_EQ_KEY_ENCODING_VERSION {
        return Err(CanonicalEqKeyCodecError::UnsupportedVersion(version));
    }
    let len = cursor.collection_len(1)?;
    let mut keys = Vec::with_capacity(len);
    for _ in 0..len {
        let key = decode_canonical_eq_key_payload(&mut cursor, 0)?;
        if !canonical_eq_key_shape_is_canonical(&key) {
            return Err(CanonicalEqKeyCodecError::NonCanonicalShape);
        }
        keys.push(key);
    }
    if !cursor.is_empty() {
        return Err(CanonicalEqKeyCodecError::TrailingBytes);
    }
    Ok(keys)
}

fn encode_canonical_eq_key_payload(key: &CanonicalEqKey, bytes: &mut Vec<u8>) {
    match key {
        CanonicalEqKey::Unit => bytes.push(0),
        CanonicalEqKey::Bool(value) => {
            bytes.push(1);
            bytes.push(u8::from(*value));
        }
        CanonicalEqKey::I64(value) => {
            bytes.push(2);
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        CanonicalEqKey::F64Bits(value) => {
            bytes.push(3);
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        CanonicalEqKey::TextExact(value) => encode_string_key(4, value, bytes),
        CanonicalEqKey::TextAsciiCaseInsensitive(value) => encode_string_key(5, value, bytes),
        CanonicalEqKey::LiveEntityId { entity_type, id } => {
            encode_entity_key(6, *entity_type, *id, bytes);
        }
        CanonicalEqKey::HistoricalEntityId { entity_type, id } => {
            encode_entity_key(7, *entity_type, *id, bytes);
        }
        CanonicalEqKey::Product(fields) => {
            bytes.push(8);
            encode_len(fields.len(), bytes);
            for (field, value) in fields {
                bytes.extend_from_slice(&field.raw().to_be_bytes());
                encode_canonical_eq_key_payload(value, bytes);
            }
        }
        CanonicalEqKey::OptionNone => bytes.push(9),
        CanonicalEqKey::OptionSome(value) => encode_boxed_key(10, value, bytes),
        CanonicalEqKey::Variant { tag, value } => {
            bytes.push(11);
            bytes.extend_from_slice(&tag.raw().to_be_bytes());
            encode_canonical_eq_key_payload(value, bytes);
        }
        CanonicalEqKey::Seq(values) => encode_key_vec(12, values, bytes),
        CanonicalEqKey::Set(entries) => {
            bytes.push(13);
            encode_len(entries.len(), bytes);
            for entry in entries {
                encode_canonical_eq_key_payload(&entry.atom, bytes);
                bytes.extend_from_slice(&entry.multiplicity.to_be_bytes());
            }
        }
        CanonicalEqKey::Bag(entries) => {
            bytes.push(14);
            encode_len(entries.len(), bytes);
            for entry in entries {
                encode_canonical_eq_key_payload(&entry.atom.value, bytes);
                bytes.extend_from_slice(&entry.atom.stored_count.to_be_bytes());
                bytes.extend_from_slice(&entry.multiplicity.to_be_bytes());
            }
        }
        CanonicalEqKey::Map(entries) => {
            bytes.push(15);
            encode_len(entries.len(), bytes);
            for entry in entries {
                encode_canonical_eq_key_payload(&entry.atom.key, bytes);
                encode_canonical_eq_key_payload(&entry.atom.value, bytes);
                bytes.extend_from_slice(&entry.multiplicity.to_be_bytes());
            }
        }
    }
}

fn encode_len(len: usize, bytes: &mut Vec<u8>) {
    let len = u64::try_from(len).unwrap_or(u64::MAX);
    bytes.extend_from_slice(&len.to_be_bytes());
}

fn encode_string_key(tag: u8, value: &str, bytes: &mut Vec<u8>) {
    bytes.push(tag);
    encode_len(value.len(), bytes);
    bytes.extend_from_slice(value.as_bytes());
}

fn encode_entity_key(
    tag: u8,
    entity_type: SemanticId,
    id: kernel_types::EntityId,
    bytes: &mut Vec<u8>,
) {
    bytes.push(tag);
    bytes.extend_from_slice(&entity_type.raw().to_be_bytes());
    bytes.extend_from_slice(&id.raw().to_be_bytes());
}

fn encode_boxed_key(tag: u8, value: &CanonicalEqKey, bytes: &mut Vec<u8>) {
    bytes.push(tag);
    encode_canonical_eq_key_payload(value, bytes);
}

fn encode_key_vec(tag: u8, values: &[CanonicalEqKey], bytes: &mut Vec<u8>) {
    bytes.push(tag);
    encode_len(values.len(), bytes);
    for value in values {
        encode_canonical_eq_key_payload(value, bytes);
    }
}

fn decode_canonical_eq_key_payload(
    cursor: &mut CanonicalKeyCursor<'_>,
    depth: usize,
) -> Result<CanonicalEqKey, CanonicalEqKeyCodecError> {
    if depth > MAX_CANONICAL_EQ_KEY_DEPTH {
        return Err(CanonicalEqKeyCodecError::DepthLimitExceeded);
    }
    let child_depth = depth.saturating_add(1);
    let tag = cursor.u8()?;
    match tag {
        0 => Ok(CanonicalEqKey::Unit),
        1 => match cursor.u8()? {
            0 => Ok(CanonicalEqKey::Bool(false)),
            1 => Ok(CanonicalEqKey::Bool(true)),
            _ => Err(CanonicalEqKeyCodecError::NonCanonicalShape),
        },
        2 => Ok(CanonicalEqKey::I64(i64::from_be_bytes(
            cursor.take_array()?,
        ))),
        3 => Ok(CanonicalEqKey::F64Bits(u64::from_be_bytes(
            cursor.take_array()?,
        ))),
        4 => cursor.string().map(CanonicalEqKey::TextExact),
        5 => cursor
            .string()
            .map(CanonicalEqKey::TextAsciiCaseInsensitive),
        6 => decode_entity_key(cursor, false),
        7 => decode_entity_key(cursor, true),
        8 => decode_product_key(cursor, child_depth),
        9 => Ok(CanonicalEqKey::OptionNone),
        10 => decode_canonical_eq_key_payload(cursor, child_depth)
            .map(Box::new)
            .map(CanonicalEqKey::OptionSome),
        11 => {
            let tag = SemanticId::new(u128::from_be_bytes(cursor.take_array()?));
            let value = Box::new(decode_canonical_eq_key_payload(cursor, child_depth)?);
            Ok(CanonicalEqKey::Variant { tag, value })
        }
        12 => decode_key_vec(cursor, child_depth).map(CanonicalEqKey::Seq),
        13 => decode_set_key(cursor, child_depth),
        14 => decode_bag_key(cursor, child_depth),
        15 => decode_map_key(cursor, child_depth),
        other => Err(CanonicalEqKeyCodecError::UnknownTag(other)),
    }
}

fn decode_entity_key(
    cursor: &mut CanonicalKeyCursor<'_>,
    historical: bool,
) -> Result<CanonicalEqKey, CanonicalEqKeyCodecError> {
    let entity_type = SemanticId::new(u128::from_be_bytes(cursor.take_array()?));
    let id = kernel_types::EntityId::new(u128::from_be_bytes(cursor.take_array()?));
    Ok(if historical {
        CanonicalEqKey::HistoricalEntityId { entity_type, id }
    } else {
        CanonicalEqKey::LiveEntityId { entity_type, id }
    })
}

fn decode_product_key(
    cursor: &mut CanonicalKeyCursor<'_>,
    depth: usize,
) -> Result<CanonicalEqKey, CanonicalEqKeyCodecError> {
    let len = cursor.collection_len(17)?;
    let mut fields = Vec::with_capacity(len);
    for _ in 0..len {
        let field = SemanticId::new(u128::from_be_bytes(cursor.take_array()?));
        fields.push((field, decode_canonical_eq_key_payload(cursor, depth)?));
    }
    Ok(CanonicalEqKey::Product(fields))
}

fn decode_key_vec(
    cursor: &mut CanonicalKeyCursor<'_>,
    depth: usize,
) -> Result<Vec<CanonicalEqKey>, CanonicalEqKeyCodecError> {
    let len = cursor.collection_len(1)?;
    (0..len)
        .map(|_| decode_canonical_eq_key_payload(cursor, depth))
        .collect()
}

fn decode_set_key(
    cursor: &mut CanonicalKeyCursor<'_>,
    depth: usize,
) -> Result<CanonicalEqKey, CanonicalEqKeyCodecError> {
    let len = cursor.collection_len(9)?;
    let mut entries = Vec::with_capacity(len);
    for _ in 0..len {
        let atom = decode_canonical_eq_key_payload(cursor, depth)?;
        let multiplicity = u64::from_be_bytes(cursor.take_array()?);
        entries.push(FiniteMeasureEntry { atom, multiplicity });
    }
    Ok(CanonicalEqKey::Set(entries))
}

fn decode_bag_key(
    cursor: &mut CanonicalKeyCursor<'_>,
    depth: usize,
) -> Result<CanonicalEqKey, CanonicalEqKeyCodecError> {
    let len = cursor.collection_len(9)?;
    let mut entries = Vec::with_capacity(len);
    for _ in 0..len {
        let value = decode_canonical_eq_key_payload(cursor, depth)?;
        let stored_count = u64::from_be_bytes(cursor.take_array()?);
        let multiplicity = u64::from_be_bytes(cursor.take_array()?);
        entries.push(FiniteMeasureEntry {
            atom: CanonicalBagAtom {
                value,
                stored_count,
            },
            multiplicity,
        });
    }
    Ok(CanonicalEqKey::Bag(entries))
}

fn decode_map_key(
    cursor: &mut CanonicalKeyCursor<'_>,
    depth: usize,
) -> Result<CanonicalEqKey, CanonicalEqKeyCodecError> {
    let len = cursor.collection_len(2)?;
    let mut entries = Vec::with_capacity(len);
    for _ in 0..len {
        let key = decode_canonical_eq_key_payload(cursor, depth)?;
        let value = decode_canonical_eq_key_payload(cursor, depth)?;
        let multiplicity = u64::from_be_bytes(cursor.take_array()?);
        entries.push(FiniteMeasureEntry {
            atom: CanonicalMapAtom { key, value },
            multiplicity,
        });
    }
    Ok(CanonicalEqKey::Map(entries))
}

fn canonical_eq_key_shape_is_canonical(key: &CanonicalEqKey) -> bool {
    match key {
        CanonicalEqKey::Product(fields) => {
            fields.windows(2).all(|pair| pair[0].0 < pair[1].0)
                && fields
                    .iter()
                    .all(|(_, child)| canonical_eq_key_shape_is_canonical(child))
        }
        CanonicalEqKey::OptionSome(value) | CanonicalEqKey::Variant { value, .. } => {
            canonical_eq_key_shape_is_canonical(value)
        }
        CanonicalEqKey::Seq(values) => values.iter().all(canonical_eq_key_shape_is_canonical),
        CanonicalEqKey::Set(entries) => {
            entries.windows(2).all(|pair| pair[0].atom < pair[1].atom)
                && entries.iter().all(|entry| {
                    entry.multiplicity > 0 && canonical_eq_key_shape_is_canonical(&entry.atom)
                })
        }
        CanonicalEqKey::Bag(entries) => {
            entries.windows(2).all(|pair| pair[0].atom < pair[1].atom)
                && entries.iter().all(|entry| {
                    entry.multiplicity > 0
                        && entry.atom.stored_count > 0
                        && canonical_eq_key_shape_is_canonical(&entry.atom.value)
                })
        }
        CanonicalEqKey::Map(entries) => {
            entries.windows(2).all(|pair| pair[0].atom < pair[1].atom)
                && entries.iter().all(|entry| {
                    entry.multiplicity > 0
                        && canonical_eq_key_shape_is_canonical(&entry.atom.key)
                        && canonical_eq_key_shape_is_canonical(&entry.atom.value)
                })
        }
        CanonicalEqKey::Unit
        | CanonicalEqKey::Bool(_)
        | CanonicalEqKey::I64(_)
        | CanonicalEqKey::F64Bits(_)
        | CanonicalEqKey::TextExact(_)
        | CanonicalEqKey::LiveEntityId { .. }
        | CanonicalEqKey::HistoricalEntityId { .. }
        | CanonicalEqKey::OptionNone => true,
        CanonicalEqKey::TextAsciiCaseInsensitive(value) => value == &value.to_ascii_lowercase(),
    }
}

struct CanonicalKeyCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> CanonicalKeyCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], CanonicalEqKeyCodecError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(CanonicalEqKeyCodecError::LengthOverflow)?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or(CanonicalEqKeyCodecError::Truncated)?;
        self.offset = end;
        Ok(slice)
    }

    fn take_array<const N: usize>(&mut self) -> Result<[u8; N], CanonicalEqKeyCodecError> {
        self.take(N)?
            .try_into()
            .map_err(|_| CanonicalEqKeyCodecError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, CanonicalEqKeyCodecError> {
        self.take(1).map(|slice| slice[0])
    }

    fn len(&mut self) -> Result<usize, CanonicalEqKeyCodecError> {
        usize::try_from(u64::from_be_bytes(self.take_array()?))
            .map_err(|_| CanonicalEqKeyCodecError::LengthOverflow)
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn collection_len(
        &mut self,
        minimum_entry_bytes: usize,
    ) -> Result<usize, CanonicalEqKeyCodecError> {
        let len = self.len()?;
        if len > self.remaining() / minimum_entry_bytes {
            return Err(CanonicalEqKeyCodecError::Truncated);
        }
        Ok(len)
    }

    fn string(&mut self) -> Result<String, CanonicalEqKeyCodecError> {
        let len = self.len()?;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| CanonicalEqKeyCodecError::InvalidUtf8)
    }
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
struct EquivalenceImplementation {
    contract: EquivalenceModule,
    implementation_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerModule {
    AsciiWhitespace,
    AsciiWhitespaceLowercase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TokenizerImplementation {
    contract: TokenizerModule,
    implementation_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
pub enum SemanticContract {
    Equivalence(EquivalenceModule),
    Tokenizer(TokenizerModule),
    Ordering(OrderingModule),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticImplementationArtifact {
    BuiltinEquivalence(EquivalenceModule),
    BuiltinTokenizer(TokenizerModule),
    BuiltinOrdering(OrderingModule),
}

impl SemanticImplementationArtifact {
    #[must_use]
    pub const fn contract(&self) -> SemanticContract {
        match self {
            Self::BuiltinEquivalence(module) => SemanticContract::Equivalence(*module),
            Self::BuiltinTokenizer(module) => SemanticContract::Tokenizer(*module),
            Self::BuiltinOrdering(module) => SemanticContract::Ordering(*module),
        }
    }
}

pub struct SemanticImplementationChecker;

impl kernel_proof::CertificateChecker for SemanticImplementationChecker {
    type Spec = SemanticContract;
    type Certificate = SemanticImplementationArtifact;
    type Error = SemanticError;

    fn check(spec: &Self::Spec, certificate: &Self::Certificate) -> Result<(), Self::Error> {
        (certificate.contract() == *spec)
            .then_some(())
            .ok_or(SemanticError::ImplementationContractMismatch)
    }
}

pub fn certify_implementation(
    contract: &SemanticContract,
    artifact: SemanticImplementationArtifact,
) -> Result<kernel_proof::CheckedCertificate<SemanticImplementationChecker>, SemanticError> {
    kernel_proof::verify_certificate::<SemanticImplementationChecker>(contract, artifact)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderingCompatibilitySpec {
    pub ordering: OrderingModule,
    pub equivalence: EquivalenceModule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderingCompatibilityArtifact {
    BuiltinLaw,
}

pub struct OrderingCompatibilityChecker;

impl kernel_proof::CertificateChecker for OrderingCompatibilityChecker {
    type Spec = OrderingCompatibilitySpec;
    type Certificate = OrderingCompatibilityArtifact;
    type Error = SemanticError;

    fn check(spec: &Self::Spec, _: &Self::Certificate) -> Result<(), Self::Error> {
        builtin_ordering_compatibility_holds(*spec)
            .then_some(())
            .ok_or(SemanticError::OrderingCompatibilityViolation)
    }
}

pub fn certify_ordering_compatibility(
    spec: &OrderingCompatibilitySpec,
    artifact: OrderingCompatibilityArtifact,
) -> Result<kernel_proof::CheckedCertificate<OrderingCompatibilityChecker>, SemanticError> {
    kernel_proof::verify_certificate::<OrderingCompatibilityChecker>(spec, artifact)
}

const fn builtin_ordering_compatibility_holds(spec: OrderingCompatibilitySpec) -> bool {
    matches!(
        (spec.ordering, spec.equivalence),
        (OrderingModule::I64Ascending, EquivalenceModule::I64Exact)
            | (OrderingModule::F64Total, EquivalenceModule::F64Bitwise)
            | (
                OrderingModule::TextBinary
                    | OrderingModule::TextAsciiCaseInsensitive
                    | OrderingModule::TextAsciiCaseInsensitiveThenBinary,
                EquivalenceModule::TextExact
            )
            | (
                OrderingModule::TextAsciiCaseInsensitive,
                EquivalenceModule::TextAsciiCaseInsensitive
            )
    )
}

const BUILTIN_ORDERING_COMPATIBILITIES: [OrderingCompatibilitySpec; 6] = [
    OrderingCompatibilitySpec {
        ordering: OrderingModule::I64Ascending,
        equivalence: EquivalenceModule::I64Exact,
    },
    OrderingCompatibilitySpec {
        ordering: OrderingModule::F64Total,
        equivalence: EquivalenceModule::F64Bitwise,
    },
    OrderingCompatibilitySpec {
        ordering: OrderingModule::TextBinary,
        equivalence: EquivalenceModule::TextExact,
    },
    OrderingCompatibilitySpec {
        ordering: OrderingModule::TextAsciiCaseInsensitive,
        equivalence: EquivalenceModule::TextExact,
    },
    OrderingCompatibilitySpec {
        ordering: OrderingModule::TextAsciiCaseInsensitiveThenBinary,
        equivalence: EquivalenceModule::TextExact,
    },
    OrderingCompatibilitySpec {
        ordering: OrderingModule::TextAsciiCaseInsensitive,
        equivalence: EquivalenceModule::TextAsciiCaseInsensitive,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OrderingImplementation {
    contract: OrderingModule,
    implementation_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticError {
    ModuleUnavailable(ModuleDigest),
    WrongModuleKind(SemanticId),
    TypeMismatch(SemanticId),
    EquivalenceDomainMismatch {
        equivalence: SemanticId,
        expected: EquivalenceDomain,
        actual: EquivalenceDomain,
    },
    DuplicateSetElement {
        equivalence: SemanticId,
    },
    DuplicateMapKey {
        equivalence: SemanticId,
    },
    DuplicateBagElement {
        equivalence: SemanticId,
    },
    DuplicateRelationRow,
    CyclicStructuralEquivalence(SemanticId),
    CyclicStructuralOrdering(SemanticId),
    FreeStructuralRecursion(SemanticId),
    UnguardedStructuralRecursion(SemanticId),
    ImplementationContractMismatch,
    OrderingCompatibilityViolation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticRegistry {
    equivalences: BTreeMap<ModuleDigest, EquivalenceImplementation>,
    tokenizers: BTreeMap<ModuleDigest, TokenizerImplementation>,
    orderings: BTreeMap<ModuleDigest, OrderingImplementation>,
    ordering_compatibilities: Vec<OrderingCompatibilitySpec>,
}

/// Durable description of one semantic implementation provided by the CFMD
/// binary itself.  The descriptor is intentionally limited to the builtin
/// implementation families currently present in `SemanticRegistry`; arbitrary
/// executable code is not smuggled through durable storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinSemanticModuleSpec {
    Equivalence {
        module: EquivalenceModule,
        implementation_revision: u64,
    },
    Tokenizer {
        module: TokenizerModule,
        implementation_revision: u64,
    },
    Ordering {
        module: OrderingModule,
        implementation_revision: u64,
    },
}

impl BuiltinSemanticModuleSpec {
    #[must_use]
    pub const fn contract(self) -> SemanticContract {
        match self {
            Self::Equivalence { module, .. } => SemanticContract::Equivalence(module),
            Self::Tokenizer { module, .. } => SemanticContract::Tokenizer(module),
            Self::Ordering { module, .. } => SemanticContract::Ordering(module),
        }
    }

    #[must_use]
    pub const fn artifact(self) -> SemanticImplementationArtifact {
        match self {
            Self::Equivalence { module, .. } => {
                SemanticImplementationArtifact::BuiltinEquivalence(module)
            }
            Self::Tokenizer { module, .. } => {
                SemanticImplementationArtifact::BuiltinTokenizer(module)
            }
            Self::Ordering { module, .. } => {
                SemanticImplementationArtifact::BuiltinOrdering(module)
            }
        }
    }

    #[must_use]
    pub fn digest(self) -> ModuleDigest {
        match self {
            Self::Equivalence {
                module,
                implementation_revision,
            } => EquivalenceImplementation {
                contract: module,
                implementation_revision,
            }
            .digest(),
            Self::Tokenizer {
                module,
                implementation_revision,
            } => TokenizerImplementation {
                contract: module,
                implementation_revision,
            }
            .digest(),
            Self::Ordering {
                module,
                implementation_revision,
            } => OrderingImplementation {
                contract: module,
                implementation_revision,
            }
            .digest(),
        }
    }
}

/// Stable identity of one executable implementation artifact. This is
/// deliberately distinct from semantic contract identity: an authenticated
/// artifact can still implement the wrong contract. Current builtins derive
/// this identity from their durable implementation descriptor; external
/// package byte hashing belongs to the deployment layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ImplementationArtifactDigest(pub [u8; 32]);

/// Identity of the execution ABI/sandbox/runtime assumptions under which a
/// semantic implementation is certified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RuntimeProfileDigest(pub [u8; 32]);

pub const BUILTIN_RUNTIME_PROFILE: RuntimeProfileDigest = RuntimeProfileDigest([0xCF; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticContractIdentity {
    /// A complete semantic specification. A different executable artifact may
    /// run it only after checked refinement to this same contract.
    Defined(SemanticContract),
    /// No independent complete specification exists; exact artifact bytes and
    /// runtime therefore participate in semantic identity.
    OpaqueArtifact {
        artifact: ImplementationArtifactDigest,
        runtime: RuntimeProfileDigest,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticRefinementCertificate {
    Builtin(SemanticImplementationArtifact),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticExecutableArtifact {
    Builtin(BuiltinSemanticModuleSpec),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticImplementationPackage {
    pub contract: SemanticContractIdentity,
    pub artifact_digest: ImplementationArtifactDigest,
    pub runtime_profile: RuntimeProfileDigest,
    pub refinement: Option<SemanticRefinementCertificate>,
    pub executable: Option<SemanticExecutableArtifact>,
}

impl SemanticImplementationPackage {
    #[must_use]
    pub fn builtin(spec: BuiltinSemanticModuleSpec) -> Self {
        Self {
            contract: SemanticContractIdentity::Defined(spec.contract()),
            artifact_digest: ImplementationArtifactDigest(spec.digest().0),
            runtime_profile: BUILTIN_RUNTIME_PROFILE,
            refinement: Some(SemanticRefinementCertificate::Builtin(spec.artifact())),
            executable: Some(SemanticExecutableArtifact::Builtin(spec)),
        }
    }
}

/// Evidence that an external deployment/security layer authenticated exactly
/// these artifact digests. The semantic kernel consumes this evidence but does
/// not decide trust roots, signature suites or key rotation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactAuthenticationSet {
    verified: BTreeSet<ImplementationArtifactDigest>,
}

impl ArtifactAuthenticationSet {
    pub fn mark_verified(&mut self, artifact: ImplementationArtifactDigest) {
        self.verified.insert(artifact);
    }

    #[must_use]
    pub fn contains(&self, artifact: ImplementationArtifactDigest) -> bool {
        self.verified.contains(&artifact)
    }

    #[must_use]
    pub fn trusted_builtins(specs: &[BuiltinSemanticModuleSpec]) -> Self {
        Self {
            verified: specs
                .iter()
                .map(|spec| ImplementationArtifactDigest(spec.digest().0))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticExecutionPolicy {
    pub allowed_runtime_profiles: BTreeSet<RuntimeProfileDigest>,
    pub revoked_artifacts: BTreeSet<ImplementationArtifactDigest>,
    pub require_authentication: bool,
}

impl SemanticExecutionPolicy {
    #[must_use]
    pub fn trusted_builtin_only() -> Self {
        Self {
            allowed_runtime_profiles: BTreeSet::from([BUILTIN_RUNTIME_PROFILE]),
            revoked_artifacts: BTreeSet::new(),
            require_authentication: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticDeploymentError {
    ArtifactDoesNotMatchOpaqueContract,
    RuntimeDoesNotMatchOpaqueContract,
    MissingRefinementCertificate,
    RefinementContractMismatch,
    UnauthenticatedArtifact,
    RuntimeProfileForbidden,
    RevokedArtifact,
    ContractUnavailable,
    ExecutableUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionAuthorization {
    package: SemanticImplementationPackage,
}

impl ExecutionAuthorization {
    #[must_use]
    pub const fn contract(&self) -> SemanticContractIdentity {
        self.package.contract
    }

    #[must_use]
    pub const fn artifact_digest(&self) -> ImplementationArtifactDigest {
        self.package.artifact_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticExecutionCapability {
    pub authorized: Vec<ExecutionAuthorization>,
    pub unavailable: Vec<(SemanticContractIdentity, SemanticDeploymentError)>,
}

impl SemanticExecutionCapability {
    #[must_use]
    pub fn executable(&self) -> bool {
        self.unavailable.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticDeploymentRegistry {
    packages: BTreeMap<ImplementationArtifactDigest, SemanticImplementationPackage>,
}

impl SemanticDeploymentRegistry {
    pub fn register(&mut self, package: SemanticImplementationPackage) {
        self.packages.insert(package.artifact_digest, package);
    }

    #[must_use]
    pub fn from_builtin_specs(specs: &[BuiltinSemanticModuleSpec]) -> Self {
        let mut registry = Self::default();
        for &spec in specs {
            registry.register(SemanticImplementationPackage::builtin(spec));
        }
        registry
    }

    pub fn authorize_contract(
        &self,
        contract: SemanticContractIdentity,
        policy: &SemanticExecutionPolicy,
        authentications: &ArtifactAuthenticationSet,
    ) -> Result<ExecutionAuthorization, SemanticDeploymentError> {
        let mut matched = false;
        let mut first_error = None;
        for package in self
            .packages
            .values()
            .filter(|package| package.contract == contract)
        {
            matched = true;
            match authorize_package(package, policy, authentications) {
                Ok(authorization) => return Ok(authorization),
                Err(error) => first_error.get_or_insert(error),
            };
        }
        if matched {
            Err(first_error.unwrap_or(SemanticDeploymentError::ContractUnavailable))
        } else {
            Err(SemanticDeploymentError::ContractUnavailable)
        }
    }

    pub fn authorize_artifact(
        &self,
        artifact: ImplementationArtifactDigest,
        policy: &SemanticExecutionPolicy,
        authentications: &ArtifactAuthenticationSet,
    ) -> Result<ExecutionAuthorization, SemanticDeploymentError> {
        let package = self
            .packages
            .get(&artifact)
            .ok_or(SemanticDeploymentError::ContractUnavailable)?;
        authorize_package(package, policy, authentications)
    }

    pub fn authorize_required(
        &self,
        required: &[SemanticContractIdentity],
        policy: &SemanticExecutionPolicy,
        authentications: &ArtifactAuthenticationSet,
    ) -> Result<Vec<ExecutionAuthorization>, SemanticDeploymentError> {
        let capability = self.execution_capability(required, policy, authentications);
        if let Some((_, error)) = capability.unavailable.first() {
            return Err(*error);
        }
        Ok(capability.authorized)
    }

    #[must_use]
    pub fn execution_capability(
        &self,
        required: &[SemanticContractIdentity],
        policy: &SemanticExecutionPolicy,
        authentications: &ArtifactAuthenticationSet,
    ) -> SemanticExecutionCapability {
        let mut authorized = Vec::new();
        let mut unavailable = Vec::new();
        for &contract in required {
            match self.authorize_contract(contract, policy, authentications) {
                Ok(authorization) => authorized.push(authorization),
                Err(error) => unavailable.push((contract, error)),
            }
        }
        SemanticExecutionCapability {
            authorized,
            unavailable,
        }
    }

    pub fn install_authorized_builtin(
        authorization: &ExecutionAuthorization,
        registry: &mut SemanticRegistry,
    ) -> Result<ModuleDigest, SemanticDeploymentError> {
        match authorization.package.executable {
            Some(SemanticExecutableArtifact::Builtin(spec)) => {
                Ok(registry.install_builtin_module_spec(spec))
            }
            None => Err(SemanticDeploymentError::ExecutableUnavailable),
        }
    }
}

fn authorize_package(
    package: &SemanticImplementationPackage,
    policy: &SemanticExecutionPolicy,
    authentications: &ArtifactAuthenticationSet,
) -> Result<ExecutionAuthorization, SemanticDeploymentError> {
    match package.contract {
        SemanticContractIdentity::Defined(contract) => {
            let Some(SemanticRefinementCertificate::Builtin(artifact)) = package.refinement else {
                return Err(SemanticDeploymentError::MissingRefinementCertificate);
            };
            certify_implementation(&contract, artifact)
                .map_err(|_| SemanticDeploymentError::RefinementContractMismatch)?;
            if let Some(SemanticExecutableArtifact::Builtin(spec)) = package.executable
                && (spec.artifact() != artifact
                    || ImplementationArtifactDigest(spec.digest().0) != package.artifact_digest
                    || package.runtime_profile != BUILTIN_RUNTIME_PROFILE)
            {
                return Err(SemanticDeploymentError::RefinementContractMismatch);
            }
        }
        SemanticContractIdentity::OpaqueArtifact { artifact, runtime } => {
            if package.artifact_digest != artifact {
                return Err(SemanticDeploymentError::ArtifactDoesNotMatchOpaqueContract);
            }
            if package.runtime_profile != runtime {
                return Err(SemanticDeploymentError::RuntimeDoesNotMatchOpaqueContract);
            }
        }
    }
    if policy.require_authentication && !authentications.contains(package.artifact_digest) {
        return Err(SemanticDeploymentError::UnauthenticatedArtifact);
    }
    if !policy
        .allowed_runtime_profiles
        .contains(&package.runtime_profile)
    {
        return Err(SemanticDeploymentError::RuntimeProfileForbidden);
    }
    if policy.revoked_artifacts.contains(&package.artifact_digest) {
        return Err(SemanticDeploymentError::RevokedArtifact);
    }
    Ok(ExecutionAuthorization {
        package: package.clone(),
    })
}

impl Default for SemanticRegistry {
    fn default() -> Self {
        let mut registry = Self {
            equivalences: BTreeMap::new(),
            tokenizers: BTreeMap::new(),
            orderings: BTreeMap::new(),
            ordering_compatibilities: Vec::new(),
        };
        for spec in BUILTIN_ORDERING_COMPATIBILITIES {
            if let Ok(checked) =
                certify_ordering_compatibility(&spec, OrderingCompatibilityArtifact::BuiltinLaw)
            {
                registry.install_certified_ordering_compatibility(checked);
            }
        }
        registry
    }
}

impl SemanticRegistry {
    /// Installs one builtin implementation from its durable descriptor and
    /// returns the digest that callers must compare with the pinned semantic
    /// environment.
    pub fn install_builtin_module_spec(&mut self, spec: BuiltinSemanticModuleSpec) -> ModuleDigest {
        match spec {
            BuiltinSemanticModuleSpec::Equivalence {
                module,
                implementation_revision,
            } => self.install_equivalence_revision(module, implementation_revision),
            BuiltinSemanticModuleSpec::Tokenizer {
                module,
                implementation_revision,
            } => self.install_tokenizer_revision(module, implementation_revision),
            BuiltinSemanticModuleSpec::Ordering {
                module,
                implementation_revision,
            } => self.install_ordering_revision(module, implementation_revision),
        }
    }

    /// Returns the builtin descriptor for an installed digest.  At present all
    /// executable semantic modules supported by CFMD are builtin contracts, so
    /// a pinned digest without a descriptor is a deployment error.
    #[must_use]
    pub fn builtin_module_spec(&self, digest: ModuleDigest) -> Option<BuiltinSemanticModuleSpec> {
        if let Some(implementation) = self.equivalences.get(&digest) {
            return Some(BuiltinSemanticModuleSpec::Equivalence {
                module: implementation.contract,
                implementation_revision: implementation.implementation_revision,
            });
        }
        if let Some(implementation) = self.tokenizers.get(&digest) {
            return Some(BuiltinSemanticModuleSpec::Tokenizer {
                module: implementation.contract,
                implementation_revision: implementation.implementation_revision,
            });
        }
        self.orderings
            .get(&digest)
            .map(|implementation| BuiltinSemanticModuleSpec::Ordering {
                module: implementation.contract,
                implementation_revision: implementation.implementation_revision,
            })
    }

    /// Collects the exact builtin implementations pinned by one semantic
    /// context, sorted by digest and deduplicated.  This is the deployment
    /// manifest persisted by the durability layer.
    pub fn builtin_modules_for_context(
        &self,
        context: &SemanticContext,
    ) -> Result<Vec<BuiltinSemanticModuleSpec>, SemanticError> {
        self.validate_context(context)?;
        let mut by_digest = BTreeMap::new();
        for (_, digest) in context.environment.modules() {
            let spec = self
                .builtin_module_spec(digest)
                .ok_or(SemanticError::ModuleUnavailable(digest))?;
            by_digest.insert(digest, spec);
        }
        Ok(by_digest.into_values().collect())
    }

    pub fn install_certified_ordering_compatibility(
        &mut self,
        checked: kernel_proof::CheckedCertificate<OrderingCompatibilityChecker>,
    ) {
        let spec = *checked.spec();
        let _ = checked.into_inner();
        if !self.ordering_compatibilities.contains(&spec) {
            self.ordering_compatibilities.push(spec);
        }
    }

    pub fn install_certified_implementation(
        &mut self,
        checked: kernel_proof::CheckedCertificate<SemanticImplementationChecker>,
        implementation_revision: u64,
    ) -> ModuleDigest {
        match checked.into_inner() {
            SemanticImplementationArtifact::BuiltinEquivalence(module) => {
                self.install_equivalence_revision(module, implementation_revision)
            }
            SemanticImplementationArtifact::BuiltinTokenizer(module) => {
                self.install_tokenizer_revision(module, implementation_revision)
            }
            SemanticImplementationArtifact::BuiltinOrdering(module) => {
                self.install_ordering_revision(module, implementation_revision)
            }
        }
    }
    pub fn contexts_semantically_equivalent(
        &self,
        left: &SemanticContext,
        right: &SemanticContext,
    ) -> Result<bool, SemanticError> {
        self.validate_context(left)?;
        self.validate_context(right)?;
        if !left.schema.definitionally_equivalent(&right.schema) {
            return Ok(false);
        }
        let left_count = left.environment.modules().count();
        let right_count = right.environment.modules().count();
        if left_count != right_count {
            return Ok(false);
        }
        for (symbol, left_digest) in left.environment.modules() {
            let Some(right_digest) = right.environment.module(symbol) else {
                return Ok(false);
            };
            if !self.equivalent_implementation_contract(left_digest, right_digest)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn context_conservatively_extends(
        &self,
        source: &SemanticContext,
        target: &SemanticContext,
    ) -> Result<bool, SemanticError> {
        self.validate_context(source)?;
        self.validate_context(target)?;
        if !source.schema.definitionally_equivalent(&target.schema) {
            return Ok(false);
        }
        for (symbol, source_digest) in source.environment.modules() {
            let Some(target_digest) = target.environment.module(symbol) else {
                return Ok(false);
            };
            if !self.equivalent_implementation_contract(source_digest, target_digest)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn validate_context(&self, context: &SemanticContext) -> Result<(), SemanticError> {
        for (_, digest) in context.environment.modules() {
            if !self.module_available(digest) {
                return Err(SemanticError::ModuleUnavailable(digest));
            }
        }
        for dependency in context.schema.semantic_dependencies() {
            let digest = context
                .environment
                .module(dependency)
                .ok_or(SemanticError::WrongModuleKind(dependency))?;
            if !self.module_available(digest) {
                return Err(SemanticError::ModuleUnavailable(digest));
            }
            if !self.equivalences.contains_key(&digest) {
                return Err(SemanticError::WrongModuleKind(dependency));
            }
        }
        self.validate_structural_equivalence_graph(context)?;
        self.validate_structural_ordering_graph(context)?;
        for relation in context.schema.relations() {
            let column_equivalences = match &relation.semantics {
                kernel_schema::RelationSemantics::Set {
                    column_equivalences,
                }
                | kernel_schema::RelationSemantics::Bag {
                    column_equivalences,
                } => column_equivalences,
            };
            for (column, equivalence) in relation.columns.iter().zip(column_equivalences) {
                let expected =
                    domain_for_type(column).ok_or(SemanticError::TypeMismatch(*equivalence))?;
                let actual = self.equivalence_domain(context, *equivalence)?;
                if expected != actual {
                    return Err(SemanticError::EquivalenceDomainMismatch {
                        equivalence: *equivalence,
                        expected,
                        actual,
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_structural_ordering_graph(
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

    fn validate_structural_equivalence_graph(
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

    fn module_available(&self, digest: ModuleDigest) -> bool {
        self.equivalences.contains_key(&digest)
            || self.tokenizers.contains_key(&digest)
            || self.orderings.contains_key(&digest)
    }

    pub fn install_ordering(&mut self, module: OrderingModule) -> ModuleDigest {
        self.install_ordering_revision(module, 0)
    }

    pub fn install_ordering_revision(
        &mut self,
        module: OrderingModule,
        implementation_revision: u64,
    ) -> ModuleDigest {
        let implementation = OrderingImplementation {
            contract: module,
            implementation_revision,
        };
        let digest = implementation.digest();
        self.orderings.insert(digest, implementation);
        digest
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
            .orderings
            .get(&digest)
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
            .orderings
            .get(&digest)
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
        Ok(self
            .ordering_compatibilities
            .contains(&OrderingCompatibilitySpec {
                ordering,
                equivalence,
            }))
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
        match self.orderings.get(&digest).copied() {
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
            return Ok(self
                .ordering_compatibilities
                .contains(&OrderingCompatibilitySpec {
                    ordering,
                    equivalence,
                }));
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
        match self.orderings.get(&digest).copied() {
            Some(implementation) => Ok(implementation.contract),
            None if self.module_available(digest) => Err(SemanticError::WrongModuleKind(ordering)),
            None => Err(SemanticError::ModuleUnavailable(digest)),
        }
    }

    pub fn install_tokenizer(&mut self, module: TokenizerModule) -> ModuleDigest {
        self.install_tokenizer_revision(module, 0)
    }

    pub fn install_tokenizer_revision(
        &mut self,
        module: TokenizerModule,
        implementation_revision: u64,
    ) -> ModuleDigest {
        let implementation = TokenizerImplementation {
            contract: module,
            implementation_revision,
        };
        let digest = implementation.digest();
        self.tokenizers.insert(digest, implementation);
        digest
    }

    pub fn tokenize(
        &self,
        context: &SemanticContext,
        tokenizer: SemanticId,
        text: &str,
    ) -> Result<Vec<String>, SemanticError> {
        let digest = context
            .environment
            .module(tokenizer)
            .ok_or(SemanticError::WrongModuleKind(tokenizer))?;
        let implementation = self
            .tokenizers
            .get(&digest)
            .ok_or(SemanticError::WrongModuleKind(tokenizer))?;
        Ok(implementation.contract.tokenize(text))
    }

    pub fn install_equivalence(&mut self, module: EquivalenceModule) -> ModuleDigest {
        self.install_equivalence_revision(module, 0)
    }

    pub fn install_equivalence_revision(
        &mut self,
        module: EquivalenceModule,
        implementation_revision: u64,
    ) -> ModuleDigest {
        let implementation = EquivalenceImplementation {
            contract: module,
            implementation_revision,
        };
        let digest = implementation.digest();
        self.equivalences.insert(digest, implementation);
        digest
    }

    pub fn equivalent_implementation_contract(
        &self,
        left: ModuleDigest,
        right: ModuleDigest,
    ) -> Result<bool, SemanticError> {
        if let (Some(left), Some(right)) =
            (self.equivalences.get(&left), self.equivalences.get(&right))
        {
            return Ok(left.contract == right.contract);
        }
        if let (Some(left), Some(right)) = (self.tokenizers.get(&left), self.tokenizers.get(&right))
        {
            return Ok(left.contract == right.contract);
        }
        if let (Some(left), Some(right)) = (self.orderings.get(&left), self.orderings.get(&right)) {
            return Ok(left.contract == right.contract);
        }
        if !self.module_available(left) {
            return Err(SemanticError::ModuleUnavailable(left));
        }
        if !self.module_available(right) {
            return Err(SemanticError::ModuleUnavailable(right));
        }
        Ok(false)
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
        let implementation = match self.equivalences.get(&digest).copied() {
            Some(implementation) => implementation,
            None if self.module_available(digest) => {
                return Err(SemanticError::WrongModuleKind(equivalence));
            }
            None => return Err(SemanticError::ModuleUnavailable(digest)),
        };
        Ok(Some(ResolvedPrimitiveEquivalence {
            equivalence,
            module_digest: digest,
            contract: implementation.contract,
        }))
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
        Ok(CompiledEquivalence {
            equivalence,
            domain,
            root,
            nodes,
        })
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

    /// Returns the canonical quotient-class witness for one pinned Γ-equivalence.
    ///
    /// For every value in the equivalence domain this operation is total, and
    /// the registry maintains the quotient-completeness law
    /// `equivalent(e, x, y) <=> canonical_key(e, x) == canonical_key(e, y)`.
    /// Structural equivalences preserve the law compositionally; primitive
    /// leaves are certified `EquivalenceModule`s whose canonical-key law is
    /// tested against their executable equivalence contract.
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

    /// Returns the primitive module closure that defines one canonical key law.
    ///
    /// Structural equivalence nodes are schema-pinned and therefore do not
    /// need executable digests of their own. Primitive/plugin leaves do: a
    /// persisted key cache is compatible only when every leaf keeps the exact
    /// pinned implementation digest.
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

    /// Returns the exact structural-definition closure that participates in
    /// one canonical key law. This complements primitive module digests: two
    /// contexts may reuse the same nominal revision identifiers while carrying
    /// different structural definitions, so long-lived caches must bind to
    /// the definitions themselves rather than trusting the nominal revision.
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

    pub fn resolve_primitive_ordering(
        &self,
        context: &SemanticContext,
        ordering: SemanticId,
    ) -> Result<ResolvedPrimitiveOrdering, SemanticError> {
        let digest = context
            .environment
            .module(ordering)
            .ok_or(SemanticError::WrongModuleKind(ordering))?;
        let implementation = match self.orderings.get(&digest).copied() {
            Some(implementation) => implementation,
            None if self.module_available(digest) => {
                return Err(SemanticError::WrongModuleKind(ordering));
            }
            None => return Err(SemanticError::ModuleUnavailable(digest)),
        };
        Ok(ResolvedPrimitiveOrdering {
            ordering,
            module_digest: digest,
            contract: implementation.contract,
        })
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

    fn equivalence_contract(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
    ) -> Result<EquivalenceModule, SemanticError> {
        let digest = context
            .environment
            .module(equivalence)
            .ok_or(SemanticError::WrongModuleKind(equivalence))?;
        match self.equivalences.get(&digest).copied() {
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
        match self.equivalences.get(&digest).copied() {
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
        let module = match self.equivalences.get(&digest) {
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

    pub fn validate_model(
        &self,
        context: &SemanticContext,
        model: &FiniteModel,
    ) -> Result<(), SemanticError> {
        for value in model.fields.values() {
            self.validate_value(context, value)?;
        }
        for tuples in model.relations.values() {
            for tuple in tuples {
                for value in tuple {
                    self.validate_value(context, value)?;
                }
            }
        }
        Ok(())
    }

    fn validate_value(
        &self,
        context: &SemanticContext,
        value: &Value,
    ) -> Result<(), SemanticError> {
        match value {
            Value::Product(values) => {
                for child in values.values() {
                    self.validate_value(context, child)?;
                }
            }
            Value::Seq(values) => {
                for child in values {
                    self.validate_value(context, child)?;
                }
            }
            Value::Variant { value, .. } => self.validate_value(context, value)?,
            Value::Option(value) => {
                if let Some(value) = value.as_deref() {
                    self.validate_value(context, value)?;
                }
            }
            Value::Set {
                equivalence,
                elements,
            } => {
                self.ensure_unique(context, *equivalence, elements, |equivalence| {
                    SemanticError::DuplicateSetElement { equivalence }
                })?;
                for child in elements {
                    self.validate_value(context, child)?;
                }
            }
            Value::Bag {
                equivalence,
                entries,
            } => {
                let elements: Vec<_> = entries.iter().map(|(value, _)| value.clone()).collect();
                self.ensure_unique(context, *equivalence, &elements, |equivalence| {
                    SemanticError::DuplicateBagElement { equivalence }
                })?;
                for (child, _) in entries {
                    self.validate_value(context, child)?;
                }
            }
            Value::Map {
                key_equivalence,
                entries,
            } => {
                let keys: Vec<_> = entries.iter().map(|(key, _)| key.clone()).collect();
                self.ensure_unique(context, *key_equivalence, &keys, |equivalence| {
                    SemanticError::DuplicateMapKey { equivalence }
                })?;
                for (key, mapped) in entries {
                    self.validate_value(context, key)?;
                    self.validate_value(context, mapped)?;
                }
            }
            Value::Unit
            | Value::Bool(_)
            | Value::I64(_)
            | Value::F64Bits(_)
            | Value::Text(_)
            | Value::LiveEntityRef { .. }
            | Value::HistoricalEntityId { .. } => {}
        }
        Ok(())
    }

    fn ensure_unique<F>(
        &self,
        context: &SemanticContext,
        equivalence: SemanticId,
        values: &[Value],
        duplicate: F,
    ) -> Result<(), SemanticError>
    where
        F: Fn(SemanticId) -> SemanticError,
    {
        let mut seen = BTreeSet::new();
        for value in values {
            let key = self.canonical_equivalence_key(context, equivalence, value)?;
            if !seen.insert(key) {
                return Err(duplicate(equivalence));
            }
        }
        Ok(())
    }
}

fn typed_entity_digest(tag: u8, entity_type: SemanticId) -> ModuleDigest {
    let raw = entity_type.raw().to_le_bytes();
    let mut bytes = [0_u8; 32];
    bytes[..16].copy_from_slice(&raw);
    bytes[16..].copy_from_slice(&raw);
    bytes[0] ^= tag;
    bytes[16] ^= tag.rotate_left(1);
    ModuleDigest(bytes)
}

impl EquivalenceModule {
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

    fn equivalent(
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
    fn digest(self) -> ModuleDigest {
        let mut digest = self.contract.digest().0;
        let revision = self.implementation_revision.to_le_bytes();
        for (index, byte) in revision.into_iter().enumerate() {
            digest[24 + index] ^= byte;
        }
        ModuleDigest(digest)
    }
}

impl TokenizerModule {
    #[must_use]
    pub fn digest(self) -> ModuleDigest {
        match self {
            Self::AsciiWhitespace => ModuleDigest([21; 32]),
            Self::AsciiWhitespaceLowercase => ModuleDigest([22; 32]),
        }
    }

    #[must_use]
    pub fn tokenize(self, text: &str) -> Vec<String> {
        match self {
            Self::AsciiWhitespace => text.split_ascii_whitespace().map(str::to_owned).collect(),
            Self::AsciiWhitespaceLowercase => text
                .split_ascii_whitespace()
                .map(str::to_ascii_lowercase)
                .collect(),
        }
    }
}

impl TokenizerImplementation {
    #[must_use]
    fn digest(self) -> ModuleDigest {
        let mut digest = self.contract.digest().0;
        let revision = self.implementation_revision.to_le_bytes();
        for (index, byte) in revision.into_iter().enumerate() {
            digest[24 + index] ^= byte;
        }
        ModuleDigest(digest)
    }
}

impl OrderingModule {
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

    fn compare(
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
    fn digest(self) -> ModuleDigest {
        let mut digest = self.contract.digest().0;
        let revision = self.implementation_revision.to_le_bytes();
        for (index, byte) in revision.into_iter().enumerate() {
            digest[24 + index] ^= byte;
        }
        ModuleDigest(digest)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use kernel_model::FiniteModel;
    use kernel_schema::{ModuleDigest, Schema, SemanticEnvironment};
    use kernel_types::{EntityId, SchemaRevisionId, SemanticEnvId};

    use super::*;

    fn context(eq: SemanticId, digest: ModuleDigest) -> SemanticContext {
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(eq, digest);
        SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(1)),
            environment,
        }
    }

    #[test]
    fn equality_is_resolved_through_pinned_digest_not_host_ord() {
        let eq = SemanticId::new(1);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let context = context(eq, digest);
        assert_eq!(
            registry.equivalent(
                &context,
                eq,
                &Value::Text("Alpha".into()),
                &Value::Text("alpha".into())
            ),
            Ok(true)
        );
    }

    #[test]
    fn bound_primitive_predicate_preserves_resolved_equivalence_contract() {
        let eq = SemanticId::new(11);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let context = context(eq, digest);
        let resolved = registry
            .resolve_primitive_equivalence(&context, eq)
            .unwrap()
            .expect("primitive equality resolves once");
        let right = Value::Text("Alpha".into());
        let bound = resolved.bind_right(&right).unwrap();
        for candidate in [
            Value::Text("ALPHA".into()),
            Value::Text("alpha".into()),
            Value::Text("Beta".into()),
        ] {
            assert_eq!(
                bound.matches(&candidate),
                resolved.equivalent(&candidate, &right).unwrap()
            );
        }
    }

    #[test]
    fn set_rejects_semantic_duplicates_even_when_rust_values_differ() {
        let eq = SemanticId::new(1);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let context = context(eq, digest);
        let owner = EntityId::new(1);
        let field = SemanticId::new(2);
        let mut model = FiniteModel::default();
        model
            .carriers
            .insert(SemanticId::new(3), BTreeSet::from([owner]));
        model.fields.insert(
            (field, owner),
            Value::Set {
                equivalence: eq,
                elements: vec![Value::Text("A".into()), Value::Text("a".into())],
            },
        );
        assert_eq!(
            registry.validate_model(&context, &model),
            Err(SemanticError::DuplicateSetElement { equivalence: eq })
        );
    }
    #[test]
    fn f64_key_equality_is_total_bitwise_not_ieee_partial_equality() {
        let eq = SemanticId::new(9);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let context = context(eq, digest);
        let nan_a = Value::F64Bits(0x7ff8_0000_0000_0001);
        let nan_same = Value::F64Bits(0x7ff8_0000_0000_0001);
        let nan_other = Value::F64Bits(0x7ff8_0000_0000_0002);
        assert_eq!(
            registry.equivalent(&context, eq, &nan_a, &nan_same),
            Ok(true)
        );
        assert_eq!(
            registry.equivalent(&context, eq, &nan_a, &nan_other),
            Ok(false)
        );
        assert_eq!(
            registry.equivalent(
                &context,
                eq,
                &Value::F64Bits(0.0_f64.to_bits()),
                &Value::F64Bits((-0.0_f64).to_bits())
            ),
            Ok(false)
        );
    }
    #[test]
    fn builtin_equivalence_modules_satisfy_equivalence_laws_on_hostile_samples() {
        let cases = [
            (EquivalenceModule::UnitExact, vec![Value::Unit]),
            (
                EquivalenceModule::BoolExact,
                vec![Value::Bool(false), Value::Bool(true)],
            ),
            (
                EquivalenceModule::I64Exact,
                vec![Value::I64(-1), Value::I64(0), Value::I64(1)],
            ),
            (
                EquivalenceModule::F64Bitwise,
                vec![
                    Value::F64Bits(0.0_f64.to_bits()),
                    Value::F64Bits((-0.0_f64).to_bits()),
                    Value::F64Bits(0x7ff8_0000_0000_0001),
                    Value::F64Bits(0x7ff8_0000_0000_0002),
                ],
            ),
            (
                EquivalenceModule::TextExact,
                vec![Value::Text("A".into()), Value::Text("a".into())],
            ),
            (
                EquivalenceModule::TextAsciiCaseInsensitive,
                vec![
                    Value::Text("A".into()),
                    Value::Text("a".into()),
                    Value::Text("B".into()),
                ],
            ),
        ];
        for (module, values) in cases {
            let eq = SemanticId::new(77);
            let mut registry = SemanticRegistry::default();
            let digest = registry.install_equivalence(module);
            let context = context(eq, digest);
            for a in &values {
                assert_eq!(registry.equivalent(&context, eq, a, a), Ok(true));
                for b in &values {
                    let ab = registry.equivalent(&context, eq, a, b).unwrap();
                    let ba = registry.equivalent(&context, eq, b, a).unwrap();
                    assert_eq!(ab, ba);
                    for c in &values {
                        let bc = registry.equivalent(&context, eq, b, c).unwrap();
                        if ab && bc {
                            assert_eq!(registry.equivalent(&context, eq, a, c), Ok(true));
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn structural_product_equivalence_is_schema_derived_and_compositional() {
        let product_eq = SemanticId::new(40);
        let name_eq = SemanticId::new(41);
        let age_eq = SemanticId::new(42);
        let name_field = SemanticId::new(1);
        let age_field = SemanticId::new(2);
        let mut registry = SemanticRegistry::default();
        let name_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let age_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                product_eq,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(name_field, name_eq), (age_field, age_eq)]),
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(name_eq, name_digest);
        environment.pin_module(age_eq, age_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        registry.validate_context(&context).unwrap();
        assert_eq!(
            registry.equivalence_domain(&context, product_eq).unwrap(),
            EquivalenceDomain::Product(BTreeMap::from([
                (name_field, EquivalenceDomain::Text),
                (age_field, EquivalenceDomain::I64),
            ]))
        );
        let left = Value::Product(BTreeMap::from([
            (name_field, Value::Text("ALICE".into())),
            (age_field, Value::I64(30)),
        ]));
        let right = Value::Product(BTreeMap::from([
            (name_field, Value::Text("alice".into())),
            (age_field, Value::I64(30)),
        ]));
        assert_eq!(
            registry.equivalent(&context, product_eq, &left, &right),
            Ok(true)
        );
    }

    #[test]
    fn structural_equivalence_cycle_is_rejected_before_data_exists() {
        let a = SemanticId::new(50);
        let b = SemanticId::new(51);
        let field = SemanticId::new(1);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                a,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field, b)]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                b,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field, a)]),
                },
            )
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        assert!(matches!(
            registry.validate_context(&context),
            Err(SemanticError::CyclicStructuralEquivalence(_))
        ));
    }

    #[test]
    fn guarded_recursive_equivalence_has_canonical_mu_domain_and_terminates_on_values() {
        let root = SemanticId::new(60);
        let sum = SemanticId::new(61);
        let product = SemanticId::new(62);
        let recursive_var = SemanticId::new(63);
        let unit_eq = SemanticId::new(64);
        let text_eq = SemanticId::new(65);
        let nil_tag = SemanticId::new(66);
        let cons_tag = SemanticId::new(67);
        let head_field = SemanticId::new(68);
        let tail_field = SemanticId::new(69);

        let mut registry = SemanticRegistry::default();
        let unit_digest = registry.install_equivalence(EquivalenceModule::UnitExact);
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(root, StructuralEquivalenceDef::Mu { body: sum })
            .unwrap();
        schema
            .define_structural_equivalence(
                sum,
                StructuralEquivalenceDef::Sum {
                    variants: BTreeMap::from([(nil_tag, unit_eq), (cons_tag, product)]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                product,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(head_field, text_eq), (tail_field, recursive_var)]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                recursive_var,
                StructuralEquivalenceDef::Var { binder: root },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(unit_eq, unit_digest);
        environment.pin_module(text_eq, text_digest);
        let context = SemanticContext {
            schema,
            environment,
        };

        let x = kernel_schema::TypeVar(0);
        let ty = TypeExpr::Mu {
            binder: x,
            body: Box::new(TypeExpr::Sum(BTreeMap::from([
                (nil_tag, TypeExpr::Scalar(ScalarType::Unit)),
                (
                    cons_tag,
                    TypeExpr::Product(BTreeMap::from([
                        (head_field, TypeExpr::Scalar(ScalarType::Text)),
                        (tail_field, TypeExpr::Var(x)),
                    ])),
                ),
            ]))),
        };
        registry.validate_context(&context).unwrap();
        assert_eq!(
            registry.equivalence_domain(&context, root).unwrap(),
            domain_for_type(&ty).unwrap()
        );

        let nil = || Value::Variant {
            tag: nil_tag,
            value: Box::new(Value::Unit),
        };
        let cons = |head: &str, tail: Value| Value::Variant {
            tag: cons_tag,
            value: Box::new(Value::Product(BTreeMap::from([
                (head_field, Value::Text(head.into())),
                (tail_field, tail),
            ]))),
        };
        let left = cons("A", cons("B", nil()));
        let right = cons("a", cons("b", nil()));
        let different = cons("a", nil());
        let compiled = registry.compile_equivalence(&context, root).unwrap();
        assert_eq!(registry.equivalent(&context, root, &left, &right), Ok(true));
        assert_eq!(
            registry.canonical_equivalence_key(&context, root, &left),
            registry.canonical_equivalence_key(&context, root, &right)
        );
        assert_eq!(compiled.equivalent(&left, &right), Ok(true));
        assert_eq!(
            compiled.canonical_key(&left),
            registry.canonical_equivalence_key(&context, root, &left)
        );
        assert_eq!(
            registry.equivalent(&context, root, &left, &different),
            Ok(false)
        );
        assert_eq!(compiled.equivalent(&left, &different), Ok(false));
        assert_ne!(
            registry.canonical_equivalence_key(&context, root, &left),
            registry.canonical_equivalence_key(&context, root, &different)
        );
    }

    #[test]
    fn recursive_equivalence_rejects_unguarded_and_free_variables() {
        let root = SemanticId::new(70);
        let recursive_var = SemanticId::new(71);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                root,
                StructuralEquivalenceDef::Mu {
                    body: recursive_var,
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                recursive_var,
                StructuralEquivalenceDef::Var { binder: root },
            )
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        assert_eq!(
            registry.validate_context(&context),
            Err(SemanticError::UnguardedStructuralRecursion(root))
        );

        let free_var = SemanticId::new(72);
        let missing_binder = SemanticId::new(73);
        let mut schema = Schema::new(SchemaRevisionId::new(2));
        schema
            .define_structural_equivalence(
                free_var,
                StructuralEquivalenceDef::Var {
                    binder: missing_binder,
                },
            )
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(2)),
        };
        assert_eq!(
            registry.validate_context(&context),
            Err(SemanticError::FreeStructuralRecursion(missing_binder))
        );
    }

    #[test]
    fn recursive_equivalence_refinement_is_checked_coinductively_through_mu_binders() {
        let exact_root = SemanticId::new(80);
        let exact_option = SemanticId::new(81);
        let exact_product = SemanticId::new(82);
        let exact_var = SemanticId::new(83);
        let ci_root = SemanticId::new(84);
        let ci_option = SemanticId::new(85);
        let ci_product = SemanticId::new(86);
        let ci_var = SemanticId::new(87);
        let exact_text = SemanticId::new(88);
        let ci_text = SemanticId::new(89);
        let head_field = SemanticId::new(90);
        let tail_field = SemanticId::new(91);

        let mut registry = SemanticRegistry::default();
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(3));
        for (root, option, product, recursive_var, text_eq) in [
            (
                exact_root,
                exact_option,
                exact_product,
                exact_var,
                exact_text,
            ),
            (ci_root, ci_option, ci_product, ci_var, ci_text),
        ] {
            schema
                .define_structural_equivalence(root, StructuralEquivalenceDef::Mu { body: option })
                .unwrap();
            schema
                .define_structural_equivalence(
                    option,
                    StructuralEquivalenceDef::Option { inner: product },
                )
                .unwrap();
            schema
                .define_structural_equivalence(
                    product,
                    StructuralEquivalenceDef::Product {
                        fields: BTreeMap::from([
                            (head_field, text_eq),
                            (tail_field, recursive_var),
                        ]),
                    },
                )
                .unwrap();
            schema
                .define_structural_equivalence(
                    recursive_var,
                    StructuralEquivalenceDef::Var { binder: root },
                )
                .unwrap();
        }
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(3));
        environment.pin_module(exact_text, exact_digest);
        environment.pin_module(ci_text, ci_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        registry.validate_context(&context).unwrap();
        assert_eq!(
            registry.equivalence_refines(&context, exact_root, ci_root),
            Ok(true)
        );
        assert_eq!(
            registry.equivalence_refines(&context, ci_root, exact_root),
            Ok(false)
        );
    }
    #[test]
    fn nominal_entity_equivalence_rejects_wrong_runtime_type() {
        let person = SemanticId::new(90);
        let order = SemanticId::new(91);
        let eq = SemanticId::new(92);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
        let context = context(eq, digest);
        let wrong = Value::LiveEntityRef {
            entity_type: order,
            id: EntityId::new(1),
        };
        assert_eq!(
            registry.equivalent(&context, eq, &wrong, &wrong),
            Err(SemanticError::TypeMismatch(eq))
        );
    }
    #[test]
    fn structural_option_and_sum_equivalence_are_compositional() {
        let text_eq = SemanticId::new(100);
        let option_eq = SemanticId::new(101);
        let sum_eq = SemanticId::new(102);
        let text_tag = SemanticId::new(103);
        let int_tag = SemanticId::new(104);
        let int_eq = SemanticId::new(105);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let int_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                option_eq,
                StructuralEquivalenceDef::Option { inner: text_eq },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                sum_eq,
                StructuralEquivalenceDef::Sum {
                    variants: BTreeMap::from([(text_tag, text_eq), (int_tag, int_eq)]),
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(int_eq, int_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        registry.validate_context(&context).unwrap();

        assert_eq!(
            registry.equivalent(
                &context,
                option_eq,
                &Value::Option(Some(Box::new(Value::Text("A".into())))),
                &Value::Option(Some(Box::new(Value::Text("a".into())))),
            ),
            Ok(true)
        );
        assert_eq!(
            registry.equivalent(
                &context,
                option_eq,
                &Value::Option(None),
                &Value::Option(Some(Box::new(Value::Text("a".into())))),
            ),
            Ok(false)
        );
        assert_eq!(
            registry.equivalent(
                &context,
                sum_eq,
                &Value::Variant {
                    tag: text_tag,
                    value: Box::new(Value::Text("X".into())),
                },
                &Value::Variant {
                    tag: text_tag,
                    value: Box::new(Value::Text("x".into())),
                },
            ),
            Ok(true)
        );
        assert_eq!(
            registry.equivalent(
                &context,
                sum_eq,
                &Value::Variant {
                    tag: text_tag,
                    value: Box::new(Value::Text("1".into())),
                },
                &Value::Variant {
                    tag: int_tag,
                    value: Box::new(Value::I64(1)),
                },
            ),
            Ok(false)
        );
    }

    #[test]
    fn structural_collection_equivalences_are_compositional() {
        let text_eq = SemanticId::new(1200);
        let set_eq = SemanticId::new(1201);
        let bag_eq = SemanticId::new(1202);
        let sequence_eq = SemanticId::new(1203);
        let map_eq = SemanticId::new(1204);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(1200));
        schema
            .define_structural_equivalence(
                set_eq,
                StructuralEquivalenceDef::Set { element: text_eq },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                bag_eq,
                StructuralEquivalenceDef::Bag { element: text_eq },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                sequence_eq,
                StructuralEquivalenceDef::Seq { element: text_eq },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                map_eq,
                StructuralEquivalenceDef::Map {
                    key: text_eq,
                    value: text_eq,
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1200));
        environment.pin_module(text_eq, text_digest);
        let context = SemanticContext {
            schema,
            environment,
        };

        let set = |values: &[&str]| Value::Set {
            equivalence: text_eq,
            elements: values
                .iter()
                .map(|value| Value::Text((*value).into()))
                .collect(),
        };
        let left_set = set(&["A", "B"]);
        let right_set = set(&["b", "a"]);
        assert_eq!(
            registry.equivalent(&context, set_eq, &left_set, &right_set),
            Ok(true)
        );
        assert_eq!(
            registry.canonical_equivalence_key(&context, set_eq, &left_set),
            registry.canonical_equivalence_key(&context, set_eq, &right_set)
        );
        let left_bag = Value::Bag {
            equivalence: text_eq,
            entries: vec![(Value::Text("A".into()), 2)],
        };
        let right_bag = Value::Bag {
            equivalence: text_eq,
            entries: vec![(Value::Text("a".into()), 2)],
        };
        assert_eq!(
            registry.equivalent(&context, bag_eq, &left_bag, &right_bag),
            Ok(true)
        );
        assert_eq!(
            registry.canonical_equivalence_key(&context, bag_eq, &left_bag),
            registry.canonical_equivalence_key(&context, bag_eq, &right_bag)
        );
        let left_sequence = Value::Seq(vec![Value::Text("A".into()), Value::Text("B".into())]);
        let right_sequence = Value::Seq(vec![Value::Text("a".into()), Value::Text("b".into())]);
        assert_eq!(
            registry.equivalent(&context, sequence_eq, &left_sequence, &right_sequence),
            Ok(true)
        );
        assert_eq!(
            registry.canonical_equivalence_key(&context, sequence_eq, &left_sequence),
            registry.canonical_equivalence_key(&context, sequence_eq, &right_sequence)
        );
        let map = |key: &str, value: &str| Value::Map {
            key_equivalence: text_eq,
            entries: vec![(Value::Text(key.into()), Value::Text(value.into()))],
        };
        let left_map = map("K", "V");
        let right_map = map("k", "v");
        assert_eq!(
            registry.equivalent(&context, map_eq, &left_map, &right_map),
            Ok(true)
        );
        assert_eq!(
            registry.canonical_equivalence_key(&context, map_eq, &left_map),
            registry.canonical_equivalence_key(&context, map_eq, &right_map)
        );
    }

    #[test]
    fn structural_equivalence_refinement_is_compositional_and_directional() {
        let exact = SemanticId::new(1210);
        let ci = SemanticId::new(1211);
        let exact_option = SemanticId::new(1212);
        let ci_option = SemanticId::new(1213);
        let field = SemanticId::new(1214);
        let exact_product = SemanticId::new(1215);
        let ci_product = SemanticId::new(1216);
        let mut registry = SemanticRegistry::default();
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(1210));
        schema
            .define_structural_equivalence(
                exact_option,
                StructuralEquivalenceDef::Option { inner: exact },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                ci_option,
                StructuralEquivalenceDef::Option { inner: ci },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                exact_product,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field, exact_option)]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                ci_product,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field, ci_option)]),
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1210));
        environment.pin_module(exact, exact_digest);
        environment.pin_module(ci, ci_digest);
        let context = SemanticContext {
            schema,
            environment,
        };

        assert_eq!(
            registry.equivalence_refines(&context, exact_product, ci_product),
            Ok(true)
        );
        assert_eq!(
            registry.equivalence_refines(&context, ci_product, exact_product),
            Ok(false)
        );
    }

    #[test]
    fn implementation_revision_changes_digest_without_changing_semantic_contract() {
        let mut registry = SemanticRegistry::default();
        let first = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
        let second = registry.install_equivalence_revision(EquivalenceModule::TextExact, 2);
        let changed_law =
            registry.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 2);

        assert_ne!(first, second);
        assert_eq!(
            registry.equivalent_implementation_contract(first, second),
            Ok(true)
        );
        assert_eq!(
            registry.equivalent_implementation_contract(first, changed_law),
            Ok(false)
        );
    }

    #[test]
    fn ordering_is_versioned_semantics_not_host_iteration_order() {
        let ordering = SemanticId::new(700);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitiveThenBinary);
        let context = context(ordering, digest);
        assert_eq!(
            registry.compare(
                &context,
                ordering,
                &Value::Text("a".into()),
                &Value::Text("B".into()),
            ),
            Ok(CmpOrdering::Less)
        );
        assert_eq!(
            registry.compare(
                &context,
                ordering,
                &Value::Text("A".into()),
                &Value::Text("a".into()),
            ),
            Ok(CmpOrdering::Less)
        );
    }

    #[test]
    fn structural_sum_order_uses_explicit_variant_rank_not_semantic_id_order() {
        let text_order = SemanticId::new(73_100);
        let sum_order = SemanticId::new(73_101);
        let high_id_first = SemanticId::new(90_000);
        let low_id_second = SemanticId::new(10);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_ordering(OrderingModule::TextBinary);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(73_100));
        environment.pin_module(text_order, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(73_100));
        schema
            .define_structural_ordering(
                sum_order,
                StructuralOrderingDef::Sum {
                    variants: vec![(high_id_first, text_order), (low_id_second, text_order)],
                },
            )
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        registry.validate_context(&context).unwrap();

        let first = Value::Variant {
            tag: high_id_first,
            value: Box::new(Value::Text("z".into())),
        };
        let second = Value::Variant {
            tag: low_id_second,
            value: Box::new(Value::Text("a".into())),
        };
        assert_eq!(
            registry.compare(&context, sum_order, &first, &second),
            Ok(std::cmp::Ordering::Less)
        );
    }

    #[test]
    fn f64_total_order_is_defined_for_signed_zero_and_nan() {
        let ordering = SemanticId::new(705);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_ordering(OrderingModule::F64Total);
        let context = context(ordering, digest);
        assert_eq!(
            registry.compare(
                &context,
                ordering,
                &Value::F64Bits((-0.0_f64).to_bits()),
                &Value::F64Bits(0.0_f64.to_bits()),
            ),
            Ok(CmpOrdering::Less)
        );
        assert_eq!(
            registry.compare(
                &context,
                ordering,
                &Value::F64Bits(f64::INFINITY.to_bits()),
                &Value::F64Bits(f64::NAN.to_bits()),
            ),
            Ok(CmpOrdering::Less)
        );
    }

    #[test]
    fn ordering_implementation_upgrade_preserves_contract_but_contract_change_does_not() {
        let mut registry = SemanticRegistry::default();
        let v1 = registry.install_ordering_revision(OrderingModule::TextBinary, 1);
        let v2 = registry.install_ordering_revision(OrderingModule::TextBinary, 2);
        let changed = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitiveThenBinary);
        assert_eq!(
            registry.equivalent_implementation_contract(v1, v2),
            Ok(true)
        );
        assert_eq!(
            registry.equivalent_implementation_contract(v1, changed),
            Ok(false)
        );
    }

    #[test]
    fn ordering_congruence_is_explicit_and_not_inferred_from_matching_scalar_type() {
        let text_eq = SemanticId::new(710);
        let binary_order = SemanticId::new(711);
        let ci_order = SemanticId::new(712);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let binary_digest = registry.install_ordering(OrderingModule::TextBinary);
        let ci_digest = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitive);
        let mut environment =
            kernel_schema::SemanticEnvironment::new(kernel_types::SemanticEnvId::new(710));
        environment.pin_module(text_eq, eq_digest);
        environment.pin_module(binary_order, binary_digest);
        environment.pin_module(ci_order, ci_digest);
        let context = SemanticContext {
            schema: kernel_schema::Schema::new(kernel_types::SchemaRevisionId::new(710)),
            environment,
        };

        assert_eq!(
            registry.ordering_congruent_with_equivalence(&context, binary_order, text_eq),
            Ok(false)
        );
        assert_eq!(
            registry.ordering_congruent_with_equivalence(&context, ci_order, text_eq),
            Ok(true)
        );
    }

    #[test]
    fn ordering_compatibility_requires_a_checked_law_certificate() {
        let valid = OrderingCompatibilitySpec {
            ordering: OrderingModule::TextAsciiCaseInsensitive,
            equivalence: EquivalenceModule::TextAsciiCaseInsensitive,
        };
        let checked =
            certify_ordering_compatibility(&valid, OrderingCompatibilityArtifact::BuiltinLaw)
                .unwrap();
        assert_eq!(checked.spec(), &valid);

        let invalid = OrderingCompatibilitySpec {
            ordering: OrderingModule::TextBinary,
            equivalence: EquivalenceModule::TextAsciiCaseInsensitive,
        };
        assert_eq!(
            certify_ordering_compatibility(&invalid, OrderingCompatibilityArtifact::BuiltinLaw,)
                .err(),
            Some(SemanticError::OrderingCompatibilityViolation)
        );
    }

    #[test]
    fn semantic_implementation_install_requires_contract_bound_checked_certificate() {
        let contract = SemanticContract::Equivalence(EquivalenceModule::TextExact);
        let checked = certify_implementation(
            &contract,
            SemanticImplementationArtifact::BuiltinEquivalence(EquivalenceModule::TextExact),
        )
        .unwrap();
        assert_eq!(checked.spec(), &contract);

        let mut registry = SemanticRegistry::default();
        let digest = registry.install_certified_implementation(checked, 7);
        let direct = registry.install_equivalence_revision(EquivalenceModule::TextExact, 7);
        assert_eq!(digest, direct);

        assert_eq!(
            certify_implementation(
                &SemanticContract::Equivalence(EquivalenceModule::TextExact),
                SemanticImplementationArtifact::BuiltinEquivalence(
                    EquivalenceModule::TextAsciiCaseInsensitive,
                ),
            )
            .err(),
            Some(SemanticError::ImplementationContractMismatch)
        );
    }

    #[test]
    fn builtin_module_spec_roundtrips_exact_implementation_revision() {
        let mut source = SemanticRegistry::default();
        let digest =
            source.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 17);
        let spec = source.builtin_module_spec(digest).unwrap();
        assert_eq!(spec.digest(), digest);

        let mut restored = SemanticRegistry::default();
        assert_eq!(restored.install_builtin_module_spec(spec), digest);
        assert_eq!(restored.builtin_module_spec(digest), Some(spec));
    }

    #[test]
    fn deployment_authentication_does_not_replace_semantic_refinement() {
        let spec = BuiltinSemanticModuleSpec::Equivalence {
            module: EquivalenceModule::TextExact,
            implementation_revision: 1,
        };
        let mut package = SemanticImplementationPackage::builtin(spec);
        package.refinement = Some(SemanticRefinementCertificate::Builtin(
            SemanticImplementationArtifact::BuiltinEquivalence(
                EquivalenceModule::TextAsciiCaseInsensitive,
            ),
        ));
        let mut deployment = SemanticDeploymentRegistry::default();
        deployment.register(package.clone());
        let authentications = ArtifactAuthenticationSet::trusted_builtins(&[spec]);
        assert_eq!(
            deployment
                .authorize_artifact(
                    package.artifact_digest,
                    &SemanticExecutionPolicy::trusted_builtin_only(),
                    &authentications,
                )
                .err(),
            Some(SemanticDeploymentError::RefinementContractMismatch)
        );
    }

    #[test]
    fn deployment_refinement_does_not_replace_artifact_authentication() {
        let spec = BuiltinSemanticModuleSpec::Tokenizer {
            module: TokenizerModule::AsciiWhitespace,
            implementation_revision: 1,
        };
        let deployment = SemanticDeploymentRegistry::from_builtin_specs(&[spec]);
        assert_eq!(
            deployment
                .authorize_artifact(
                    ImplementationArtifactDigest(spec.digest().0),
                    &SemanticExecutionPolicy::trusted_builtin_only(),
                    &ArtifactAuthenticationSet::default(),
                )
                .err(),
            Some(SemanticDeploymentError::UnauthenticatedArtifact)
        );
    }

    #[test]
    fn deployment_revocation_can_select_certified_defined_contract_replacement() {
        let old = BuiltinSemanticModuleSpec::Ordering {
            module: OrderingModule::I64Ascending,
            implementation_revision: 1,
        };
        let replacement = BuiltinSemanticModuleSpec::Ordering {
            module: OrderingModule::I64Ascending,
            implementation_revision: 2,
        };
        let deployment = SemanticDeploymentRegistry::from_builtin_specs(&[old, replacement]);
        let authentications = ArtifactAuthenticationSet::trusted_builtins(&[old, replacement]);
        let mut policy = SemanticExecutionPolicy::trusted_builtin_only();
        policy
            .revoked_artifacts
            .insert(ImplementationArtifactDigest(old.digest().0));
        let authorization = deployment
            .authorize_contract(
                SemanticContractIdentity::Defined(old.contract()),
                &policy,
                &authentications,
            )
            .unwrap();
        assert_eq!(
            authorization.artifact_digest(),
            ImplementationArtifactDigest(replacement.digest().0)
        );
        assert_eq!(
            authorization.contract(),
            SemanticContractIdentity::Defined(old.contract())
        );
    }

    #[test]
    fn opaque_deployment_identity_requires_exact_artifact_and_runtime() {
        let artifact = ImplementationArtifactDigest([71; 32]);
        let runtime = RuntimeProfileDigest([72; 32]);
        let package = SemanticImplementationPackage {
            contract: SemanticContractIdentity::OpaqueArtifact { artifact, runtime },
            artifact_digest: artifact,
            runtime_profile: runtime,
            refinement: None,
            executable: None,
        };
        let mut deployment = SemanticDeploymentRegistry::default();
        deployment.register(package.clone());
        let mut authentications = ArtifactAuthenticationSet::default();
        authentications.mark_verified(artifact);
        let policy = SemanticExecutionPolicy {
            allowed_runtime_profiles: BTreeSet::from([runtime]),
            revoked_artifacts: BTreeSet::new(),
            require_authentication: true,
        };
        let authorization = deployment
            .authorize_contract(package.contract, &policy, &authentications)
            .unwrap();
        assert_eq!(authorization.artifact_digest(), artifact);
        assert_eq!(
            SemanticDeploymentRegistry::install_authorized_builtin(
                &authorization,
                &mut SemanticRegistry::default(),
            ),
            Err(SemanticDeploymentError::ExecutableUnavailable)
        );

        let mut wrong = package;
        wrong.runtime_profile = RuntimeProfileDigest([73; 32]);
        let mut deployment = SemanticDeploymentRegistry::default();
        deployment.register(wrong.clone());
        let mut wrong_auth = ArtifactAuthenticationSet::default();
        wrong_auth.mark_verified(artifact);
        assert_eq!(
            deployment
                .authorize_artifact(wrong.artifact_digest, &policy, &wrong_auth)
                .err(),
            Some(SemanticDeploymentError::RuntimeDoesNotMatchOpaqueContract)
        );
    }

    #[test]
    fn execution_capability_checks_only_the_requested_contract_closure() {
        let needed = BuiltinSemanticModuleSpec::Equivalence {
            module: EquivalenceModule::I64Exact,
            implementation_revision: 1,
        };
        let unrelated = BuiltinSemanticModuleSpec::Tokenizer {
            module: TokenizerModule::AsciiWhitespaceLowercase,
            implementation_revision: 1,
        };
        let deployment = SemanticDeploymentRegistry::from_builtin_specs(&[needed, unrelated]);
        let authentications = ArtifactAuthenticationSet::trusted_builtins(&[needed]);
        let policy = SemanticExecutionPolicy::trusted_builtin_only();
        assert!(
            deployment
                .authorize_required(
                    &[SemanticContractIdentity::Defined(needed.contract())],
                    &policy,
                    &authentications,
                )
                .is_ok()
        );
        assert_eq!(
            deployment
                .authorize_required(
                    &[
                        SemanticContractIdentity::Defined(needed.contract()),
                        SemanticContractIdentity::Defined(unrelated.contract()),
                    ],
                    &policy,
                    &authentications,
                )
                .err(),
            Some(SemanticDeploymentError::UnauthenticatedArtifact)
        );

        let capability = deployment.execution_capability(
            &[
                SemanticContractIdentity::Defined(needed.contract()),
                SemanticContractIdentity::Defined(unrelated.contract()),
            ],
            &policy,
            &authentications,
        );
        assert!(!capability.executable());
        assert_eq!(capability.authorized.len(), 1);
        assert_eq!(
            capability.unavailable,
            vec![(
                SemanticContractIdentity::Defined(unrelated.contract()),
                SemanticDeploymentError::UnauthenticatedArtifact,
            )]
        );
    }

    #[test]
    fn registry_validates_every_pinned_environment_module_not_only_schema_dependencies() {
        let extra = SemanticId::new(9900);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(extra, ModuleDigest([99; 32]));
        let context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(1)),
            environment,
        };
        assert_eq!(
            SemanticRegistry::default().validate_context(&context),
            Err(SemanticError::ModuleUnavailable(ModuleDigest([99; 32])))
        );
    }
    #[test]
    fn tokenizer_is_versioned_semantics_not_ambient_library_behavior() {
        let tokenizer = SemanticId::new(5000);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_tokenizer(TokenizerModule::AsciiWhitespaceLowercase);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(50));
        environment.pin_module(tokenizer, digest);
        let context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(50)),
            environment,
        };
        registry.validate_context(&context).unwrap();
        assert_eq!(
            registry.tokenize(&context, tokenizer, "Hello   WORLD"),
            Ok(vec!["hello".to_string(), "world".to_string()])
        );
    }

    #[test]
    fn tokenizer_implementation_revision_preserves_contract_but_contract_change_does_not() {
        let tokenizer = SemanticId::new(5001);
        let mut registry = SemanticRegistry::default();
        let v1 = registry.install_tokenizer_revision(TokenizerModule::AsciiWhitespace, 1);
        let v2 = registry.install_tokenizer_revision(TokenizerModule::AsciiWhitespace, 2);
        let lower = registry.install_tokenizer(TokenizerModule::AsciiWhitespaceLowercase);
        let make_context = |revision, digest| {
            let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision));
            environment.pin_module(tokenizer, digest);
            SemanticContext {
                schema: Schema::new(SchemaRevisionId::new(51)),
                environment,
            }
        };
        let old = make_context(51, v1);
        let new_impl = make_context(52, v2);
        let changed = make_context(53, lower);
        assert_eq!(
            registry.contexts_semantically_equivalent(&old, &new_impl),
            Ok(true)
        );
        assert_eq!(
            registry.contexts_semantically_equivalent(&old, &changed),
            Ok(false)
        );
    }

    #[test]
    fn tokenizer_digest_cannot_masquerade_as_equality_module() {
        let symbol = SemanticId::new(5002);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_tokenizer(TokenizerModule::AsciiWhitespace);
        let context = context(symbol, digest);
        assert_eq!(
            registry.equivalence_domain(&context, symbol),
            Err(SemanticError::WrongModuleKind(symbol))
        );
    }

    #[test]
    fn canonical_equality_keys_match_every_builtin_primitive_contract() {
        let modules = [
            (
                EquivalenceModule::UnitExact,
                vec![(Value::Unit, Value::Unit)],
            ),
            (
                EquivalenceModule::BoolExact,
                vec![
                    (Value::Bool(false), Value::Bool(false)),
                    (Value::Bool(false), Value::Bool(true)),
                ],
            ),
            (
                EquivalenceModule::I64Exact,
                vec![
                    (Value::I64(-7), Value::I64(-7)),
                    (Value::I64(-7), Value::I64(7)),
                ],
            ),
            (
                EquivalenceModule::F64Bitwise,
                vec![
                    (
                        Value::F64Bits(0.0_f64.to_bits()),
                        Value::F64Bits((-0.0_f64).to_bits()),
                    ),
                    (
                        Value::F64Bits(0x7ff8_0000_0000_0001),
                        Value::F64Bits(0x7ff8_0000_0000_0001),
                    ),
                    (
                        Value::F64Bits(0x7ff8_0000_0000_0001),
                        Value::F64Bits(0x7ff8_0000_0000_0002),
                    ),
                ],
            ),
            (
                EquivalenceModule::TextExact,
                vec![
                    (Value::Text("A".into()), Value::Text("A".into())),
                    (Value::Text("A".into()), Value::Text("a".into())),
                ],
            ),
            (
                EquivalenceModule::TextAsciiCaseInsensitive,
                vec![
                    (Value::Text("AΩz".into()), Value::Text("aΩZ".into())),
                    (Value::Text("Ω".into()), Value::Text("ω".into())),
                ],
            ),
        ];

        for (offset, (module, pairs)) in modules.into_iter().enumerate() {
            let semantic = SemanticId::new(71_000 + offset as u128);
            let mut registry = SemanticRegistry::default();
            let digest = registry.install_equivalence(module);
            let context = context(semantic, digest);
            let resolved = registry
                .resolve_primitive_equivalence(&context, semantic)
                .unwrap()
                .unwrap();
            assert_eq!(resolved.module_digest(), digest);
            for (left, right) in pairs {
                assert_eq!(
                    resolved.canonical_key(&left).unwrap()
                        == resolved.canonical_key(&right).unwrap(),
                    resolved.equivalent(&left, &right).unwrap(),
                    "module={module:?}, left={left:?}, right={right:?}"
                );
            }
        }
    }

    #[test]
    fn canonical_entity_equality_keys_match_builtin_contracts() {
        let person = SemanticId::new(70_001);
        let modules = [
            EquivalenceModule::LiveEntityIdExact(person),
            EquivalenceModule::HistoricalEntityIdExact(person),
        ];
        for (offset, module) in modules.into_iter().enumerate() {
            let semantic = SemanticId::new(71_100 + offset as u128);
            let mut registry = SemanticRegistry::default();
            let digest = registry.install_equivalence(module);
            let context = context(semantic, digest);
            let resolved = registry
                .resolve_primitive_equivalence(&context, semantic)
                .unwrap()
                .unwrap();
            let values = match module {
                EquivalenceModule::LiveEntityIdExact(entity_type) => vec![
                    Value::LiveEntityRef {
                        entity_type,
                        id: EntityId::new(5),
                    },
                    Value::LiveEntityRef {
                        entity_type,
                        id: EntityId::new(6),
                    },
                ],
                EquivalenceModule::HistoricalEntityIdExact(entity_type) => vec![
                    Value::HistoricalEntityId {
                        entity_type,
                        id: EntityId::new(5),
                    },
                    Value::HistoricalEntityId {
                        entity_type,
                        id: EntityId::new(6),
                    },
                ],
                _ => unreachable!(),
            };
            for left in &values {
                for right in &values {
                    assert_eq!(
                        resolved.canonical_key(left).unwrap()
                            == resolved.canonical_key(right).unwrap(),
                        resolved.equivalent(left, right).unwrap(),
                        "module={module:?}, left={left:?}, right={right:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn canonical_order_keys_match_every_builtin_ordering_contract() {
        let entity_type = SemanticId::new(71_999);
        let modules = [
            OrderingModule::UnitExact,
            OrderingModule::BoolAscending,
            OrderingModule::I64Ascending,
            OrderingModule::F64Total,
            OrderingModule::TextBinary,
            OrderingModule::TextAsciiCaseInsensitive,
            OrderingModule::TextAsciiCaseInsensitiveThenBinary,
            OrderingModule::LiveEntityIdAscending(entity_type),
            OrderingModule::HistoricalEntityIdAscending(entity_type),
        ];
        for (offset, module) in modules.into_iter().enumerate() {
            let ordering = SemanticId::new(72_000 + offset as u128);
            let mut registry = SemanticRegistry::default();
            let digest = registry.install_ordering(module);
            let context = context(ordering, digest);
            let resolved = registry
                .resolve_primitive_ordering(&context, ordering)
                .unwrap();
            assert_eq!(resolved.module_digest(), digest);
            let values = match module {
                OrderingModule::UnitExact => vec![Value::Unit],
                OrderingModule::BoolAscending => vec![Value::Bool(false), Value::Bool(true)],
                OrderingModule::I64Ascending => {
                    vec![
                        Value::I64(i64::MIN),
                        Value::I64(-1),
                        Value::I64(0),
                        Value::I64(i64::MAX),
                    ]
                }
                OrderingModule::F64Total => vec![
                    Value::F64Bits(f64::NEG_INFINITY.to_bits()),
                    Value::F64Bits(0xfff8_0000_0000_0002),
                    Value::F64Bits((-0.0_f64).to_bits()),
                    Value::F64Bits(0.0_f64.to_bits()),
                    Value::F64Bits(f64::INFINITY.to_bits()),
                    Value::F64Bits(0x7ff8_0000_0000_0001),
                    Value::F64Bits(0x7ff8_0000_0000_0002),
                ],
                OrderingModule::TextBinary
                | OrderingModule::TextAsciiCaseInsensitive
                | OrderingModule::TextAsciiCaseInsensitiveThenBinary => vec![
                    Value::Text("A".into()),
                    Value::Text("a".into()),
                    Value::Text("B".into()),
                    Value::Text("aΩZ".into()),
                    Value::Text("AΩz".into()),
                    Value::Text("ω".into()),
                ],
                OrderingModule::LiveEntityIdAscending(entity_type) => vec![
                    Value::LiveEntityRef {
                        entity_type,
                        id: kernel_types::EntityId::new(1),
                    },
                    Value::LiveEntityRef {
                        entity_type,
                        id: kernel_types::EntityId::new(2),
                    },
                ],
                OrderingModule::HistoricalEntityIdAscending(entity_type) => vec![
                    Value::HistoricalEntityId {
                        entity_type,
                        id: kernel_types::EntityId::new(1),
                    },
                    Value::HistoricalEntityId {
                        entity_type,
                        id: kernel_types::EntityId::new(2),
                    },
                ],
            };
            for left in &values {
                for right in &values {
                    assert_eq!(
                        resolved
                            .canonical_key(left)
                            .unwrap()
                            .cmp(&resolved.canonical_key(right).unwrap()),
                        resolved.compare(left, right).unwrap(),
                        "module={module:?}, left={left:?}, right={right:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn f64_total_canonical_key_matches_total_cmp_on_many_bit_patterns() {
        let ordering = SemanticId::new(73_000);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_ordering(OrderingModule::F64Total);
        let context = context(ordering, digest);
        let resolved = registry
            .resolve_primitive_ordering(&context, ordering)
            .unwrap();
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for _ in 0..100_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let left = Value::F64Bits(state);
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let right = Value::F64Bits(state);
            assert_eq!(
                resolved
                    .canonical_key(&left)
                    .unwrap()
                    .cmp(&resolved.canonical_key(&right).unwrap()),
                resolved.compare(&left, &right).unwrap()
            );
        }
    }

    fn assert_structural_key_law(
        registry: &SemanticRegistry,
        context: &SemanticContext,
        equivalence: SemanticId,
        values: &[Value],
    ) {
        let compiled = registry.compile_equivalence(context, equivalence).unwrap();
        assert_eq!(compiled.equivalence(), equivalence);
        assert_eq!(
            compiled.domain(),
            &registry.equivalence_domain(context, equivalence).unwrap()
        );
        for left in values {
            for right in values {
                let oracle = registry
                    .equivalent(context, equivalence, left, right)
                    .unwrap();
                let left_key = registry
                    .canonical_equivalence_key(context, equivalence, left)
                    .unwrap();
                let right_key = registry
                    .canonical_equivalence_key(context, equivalence, right)
                    .unwrap();
                assert_eq!(left_key == right_key, oracle);
                assert_eq!(compiled.canonical_key(left).unwrap(), left_key);
                assert_eq!(compiled.canonical_key(right).unwrap(), right_key);
                assert_eq!(compiled.equivalent(left, right).unwrap(), oracle);
            }
        }
    }

    #[test]
    fn structural_product_option_and_sum_keys_match_equivalence_oracle() {
        let exact = SemanticId::new(80_000);
        let ci = SemanticId::new(80_001);
        let product = SemanticId::new(80_002);
        let option = SemanticId::new(80_003);
        let sum = SemanticId::new(80_004);
        let field_text = SemanticId::new(80_005);
        let field_int = SemanticId::new(80_006);
        let tag_text = SemanticId::new(80_007);
        let tag_int = SemanticId::new(80_008);
        let mut registry = SemanticRegistry::default();
        let exact_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(80_000));
        schema
            .define_structural_equivalence(
                product,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field_text, ci), (field_int, exact)]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(option, StructuralEquivalenceDef::Option { inner: ci })
            .unwrap();
        schema
            .define_structural_equivalence(
                sum,
                StructuralEquivalenceDef::Sum {
                    variants: BTreeMap::from([(tag_text, ci), (tag_int, exact)]),
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(80_000));
        environment.pin_module(exact, exact_digest);
        environment.pin_module(ci, ci_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let product_value = |text: &str, int: i64| {
            Value::Product(BTreeMap::from([
                (field_text, Value::Text(text.into())),
                (field_int, Value::I64(int)),
            ]))
        };
        assert_structural_key_law(
            &registry,
            &context,
            product,
            &[
                product_value("A", 1),
                product_value("a", 1),
                product_value("a", 2),
            ],
        );
        assert_structural_key_law(
            &registry,
            &context,
            option,
            &[
                Value::Option(None),
                Value::Option(Some(Box::new(Value::Text("A".into())))),
                Value::Option(Some(Box::new(Value::Text("a".into())))),
                Value::Option(Some(Box::new(Value::Text("B".into())))),
            ],
        );
        assert_structural_key_law(
            &registry,
            &context,
            sum,
            &[
                Value::Variant {
                    tag: tag_text,
                    value: Box::new(Value::Text("A".into())),
                },
                Value::Variant {
                    tag: tag_text,
                    value: Box::new(Value::Text("a".into())),
                },
                Value::Variant {
                    tag: tag_int,
                    value: Box::new(Value::I64(1)),
                },
            ],
        );
    }

    #[test]
    fn structural_collection_keys_match_equivalence_oracle() {
        let ci = SemanticId::new(81_000);
        let set = SemanticId::new(81_001);
        let bag = SemanticId::new(81_002);
        let seq = SemanticId::new(81_003);
        let map = SemanticId::new(81_004);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(81_000));
        schema
            .define_structural_equivalence(set, StructuralEquivalenceDef::Set { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(bag, StructuralEquivalenceDef::Bag { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(seq, StructuralEquivalenceDef::Seq { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(
                map,
                StructuralEquivalenceDef::Map { key: ci, value: ci },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(81_000));
        environment.pin_module(ci, digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let text = |value: &str| Value::Text(value.into());
        assert_structural_key_law(
            &registry,
            &context,
            set,
            &[
                Value::Set {
                    equivalence: ci,
                    elements: vec![text("A"), text("B")],
                },
                Value::Set {
                    equivalence: ci,
                    elements: vec![text("b"), text("a")],
                },
                Value::Set {
                    equivalence: ci,
                    elements: vec![text("A"), text("C")],
                },
            ],
        );
        assert_structural_key_law(
            &registry,
            &context,
            bag,
            &[
                Value::Bag {
                    equivalence: ci,
                    entries: vec![(text("A"), 2), (text("B"), 1)],
                },
                Value::Bag {
                    equivalence: ci,
                    entries: vec![(text("b"), 1), (text("a"), 2)],
                },
                Value::Bag {
                    equivalence: ci,
                    entries: vec![(text("a"), 1), (text("b"), 1)],
                },
            ],
        );
        assert_structural_key_law(
            &registry,
            &context,
            seq,
            &[
                Value::Seq(vec![text("A"), text("B")]),
                Value::Seq(vec![text("a"), text("b")]),
                Value::Seq(vec![text("b"), text("a")]),
            ],
        );
        assert_structural_key_law(
            &registry,
            &context,
            map,
            &[
                Value::Map {
                    key_equivalence: ci,
                    entries: vec![(text("K1"), text("V1")), (text("K2"), text("V2"))],
                },
                Value::Map {
                    key_equivalence: ci,
                    entries: vec![(text("k2"), text("v2")), (text("k1"), text("v1"))],
                },
                Value::Map {
                    key_equivalence: ci,
                    entries: vec![(text("k1"), text("v2")), (text("k2"), text("v1"))],
                },
            ],
        );
    }

    #[test]
    fn coarse_unordered_structural_keys_use_canonical_finite_measures() {
        let exact = SemanticId::new(81_100);
        let ci = SemanticId::new(81_101);
        let set_ci = SemanticId::new(81_102);
        let bag_ci = SemanticId::new(81_103);
        let map_ci = SemanticId::new(81_104);
        let mut registry = SemanticRegistry::default();
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(81_100));
        schema
            .define_structural_equivalence(set_ci, StructuralEquivalenceDef::Set { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(bag_ci, StructuralEquivalenceDef::Bag { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(
                map_ci,
                StructuralEquivalenceDef::Map {
                    key: ci,
                    value: exact,
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(81_100));
        environment.pin_module(exact, exact_digest);
        environment.pin_module(ci, ci_digest);
        let context = SemanticContext {
            schema,
            environment,
        };

        let values = [
            (
                set_ci,
                Value::Set {
                    equivalence: exact,
                    elements: vec![Value::Text("A".into()), Value::Text("a".into())],
                },
            ),
            (
                bag_ci,
                Value::Bag {
                    equivalence: exact,
                    entries: vec![(Value::Text("A".into()), 2), (Value::Text("a".into()), 2)],
                },
            ),
            (
                map_ci,
                Value::Map {
                    key_equivalence: exact,
                    entries: vec![
                        (Value::Text("A".into()), Value::Text("same".into())),
                        (Value::Text("a".into()), Value::Text("same".into())),
                    ],
                },
            ),
        ];
        for (equivalence, value) in values {
            let key = registry
                .canonical_equivalence_key(&context, equivalence, &value)
                .unwrap();
            assert_eq!(
                decode_canonical_eq_key(&encode_canonical_eq_key(&key)),
                Ok(key.clone())
            );
            match key {
                CanonicalEqKey::Set(entries) => assert_eq!(entries[0].multiplicity, 2),
                CanonicalEqKey::Bag(entries) => assert_eq!(entries[0].multiplicity, 2),
                CanonicalEqKey::Map(entries) => assert_eq!(entries[0].multiplicity, 2),
                _ => panic!("fixture must lower to an unordered finite measure"),
            }
        }
    }

    #[test]
    fn canonical_eq_key_codec_roundtrips_recursive_structures_and_rejects_version_drift() {
        let key = CanonicalEqKey::Product(vec![
            (
                SemanticId::new(1),
                CanonicalEqKey::OptionSome(Box::new(CanonicalEqKey::Seq(vec![
                    CanonicalEqKey::TextAsciiCaseInsensitive("alpha".into()),
                    CanonicalEqKey::I64(-7),
                ]))),
            ),
            (
                SemanticId::new(2),
                CanonicalEqKey::Map(finite_measure_from_atoms([
                    CanonicalMapAtom {
                        key: CanonicalEqKey::TextExact("a".into()),
                        value: CanonicalEqKey::Bag(finite_measure_from_atoms([CanonicalBagAtom {
                            value: CanonicalEqKey::Bool(false),
                            stored_count: 2,
                        }])),
                    },
                    CanonicalMapAtom {
                        key: CanonicalEqKey::TextExact("b".into()),
                        value: CanonicalEqKey::Set(finite_measure_from_atoms([
                            CanonicalEqKey::I64(1),
                            CanonicalEqKey::I64(2),
                        ])),
                    },
                ])),
            ),
        ]);
        let encoded = encode_canonical_eq_key(&key);
        assert_eq!(decode_canonical_eq_key(&encoded), Ok(key));

        let mut previous = encoded.clone();
        previous[4..8].copy_from_slice(&(CANONICAL_EQ_KEY_ENCODING_VERSION - 1).to_be_bytes());
        assert_eq!(
            decode_canonical_eq_key(&previous),
            Err(CanonicalEqKeyCodecError::UnsupportedVersion(
                CANONICAL_EQ_KEY_ENCODING_VERSION - 1
            ))
        );

        let mut future = encoded.clone();
        future[4..8].copy_from_slice(&(CANONICAL_EQ_KEY_ENCODING_VERSION + 1).to_be_bytes());
        assert_eq!(
            decode_canonical_eq_key(&future),
            Err(CanonicalEqKeyCodecError::UnsupportedVersion(
                CANONICAL_EQ_KEY_ENCODING_VERSION + 1
            ))
        );

        let tuple = vec![
            CanonicalEqKey::I64(9),
            CanonicalEqKey::TextExact("x".into()),
        ];
        assert_eq!(
            decode_canonical_eq_key_tuple(&encode_canonical_eq_key_tuple(&tuple)),
            Ok(tuple)
        );
    }

    #[test]
    fn canonical_eq_key_v2_has_golden_bytes_independent_of_rust_layout() {
        let key = CanonicalEqKey::Product(vec![
            (
                SemanticId::new(1),
                CanonicalEqKey::TextAsciiCaseInsensitive("a".into()),
            ),
            (SemanticId::new(2), CanonicalEqKey::I64(-1)),
        ]);
        assert_eq!(
            encode_canonical_eq_key(&key),
            vec![
                67, 69, 75, 0, 0, 0, 0, 2, 8, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 0, 0, 1, 97, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 2, 2, 255, 255, 255, 255, 255, 255, 255, 255,
            ]
        );
    }

    #[test]
    fn canonical_eq_key_codec_rejects_noncanonical_and_hostile_lengths() {
        let noncanonical = CanonicalEqKey::Set(vec![
            FiniteMeasureEntry {
                atom: CanonicalEqKey::I64(2),
                multiplicity: 1,
            },
            FiniteMeasureEntry {
                atom: CanonicalEqKey::I64(1),
                multiplicity: 1,
            },
        ]);
        assert_eq!(
            decode_canonical_eq_key(&encode_canonical_eq_key(&noncanonical)),
            Err(CanonicalEqKeyCodecError::NonCanonicalShape)
        );
        let noncanonical_ci = CanonicalEqKey::TextAsciiCaseInsensitive("Alpha".into());
        assert_eq!(
            decode_canonical_eq_key(&encode_canonical_eq_key(&noncanonical_ci)),
            Err(CanonicalEqKeyCodecError::NonCanonicalShape)
        );

        let mut hostile = Vec::from(CANONICAL_EQ_KEY_MAGIC);
        hostile.extend_from_slice(&CANONICAL_EQ_KEY_ENCODING_VERSION.to_be_bytes());
        hostile.push(13);
        hostile.extend_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(
            decode_canonical_eq_key(&hostile),
            Err(CanonicalEqKeyCodecError::Truncated)
        );

        let mut too_deep = Vec::from(CANONICAL_EQ_KEY_MAGIC);
        too_deep.extend_from_slice(&CANONICAL_EQ_KEY_ENCODING_VERSION.to_be_bytes());
        too_deep.extend(std::iter::repeat_n(10_u8, MAX_CANONICAL_EQ_KEY_DEPTH + 1));
        too_deep.push(0);
        assert_eq!(
            decode_canonical_eq_key(&too_deep),
            Err(CanonicalEqKeyCodecError::DepthLimitExceeded)
        );
    }

    #[test]
    fn semantic_work_estimate_distinguishes_payload_and_nested_structure() {
        let scalar = semantic_value_work_estimate(&Value::I64(1));
        let text = semantic_value_work_estimate(&Value::Text("abcdefghij".into()));
        let nested = semantic_value_work_estimate(&Value::Seq(vec![
            Value::I64(1),
            Value::Seq(vec![Value::I64(2), Value::I64(3)]),
        ]));
        assert_eq!(scalar.total_units(), 1);
        assert_eq!(text.total_units(), 11);
        assert!(nested.total_units() > scalar.total_units());

        let key = CanonicalEqKey::Seq(vec![
            CanonicalEqKey::TextExact("abcd".into()),
            CanonicalEqKey::I64(1),
        ]);
        assert_eq!(canonical_eq_key_work_estimate(&key).total_units(), 7);
    }

    #[test]
    fn structural_canonical_dependency_closure_tracks_only_primitive_leaf_digests() {
        let ci = SemanticId::new(82_000);
        let exact = SemanticId::new(82_001);
        let product = SemanticId::new(82_002);
        let set = SemanticId::new(82_003);
        let field_a = SemanticId::new(82_004);
        let field_b = SemanticId::new(82_005);
        let mut registry = SemanticRegistry::default();
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let exact_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut schema = Schema::new(SchemaRevisionId::new(82_000));
        schema
            .define_structural_equivalence(set, StructuralEquivalenceDef::Set { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(
                product,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field_a, set), (field_b, exact)]),
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(82_000));
        environment.pin_module(ci, ci_digest);
        environment.pin_module(exact, exact_digest);
        let context = SemanticContext {
            schema,
            environment,
        };

        assert_eq!(
            registry
                .canonical_equivalence_dependencies(&context, product)
                .unwrap(),
            vec![
                CanonicalEquivalenceDependency {
                    semantic_id: ci,
                    module_digest: ci_digest,
                },
                CanonicalEquivalenceDependency {
                    semantic_id: exact,
                    module_digest: exact_digest,
                },
            ]
        );
    }
}
