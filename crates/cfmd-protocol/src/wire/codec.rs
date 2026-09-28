use std::collections::BTreeMap;

use crate::{
    CommitRequest, CommitResponse, EntityRef, HistoryEntryDto, HistoryKind,
    HistoryRelationChangeDto, HistoryReversibilityDto, HostedRequest, HostedResponse,
    OpenWatchRequest, OpenWatchResponse, OrderDirection, ProtocolErrorCode, ProtocolQuery,
    ProtocolValue, QueryRequest, QueryResponse, RelationMutation, Result, Row, SnapshotTarget,
    SubscriptionId, WatchEventDto, WatchStatusDto,
};

use super::{WireLimits, WireResponse, resource_limit, wire_error};

pub fn encode_request_payload(request: &HostedRequest, limits: WireLimits) -> Result<Vec<u8>> {
    let mut encoder = Encoder::new(limits);
    encoder.hosted_request(request, 0)?;
    Ok(encoder.finish())
}

pub fn decode_request_payload(payload: &[u8], limits: WireLimits) -> Result<HostedRequest> {
    check_payload_size(payload, limits)?;
    let mut decoder = Decoder::new(payload, limits);
    let request = decoder.hosted_request(0)?;
    decoder.finish()?;
    Ok(request)
}

pub fn encode_response_payload(response: &WireResponse, limits: WireLimits) -> Result<Vec<u8>> {
    let mut encoder = Encoder::new(limits);
    match response {
        WireResponse::Success(response) => {
            encoder.u8(0)?;
            encoder.hosted_response(response, 0)?;
        }
        WireResponse::Error { code, message } => {
            encoder.u8(1)?;
            encoder.error_code(*code)?;
            encoder.string(message)?;
        }
    }
    Ok(encoder.finish())
}

pub fn decode_response_payload(payload: &[u8], limits: WireLimits) -> Result<WireResponse> {
    check_payload_size(payload, limits)?;
    let mut decoder = Decoder::new(payload, limits);
    let response = match decoder.u8()? {
        0 => WireResponse::Success(decoder.hosted_response(0)?),
        1 => WireResponse::Error {
            code: decoder.error_code()?,
            message: decoder.string()?,
        },
        _ => return Err(wire_error("unknown wire response status")),
    };
    decoder.finish()?;
    Ok(response)
}

fn check_payload_size(payload: &[u8], limits: WireLimits) -> Result<()> {
    if payload.len() > limits.max_payload_bytes as usize {
        return Err(resource_limit("wire payload exceeds configured limit"));
    }
    Ok(())
}

struct Encoder {
    bytes: Vec<u8>,
    limits: WireLimits,
    nodes: u32,
}

impl Encoder {
    fn new(limits: WireLimits) -> Self {
        Self {
            bytes: Vec::new(),
            limits,
            nodes: 0,
        }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }

    fn enter(&mut self, depth: u16) -> Result<()> {
        if depth > self.limits.max_decode_depth {
            return Err(resource_limit("wire value exceeds depth limit"));
        }
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_decode_nodes {
            return Err(resource_limit("wire value exceeds node limit"));
        }
        Ok(())
    }

    fn extend(&mut self, bytes: &[u8]) -> Result<()> {
        let next = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| resource_limit("wire payload size overflow"))?;
        if next > self.limits.max_payload_bytes as usize {
            return Err(resource_limit("wire payload exceeds configured limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn u8(&mut self, value: u8) -> Result<()> {
        self.extend(&[value])
    }

    fn bool(&mut self, value: bool) -> Result<()> {
        self.u8(u8::from(value))
    }

    fn u32(&mut self, value: u32) -> Result<()> {
        self.extend(&value.to_be_bytes())
    }

    fn u64(&mut self, value: u64) -> Result<()> {
        self.extend(&value.to_be_bytes())
    }

    fn i64(&mut self, value: i64) -> Result<()> {
        self.extend(&value.to_be_bytes())
    }

    fn u128(&mut self, value: u128) -> Result<()> {
        self.extend(&value.to_be_bytes())
    }

    fn usize(&mut self, value: usize) -> Result<()> {
        let value = u32::try_from(value)
            .map_err(|_| resource_limit("wire integer exceeds canonical u32 range"))?;
        self.u32(value)
    }

    fn collection_len(&mut self, len: usize) -> Result<()> {
        let len = u32::try_from(len).map_err(|_| resource_limit("wire collection is too large"))?;
        if len > self.limits.max_collection_items {
            return Err(resource_limit("wire collection exceeds configured limit"));
        }
        self.u32(len)
    }

    fn string(&mut self, value: &str) -> Result<()> {
        let len =
            u32::try_from(value.len()).map_err(|_| resource_limit("wire string is too large"))?;
        if len > self.limits.max_string_bytes {
            return Err(resource_limit("wire string exceeds configured limit"));
        }
        self.u32(len)?;
        self.extend(value.as_bytes())
    }

    fn snapshot_target(&mut self, target: SnapshotTarget) -> Result<()> {
        match target {
            SnapshotTarget::Head => self.u8(0),
            SnapshotTarget::Revision(revision) => {
                self.u8(1)?;
                self.u64(revision)
            }
        }
    }

    fn value(&mut self, value: &ProtocolValue, depth: u16) -> Result<()> {
        self.enter(depth)?;
        let next = depth.saturating_add(1);
        match value {
            ProtocolValue::Unit => self.u8(0),
            ProtocolValue::Bool(value) => {
                self.u8(1)?;
                self.bool(*value)
            }
            ProtocolValue::I64(value) => {
                self.u8(2)?;
                self.i64(*value)
            }
            ProtocolValue::F64Bits(value) => {
                self.u8(3)?;
                self.u64(*value)
            }
            ProtocolValue::Text(value) => {
                self.u8(4)?;
                self.string(value)
            }
            ProtocolValue::LiveEntityRef(value) => {
                self.u8(5)?;
                self.entity_ref(*value)
            }
            ProtocolValue::HistoricalEntityRef(value) => {
                self.u8(6)?;
                self.entity_ref(*value)
            }
            ProtocolValue::Product(fields) => {
                self.u8(7)?;
                self.collection_len(fields.len())?;
                for (field, value) in fields {
                    self.u128(*field)?;
                    self.value(value, next)?;
                }
                Ok(())
            }
            ProtocolValue::Option(value) => {
                self.u8(8)?;
                match value {
                    None => self.u8(0),
                    Some(value) => {
                        self.u8(1)?;
                        self.value(value, next)
                    }
                }
            }
            ProtocolValue::Variant { tag, value } => {
                self.u8(9)?;
                self.u128(*tag)?;
                self.value(value, next)
            }
            ProtocolValue::Seq(values) => {
                self.u8(10)?;
                self.values(values, next)
            }
            ProtocolValue::Set {
                equivalence,
                elements,
            } => {
                self.u8(11)?;
                self.u128(*equivalence)?;
                self.values(elements, next)
            }
            ProtocolValue::Bag {
                equivalence,
                entries,
            } => {
                self.u8(12)?;
                self.u128(*equivalence)?;
                self.collection_len(entries.len())?;
                for (value, count) in entries {
                    self.value(value, next)?;
                    self.u64(*count)?;
                }
                Ok(())
            }
            ProtocolValue::Map {
                key_equivalence,
                entries,
            } => {
                self.u8(13)?;
                self.u128(*key_equivalence)?;
                self.collection_len(entries.len())?;
                for (key, value) in entries {
                    self.value(key, next)?;
                    self.value(value, next)?;
                }
                Ok(())
            }
        }
    }

    fn entity_ref(&mut self, value: EntityRef) -> Result<()> {
        self.u128(value.entity_type)?;
        self.u128(value.id)
    }

    fn values(&mut self, values: &[ProtocolValue], depth: u16) -> Result<()> {
        self.collection_len(values.len())?;
        for value in values {
            self.value(value, depth)?;
        }
        Ok(())
    }

    fn row(&mut self, row: &Row, depth: u16) -> Result<()> {
        self.values(row, depth)
    }

    fn rows(&mut self, rows: &[Row], depth: u16) -> Result<()> {
        self.collection_len(rows.len())?;
        for row in rows {
            self.row(row, depth)?;
        }
        Ok(())
    }

    fn query(&mut self, query: &ProtocolQuery, depth: u16) -> Result<()> {
        self.enter(depth)?;
        let next = depth.saturating_add(1);
        match query {
            ProtocolQuery::Scan { relation } => {
                self.u8(0)?;
                self.u128(*relation)
            }
            ProtocolQuery::FilterEq {
                input,
                column,
                value,
                equivalence,
            } => {
                self.u8(1)?;
                self.query(input, next)?;
                self.usize(*column)?;
                self.value(value, next)?;
                self.u128(*equivalence)
            }
            ProtocolQuery::Project { input, columns } => {
                self.u8(2)?;
                self.query(input, next)?;
                self.collection_len(columns.len())?;
                for column in columns {
                    self.usize(*column)?;
                }
                Ok(())
            }
            _ => self.composite_query(query, next),
        }
    }

    fn composite_query(&mut self, query: &ProtocolQuery, depth: u16) -> Result<()> {
        match query {
            ProtocolQuery::JoinEq {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => {
                self.u8(3)?;
                self.query(left, depth)?;
                self.query(right, depth)?;
                self.usize(*left_column)?;
                self.usize(*right_column)?;
                self.u128(*equivalence)
            }
            ProtocolQuery::Difference { left, right } => {
                self.u8(4)?;
                self.query(left, depth)?;
                self.query(right, depth)
            }
            ProtocolQuery::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => {
                self.u8(5)?;
                self.query(left, depth)?;
                self.query(right, depth)?;
                self.usize(*left_column)?;
                self.usize(*right_column)?;
                self.u128(*equivalence)
            }
            ProtocolQuery::GroupCount {
                input,
                group_column,
                group_equivalence,
                result_equivalence,
            } => {
                self.u8(6)?;
                self.query(input, depth)?;
                self.usize(*group_column)?;
                self.u128(*group_equivalence)?;
                self.u128(*result_equivalence)
            }
            ProtocolQuery::Distinct {
                input,
                column_equivalences,
            } => {
                self.u8(7)?;
                self.query(input, depth)?;
                self.collection_len(column_equivalences.len())?;
                for equivalence in column_equivalences {
                    self.u128(*equivalence)?;
                }
                Ok(())
            }
            ProtocolQuery::TopKWithTies {
                input,
                column,
                ordering,
                direction,
                k,
            } => {
                self.u8(8)?;
                self.query(input, depth)?;
                self.usize(*column)?;
                self.u128(*ordering)?;
                self.u8(match direction {
                    OrderDirection::Ascending => 0,
                    OrderDirection::Descending => 1,
                })?;
                self.usize(*k)
            }
            ProtocolQuery::Scan { .. }
            | ProtocolQuery::FilterEq { .. }
            | ProtocolQuery::Project { .. } => unreachable!("simple query encoded by query"),
        }
    }

    fn query_request(&mut self, request: &QueryRequest, depth: u16) -> Result<()> {
        self.snapshot_target(request.target)?;
        self.query(&request.query, depth)
    }

    fn query_response(&mut self, response: &QueryResponse, depth: u16) -> Result<()> {
        self.u64(response.revision)?;
        self.rows(&response.rows, depth)?;
        self.bool(response.is_set)?;
        self.collection_len(response.column_equivalences.len())?;
        for equivalence in &response.column_equivalences {
            self.u128(*equivalence)?;
        }
        Ok(())
    }

    fn commit_request(&mut self, request: &CommitRequest, depth: u16) -> Result<()> {
        self.u64(request.base_revision)?;
        self.u128(request.transaction)?;
        self.collection_len(request.mutations.len())?;
        for mutation in &request.mutations {
            self.u128(mutation.relation)?;
            self.rows(&mutation.inserted, depth)?;
            self.rows(&mutation.removed, depth)?;
        }
        Ok(())
    }

    fn commit_response(&mut self, response: CommitResponse) -> Result<()> {
        match response {
            CommitResponse::Committed { revision } => {
                self.u8(0)?;
                self.u64(revision)
            }
            CommitResponse::AlreadyCommitted { revision } => {
                self.u8(1)?;
                self.u64(revision)
            }
        }
    }

    fn hosted_request(&mut self, request: &HostedRequest, depth: u16) -> Result<()> {
        self.enter(depth)?;
        let next = depth.saturating_add(1);
        match request {
            HostedRequest::CurrentRevision => self.u8(0),
            HostedRequest::Query(request) => {
                self.u8(1)?;
                self.query_request(request, next)
            }
            HostedRequest::History { target } => {
                self.u8(2)?;
                self.snapshot_target(*target)
            }
            HostedRequest::Commit(request) => {
                self.u8(3)?;
                self.commit_request(request, next)
            }
            HostedRequest::OpenWatch(request) => {
                self.u8(4)?;
                self.query(&request.query, next)
            }
            HostedRequest::NextWatch { subscription } => {
                self.u8(5)?;
                self.u64(subscription.raw())
            }
            HostedRequest::WatchStatus { subscription } => {
                self.u8(6)?;
                self.u64(subscription.raw())
            }
            HostedRequest::CancelWatch { subscription } => {
                self.u8(7)?;
                self.u64(subscription.raw())
            }
            HostedRequest::CloseWatch { subscription } => {
                self.u8(8)?;
                self.u64(subscription.raw())
            }
            HostedRequest::CloseSession => self.u8(9),
        }
    }

    fn hosted_response(&mut self, response: &HostedResponse, depth: u16) -> Result<()> {
        self.enter(depth)?;
        let next = depth.saturating_add(1);
        match response {
            HostedResponse::CurrentRevision { revision } => {
                self.u8(0)?;
                self.u64(*revision)
            }
            HostedResponse::Query(response) => {
                self.u8(1)?;
                self.query_response(response, next)
            }
            HostedResponse::History {
                anchor_revision,
                entries,
            } => {
                self.u8(2)?;
                self.u64(*anchor_revision)?;
                self.collection_len(entries.len())?;
                for entry in entries {
                    self.history_entry(entry, next)?;
                }
                Ok(())
            }
            HostedResponse::Commit(response) => {
                self.u8(3)?;
                self.commit_response(*response)
            }
            HostedResponse::WatchOpened(response) => {
                self.u8(4)?;
                self.u64(response.subscription.raw())?;
                self.query_response(&response.initial, next)
            }
            HostedResponse::WatchEvent(response) => {
                self.u8(5)?;
                self.watch_event(response, next)
            }
            HostedResponse::WatchStatus {
                subscription,
                status,
            } => {
                self.u8(6)?;
                self.u64(subscription.raw())?;
                self.watch_status(*status)
            }
            HostedResponse::WatchCancelled { subscription } => {
                self.u8(7)?;
                self.u64(subscription.raw())
            }
            HostedResponse::WatchClosed { subscription } => {
                self.u8(8)?;
                self.u64(subscription.raw())
            }
            HostedResponse::SessionClosed => self.u8(9),
        }
    }

    fn history_entry(&mut self, entry: &HistoryEntryDto, depth: u16) -> Result<()> {
        self.u128(entry.effect_id)?;
        self.collection_len(entry.prerequisites.len())?;
        for prerequisite in &entry.prerequisites {
            self.u128(*prerequisite)?;
        }
        self.u128(entry.transaction)?;
        self.u64(entry.source_revision)?;
        self.u64(entry.target_revision)?;
        self.u8(history_kind_tag(entry.kind))?;
        self.u8(history_reversibility_tag(entry.reversibility))?;
        self.collection_len(entry.changes.len())?;
        for change in &entry.changes {
            self.u128(change.relation)?;
            self.rows(&change.inserted, depth)?;
            self.rows(&change.removed, depth)?;
        }
        Ok(())
    }

    fn watch_event(&mut self, event: &WatchEventDto, depth: u16) -> Result<()> {
        self.u64(event.subscription.raw())?;
        self.u64(event.source_revision)?;
        self.u64(event.target_revision)?;
        self.rows(&event.inserted, depth)?;
        self.rows(&event.removed, depth)
    }

    fn watch_status(&mut self, status: WatchStatusDto) -> Result<()> {
        match status {
            WatchStatusDto::Current { revision } => {
                self.u8(0)?;
                self.u64(revision)
            }
            WatchStatusDto::Lagging {
                anchor_revision,
                head_revision,
                pending_transitions,
            } => {
                self.u8(1)?;
                self.u64(anchor_revision)?;
                self.u64(head_revision)?;
                self.usize(pending_transitions)
            }
            WatchStatusDto::Cancelled { revision } => {
                self.u8(2)?;
                self.u64(revision)
            }
            WatchStatusDto::RuntimeClosed { revision } => {
                self.u8(3)?;
                self.u64(revision)
            }
            WatchStatusDto::Unavailable {
                anchor_revision,
                head_revision,
            } => {
                self.u8(4)?;
                self.u64(anchor_revision)?;
                self.u64(head_revision)
            }
        }
    }

    fn error_code(&mut self, code: ProtocolErrorCode) -> Result<()> {
        self.u8(error_code_tag(code))
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    position: usize,
    limits: WireLimits,
    nodes: u32,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8], limits: WireLimits) -> Self {
        Self {
            bytes,
            position: 0,
            limits,
            nodes: 0,
        }
    }

    fn finish(&self) -> Result<()> {
        if self.position != self.bytes.len() {
            return Err(wire_error("wire payload contains trailing bytes"));
        }
        Ok(())
    }

    fn enter(&mut self, depth: u16) -> Result<()> {
        if depth > self.limits.max_decode_depth {
            return Err(resource_limit("wire value exceeds depth limit"));
        }
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_decode_nodes {
            return Err(resource_limit("wire value exceeds node limit"));
        }
        Ok(())
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(len)
            .ok_or_else(|| wire_error("wire payload offset overflow"))?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| wire_error("truncated wire payload"))?;
        self.position = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(wire_error("non-canonical wire boolean")),
        }
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| wire_error("invalid u32"))?,
        ))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| wire_error("invalid u64"))?,
        ))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| wire_error("invalid i64"))?,
        ))
    }

    fn u128(&mut self) -> Result<u128> {
        Ok(u128::from_be_bytes(
            self.take(16)?
                .try_into()
                .map_err(|_| wire_error("invalid u128"))?,
        ))
    }

    fn usize(&mut self) -> Result<usize> {
        usize::try_from(self.u32()?).map_err(|_| wire_error("wire integer does not fit usize"))
    }

    fn collection_len(&mut self) -> Result<usize> {
        let len = self.u32()?;
        if len > self.limits.max_collection_items {
            return Err(resource_limit("wire collection exceeds configured limit"));
        }
        usize::try_from(len).map_err(|_| wire_error("wire collection length does not fit usize"))
    }

    fn string(&mut self) -> Result<String> {
        let len = self.u32()?;
        if len > self.limits.max_string_bytes {
            return Err(resource_limit("wire string exceeds configured limit"));
        }
        let len = usize::try_from(len)
            .map_err(|_| wire_error("wire string length does not fit usize"))?;
        let bytes = self.take(len)?;
        let value =
            std::str::from_utf8(bytes).map_err(|_| wire_error("wire string is not valid UTF-8"))?;
        Ok(value.to_owned())
    }

    fn snapshot_target(&mut self) -> Result<SnapshotTarget> {
        match self.u8()? {
            0 => Ok(SnapshotTarget::Head),
            1 => Ok(SnapshotTarget::Revision(self.u64()?)),
            _ => Err(wire_error("unknown snapshot target")),
        }
    }

    fn value(&mut self, depth: u16) -> Result<ProtocolValue> {
        self.enter(depth)?;
        let next = depth.saturating_add(1);
        match self.u8()? {
            0 => Ok(ProtocolValue::Unit),
            1 => Ok(ProtocolValue::Bool(self.bool()?)),
            2 => Ok(ProtocolValue::I64(self.i64()?)),
            3 => Ok(ProtocolValue::F64Bits(self.u64()?)),
            4 => Ok(ProtocolValue::Text(self.string()?)),
            5 => Ok(ProtocolValue::LiveEntityRef(self.entity_ref()?)),
            6 => Ok(ProtocolValue::HistoricalEntityRef(self.entity_ref()?)),
            7 => {
                let len = self.collection_len()?;
                let mut fields = BTreeMap::new();
                for _ in 0..len {
                    let field = self.u128()?;
                    let value = self.value(next)?;
                    if fields.insert(field, value).is_some() {
                        return Err(wire_error("product contains duplicate field id"));
                    }
                }
                Ok(ProtocolValue::Product(fields))
            }
            8 => match self.u8()? {
                0 => Ok(ProtocolValue::Option(None)),
                1 => Ok(ProtocolValue::Option(Some(Box::new(self.value(next)?)))),
                _ => Err(wire_error("non-canonical option discriminant")),
            },
            9 => Ok(ProtocolValue::Variant {
                tag: self.u128()?,
                value: Box::new(self.value(next)?),
            }),
            10 => Ok(ProtocolValue::Seq(self.values(next)?)),
            11 => Ok(ProtocolValue::Set {
                equivalence: self.u128()?,
                elements: self.values(next)?,
            }),
            12 => {
                let equivalence = self.u128()?;
                let len = self.collection_len()?;
                let mut entries = Vec::with_capacity(len);
                for _ in 0..len {
                    entries.push((self.value(next)?, self.u64()?));
                }
                Ok(ProtocolValue::Bag {
                    equivalence,
                    entries,
                })
            }
            13 => {
                let key_equivalence = self.u128()?;
                let len = self.collection_len()?;
                let mut entries = Vec::with_capacity(len);
                for _ in 0..len {
                    entries.push((self.value(next)?, self.value(next)?));
                }
                Ok(ProtocolValue::Map {
                    key_equivalence,
                    entries,
                })
            }
            _ => Err(wire_error("unknown protocol value variant")),
        }
    }

    fn entity_ref(&mut self) -> Result<EntityRef> {
        Ok(EntityRef {
            entity_type: self.u128()?,
            id: self.u128()?,
        })
    }

    fn values(&mut self, depth: u16) -> Result<Vec<ProtocolValue>> {
        let len = self.collection_len()?;
        let mut values = Vec::with_capacity(len);
        for _ in 0..len {
            values.push(self.value(depth)?);
        }
        Ok(values)
    }

    fn row(&mut self, depth: u16) -> Result<Row> {
        self.values(depth)
    }

    fn rows(&mut self, depth: u16) -> Result<Vec<Row>> {
        let len = self.collection_len()?;
        let mut rows = Vec::with_capacity(len);
        for _ in 0..len {
            rows.push(self.row(depth)?);
        }
        Ok(rows)
    }

    fn query(&mut self, depth: u16) -> Result<ProtocolQuery> {
        self.enter(depth)?;
        let next = depth.saturating_add(1);
        match self.u8()? {
            0 => Ok(ProtocolQuery::Scan {
                relation: self.u128()?,
            }),
            1 => Ok(ProtocolQuery::FilterEq {
                input: Box::new(self.query(next)?),
                column: self.usize()?,
                value: self.value(next)?,
                equivalence: self.u128()?,
            }),
            2 => {
                let input = Box::new(self.query(next)?);
                let len = self.collection_len()?;
                let mut columns = Vec::with_capacity(len);
                for _ in 0..len {
                    columns.push(self.usize()?);
                }
                Ok(ProtocolQuery::Project { input, columns })
            }
            3 => Ok(ProtocolQuery::JoinEq {
                left: Box::new(self.query(next)?),
                right: Box::new(self.query(next)?),
                left_column: self.usize()?,
                right_column: self.usize()?,
                equivalence: self.u128()?,
            }),
            4 => Ok(ProtocolQuery::Difference {
                left: Box::new(self.query(next)?),
                right: Box::new(self.query(next)?),
            }),
            5 => Ok(ProtocolQuery::AntiJoin {
                left: Box::new(self.query(next)?),
                right: Box::new(self.query(next)?),
                left_column: self.usize()?,
                right_column: self.usize()?,
                equivalence: self.u128()?,
            }),
            6 => Ok(ProtocolQuery::GroupCount {
                input: Box::new(self.query(next)?),
                group_column: self.usize()?,
                group_equivalence: self.u128()?,
                result_equivalence: self.u128()?,
            }),
            7 => {
                let input = Box::new(self.query(next)?);
                let len = self.collection_len()?;
                let mut column_equivalences = Vec::with_capacity(len);
                for _ in 0..len {
                    column_equivalences.push(self.u128()?);
                }
                Ok(ProtocolQuery::Distinct {
                    input,
                    column_equivalences,
                })
            }
            8 => {
                let input = Box::new(self.query(next)?);
                let column = self.usize()?;
                let ordering = self.u128()?;
                let direction = match self.u8()? {
                    0 => OrderDirection::Ascending,
                    1 => OrderDirection::Descending,
                    _ => return Err(wire_error("unknown order direction")),
                };
                Ok(ProtocolQuery::TopKWithTies {
                    input,
                    column,
                    ordering,
                    direction,
                    k: self.usize()?,
                })
            }
            _ => Err(wire_error("unknown protocol query variant")),
        }
    }

    fn query_request(&mut self, depth: u16) -> Result<QueryRequest> {
        Ok(QueryRequest {
            target: self.snapshot_target()?,
            query: self.query(depth)?,
        })
    }

    fn query_response(&mut self, depth: u16) -> Result<QueryResponse> {
        let revision = self.u64()?;
        let rows = self.rows(depth)?;
        let is_set = self.bool()?;
        let len = self.collection_len()?;
        let mut column_equivalences = Vec::with_capacity(len);
        for _ in 0..len {
            column_equivalences.push(self.u128()?);
        }
        Ok(QueryResponse {
            revision,
            rows,
            is_set,
            column_equivalences,
        })
    }

    fn commit_request(&mut self, depth: u16) -> Result<CommitRequest> {
        let base_revision = self.u64()?;
        let transaction = self.u128()?;
        let len = self.collection_len()?;
        let mut mutations = Vec::with_capacity(len);
        for _ in 0..len {
            mutations.push(RelationMutation {
                relation: self.u128()?,
                inserted: self.rows(depth)?,
                removed: self.rows(depth)?,
            });
        }
        Ok(CommitRequest {
            base_revision,
            transaction,
            mutations,
        })
    }

    fn commit_response(&mut self) -> Result<CommitResponse> {
        match self.u8()? {
            0 => Ok(CommitResponse::Committed {
                revision: self.u64()?,
            }),
            1 => Ok(CommitResponse::AlreadyCommitted {
                revision: self.u64()?,
            }),
            _ => Err(wire_error("unknown commit response variant")),
        }
    }

    fn hosted_request(&mut self, depth: u16) -> Result<HostedRequest> {
        self.enter(depth)?;
        let next = depth.saturating_add(1);
        match self.u8()? {
            0 => Ok(HostedRequest::CurrentRevision),
            1 => Ok(HostedRequest::Query(self.query_request(next)?)),
            2 => Ok(HostedRequest::History {
                target: self.snapshot_target()?,
            }),
            3 => Ok(HostedRequest::Commit(self.commit_request(next)?)),
            4 => Ok(HostedRequest::OpenWatch(OpenWatchRequest {
                query: self.query(next)?,
            })),
            5 => Ok(HostedRequest::NextWatch {
                subscription: SubscriptionId::new(self.u64()?),
            }),
            6 => Ok(HostedRequest::WatchStatus {
                subscription: SubscriptionId::new(self.u64()?),
            }),
            7 => Ok(HostedRequest::CancelWatch {
                subscription: SubscriptionId::new(self.u64()?),
            }),
            8 => Ok(HostedRequest::CloseWatch {
                subscription: SubscriptionId::new(self.u64()?),
            }),
            9 => Ok(HostedRequest::CloseSession),
            _ => Err(wire_error("unknown hosted request variant")),
        }
    }

    fn hosted_response(&mut self, depth: u16) -> Result<HostedResponse> {
        self.enter(depth)?;
        let next = depth.saturating_add(1);
        match self.u8()? {
            0 => Ok(HostedResponse::CurrentRevision {
                revision: self.u64()?,
            }),
            1 => Ok(HostedResponse::Query(self.query_response(next)?)),
            2 => {
                let anchor_revision = self.u64()?;
                let len = self.collection_len()?;
                let mut entries = Vec::with_capacity(len);
                for _ in 0..len {
                    entries.push(self.history_entry(next)?);
                }
                Ok(HostedResponse::History {
                    anchor_revision,
                    entries,
                })
            }
            3 => Ok(HostedResponse::Commit(self.commit_response()?)),
            4 => Ok(HostedResponse::WatchOpened(OpenWatchResponse {
                subscription: SubscriptionId::new(self.u64()?),
                initial: self.query_response(next)?,
            })),
            5 => Ok(HostedResponse::WatchEvent(self.watch_event(next)?)),
            6 => Ok(HostedResponse::WatchStatus {
                subscription: SubscriptionId::new(self.u64()?),
                status: self.watch_status()?,
            }),
            7 => Ok(HostedResponse::WatchCancelled {
                subscription: SubscriptionId::new(self.u64()?),
            }),
            8 => Ok(HostedResponse::WatchClosed {
                subscription: SubscriptionId::new(self.u64()?),
            }),
            9 => Ok(HostedResponse::SessionClosed),
            _ => Err(wire_error("unknown hosted response variant")),
        }
    }

    fn history_entry(&mut self, depth: u16) -> Result<HistoryEntryDto> {
        let effect_id = self.u128()?;
        let prerequisite_len = self.collection_len()?;
        let mut prerequisites = Vec::with_capacity(prerequisite_len);
        for _ in 0..prerequisite_len {
            prerequisites.push(self.u128()?);
        }
        let transaction = self.u128()?;
        let source_revision = self.u64()?;
        let target_revision = self.u64()?;
        let kind = history_kind_from_tag(self.u8()?)?;
        let reversibility = history_reversibility_from_tag(self.u8()?)?;
        let change_len = self.collection_len()?;
        let mut changes = Vec::with_capacity(change_len);
        for _ in 0..change_len {
            changes.push(HistoryRelationChangeDto {
                relation: self.u128()?,
                inserted: self.rows(depth)?,
                removed: self.rows(depth)?,
            });
        }
        Ok(HistoryEntryDto {
            effect_id,
            prerequisites,
            transaction,
            source_revision,
            target_revision,
            kind,
            reversibility,
            changes,
        })
    }

    fn watch_event(&mut self, depth: u16) -> Result<WatchEventDto> {
        Ok(WatchEventDto {
            subscription: SubscriptionId::new(self.u64()?),
            source_revision: self.u64()?,
            target_revision: self.u64()?,
            inserted: self.rows(depth)?,
            removed: self.rows(depth)?,
        })
    }

    fn watch_status(&mut self) -> Result<WatchStatusDto> {
        match self.u8()? {
            0 => Ok(WatchStatusDto::Current {
                revision: self.u64()?,
            }),
            1 => Ok(WatchStatusDto::Lagging {
                anchor_revision: self.u64()?,
                head_revision: self.u64()?,
                pending_transitions: self.usize()?,
            }),
            2 => Ok(WatchStatusDto::Cancelled {
                revision: self.u64()?,
            }),
            3 => Ok(WatchStatusDto::RuntimeClosed {
                revision: self.u64()?,
            }),
            4 => Ok(WatchStatusDto::Unavailable {
                anchor_revision: self.u64()?,
                head_revision: self.u64()?,
            }),
            _ => Err(wire_error("unknown watch status variant")),
        }
    }

    fn error_code(&mut self) -> Result<ProtocolErrorCode> {
        error_code_from_tag(self.u8()?)
    }
}

const fn history_kind_tag(kind: HistoryKind) -> u8 {
    match kind {
        HistoryKind::RelationData => 0,
        HistoryKind::RelationRewrite => 1,
        HistoryKind::RelationResolution => 2,
        HistoryKind::MixedRevision => 3,
        HistoryKind::FullRevision => 4,
        HistoryKind::SchemaMigration => 5,
        HistoryKind::LegacyTargetOnly => 6,
    }
}

fn history_kind_from_tag(tag: u8) -> Result<HistoryKind> {
    match tag {
        0 => Ok(HistoryKind::RelationData),
        1 => Ok(HistoryKind::RelationRewrite),
        2 => Ok(HistoryKind::RelationResolution),
        3 => Ok(HistoryKind::MixedRevision),
        4 => Ok(HistoryKind::FullRevision),
        5 => Ok(HistoryKind::SchemaMigration),
        6 => Ok(HistoryKind::LegacyTargetOnly),
        _ => Err(wire_error("unknown history kind")),
    }
}

const fn history_reversibility_tag(value: HistoryReversibilityDto) -> u8 {
    match value {
        HistoryReversibilityDto::ExactPlanInverse => 0,
        HistoryReversibilityDto::ComplementRequired => 1,
        HistoryReversibilityDto::NonPlanTransition => 2,
    }
}

fn history_reversibility_from_tag(tag: u8) -> Result<HistoryReversibilityDto> {
    match tag {
        0 => Ok(HistoryReversibilityDto::ExactPlanInverse),
        1 => Ok(HistoryReversibilityDto::ComplementRequired),
        2 => Ok(HistoryReversibilityDto::NonPlanTransition),
        _ => Err(wire_error("unknown history reversibility")),
    }
}

const fn error_code_tag(code: ProtocolErrorCode) -> u8 {
    match code {
        ProtocolErrorCode::Recovery => 0,
        ProtocolErrorCode::Query => 1,
        ProtocolErrorCode::InvalidRequest => 2,
        ProtocolErrorCode::InvalidSchema => 3,
        ProtocolErrorCode::TypeMismatch => 4,
        ProtocolErrorCode::Cardinality => 5,
        ProtocolErrorCode::NotFound => 6,
        ProtocolErrorCode::StaleRevision => 7,
        ProtocolErrorCode::TransactionConflict => 8,
        ProtocolErrorCode::HistoryConflict => 9,
        ProtocolErrorCode::InvariantViolation => 10,
        ProtocolErrorCode::NonReversibleHistory => 11,
        ProtocolErrorCode::WatchUnavailable => 12,
        ProtocolErrorCode::WatchClosed => 13,
        ProtocolErrorCode::ResourceLimit => 14,
        ProtocolErrorCode::PermissionDenied => 15,
        ProtocolErrorCode::SessionClosed => 16,
        ProtocolErrorCode::Internal => 17,
    }
}

fn error_code_from_tag(tag: u8) -> Result<ProtocolErrorCode> {
    match tag {
        0 => Ok(ProtocolErrorCode::Recovery),
        1 => Ok(ProtocolErrorCode::Query),
        2 => Ok(ProtocolErrorCode::InvalidRequest),
        3 => Ok(ProtocolErrorCode::InvalidSchema),
        4 => Ok(ProtocolErrorCode::TypeMismatch),
        5 => Ok(ProtocolErrorCode::Cardinality),
        6 => Ok(ProtocolErrorCode::NotFound),
        7 => Ok(ProtocolErrorCode::StaleRevision),
        8 => Ok(ProtocolErrorCode::TransactionConflict),
        9 => Ok(ProtocolErrorCode::HistoryConflict),
        10 => Ok(ProtocolErrorCode::InvariantViolation),
        11 => Ok(ProtocolErrorCode::NonReversibleHistory),
        12 => Ok(ProtocolErrorCode::WatchUnavailable),
        13 => Ok(ProtocolErrorCode::WatchClosed),
        14 => Ok(ProtocolErrorCode::ResourceLimit),
        15 => Ok(ProtocolErrorCode::PermissionDenied),
        16 => Ok(ProtocolErrorCode::SessionClosed),
        17 => Ok(ProtocolErrorCode::Internal),
        _ => Err(wire_error("unknown protocol error code")),
    }
}
