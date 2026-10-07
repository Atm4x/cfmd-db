use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use kernel_auth::{
    AuthorityDigest, FreshnessAuthorityState, FreshnessCut, KeyId, SignedFreshnessAuthority,
    VolatileFence, freshness_authority_record_digest, sign_freshness_authority,
    volatile_fence_nonce,
};

use crate::runtime::DurabilityError;
use crate::store::ExternalFreshnessAuthority;

const MAGIC: [u8; 4] = *b"CFFA";
const VERSION: u16 = 2;
const OP_READ: u8 = 1;
const OP_COMPARE_SET: u8 = 2;
const OP_COMPARE_REBIND: u8 = 3;
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
        lineage_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessAuthority>, DurabilityError> {
        let mut request = header(OP_READ);
        request.extend_from_slice(&lineage_id);
        let response = self.transact(&request)?;
        decode_response(&response)
    }

    fn compare_and_set_signed(
        &mut self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessAuthorityState,
    ) -> Result<SignedFreshnessAuthority, DurabilityError> {
        let mut request = header(OP_COMPARE_SET);
        push_optional_digest(&mut request, expected_record);
        encode_state(&mut request, next);
        let response = self.transact(&request)?;
        decode_response(&response)?.ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "freshness authority returned no record after compare-and-set",
        })
    }

    fn compare_and_rebind_signed(
        &mut self,
        source_lineage_id: [u8; 32],
        expected_source_record: AuthorityDigest,
        next: FreshnessAuthorityState,
    ) -> Result<SignedFreshnessAuthority, DurabilityError> {
        let mut request = header(OP_COMPARE_REBIND);
        request.extend_from_slice(&source_lineage_id);
        request.extend_from_slice(&expected_source_record.0);
        encode_state(&mut request, next);
        let response = self.transact(&request)?;
        decode_response(&response)?.ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "freshness authority returned no record after compare-and-rebind",
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

fn decode_response(bytes: &[u8]) -> Result<Option<SignedFreshnessAuthority>, DurabilityError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take_array::<4>()? != MAGIC || cursor.take_u16()? != VERSION {
        return Err(protocol(
            "freshness authority returned incompatible wire header",
        ));
    }
    let status = cursor.take_u8()?;
    let result = match status {
        STATUS_NONE => None,
        STATUS_RECORD => Some(*decode_signed_authority(&mut cursor)?),
        STATUS_CAS_MISMATCH => {
            return Err(protocol(
                "freshness authority compare-and-set lost CAS race",
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

fn decode_request(bytes: &[u8]) -> Result<FreshnessWireRequest, DurabilityError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take_array::<4>()? != MAGIC || cursor.take_u16()? != VERSION {
        return Err(protocol("freshness request has incompatible wire header"));
    }
    let request = match cursor.take_u8()? {
        OP_READ => FreshnessWireRequest::Read {
            store_id: cursor.take_array::<32>()?,
        },
        OP_COMPARE_SET => FreshnessWireRequest::CompareAndSet {
            expected_record: cursor.take_optional_digest()?,
            next: decode_state(&mut cursor)?,
        },
        OP_COMPARE_REBIND => FreshnessWireRequest::CompareAndRebind {
            source_store_id: cursor.take_array::<32>()?,
            expected_source_record: AuthorityDigest(cursor.take_array::<32>()?),
            next: decode_state(&mut cursor)?,
        },
        _ => return Err(protocol("freshness request has unknown operation")),
    };
    if cursor.remaining() != 0 {
        return Err(protocol("freshness request has trailing bytes"));
    }
    Ok(request)
}

fn encode_response(response: FreshnessWireResponse) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_be_bytes());
    match response {
        FreshnessWireResponse::None => out.push(STATUS_NONE),
        FreshnessWireResponse::Record(record) => {
            out.push(STATUS_RECORD);
            encode_signed_authority(&mut out, &record);
        }
        FreshnessWireResponse::CasMismatch => out.push(STATUS_CAS_MISMATCH),
        FreshnessWireResponse::Rejected => out.push(STATUS_REJECTED),
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FreshnessWireRequest {
    Read {
        store_id: [u8; 32],
    },
    CompareAndSet {
        expected_record: Option<AuthorityDigest>,
        next: FreshnessAuthorityState,
    },
    CompareAndRebind {
        source_store_id: [u8; 32],
        expected_source_record: AuthorityDigest,
        next: FreshnessAuthorityState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FreshnessWireResponse {
    None,
    Record(Box<SignedFreshnessAuthority>),
    CasMismatch,
    Rejected,
}

fn read_wire_frame(stream: &mut impl Read) -> Result<Vec<u8>, DurabilityError> {
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

fn write_wire_frame(stream: &mut impl Write, bytes: &[u8]) -> Result<(), DurabilityError> {
    let len = u32::try_from(bytes.len())
        .map_err(|_| protocol("freshness wire frame exceeds length encoding"))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(bytes)?;
    stream.flush()?;
    Ok(())
}

fn encode_signed_authority(out: &mut Vec<u8>, record: &SignedFreshnessAuthority) {
    encode_state(out, record.state);
    out.extend_from_slice(&record.signer.0);
    out.extend_from_slice(&record.signature);
}

fn decode_signed_authority(
    cursor: &mut Cursor<'_>,
) -> Result<Box<SignedFreshnessAuthority>, DurabilityError> {
    Ok(Box::new(SignedFreshnessAuthority {
        state: decode_state(cursor)?,
        signer: KeyId(cursor.take_array::<32>()?),
        signature: cursor.take_array::<64>()?,
    }))
}

fn encode_state(out: &mut Vec<u8>, state: FreshnessAuthorityState) {
    match state {
        FreshnessAuthorityState::DurableCut(cut) => {
            out.push(0);
            encode_cut(out, cut);
        }
        FreshnessAuthorityState::VolatileFence(fence) => {
            out.push(1);
            out.extend_from_slice(&fence.lineage_id);
            out.extend_from_slice(&fence.predecessor_record_digest.0);
            out.extend_from_slice(&fence.source_generation_digest.0);
            out.extend_from_slice(&fence.trust_root_epoch.to_be_bytes());
            out.extend_from_slice(&fence.deployment_policy_epoch.to_be_bytes());
            out.extend_from_slice(&fence.transition_nonce.0);
        }
    }
}

fn decode_state(cursor: &mut Cursor<'_>) -> Result<FreshnessAuthorityState, DurabilityError> {
    match cursor.take_u8()? {
        0 => Ok(FreshnessAuthorityState::DurableCut(decode_cut(cursor)?)),
        1 => Ok(FreshnessAuthorityState::VolatileFence(VolatileFence {
            lineage_id: cursor.take_array::<32>()?,
            predecessor_record_digest: AuthorityDigest(cursor.take_array::<32>()?),
            source_generation_digest: AuthorityDigest(cursor.take_array::<32>()?),
            trust_root_epoch: cursor.take_u64()?,
            deployment_policy_epoch: cursor.take_u64()?,
            transition_nonce: AuthorityDigest(cursor.take_array::<32>()?),
        })),
        _ => Err(protocol("freshness authority state tag is invalid")),
    }
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
        let record = SignedFreshnessAuthority {
            state: FreshnessAuthorityState::DurableCut(cut),
            signer: KeyId([7; 32]),
            signature: [8; 64],
        };
        let encoded = encode_response(FreshnessWireResponse::Record(Box::new(record.clone())));
        assert_eq!(decode_response(&encoded).unwrap(), Some(record));

        let mut request = header(OP_COMPARE_SET);
        push_optional_digest(&mut request, Some(AuthorityDigest([9; 32])));
        encode_state(&mut request, FreshnessAuthorityState::DurableCut(cut));
        assert_eq!(
            decode_request(&request).unwrap(),
            FreshnessWireRequest::CompareAndSet {
                expected_record: Some(AuthorityDigest([9; 32])),
                next: FreshnessAuthorityState::DurableCut(cut),
            }
        );
    }

    #[test]
    fn pending_rebind_transaction_is_completed_before_server_accepts_requests() {
        let dir = std::env::temp_dir().join(format!(
            "cfmd-freshness-rebind-recovery-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let signing = ed25519_dalek::SigningKey::from_bytes(&[0x41; 32]);
        let source_id = [0x11; 32];
        let target_id = [0x22; 32];
        let source = kernel_auth::sign_freshness_authority(
            &signing,
            FreshnessAuthorityState::DurableCut(FreshnessCut {
                store_id: source_id,
                generation: 7,
                previous_generation: None,
                generation_digest: AuthorityDigest([0x33; 32]),
                wal_lsn: 9,
                wal_digest: AuthorityDigest([0x44; 32]),
                trust_root_epoch: 5,
                deployment_policy_epoch: 6,
            }),
        );
        persist_record_bytes(&dir, &source).unwrap();
        let target = kernel_auth::sign_freshness_authority(
            &signing,
            FreshnessAuthorityState::DurableCut(FreshnessCut {
                store_id: target_id,
                generation: 1,
                previous_generation: Some(source.durable_cut().unwrap().generation_digest),
                generation_digest: AuthorityDigest([0x55; 32]),
                wal_lsn: 0,
                wal_digest: AuthorityDigest([0x66; 32]),
                trust_root_epoch: 5,
                deployment_policy_epoch: 6,
            }),
        );
        let txn = rebind_transaction_path(&dir, source_id, target_id);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&source_id);
        bytes.extend_from_slice(&target_id);
        bytes.extend_from_slice(&encode_response(FreshnessWireResponse::Record(Box::new(
            target.clone(),
        ))));
        std::fs::write(&txn, bytes).unwrap();
        File::open(&txn).unwrap().sync_all().unwrap();
        recover_rebind_transactions(&dir).unwrap();
        assert!(!record_path(&dir, source_id).exists());
        assert!(!txn.exists());
        let recovered = decode_response(&std::fs::read(record_path(&dir, target_id)).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(recovered, target);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn oversized_state_record_is_rejected_before_decode() {
        let dir = std::env::temp_dir().join(format!(
            "cfmd-freshness-record-bound-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let server = TcpExternalFreshnessAuthorityServer::from_listener(
            listener,
            &dir,
            ed25519_dalek::SigningKey::from_bytes(&[0x5a; 32]),
        )
        .unwrap();
        let store_id = [0x33; 32];
        std::fs::write(record_path(&dir, store_id), vec![0_u8; MAX_FRAME_LEN + 1]).unwrap();
        assert!(matches!(
            server.read_record(store_id),
            Err(DurabilityError::Protocol {
                reason: "freshness authority state record exceeds hard limit",
                ..
            })
        ));
        drop(server);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

use std::fs::{self, File, OpenOptions};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;

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
        recover_rebind_transactions(&state_directory)?;
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
        recover_rebind_transactions(&state_directory)?;
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
            FreshnessWireRequest::CompareAndSet {
                expected_record,
                next,
            } => self.compare_and_set(expected_record, next)?,
            FreshnessWireRequest::CompareAndRebind {
                source_store_id,
                expected_source_record,
                next,
            } => self.compare_and_rebind(source_store_id, expected_source_record, next)?,
        };
        write_wire_frame(&mut stream, &encode_response(response))
    }

    fn compare_and_set(
        &self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessAuthorityState,
    ) -> Result<FreshnessWireResponse, DurabilityError> {
        let lineage_id = next.lineage_id();
        let current = self.read_record(lineage_id)?;
        let observed = current.as_ref().map(freshness_authority_record_digest);
        if observed != expected_record {
            return Ok(FreshnessWireResponse::CasMismatch);
        }
        if let Some(current) = &current {
            if !valid_state_extension(current, next) {
                return Ok(FreshnessWireResponse::Rejected);
            }
        } else if !matches!(next, FreshnessAuthorityState::DurableCut(cut) if cut.previous_generation.is_none())
        {
            return Ok(FreshnessWireResponse::Rejected);
        }
        let signed = sign_freshness_authority(&self.signing_key, next);
        self.persist_record(&signed)?;
        Ok(FreshnessWireResponse::Record(Box::new(signed)))
    }

    fn compare_and_rebind(
        &self,
        source_store_id: [u8; 32],
        expected_source_record: AuthorityDigest,
        next: FreshnessAuthorityState,
    ) -> Result<FreshnessWireResponse, DurabilityError> {
        let FreshnessAuthorityState::DurableCut(next_cut) = next else {
            return Ok(FreshnessWireResponse::Rejected);
        };
        if source_store_id == next_cut.store_id || self.read_record(next_cut.store_id)?.is_some() {
            return Ok(FreshnessWireResponse::Rejected);
        }
        let source = self
            .read_record(source_store_id)?
            .ok_or_else(|| protocol("freshness rebind source record is missing"))?;
        if freshness_authority_record_digest(&source) != expected_source_record {
            return Ok(FreshnessWireResponse::CasMismatch);
        }
        let Some(source_cut) = source.durable_cut() else {
            return Ok(FreshnessWireResponse::Rejected);
        };
        if next_cut.previous_generation != Some(source_cut.generation_digest)
            || next_cut.trust_root_epoch < source_cut.trust_root_epoch
            || next_cut.deployment_policy_epoch < source_cut.deployment_policy_epoch
        {
            return Ok(FreshnessWireResponse::Rejected);
        }
        let signed = sign_freshness_authority(
            &self.signing_key,
            FreshnessAuthorityState::DurableCut(next_cut),
        );
        persist_rebind_transaction(&self.state_directory, source_store_id, &signed)?;
        Ok(FreshnessWireResponse::Record(Box::new(signed)))
    }

    fn read_record(
        &self,
        lineage_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessAuthority>, DurabilityError> {
        let path = record_path(&self.state_directory, lineage_id);
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if file.metadata()?.len()
            > u64::try_from(MAX_FRAME_LEN).expect("freshness frame limit fits u64")
        {
            return Err(protocol(
                "freshness authority state record exceeds hard limit",
            ));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(MAX_FRAME_LEN)
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        file.take(
            u64::try_from(MAX_FRAME_LEN + 1).expect("freshness frame limit plus sentinel fits u64"),
        )
        .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_FRAME_LEN {
            return Err(protocol(
                "freshness authority state record exceeds hard limit",
            ));
        }
        let record = decode_response(&bytes)?
            .ok_or_else(|| protocol("freshness authority state record is empty"))?;
        if record.state.lineage_id() != lineage_id {
            return Err(protocol("freshness authority state record is corrupt"));
        }
        Ok(Some(record))
    }

    fn persist_record(&self, record: &SignedFreshnessAuthority) -> Result<(), DurabilityError> {
        persist_record_bytes(&self.state_directory, record)
    }
}

fn valid_state_extension(
    current: &SignedFreshnessAuthority,
    next: FreshnessAuthorityState,
) -> bool {
    let current_digest = freshness_authority_record_digest(current);
    match (current.state, next) {
        (
            FreshnessAuthorityState::DurableCut(current),
            FreshnessAuthorityState::DurableCut(next),
        ) => valid_cut_extension(current, next),
        (
            FreshnessAuthorityState::DurableCut(current),
            FreshnessAuthorityState::VolatileFence(next),
        ) => {
            next.lineage_id == current.store_id
                && next.predecessor_record_digest == current_digest
                && next.source_generation_digest == current.generation_digest
                && next.trust_root_epoch >= current.trust_root_epoch
                && next.deployment_policy_epoch >= current.deployment_policy_epoch
                && next.transition_nonce
                    == volatile_fence_nonce(current_digest, current.generation_digest)
        }
        (
            FreshnessAuthorityState::VolatileFence(current),
            FreshnessAuthorityState::DurableCut(next),
        ) => {
            next.store_id == current.lineage_id
                && next.previous_generation == Some(current.source_generation_digest)
                && next.trust_root_epoch >= current.trust_root_epoch
                && next.deployment_policy_epoch >= current.deployment_policy_epoch
        }
        (FreshnessAuthorityState::VolatileFence(_), FreshnessAuthorityState::VolatileFence(_)) => {
            false
        }
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

const REBIND_TXN_PREFIX: &str = ".cfmd-rebind-";

fn persist_rebind_transaction(
    directory: &Path,
    source_store_id: [u8; 32],
    target: &SignedFreshnessAuthority,
) -> Result<(), DurabilityError> {
    let txn = rebind_transaction_path(directory, source_store_id, target.state.lineage_id());
    let mut bytes = Vec::with_capacity(64 + 256);
    bytes.extend_from_slice(&source_store_id);
    bytes.extend_from_slice(&target.state.lineage_id());
    bytes.extend_from_slice(&encode_response(FreshnessWireResponse::Record(Box::new(
        target.clone(),
    ))));
    let _ = fs::remove_file(&txn);
    let mut file = OpenOptions::new().create_new(true).write(true).open(&txn)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    sync_directory(directory)?;
    finish_rebind_transaction(directory, &txn, &bytes)
}

fn recover_rebind_transactions(directory: &Path) -> Result<(), DurabilityError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with(REBIND_TXN_PREFIX)
            || !Path::new(&name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("txn"))
        {
            continue;
        }
        let bytes = fs::read(entry.path())?;
        finish_rebind_transaction(directory, &entry.path(), &bytes)?;
    }
    Ok(())
}

fn finish_rebind_transaction(
    directory: &Path,
    txn: &Path,
    bytes: &[u8],
) -> Result<(), DurabilityError> {
    if bytes.len() < 64 {
        return Err(protocol("freshness rebind transaction is truncated"));
    }
    let mut source_store_id = [0_u8; 32];
    source_store_id.copy_from_slice(&bytes[..32]);
    let mut target_store_id = [0_u8; 32];
    target_store_id.copy_from_slice(&bytes[32..64]);
    let target = decode_response(&bytes[64..])?
        .ok_or_else(|| protocol("freshness rebind transaction has no target record"))?;
    if target.state.lineage_id() != target_store_id || source_store_id == target_store_id {
        return Err(protocol("freshness rebind transaction identity is corrupt"));
    }
    persist_record_bytes(directory, &target)?;
    match fs::remove_file(record_path(directory, source_store_id)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    sync_directory(directory)?;
    fs::remove_file(txn)?;
    sync_directory(directory)
}

fn persist_record_bytes(
    directory: &Path,
    record: &SignedFreshnessAuthority,
) -> Result<(), DurabilityError> {
    let path = record_path(directory, record.state.lineage_id());
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
    sync_directory(directory)
}

fn rebind_transaction_path(
    directory: &Path,
    source_store_id: [u8; 32],
    target_store_id: [u8; 32],
) -> PathBuf {
    let mut name = String::from(REBIND_TXN_PREFIX);
    for byte in source_store_id.into_iter().chain(target_store_id) {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    name.push_str(".txn");
    directory.join(name)
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
