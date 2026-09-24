use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use kernel_auth::{AuthorityDigest, FreshnessCut, KeyId, SignedFreshnessCut};

use crate::{DurabilityError, ExternalFreshnessAuthority};

const MAGIC: [u8; 4] = *b"CFFA";
const VERSION: u16 = 1;
const OP_READ: u8 = 1;
const OP_COMPARE_ADVANCE: u8 = 2;
const STATUS_NONE: u8 = 0;
const STATUS_RECORD: u8 = 1;
const STATUS_CAS_MISMATCH: u8 = 2;
const STATUS_REJECTED: u8 = 3;
const MAX_FRAME_LEN: usize = 1024;
const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Debug, Clone)]
pub struct TcpExternalFreshnessAuthority {
    endpoint: SocketAddr,
    connect_timeout: Duration,
    io_timeout: Duration,
}

impl TcpExternalFreshnessAuthority {
    #[must_use]
    pub const fn new(
        endpoint: SocketAddr,
        connect_timeout: Duration,
        io_timeout: Duration,
    ) -> Self {
        Self {
            endpoint,
            connect_timeout,
            io_timeout,
        }
    }

    fn transact(&self, request: &[u8]) -> Result<Vec<u8>, DurabilityError> {
        let mut stream = TcpStream::connect_timeout(&self.endpoint, self.connect_timeout)?;
        stream.set_read_timeout(Some(self.io_timeout))?;
        stream.set_write_timeout(Some(self.io_timeout))?;
        let request_len = u32::try_from(request.len()).map_err(|_| DurabilityError::Protocol {
            offset: 0,
            reason: "freshness request exceeds wire limit",
        })?;
        stream.write_all(&request_len.to_be_bytes())?;
        stream.write_all(request)?;
        stream.flush()?;

        let mut len = [0_u8; 4];
        stream.read_exact(&mut len)?;
        let response_len =
            usize::try_from(u32::from_be_bytes(len)).map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "freshness response length is not representable",
            })?;
        if response_len > MAX_FRAME_LEN {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "freshness response exceeds wire limit",
            });
        }
        let mut response = vec![0_u8; response_len];
        stream.read_exact(&mut response)?;
        Ok(response)
    }
}

impl ExternalFreshnessAuthority for TcpExternalFreshnessAuthority {
    fn read_signed(
        &mut self,
        store_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessCut>, DurabilityError> {
        let mut request = header(OP_READ);
        request.extend_from_slice(&store_id);
        let response = self.transact(&request)?;
        decode_response(&response)
    }

    fn compare_and_advance_signed(
        &mut self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessCut,
    ) -> Result<SignedFreshnessCut, DurabilityError> {
        let mut request = header(OP_COMPARE_ADVANCE);
        push_optional_digest(&mut request, expected_record);
        encode_cut(&mut request, next);
        let response = self.transact(&request)?;
        decode_response(&response)?.ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "freshness authority returned no record after compare-and-advance",
        })
    }
}

fn header(op: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_be_bytes());
    out.push(op);
    out
}

fn decode_response(bytes: &[u8]) -> Result<Option<SignedFreshnessCut>, DurabilityError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take_array::<4>()? != MAGIC || cursor.take_u16()? != VERSION {
        return Err(protocol(
            "freshness authority returned incompatible wire header",
        ));
    }
    let status = cursor.take_u8()?;
    let result = match status {
        STATUS_NONE => None,
        STATUS_RECORD => Some(*decode_signed_cut(&mut cursor)?),
        STATUS_CAS_MISMATCH => {
            return Err(protocol(
                "freshness authority compare-and-advance lost CAS race",
            ));
        }
        STATUS_REJECTED => return Err(protocol("freshness authority rejected request")),
        _ => return Err(protocol("freshness authority returned unknown status")),
    };
    if cursor.remaining() != 0 {
        return Err(protocol("freshness authority response has trailing bytes"));
    }
    Ok(result)
}

pub(crate) fn decode_request(bytes: &[u8]) -> Result<FreshnessWireRequest, DurabilityError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take_array::<4>()? != MAGIC || cursor.take_u16()? != VERSION {
        return Err(protocol("freshness request has incompatible wire header"));
    }
    let request = match cursor.take_u8()? {
        OP_READ => FreshnessWireRequest::Read {
            store_id: cursor.take_array::<32>()?,
        },
        OP_COMPARE_ADVANCE => FreshnessWireRequest::CompareAndAdvance {
            expected_record: cursor.take_optional_digest()?,
            next: decode_cut(&mut cursor)?,
        },
        _ => return Err(protocol("freshness request has unknown operation")),
    };
    if cursor.remaining() != 0 {
        return Err(protocol("freshness request has trailing bytes"));
    }
    Ok(request)
}

pub(crate) fn encode_response(response: FreshnessWireResponse) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_be_bytes());
    match response {
        FreshnessWireResponse::None => out.push(STATUS_NONE),
        FreshnessWireResponse::Record(record) => {
            out.push(STATUS_RECORD);
            encode_signed_cut(&mut out, &record);
        }
        FreshnessWireResponse::CasMismatch => out.push(STATUS_CAS_MISMATCH),
        FreshnessWireResponse::Rejected => out.push(STATUS_REJECTED),
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FreshnessWireRequest {
    Read {
        store_id: [u8; 32],
    },
    CompareAndAdvance {
        expected_record: Option<AuthorityDigest>,
        next: FreshnessCut,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FreshnessWireResponse {
    None,
    Record(Box<SignedFreshnessCut>),
    CasMismatch,
    Rejected,
}

pub(crate) fn read_wire_frame(stream: &mut impl Read) -> Result<Vec<u8>, DurabilityError> {
    let mut len = [0_u8; 4];
    stream.read_exact(&mut len)?;
    let len = usize::try_from(u32::from_be_bytes(len))
        .map_err(|_| protocol("freshness wire frame length is not representable"))?;
    if len > MAX_FRAME_LEN {
        return Err(protocol("freshness wire frame exceeds limit"));
    }
    let mut bytes = vec![0_u8; len];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn write_wire_frame(
    stream: &mut impl Write,
    bytes: &[u8],
) -> Result<(), DurabilityError> {
    let len = u32::try_from(bytes.len())
        .map_err(|_| protocol("freshness wire frame exceeds length encoding"))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}

fn encode_signed_cut(out: &mut Vec<u8>, record: &SignedFreshnessCut) {
    encode_cut(out, record.cut);
    out.extend_from_slice(&record.signer.0);
    out.extend_from_slice(&record.signature);
}

fn decode_signed_cut(cursor: &mut Cursor<'_>) -> Result<Box<SignedFreshnessCut>, DurabilityError> {
    Ok(Box::new(SignedFreshnessCut {
        cut: decode_cut(cursor)?,
        signer: KeyId(cursor.take_array::<32>()?),
        signature: cursor.take_array::<64>()?,
    }))
}

fn encode_cut(out: &mut Vec<u8>, cut: FreshnessCut) {
    out.extend_from_slice(&cut.store_id);
    out.extend_from_slice(&cut.generation.to_be_bytes());
    push_optional_digest(out, cut.previous_generation);
    out.extend_from_slice(&cut.generation_digest.0);
    out.extend_from_slice(&cut.wal_lsn.to_be_bytes());
    out.extend_from_slice(&cut.wal_digest.0);
    out.extend_from_slice(&cut.trust_root_epoch.to_be_bytes());
    out.extend_from_slice(&cut.deployment_policy_epoch.to_be_bytes());
}

fn decode_cut(cursor: &mut Cursor<'_>) -> Result<FreshnessCut, DurabilityError> {
    Ok(FreshnessCut {
        store_id: cursor.take_array::<32>()?,
        generation: cursor.take_u64()?,
        previous_generation: cursor.take_optional_digest()?,
        generation_digest: AuthorityDigest(cursor.take_array::<32>()?),
        wal_lsn: cursor.take_u64()?,
        wal_digest: AuthorityDigest(cursor.take_array::<32>()?),
        trust_root_epoch: cursor.take_u64()?,
        deployment_policy_epoch: cursor.take_u64()?,
    })
}

fn push_optional_digest(out: &mut Vec<u8>, digest: Option<AuthorityDigest>) {
    match digest {
        Some(digest) => {
            out.push(1);
            out.extend_from_slice(&digest.0);
        }
        None => out.push(0),
    }
}

fn protocol(reason: &'static str) -> DurabilityError {
    DurabilityError::Protocol { offset: 0, reason }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn take_u8(&mut self) -> Result<u8, DurabilityError> {
        Ok(self.take_array::<1>()?[0])
    }

    fn take_u16(&mut self) -> Result<u16, DurabilityError> {
        Ok(u16::from_be_bytes(self.take_array::<2>()?))
    }

    fn take_u64(&mut self) -> Result<u64, DurabilityError> {
        Ok(u64::from_be_bytes(self.take_array::<8>()?))
    }

    fn take_optional_digest(&mut self) -> Result<Option<AuthorityDigest>, DurabilityError> {
        match self.take_u8()? {
            0 => Ok(None),
            1 => Ok(Some(AuthorityDigest(self.take_array::<32>()?))),
            _ => Err(protocol("freshness wire optional digest tag is invalid")),
        }
    }

    fn take_array<const N: usize>(&mut self) -> Result<[u8; N], DurabilityError> {
        let end = self
            .offset
            .checked_add(N)
            .ok_or_else(|| protocol("freshness wire offset overflow"))?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| protocol("freshness wire frame is truncated"))?;
        self.offset = end;
        let mut out = [0_u8; N];
        out.copy_from_slice(slice);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_roundtrip_preserves_signed_cut() {
        let cut = FreshnessCut {
            store_id: [1; 32],
            generation: 7,
            previous_generation: Some(AuthorityDigest([2; 32])),
            generation_digest: AuthorityDigest([3; 32]),
            wal_lsn: 11,
            wal_digest: AuthorityDigest([4; 32]),
            trust_root_epoch: 5,
            deployment_policy_epoch: 6,
        };
        let record = SignedFreshnessCut {
            cut,
            signer: KeyId([7; 32]),
            signature: [8; 64],
        };
        let encoded = encode_response(FreshnessWireResponse::Record(Box::new(record.clone())));
        assert_eq!(decode_response(&encoded).unwrap(), Some(record));

        let mut request = header(OP_COMPARE_ADVANCE);
        push_optional_digest(&mut request, Some(AuthorityDigest([9; 32])));
        encode_cut(&mut request, cut);
        assert_eq!(
            decode_request(&request).unwrap(),
            FreshnessWireRequest::CompareAndAdvance {
                expected_record: Some(AuthorityDigest([9; 32])),
                next: cut,
            }
        );
    }
}

use std::fs::{self, File, OpenOptions};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;
use kernel_auth::{freshness_record_digest, sign_freshness_cut};

#[derive(Debug)]
pub struct TcpExternalFreshnessAuthorityServer {
    listener: TcpListener,
    state_directory: PathBuf,
    signing_key: SigningKey,
    _state_lock: File,
}

impl TcpExternalFreshnessAuthorityServer {
    pub fn bind(
        endpoint: SocketAddr,
        state_directory: impl Into<PathBuf>,
        signing_key: SigningKey,
    ) -> Result<Self, DurabilityError> {
        let state_directory = state_directory.into();
        fs::create_dir_all(&state_directory)?;
        sync_directory(&state_directory)?;
        let state_lock = lock_state_directory(&state_directory)?;
        let listener = TcpListener::bind(endpoint)?;
        Ok(Self {
            listener,
            state_directory,
            signing_key,
            _state_lock: state_lock,
        })
    }

    pub fn from_listener(
        listener: TcpListener,
        state_directory: impl Into<PathBuf>,
        signing_key: SigningKey,
    ) -> Result<Self, DurabilityError> {
        let state_directory = state_directory.into();
        fs::create_dir_all(&state_directory)?;
        sync_directory(&state_directory)?;
        let state_lock = lock_state_directory(&state_directory)?;
        Ok(Self {
            listener,
            state_directory,
            signing_key,
            _state_lock: state_lock,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, DurabilityError> {
        Ok(self.listener.local_addr()?)
    }

    pub fn serve_one(&self) -> Result<(), DurabilityError> {
        let (mut stream, _) = self.listener.accept()?;
        let request = read_wire_frame(&mut stream)?;
        let response = match decode_request(&request)? {
            FreshnessWireRequest::Read { store_id } => self
                .read_record(store_id)?
                .map_or(FreshnessWireResponse::None, |record| {
                    FreshnessWireResponse::Record(Box::new(record))
                }),
            FreshnessWireRequest::CompareAndAdvance {
                expected_record,
                next,
            } => self.compare_and_advance(expected_record, next)?,
        };
        write_wire_frame(&mut stream, &encode_response(response))
    }

    fn compare_and_advance(
        &self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessCut,
    ) -> Result<FreshnessWireResponse, DurabilityError> {
        let current = self.read_record(next.store_id)?;
        let observed = current.as_ref().map(freshness_record_digest);
        if observed != expected_record {
            return Ok(FreshnessWireResponse::CasMismatch);
        }
        if let Some(current) = &current
            && !valid_cut_extension(current.cut, next)
        {
            return Ok(FreshnessWireResponse::Rejected);
        }
        let signed = sign_freshness_cut(&self.signing_key, next);
        self.persist_record(&signed)?;
        Ok(FreshnessWireResponse::Record(Box::new(signed)))
    }

    fn read_record(
        &self,
        store_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessCut>, DurabilityError> {
        let path = record_path(&self.state_directory, store_id);
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let record = decode_response(&bytes)?
            .ok_or_else(|| protocol("freshness authority state record is empty"))?;
        if record.cut.store_id != store_id {
            return Err(protocol("freshness authority state record is corrupt"));
        }
        Ok(Some(record))
    }

    fn persist_record(&self, record: &SignedFreshnessCut) -> Result<(), DurabilityError> {
        let path = record_path(&self.state_directory, record.cut.store_id);
        let pending = path.with_extension(format!("pending-{}", std::process::id()));
        let bytes = encode_response(FreshnessWireResponse::Record(Box::new(record.clone())));
        let _ = fs::remove_file(&pending);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pending)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&pending, &path)?;
        sync_directory(&self.state_directory)
    }
}

fn valid_cut_extension(current: FreshnessCut, next: FreshnessCut) -> bool {
    if current.store_id != next.store_id
        || next.generation < current.generation
        || next.generation > current.generation.saturating_add(1)
        || next.trust_root_epoch < current.trust_root_epoch
        || next.deployment_policy_epoch < current.deployment_policy_epoch
    {
        return false;
    }
    if next.generation == current.generation {
        return next.previous_generation == current.previous_generation
            && next.generation_digest == current.generation_digest
            && next.wal_lsn >= current.wal_lsn
            && (next.wal_lsn != current.wal_lsn || next.wal_digest == current.wal_digest);
    }
    next.previous_generation == Some(current.generation_digest)
}

fn record_path(directory: &Path, store_id: [u8; 32]) -> PathBuf {
    let mut name = String::with_capacity(64 + ".freshness".len());
    for byte in store_id {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    name.push_str(".freshness");
    directory.join(name)
}

fn lock_state_directory(directory: &Path) -> Result<File, DurabilityError> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(".cfmd-freshness.lock"))?;
    lock.lock()?;
    Ok(lock)
}

fn sync_directory(path: &Path) -> Result<(), DurabilityError> {
    File::open(path)?.sync_all()?;
    Ok(())
}
