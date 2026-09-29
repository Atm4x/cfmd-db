use std::collections::BTreeMap;
use std::io::Read;

use kernel_model::Value;
use kernel_types::{EntityId, SemanticId};

use crate::runtime::{CodecError, DurabilityError};

pub(crate) const MAX_COLLECTION_LEN: usize = 1_000_000;
pub(crate) const MAX_VALUE_DEPTH: usize = 128;

pub(crate) trait BinarySink {
    fn push(&mut self, byte: u8);
    fn extend_from_slice(&mut self, bytes: &[u8]);
}

impl BinarySink for Vec<u8> {
    fn push(&mut self, byte: u8) {
        Vec::push(self, byte);
    }

    fn extend_from_slice(&mut self, bytes: &[u8]) {
        Vec::extend_from_slice(self, bytes);
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CountingBinarySink {
    len: u64,
    overflowed: bool,
}

impl CountingBinarySink {
    pub(crate) fn len(self) -> Result<u64, CodecError> {
        if self.overflowed {
            Err(CodecError::LengthOverflow)
        } else {
            Ok(self.len)
        }
    }
}

impl BinarySink for CountingBinarySink {
    fn push(&mut self, _byte: u8) {
        self.len = self.len.checked_add(1).unwrap_or_else(|| {
            self.overflowed = true;
            u64::MAX
        });
    }

    fn extend_from_slice(&mut self, bytes: &[u8]) {
        if let Some(len) = u64::try_from(bytes.len())
            .ok()
            .and_then(|len| self.len.checked_add(len))
        {
            self.len = len;
        } else {
            self.overflowed = true;
            self.len = u64::MAX;
        }
    }
}

pub(crate) struct StreamingBinarySink<'a> {
    emit: &'a mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    buffer: Vec<u8>,
    failure: Option<DurabilityError>,
}

impl<'a> StreamingBinarySink<'a> {
    const BUFFER_LEN: usize = 64 * 1024;

    pub(crate) fn new(emit: &'a mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>) -> Self {
        Self {
            emit,
            buffer: Vec::with_capacity(Self::BUFFER_LEN),
            failure: None,
        }
    }

    fn flush_buffer(&mut self) {
        if self.buffer.is_empty() || self.failure.is_some() {
            self.buffer.clear();
            return;
        }
        if let Err(error) = (self.emit)(&self.buffer) {
            self.failure = Some(error);
        }
        self.buffer.clear();
    }

    pub(crate) fn finish(mut self) -> Result<(), DurabilityError> {
        self.flush_buffer();
        match self.failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl BinarySink for StreamingBinarySink<'_> {
    fn push(&mut self, byte: u8) {
        if self.failure.is_some() {
            return;
        }
        if self.buffer.len() == Self::BUFFER_LEN {
            self.flush_buffer();
            if self.failure.is_some() {
                return;
            }
        }
        self.buffer.push(byte);
    }

    fn extend_from_slice(&mut self, mut bytes: &[u8]) {
        if self.failure.is_some() {
            return;
        }
        while !bytes.is_empty() {
            let remaining = Self::BUFFER_LEN - self.buffer.len();
            let take = remaining.min(bytes.len());
            self.buffer.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.buffer.len() == Self::BUFFER_LEN {
                self.flush_buffer();
                if self.failure.is_some() {
                    return;
                }
            }
        }
    }
}

pub(crate) fn encode_rows(
    out: &mut impl BinarySink,
    rows: &[Vec<Value>],
) -> Result<(), CodecError> {
    push_len(out, rows.len())?;
    for row in rows {
        push_len(out, row.len())?;
        for value in row {
            encode_value(out, value, 0)?;
        }
    }
    Ok(())
}

pub(crate) fn encode_value(
    out: &mut impl BinarySink,
    value: &Value,
    depth: usize,
) -> Result<(), CodecError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(CodecError::ValueNestingTooDeep);
    }
    match value {
        Value::Unit => out.push(0),
        Value::Bool(value) => {
            out.push(1);
            out.push(u8::from(*value));
        }
        Value::I64(value) => {
            out.push(2);
            out.extend_from_slice(&value.to_le_bytes());
        }
        Value::F64Bits(value) => {
            out.push(3);
            push_u64(out, *value);
        }
        Value::Text(value) => {
            out.push(4);
            push_bytes(out, value.as_bytes())?;
        }
        Value::LiveEntityRef { entity_type, id } => {
            out.push(5);
            push_u128(out, entity_type.raw());
            push_u128(out, id.raw());
        }
        Value::HistoricalEntityId { entity_type, id } => {
            out.push(6);
            push_u128(out, entity_type.raw());
            push_u128(out, id.raw());
        }
        Value::Product(fields) => {
            out.push(7);
            push_len(out, fields.len())?;
            for (field, child) in fields {
                push_u128(out, field.raw());
                encode_value(out, child, depth + 1)?;
            }
        }
        Value::Option(value) => {
            out.push(8);
            if let Some(value) = value {
                out.push(1);
                encode_value(out, value, depth + 1)?;
            } else {
                out.push(0);
            }
        }
        Value::Variant { tag, value } => {
            out.push(9);
            push_u128(out, tag.raw());
            encode_value(out, value, depth + 1)?;
        }
        Value::Seq(values) => {
            out.push(10);
            push_len(out, values.len())?;
            for value in values {
                encode_value(out, value, depth + 1)?;
            }
        }
        Value::Set {
            equivalence,
            elements,
        } => {
            out.push(11);
            push_u128(out, equivalence.raw());
            push_len(out, elements.len())?;
            for value in elements {
                encode_value(out, value, depth + 1)?;
            }
        }
        Value::Bag {
            equivalence,
            entries,
        } => {
            out.push(12);
            push_u128(out, equivalence.raw());
            push_len(out, entries.len())?;
            for (value, count) in entries {
                encode_value(out, value, depth + 1)?;
                push_u64(out, *count);
            }
        }
        Value::Map {
            key_equivalence,
            entries,
        } => {
            out.push(13);
            push_u128(out, key_equivalence.raw());
            push_len(out, entries.len())?;
            for (key, value) in entries {
                encode_value(out, key, depth + 1)?;
                encode_value(out, value, depth + 1)?;
            }
        }
    }
    Ok(())
}

pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8], &'static str> {
        let end = self
            .position
            .checked_add(len)
            .ok_or("codec offset overflow")?;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or("truncated mutation payload")?;
        self.position = end;
        Ok(slice)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, &'static str> {
        Ok(read_u16(self.take(2)?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, &'static str> {
        Ok(read_u32(self.take(4)?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, &'static str> {
        Ok(read_u64(self.take(8)?))
    }

    pub(crate) fn u128(&mut self) -> Result<u128, &'static str> {
        Ok(u128::from_le_bytes(
            self.take(16)?.try_into().map_err(|_| "u128 decode")?,
        ))
    }

    pub(crate) fn len(&mut self) -> Result<usize, &'static str> {
        let value = usize::try_from(self.u32()?).map_err(|_| "collection length overflow")?;
        if value > MAX_COLLECTION_LEN {
            return Err("collection length exceeds hard limit");
        }
        Ok(value)
    }

    pub(crate) fn bounded_capacity(&self, count: usize) -> usize {
        count.min(self.bytes.len().saturating_sub(self.position))
    }

    pub(crate) fn finish(self) -> Result<(), &'static str> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err("trailing bytes in mutation payload")
        }
    }
}

pub(crate) trait BinarySource {
    fn read_exact_into(&mut self, out: &mut [u8]) -> Result<(), &'static str>;
    fn remaining(&self) -> u64;

    fn take_owned(&mut self, len: usize) -> Result<Vec<u8>, &'static str> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|_| "codec allocation failed")?;
        bytes.resize(len, 0);
        self.read_exact_into(&mut bytes)?;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, &'static str> {
        let mut bytes = [0_u8; 1];
        self.read_exact_into(&mut bytes)?;
        Ok(bytes[0])
    }

    fn u16(&mut self) -> Result<u16, &'static str> {
        let mut bytes = [0_u8; 2];
        self.read_exact_into(&mut bytes)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, &'static str> {
        let mut bytes = [0_u8; 4];
        self.read_exact_into(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, &'static str> {
        let mut bytes = [0_u8; 8];
        self.read_exact_into(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn u128(&mut self) -> Result<u128, &'static str> {
        let mut bytes = [0_u8; 16];
        self.read_exact_into(&mut bytes)?;
        Ok(u128::from_le_bytes(bytes))
    }

    fn len(&mut self) -> Result<usize, &'static str> {
        let value = usize::try_from(self.u32()?).map_err(|_| "collection length overflow")?;
        if value > MAX_COLLECTION_LEN {
            return Err("collection length exceeds hard limit");
        }
        Ok(value)
    }

    fn bounded_capacity(&self, count: usize) -> usize {
        count.min(usize::try_from(self.remaining()).unwrap_or(usize::MAX))
    }

    fn string(&mut self) -> Result<String, &'static str> {
        let len = self.len()?;
        let bytes = self.take_owned(len)?;
        std::str::from_utf8(&bytes)
            .map(str::to_owned)
            .map_err(|_| "invalid utf-8 string")
    }

    fn rows(&mut self, depth: usize) -> Result<Vec<Vec<Value>>, &'static str> {
        let count = self.len()?;
        let mut rows = Vec::with_capacity(self.bounded_capacity(count));
        for _ in 0..count {
            let columns = self.len()?;
            let mut row = Vec::with_capacity(self.bounded_capacity(columns));
            for _ in 0..columns {
                row.push(self.value(depth)?);
            }
            rows.push(row);
        }
        Ok(rows)
    }

    fn value(&mut self, depth: usize) -> Result<Value, &'static str> {
        if depth > MAX_VALUE_DEPTH {
            return Err("value nesting exceeds hard limit");
        }
        match self.u8()? {
            0 => Ok(Value::Unit),
            1 => match self.u8()? {
                0 => Ok(Value::Bool(false)),
                1 => Ok(Value::Bool(true)),
                _ => Err("invalid bool encoding"),
            },
            2 => {
                let mut bytes = [0_u8; 8];
                self.read_exact_into(&mut bytes)?;
                Ok(Value::I64(i64::from_le_bytes(bytes)))
            }
            3 => Ok(Value::F64Bits(self.u64()?)),
            4 => {
                let len = self.len()?;
                let bytes = self.take_owned(len)?;
                let text = std::str::from_utf8(&bytes).map_err(|_| "invalid utf-8 text")?;
                Ok(Value::Text(text.to_owned()))
            }
            5 => Ok(Value::LiveEntityRef {
                entity_type: SemanticId::new(self.u128()?),
                id: EntityId::new(self.u128()?),
            }),
            6 => Ok(Value::HistoricalEntityId {
                entity_type: SemanticId::new(self.u128()?),
                id: EntityId::new(self.u128()?),
            }),
            7 => {
                let count = self.len()?;
                let mut fields = BTreeMap::new();
                let mut previous = None;
                for _ in 0..count {
                    let field = SemanticId::new(self.u128()?);
                    if previous.is_some_and(|id: SemanticId| id >= field) {
                        return Err("product fields are not strictly sorted and unique");
                    }
                    previous = Some(field);
                    fields.insert(field, self.value(depth + 1)?);
                }
                Ok(Value::Product(fields))
            }
            8 => match self.u8()? {
                0 => Ok(Value::Option(None)),
                1 => Ok(Value::Option(Some(Box::new(self.value(depth + 1)?)))),
                _ => Err("invalid option discriminant"),
            },
            9 => Ok(Value::Variant {
                tag: SemanticId::new(self.u128()?),
                value: Box::new(self.value(depth + 1)?),
            }),
            10 => {
                let count = self.len()?;
                let mut values = Vec::with_capacity(self.bounded_capacity(count));
                for _ in 0..count {
                    values.push(self.value(depth + 1)?);
                }
                Ok(Value::Seq(values))
            }
            11 => {
                let equivalence = SemanticId::new(self.u128()?);
                let count = self.len()?;
                let mut elements = Vec::with_capacity(self.bounded_capacity(count));
                for _ in 0..count {
                    elements.push(self.value(depth + 1)?);
                }
                Ok(Value::Set {
                    equivalence,
                    elements,
                })
            }
            12 => {
                let equivalence = SemanticId::new(self.u128()?);
                let count = self.len()?;
                let mut entries = Vec::with_capacity(self.bounded_capacity(count));
                for _ in 0..count {
                    entries.push((self.value(depth + 1)?, self.u64()?));
                }
                Ok(Value::Bag {
                    equivalence,
                    entries,
                })
            }
            13 => {
                let key_equivalence = SemanticId::new(self.u128()?);
                let count = self.len()?;
                let mut entries = Vec::with_capacity(self.bounded_capacity(count));
                for _ in 0..count {
                    entries.push((self.value(depth + 1)?, self.value(depth + 1)?));
                }
                Ok(Value::Map {
                    key_equivalence,
                    entries,
                })
            }
            _ => Err("unknown value tag"),
        }
    }

    fn finish(&self) -> Result<(), &'static str> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err("trailing bytes in mutation payload")
        }
    }
}

impl BinarySource for Cursor<'_> {
    fn read_exact_into(&mut self, out: &mut [u8]) -> Result<(), &'static str> {
        out.copy_from_slice(self.take(out.len())?);
        Ok(())
    }

    fn remaining(&self) -> u64 {
        u64::try_from(self.bytes.len().saturating_sub(self.position)).unwrap_or(u64::MAX)
    }
}

pub(crate) struct ReadBinarySource<'a> {
    reader: &'a mut dyn Read,
    remaining: u64,
}

impl<'a> ReadBinarySource<'a> {
    pub(crate) const fn new(reader: &'a mut dyn Read, len: u64) -> Self {
        Self {
            reader,
            remaining: len,
        }
    }
}

impl BinarySource for ReadBinarySource<'_> {
    fn read_exact_into(&mut self, out: &mut [u8]) -> Result<(), &'static str> {
        let len = u64::try_from(out.len()).map_err(|_| "codec length overflow")?;
        if len > self.remaining {
            return Err("truncated mutation payload");
        }
        self.reader
            .read_exact(out)
            .map_err(|_| "binary source read failed")?;
        self.remaining -= len;
        Ok(())
    }

    fn remaining(&self) -> u64 {
        self.remaining
    }
}

pub(crate) fn push_bytes(out: &mut impl BinarySink, bytes: &[u8]) -> Result<(), CodecError> {
    push_len(out, bytes.len())?;
    out.extend_from_slice(bytes);
    Ok(())
}

pub(crate) fn push_len(out: &mut impl BinarySink, value: usize) -> Result<(), CodecError> {
    if value > MAX_COLLECTION_LEN {
        return Err(CodecError::CollectionTooLarge);
    }
    push_u32(
        out,
        u32::try_from(value).map_err(|_| CodecError::LengthOverflow)?,
    );
    Ok(())
}

pub(crate) fn push_u16(out: &mut impl BinarySink, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn push_u32(out: &mut impl BinarySink, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn push_u64(out: &mut impl BinarySink, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn push_u128(out: &mut impl BinarySink, value: u128) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(crate) fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().expect("exact u16 slice"))
}

pub(crate) fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("exact u32 slice"))
}

pub(crate) fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("exact u64 slice"))
}

#[must_use]
pub fn crc32c(bytes: &[u8]) -> u32 {
    !crc32c_update(!0_u32, bytes)
}

pub(crate) fn crc32c_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0x82F6_3B78 & mask);
        }
    }
    crc
}
