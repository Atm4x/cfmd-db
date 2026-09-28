//! First-party local IPC transport adapter for hosted CFMD.
//!
//! The transport owns local endpoint I/O, bounded authentication evidence and
//! wire-frame delivery. Authentication, authorization and database semantics
//! remain owned by `cfmd-host`, `cfmd-protocol` and `cfmd-runtime`.

use std::{error::Error as StdError, fmt, io, time::Duration};

use cfmd_host::{HostError, HostErrorCode};
use cfmd_protocol::{ProtocolError, ProtocolErrorCode};

pub const LOCAL_AUTH_MAGIC: [u8; 4] = *b"CFMA";
pub const LOCAL_AUTH_VERSION: u16 = 1;
pub const LOCAL_AUTH_HEADER_LEN: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalIpcLimits {
    pub max_auth_evidence_bytes: u32,
    pub max_workers_per_connection: usize,
    pub max_pending_responses_per_connection: usize,
    pub authentication_read_timeout: Duration,
}

impl Default for LocalIpcLimits {
    fn default() -> Self {
        Self {
            max_auth_evidence_bytes: 64 * 1024,
            max_workers_per_connection: 16,
            max_pending_responses_per_connection: 32,
            authentication_read_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LocalIpcErrorCode {
    Io,
    InvalidAuthenticationPrelude,
    AuthenticationEvidenceTooLarge,
    AccessDenied,
    HostUnavailable,
    Protocol,
    ResourceLimit,
}

#[derive(Debug)]
pub struct LocalIpcError {
    code: LocalIpcErrorCode,
    message: &'static str,
    source: Option<Box<dyn StdError + Send + Sync>>,
}

impl LocalIpcError {
    #[must_use]
    pub const fn code(&self) -> LocalIpcErrorCode {
        self.code
    }

    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }

    fn new(code: LocalIpcErrorCode, message: &'static str) -> Self {
        Self {
            code,
            message,
            source: None,
        }
    }

    fn with_source<E>(code: LocalIpcErrorCode, message: &'static str, source: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self {
            code,
            message,
            source: Some(Box::new(source)),
        }
    }
}

impl fmt::Display for LocalIpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl StdError for LocalIpcError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}

impl From<io::Error> for LocalIpcError {
    fn from(error: io::Error) -> Self {
        Self::with_source(LocalIpcErrorCode::Io, "local IPC I/O failure", error)
    }
}

impl From<HostError> for LocalIpcError {
    fn from(error: HostError) -> Self {
        let code = match error.code() {
            HostErrorCode::AccessDenied => LocalIpcErrorCode::AccessDenied,
            HostErrorCode::ConnectionLimit => LocalIpcErrorCode::ResourceLimit,
            _ => LocalIpcErrorCode::HostUnavailable,
        };
        Self::with_source(code, "hosted CFMD connection rejected", error)
    }
}

impl From<ProtocolError> for LocalIpcError {
    fn from(error: ProtocolError) -> Self {
        let code = if error.code() == ProtocolErrorCode::ResourceLimit {
            LocalIpcErrorCode::ResourceLimit
        } else {
            LocalIpcErrorCode::Protocol
        };
        Self::with_source(code, "local IPC wire protocol failure", error)
    }
}

pub type Result<T> = std::result::Result<T, LocalIpcError>;

#[cfg(unix)]
pub mod unix;

fn invalid_auth_prelude(message: &'static str) -> LocalIpcError {
    LocalIpcError::new(LocalIpcErrorCode::InvalidAuthenticationPrelude, message)
}

fn auth_evidence_too_large() -> LocalIpcError {
    LocalIpcError::new(
        LocalIpcErrorCode::AuthenticationEvidenceTooLarge,
        "local IPC authentication evidence exceeds configured limit",
    )
}

fn resource_limit(message: &'static str) -> LocalIpcError {
    LocalIpcError::new(LocalIpcErrorCode::ResourceLimit, message)
}
