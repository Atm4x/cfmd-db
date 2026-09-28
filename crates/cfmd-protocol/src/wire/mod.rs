//! Canonical transport-neutral wire framing for the hosted CFMD protocol.
//!
//! The wire layer owns framing, version negotiation and bounded binary
//! encoding. It owns no socket, listener, TLS implementation or authentication
//! mechanism.

mod codec;

use std::sync::atomic::{AtomicU16, Ordering};

use crate::{
    HostedRequest, HostedResponse, HostedSession, PROTOCOL_VERSION, ProtocolError,
    ProtocolErrorCode, Result,
};

pub use codec::{
    decode_request_payload, decode_response_payload, encode_request_payload,
    encode_response_payload,
};

pub const WIRE_MAGIC: [u8; 4] = *b"CFMD";
pub const WIRE_VERSION: u16 = 1;
pub const WIRE_HEADER_LEN: usize = 20;

pub const CAP_QUERY: u64 = 1 << 0;
pub const CAP_HISTORY: u64 = 1 << 1;
pub const CAP_COMMIT: u64 = 1 << 2;
pub const CAP_WATCH: u64 = 1 << 3;
pub const SERVER_CAPABILITIES: u64 = CAP_QUERY | CAP_HISTORY | CAP_COMMIT | CAP_WATCH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireLimits {
    pub max_payload_bytes: u32,
    pub max_collection_items: u32,
    pub max_string_bytes: u32,
    pub max_decode_nodes: u32,
    pub max_decode_depth: u16,
}

impl Default for WireLimits {
    fn default() -> Self {
        Self {
            max_payload_bytes: 16 * 1024 * 1024,
            max_collection_items: 100_000,
            max_string_bytes: 1024 * 1024,
            max_decode_nodes: 1_000_000,
            max_decode_depth: 256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WireFrameKind {
    Hello = 1,
    HelloAck = 2,
    Request = 3,
    Response = 4,
}

impl TryFrom<u8> for WireFrameKind {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            1 => Ok(Self::Hello),
            2 => Ok(Self::HelloAck),
            3 => Ok(Self::Request),
            4 => Ok(Self::Response),
            _ => Err(wire_error("unknown wire frame kind")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireHeader {
    pub kind: WireFrameKind,
    pub request_id: u64,
    pub payload_len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolHello {
    pub min_protocol_version: u16,
    pub max_protocol_version: u16,
    pub capabilities: u64,
}

impl ProtocolHello {
    #[must_use]
    pub const fn current() -> Self {
        Self {
            min_protocol_version: PROTOCOL_VERSION,
            max_protocol_version: PROTOCOL_VERSION,
            capabilities: SERVER_CAPABILITIES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolHelloAck {
    pub protocol_version: u16,
    pub capabilities: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireResponse {
    Success(HostedResponse),
    Error {
        code: ProtocolErrorCode,
        message: String,
    },
}

impl WireResponse {
    #[must_use]
    pub fn from_result(result: Result<HostedResponse>) -> Self {
        match result {
            Ok(response) => Self::Success(response),
            Err(error) => Self::Error {
                code: error.code(),
                message: error.message().to_owned(),
            },
        }
    }
}

#[derive(Debug)]
pub struct WireHostedSession {
    hosted: HostedSession,
    limits: WireLimits,
    negotiated_protocol: AtomicU16,
}

impl WireHostedSession {
    #[must_use]
    pub fn new(hosted: HostedSession) -> Self {
        Self::with_limits(hosted, WireLimits::default())
    }

    #[must_use]
    pub const fn with_limits(hosted: HostedSession, limits: WireLimits) -> Self {
        Self {
            hosted,
            limits,
            negotiated_protocol: AtomicU16::new(0),
        }
    }

    #[must_use]
    pub const fn limits(&self) -> WireLimits {
        self.limits
    }

    pub fn cancel_all_watches(&self) -> Result<()> {
        self.hosted.cancel_all_watches()
    }

    pub fn close(&self) -> Result<()> {
        self.hosted.close()
    }

    pub fn is_closed(&self) -> Result<bool> {
        self.hosted.is_closed()
    }

    #[must_use]
    pub fn negotiated_protocol(&self) -> Option<u16> {
        match self.negotiated_protocol.load(Ordering::Acquire) {
            0 => None,
            version => Some(version),
        }
    }

    pub fn handle_frame(&self, frame: &[u8]) -> Result<Vec<u8>> {
        let header = decode_header(frame, self.limits)?;
        let payload = frame_payload(frame, header)?;
        match header.kind {
            WireFrameKind::Hello => {
                let hello = decode_hello_payload(payload)?;
                let ack = negotiate(hello)?;
                self.negotiated_protocol
                    .store(ack.protocol_version, Ordering::Release);
                encode_frame(
                    WireFrameKind::HelloAck,
                    header.request_id,
                    &encode_hello_ack_payload(ack),
                    self.limits,
                )
            }
            WireFrameKind::Request => {
                if self.negotiated_protocol().is_none() {
                    return Err(wire_error(
                        "wire session has not negotiated a protocol version",
                    ));
                }
                let request = decode_request_payload(payload, self.limits)?;
                let response = WireResponse::from_result(self.hosted.execute(request));
                let payload = match encode_response_payload(&response, self.limits) {
                    Ok(payload) => payload,
                    Err(error) => encode_response_payload(
                        &WireResponse::Error {
                            code: error.code(),
                            message: error.message().to_owned(),
                        },
                        self.limits,
                    )?,
                };
                encode_frame(
                    WireFrameKind::Response,
                    header.request_id,
                    &payload,
                    self.limits,
                )
            }
            WireFrameKind::HelloAck | WireFrameKind::Response => {
                Err(wire_error("server wire session received a response frame"))
            }
        }
    }
}

pub fn negotiate(hello: ProtocolHello) -> Result<ProtocolHelloAck> {
    if hello.min_protocol_version > hello.max_protocol_version {
        return Err(wire_error("invalid protocol version range"));
    }
    if PROTOCOL_VERSION < hello.min_protocol_version
        || PROTOCOL_VERSION > hello.max_protocol_version
    {
        return Err(ProtocolError::new(
            ProtocolErrorCode::InvalidRequest,
            "no compatible hosted protocol version",
        ));
    }
    Ok(ProtocolHelloAck {
        protocol_version: PROTOCOL_VERSION,
        capabilities: hello.capabilities & SERVER_CAPABILITIES,
    })
}

pub fn encode_hello_frame(
    request_id: u64,
    hello: ProtocolHello,
    limits: WireLimits,
) -> Result<Vec<u8>> {
    encode_frame(
        WireFrameKind::Hello,
        request_id,
        &encode_hello_payload(hello),
        limits,
    )
}

pub fn encode_request_frame(
    request_id: u64,
    request: &HostedRequest,
    limits: WireLimits,
) -> Result<Vec<u8>> {
    let payload = encode_request_payload(request, limits)?;
    encode_frame(WireFrameKind::Request, request_id, &payload, limits)
}

pub fn encode_error_response_frame(
    request_id: u64,
    code: ProtocolErrorCode,
    message: &str,
    limits: WireLimits,
) -> Result<Vec<u8>> {
    let payload = encode_response_payload(
        &WireResponse::Error {
            code,
            message: message.to_owned(),
        },
        limits,
    )?;
    encode_frame(WireFrameKind::Response, request_id, &payload, limits)
}

pub fn decode_response_frame(frame: &[u8], limits: WireLimits) -> Result<(u64, WireResponse)> {
    let header = decode_header(frame, limits)?;
    if header.kind != WireFrameKind::Response {
        return Err(wire_error("expected wire response frame"));
    }
    let response = decode_response_payload(frame_payload(frame, header)?, limits)?;
    Ok((header.request_id, response))
}

pub fn decode_hello_ack_frame(frame: &[u8], limits: WireLimits) -> Result<(u64, ProtocolHelloAck)> {
    let header = decode_header(frame, limits)?;
    if header.kind != WireFrameKind::HelloAck {
        return Err(wire_error("expected hello acknowledgement frame"));
    }
    Ok((
        header.request_id,
        decode_hello_ack_payload(frame_payload(frame, header)?)?,
    ))
}

pub fn decode_header(frame_prefix: &[u8], limits: WireLimits) -> Result<WireHeader> {
    if frame_prefix.len() < WIRE_HEADER_LEN {
        return Err(wire_error("incomplete wire frame header"));
    }
    if frame_prefix[..4] != WIRE_MAGIC {
        return Err(wire_error("invalid wire frame magic"));
    }
    let wire_version = u16::from_be_bytes([frame_prefix[4], frame_prefix[5]]);
    if wire_version != WIRE_VERSION {
        return Err(ProtocolError::new(
            ProtocolErrorCode::InvalidRequest,
            "unsupported wire framing version",
        ));
    }
    if frame_prefix[7] != 0 {
        return Err(wire_error("unsupported wire frame flags"));
    }
    let kind = WireFrameKind::try_from(frame_prefix[6])?;
    let request_id = u64::from_be_bytes(
        frame_prefix[8..16]
            .try_into()
            .map_err(|_| wire_error("invalid request id"))?,
    );
    let payload_len = u32::from_be_bytes(
        frame_prefix[16..20]
            .try_into()
            .map_err(|_| wire_error("invalid payload length"))?,
    );
    if payload_len > limits.max_payload_bytes {
        return Err(ProtocolError::new(
            ProtocolErrorCode::ResourceLimit,
            "wire frame payload exceeds configured limit",
        ));
    }
    Ok(WireHeader {
        kind,
        request_id,
        payload_len,
    })
}

fn encode_frame(
    kind: WireFrameKind,
    request_id: u64,
    payload: &[u8],
    limits: WireLimits,
) -> Result<Vec<u8>> {
    let payload_len =
        u32::try_from(payload.len()).map_err(|_| resource_limit("wire payload is too large"))?;
    if payload_len > limits.max_payload_bytes {
        return Err(resource_limit("wire payload exceeds configured limit"));
    }
    let mut frame = Vec::with_capacity(WIRE_HEADER_LEN + payload.len());
    frame.extend_from_slice(&WIRE_MAGIC);
    frame.extend_from_slice(&WIRE_VERSION.to_be_bytes());
    frame.push(kind as u8);
    frame.push(0);
    frame.extend_from_slice(&request_id.to_be_bytes());
    frame.extend_from_slice(&payload_len.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

fn frame_payload(frame: &[u8], header: WireHeader) -> Result<&[u8]> {
    let payload_len =
        usize::try_from(header.payload_len).map_err(|_| wire_error("invalid payload length"))?;
    let expected = WIRE_HEADER_LEN
        .checked_add(payload_len)
        .ok_or_else(|| wire_error("wire frame length overflow"))?;
    if frame.len() != expected {
        return Err(wire_error("wire frame length does not match header"));
    }
    Ok(&frame[WIRE_HEADER_LEN..])
}

fn encode_hello_payload(hello: ProtocolHello) -> Vec<u8> {
    let mut payload = Vec::with_capacity(12);
    payload.extend_from_slice(&hello.min_protocol_version.to_be_bytes());
    payload.extend_from_slice(&hello.max_protocol_version.to_be_bytes());
    payload.extend_from_slice(&hello.capabilities.to_be_bytes());
    payload
}

fn decode_hello_payload(payload: &[u8]) -> Result<ProtocolHello> {
    if payload.len() != 12 {
        return Err(wire_error("invalid hello payload length"));
    }
    Ok(ProtocolHello {
        min_protocol_version: u16::from_be_bytes([payload[0], payload[1]]),
        max_protocol_version: u16::from_be_bytes([payload[2], payload[3]]),
        capabilities: u64::from_be_bytes(
            payload[4..12]
                .try_into()
                .map_err(|_| wire_error("invalid hello capabilities"))?,
        ),
    })
}

fn encode_hello_ack_payload(ack: ProtocolHelloAck) -> Vec<u8> {
    let mut payload = Vec::with_capacity(10);
    payload.extend_from_slice(&ack.protocol_version.to_be_bytes());
    payload.extend_from_slice(&ack.capabilities.to_be_bytes());
    payload
}

fn decode_hello_ack_payload(payload: &[u8]) -> Result<ProtocolHelloAck> {
    if payload.len() != 10 {
        return Err(wire_error("invalid hello acknowledgement payload length"));
    }
    Ok(ProtocolHelloAck {
        protocol_version: u16::from_be_bytes([payload[0], payload[1]]),
        capabilities: u64::from_be_bytes(
            payload[2..10]
                .try_into()
                .map_err(|_| wire_error("invalid hello acknowledgement capabilities"))?,
        ),
    })
}

pub(crate) fn wire_error(message: &'static str) -> ProtocolError {
    ProtocolError::new(ProtocolErrorCode::InvalidRequest, message)
}

pub(crate) fn resource_limit(message: &'static str) -> ProtocolError {
    ProtocolError::new(ProtocolErrorCode::ResourceLimit, message)
}
