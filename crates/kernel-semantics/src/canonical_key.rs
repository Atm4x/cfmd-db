use std::collections::BTreeMap;

use kernel_model::Value;
use kernel_types::SemanticId;

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

pub(super) const CANONICAL_EQ_KEY_MAGIC: [u8; 4] = *b"CEK\0";
const CANONICAL_EQ_KEY_TUPLE_MAGIC: [u8; 4] = *b"CKT\0";
pub(super) const MAX_CANONICAL_EQ_KEY_DEPTH: usize = 256;

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
