use std::marker::PhantomData;

use aes_gcm_siv::{
    Aes256GcmSiv, Nonce, Tag,
    aead::{AeadInOut, KeyInit, inout::InOutBuf},
};
use cfmd_secure_memory::{SecretHandle, SecureBox, SecureBytes, SecureMemoryError};

use super::{
    KEY_LEN, NONCE_LEN, StorageAeadAlgorithm, StorageAeadBackendCapabilities,
    StorageEncryptionDomain, TAG_LEN,
};

pub(super) trait StorageAeadBackend {
    type Context: Send + Sync + 'static;

    const ALGORITHM: StorageAeadAlgorithm;
    const CAPABILITIES: StorageAeadBackendCapabilities;

    fn initialize_exclusive(
        key: &SecureBytes<KEY_LEN>,
    ) -> Result<SecureBox<Self::Context>, SecureMemoryError>;

    fn initialize(
        key: &SecureBytes<KEY_LEN>,
    ) -> Result<SecretHandle<Self::Context>, SecureMemoryError> {
        Self::initialize_exclusive(key).map(SecretHandle::from)
    }

    fn seal_detached(
        context: &Self::Context,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        buffer: &mut [u8],
    ) -> Result<[u8; TAG_LEN], BackendError>;

    fn open_detached(
        context: &Self::Context,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        buffer: &mut [u8],
        tag: &[u8; TAG_LEN],
    ) -> Result<(), BackendError>;
}

pub(super) struct RustCryptoAes256GcmSiv;

pub(super) struct Aes256GcmSivContext(Aes256GcmSiv);

impl StorageAeadBackend for RustCryptoAes256GcmSiv {
    type Context = Aes256GcmSivContext;

    const ALGORITHM: StorageAeadAlgorithm = StorageAeadAlgorithm::Aes256GcmSiv;
    const CAPABILITIES: StorageAeadBackendCapabilities =
        StorageAeadBackendCapabilities::protected_with_constructor_transients();

    fn initialize_exclusive(
        key: &SecureBytes<KEY_LEN>,
    ) -> Result<SecureBox<Self::Context>, SecureMemoryError> {
        SecureBox::try_new_with(|| {
            key.with_secret(|bytes| Aes256GcmSivContext(Aes256GcmSiv::new(bytes.into())))
        })
    }

    fn seal_detached(
        context: &Self::Context,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        buffer: &mut [u8],
    ) -> Result<[u8; TAG_LEN], BackendError> {
        let nonce = Nonce::try_from(nonce.as_slice()).map_err(|_| BackendError)?;
        let tag = context
            .0
            .encrypt_inout_detached(&nonce, aad, InOutBuf::from(buffer))
            .map_err(|_| BackendError)?;
        let mut output = [0_u8; TAG_LEN];
        output.copy_from_slice(tag.as_slice());
        Ok(output)
    }

    fn open_detached(
        context: &Self::Context,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        buffer: &mut [u8],
        tag: &[u8; TAG_LEN],
    ) -> Result<(), BackendError> {
        let nonce = Nonce::try_from(nonce.as_slice()).map_err(|_| BackendError)?;
        let tag = Tag::try_from(tag.as_slice()).map_err(|_| BackendError)?;
        context
            .0
            .decrypt_inout_detached(&nonce, aad, InOutBuf::from(buffer), &tag)
            .map_err(|_| BackendError)
    }
}

pub(super) struct EphemeralContext<B: StorageAeadBackend> {
    context: SecureBox<B::Context>,
    _backend: PhantomData<B>,
}

impl<B: StorageAeadBackend> EphemeralContext<B> {
    pub(super) fn initialize(key: &SecureBytes<KEY_LEN>) -> Result<Self, SecureMemoryError> {
        Ok(Self {
            context: B::initialize_exclusive(key)?,
            _backend: PhantomData,
        })
    }

    pub(super) fn seal_detached(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        buffer: &mut [u8],
    ) -> Result<[u8; TAG_LEN], BackendError> {
        self.context
            .with_secret(|context| B::seal_detached(context, nonce, aad, buffer))
    }

    pub(super) fn open_detached(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        buffer: &mut [u8],
        tag: &[u8; TAG_LEN],
    ) -> Result<(), BackendError> {
        self.context
            .with_secret(|context| B::open_detached(context, nonce, aad, buffer, tag))
    }
}

pub(super) type Aes256GcmSivEphemeralContext = EphemeralContext<RustCryptoAes256GcmSiv>;

pub(super) struct DomainContexts<B: StorageAeadBackend> {
    section: SecretHandle<B::Context>,
    wal: SecretHandle<B::Context>,
    immutable_object: SecretHandle<B::Context>,
    _backend: PhantomData<B>,
}

impl<B: StorageAeadBackend> DomainContexts<B> {
    fn new(
        section: SecretHandle<B::Context>,
        wal: SecretHandle<B::Context>,
        immutable_object: SecretHandle<B::Context>,
    ) -> Self {
        Self {
            section,
            wal,
            immutable_object,
            _backend: PhantomData,
        }
    }

    fn context(&self, domain: StorageEncryptionDomain) -> &SecretHandle<B::Context> {
        match domain {
            StorageEncryptionDomain::Section => &self.section,
            StorageEncryptionDomain::Wal => &self.wal,
            StorageEncryptionDomain::ImmutableObject => &self.immutable_object,
        }
    }
}

impl<B: StorageAeadBackend> Clone for DomainContexts<B> {
    fn clone(&self) -> Self {
        Self {
            section: self.section.clone(),
            wal: self.wal.clone(),
            immutable_object: self.immutable_object.clone(),
            _backend: PhantomData,
        }
    }
}

#[derive(Clone)]
pub(super) enum BackendState {
    Aes256GcmSiv(DomainContexts<RustCryptoAes256GcmSiv>),
}

impl BackendState {
    pub(super) fn aes256_gcm_siv(
        section: SecretHandle<<RustCryptoAes256GcmSiv as StorageAeadBackend>::Context>,
        wal: SecretHandle<<RustCryptoAes256GcmSiv as StorageAeadBackend>::Context>,
        immutable_object: SecretHandle<<RustCryptoAes256GcmSiv as StorageAeadBackend>::Context>,
    ) -> Self {
        Self::Aes256GcmSiv(DomainContexts::new(section, wal, immutable_object))
    }

    pub(super) const fn algorithm(&self) -> StorageAeadAlgorithm {
        match self {
            Self::Aes256GcmSiv(_) => RustCryptoAes256GcmSiv::ALGORITHM,
        }
    }

    pub(super) const fn capabilities(&self) -> StorageAeadBackendCapabilities {
        match self {
            Self::Aes256GcmSiv(_) => RustCryptoAes256GcmSiv::CAPABILITIES,
        }
    }

    pub(super) fn seal_detached(
        &self,
        domain: StorageEncryptionDomain,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        buffer: &mut [u8],
    ) -> Result<[u8; TAG_LEN], BackendError> {
        match self {
            Self::Aes256GcmSiv(contexts) => contexts.context(domain).with_secret(|context| {
                RustCryptoAes256GcmSiv::seal_detached(context, nonce, aad, buffer)
            }),
        }
    }

    pub(super) fn open_detached(
        &self,
        domain: StorageEncryptionDomain,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        buffer: &mut [u8],
        tag: &[u8; TAG_LEN],
    ) -> Result<(), BackendError> {
        match self {
            Self::Aes256GcmSiv(contexts) => contexts.context(domain).with_secret(|context| {
                RustCryptoAes256GcmSiv::open_detached(context, nonce, aad, buffer, tag)
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct BackendError;

#[cfg(test)]
mod tests {
    use std::{hint::black_box, time::Instant};

    use super::*;

    #[test]
    fn cloned_backend_state_shares_protected_contexts() {
        let key = SecureBytes::try_from_array([0x39; KEY_LEN]).unwrap();
        let section = RustCryptoAes256GcmSiv::initialize(&key).unwrap();
        let wal = RustCryptoAes256GcmSiv::initialize(&key).unwrap();
        let immutable_object = RustCryptoAes256GcmSiv::initialize(&key).unwrap();
        let backend = BackendState::aes256_gcm_siv(section, wal, immutable_object);
        let clone = backend.clone();

        let BackendState::Aes256GcmSiv(left) = &backend;
        let BackendState::Aes256GcmSiv(right) = &clone;
        for (left, right) in [
            (&left.section, &right.section),
            (&left.wal, &right.wal),
            (&left.immutable_object, &right.immutable_object),
        ] {
            let left_ptr = left.with_secret(std::ptr::from_ref);
            let right_ptr = right.with_secret(std::ptr::from_ref);
            assert_eq!(left_ptr, right_ptr);
        }
    }

    #[test]
    #[ignore = "manual R&D backend-dispatch microbenchmark"]
    fn backend_dispatch_microbench() {
        const KEY: [u8; KEY_LEN] = [0x51; KEY_LEN];
        let secure_key = SecureBytes::try_from_array(KEY).unwrap();
        let section = RustCryptoAes256GcmSiv::initialize(&secure_key).unwrap();
        let backend = BackendState::aes256_gcm_siv(section.clone(), section.clone(), section);
        let direct = Aes256GcmSiv::new((&KEY).into());
        let aad = b"CFMD backend dispatch benchmark";

        for (size, iterations) in [(4096_usize, 50_000_u32), (65_536, 5_000)] {
            let mut direct_buffer = vec![0xA5_u8; size];
            let started = Instant::now();
            for iteration in 0..iterations {
                let mut nonce = [0_u8; NONCE_LEN];
                nonce[4..].copy_from_slice(&u64::from(iteration).to_le_bytes());
                let nonce_ref = Nonce::try_from(nonce.as_slice()).unwrap();
                black_box(
                    direct
                        .encrypt_inout_detached(
                            &nonce_ref,
                            aad,
                            InOutBuf::from(&mut direct_buffer[..]),
                        )
                        .unwrap(),
                );
            }
            let direct_elapsed = started.elapsed();

            let mut backend_buffer = vec![0xA5_u8; size];
            let started = Instant::now();
            for iteration in 0..iterations {
                let mut nonce = [0_u8; NONCE_LEN];
                nonce[4..].copy_from_slice(&u64::from(iteration).to_le_bytes());
                black_box(
                    backend
                        .seal_detached(
                            StorageEncryptionDomain::Section,
                            &nonce,
                            aad,
                            &mut backend_buffer,
                        )
                        .unwrap(),
                );
            }
            let backend_elapsed = started.elapsed();

            println!(
                "backend_dispatch size={size} direct={:.3}ns/op backend={:.3}ns/op ratio={:.4}",
                direct_elapsed.as_secs_f64() * 1e9 / f64::from(iterations),
                backend_elapsed.as_secs_f64() * 1e9 / f64::from(iterations),
                backend_elapsed.as_secs_f64() / direct_elapsed.as_secs_f64(),
            );
        }
    }
}
