use std::{fmt, sync::Arc};

use cfmd_secure_memory::{SecureBox, SecureBytes, SecureMemoryError};
use sha2::{Digest, Sha256, digest::Output};

mod backend;
mod secure_hkdf;

use backend::{
    Aes256GcmSivEphemeralContext, BackendState, RustCryptoAes256GcmSiv, StorageAeadBackend,
};
use secure_hkdf::SecureHkdfSha256;

use crate::runtime::DurabilityError;

const AE_MAGIC: [u8; 4] = *b"CFAE";
const AE_VERSION: u8 = 1;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const ENVELOPE_PREFIX_LEN: usize = AE_MAGIC.len() + 2 + NONCE_LEN;
const KEY_LEN: usize = 32;
const KEY_COMMITMENT_DOMAIN: &[u8] = b"CFMD-AE-v1/key-commitment\0";
const DMK_WRAP_KEY_DOMAIN: &[u8] = b"CFMD-AE-v1/dmk-wrap-key\0";
const DMK_WRAP_AAD_DOMAIN: &[u8] = b"CFMD-AE-v1/dmk-wrap\0";
const NONCE_NAMESPACE_PREFIX_LEN: usize = 10;
const NONCES_PER_NAMESPACE: u32 = 1_u32 << 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StorageAeadAlgorithm {
    Aes256GcmSiv = 1,
}

impl StorageAeadAlgorithm {
    pub(crate) fn decode(raw: u8) -> Result<Self, DurabilityError> {
        match raw {
            1 => Ok(Self::Aes256GcmSiv),
            _ => Err(DurabilityError::Corruption {
                offset: 0,
                reason: "unsupported storage AEAD algorithm",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistentSecretStateProtection {
    HardenedProcessMemory,
    ExternalOpaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStateInitializationProtection {
    ConstructorTransientsMayExist,
    TransientZeroized,
    StrictInPlace,
    ExternalOpaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageAeadBackendCapabilities {
    persistent_state: PersistentSecretStateProtection,
    initialization: SecretStateInitializationProtection,
}

impl StorageAeadBackendCapabilities {
    #[must_use]
    pub const fn protected_with_constructor_transients() -> Self {
        Self {
            persistent_state: PersistentSecretStateProtection::HardenedProcessMemory,
            initialization: SecretStateInitializationProtection::ConstructorTransientsMayExist,
        }
    }

    #[must_use]
    pub const fn persistent_state(self) -> PersistentSecretStateProtection {
        self.persistent_state
    }

    #[must_use]
    pub const fn initialization(self) -> SecretStateInitializationProtection {
        self.initialization
    }

    #[must_use]
    pub const fn is_strict_in_place(self) -> bool {
        matches!(
            self.initialization,
            SecretStateInitializationProtection::StrictInPlace
                | SecretStateInitializationProtection::ExternalOpaque
        )
    }
}

pub struct StorageEncryptionKey(Arc<SecureBytes<KEY_LEN>>);

#[derive(Debug)]
pub enum StorageEncryptionKeyInitError<E> {
    Memory(DurabilityError),
    Initializer(E),
}

impl StorageEncryptionKey {
    pub fn try_new(bytes: [u8; KEY_LEN]) -> Result<Self, DurabilityError> {
        let secret = SecureBytes::try_from_array(bytes).map_err(secure_memory_error)?;
        Ok(Self(Arc::new(secret)))
    }

    pub fn try_initialize<E, O>(
        initializer: impl FnOnce(&mut [u8; KEY_LEN]) -> Result<O, E>,
    ) -> Result<(Self, O), StorageEncryptionKeyInitError<E>> {
        let mut secret = SecureBytes::<KEY_LEN>::try_zeroed()
            .map_err(secure_memory_error)
            .map_err(StorageEncryptionKeyInitError::Memory)?;
        let output = secret
            .with_secret_mut(initializer)
            .map_err(StorageEncryptionKeyInitError::Initializer)?;
        Ok((Self::from_secure(secret), output))
    }

    fn from_secure(secret: SecureBytes<KEY_LEN>) -> Self {
        Self(Arc::new(secret))
    }

    fn with_bytes<R>(&self, operation: impl FnOnce(&[u8; KEY_LEN]) -> R) -> R {
        self.0.with_secret(operation)
    }
}

impl Clone for StorageEncryptionKey {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl fmt::Debug for StorageEncryptionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StorageEncryptionKey(<redacted>)")
    }
}

#[derive(Debug, Clone, Default)]
pub enum StorageEncryption {
    #[default]
    None,
    Direct {
        algorithm: StorageAeadAlgorithm,
        key: StorageEncryptionKey,
    },
    Wrapped {
        algorithm: StorageAeadAlgorithm,
        wrapping_key: StorageEncryptionKey,
        provider_key_id: [u8; 16],
        provider_key_epoch: u64,
        minimum_database_key_epoch: u64,
    },
}

impl StorageEncryption {
    #[must_use]
    pub fn direct(algorithm: StorageAeadAlgorithm, key: StorageEncryptionKey) -> Self {
        Self::Direct { algorithm, key }
    }

    #[must_use]
    pub fn wrapped(
        algorithm: StorageAeadAlgorithm,
        wrapping_key: StorageEncryptionKey,
        provider_key_id: [u8; 16],
        provider_key_epoch: u64,
        minimum_database_key_epoch: u64,
    ) -> Self {
        Self::Wrapped {
            algorithm,
            wrapping_key,
            provider_key_id,
            provider_key_epoch,
            minimum_database_key_epoch,
        }
    }

    #[must_use]
    pub fn aes256_gcm_siv(key: StorageEncryptionKey) -> Self {
        Self::direct(StorageAeadAlgorithm::Aes256GcmSiv, key)
    }

    #[must_use]
    pub fn aes256_gcm_siv_wrapped(
        wrapping_key: StorageEncryptionKey,
        provider_key_id: [u8; 16],
        provider_key_epoch: u64,
    ) -> Self {
        Self::wrapped(
            StorageAeadAlgorithm::Aes256GcmSiv,
            wrapping_key,
            provider_key_id,
            provider_key_epoch,
            1,
        )
    }

    #[must_use]
    pub fn aes256_gcm_siv_wrapped_with_minimum_database_key_epoch(
        wrapping_key: StorageEncryptionKey,
        provider_key_id: [u8; 16],
        provider_key_epoch: u64,
        minimum_database_key_epoch: u64,
    ) -> Self {
        Self::wrapped(
            StorageAeadAlgorithm::Aes256GcmSiv,
            wrapping_key,
            provider_key_id,
            provider_key_epoch,
            minimum_database_key_epoch,
        )
    }

    #[must_use]
    pub const fn algorithm(&self) -> Option<StorageAeadAlgorithm> {
        match self {
            Self::None => None,
            Self::Direct { algorithm, .. } | Self::Wrapped { algorithm, .. } => Some(*algorithm),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WrappedDatabaseMasterKey {
    pub(crate) nonce: [u8; NONCE_LEN],
    pub(crate) ciphertext_and_tag: [u8; KEY_LEN + TAG_LEN],
}

pub(crate) fn random_database_master_key() -> Result<StorageEncryptionKey, DurabilityError> {
    let mut secret = SecureBytes::<KEY_LEN>::try_zeroed().map_err(secure_memory_error)?;
    secret
        .with_secret_mut(|bytes| getrandom::fill(bytes))
        .map_err(|_| {
            crypto_protocol_error("OS CSPRNG failed while creating database master key")
        })?;
    Ok(StorageEncryptionKey::from_secure(secret))
}

pub(crate) fn wrap_database_master_key(
    wrapping_key: &StorageEncryptionKey,
    database_salt: &[u8; 32],
    provider_key_id: &[u8; 16],
    provider_key_epoch: u64,
    key_epoch: u64,
    publication_sequence: u64,
    master_key: &StorageEncryptionKey,
) -> Result<WrappedDatabaseMasterKey, DurabilityError> {
    let cipher = dmk_wrap_cipher(wrapping_key, database_salt)?;
    let mut nonce = [0_u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| {
        crypto_protocol_error("OS CSPRNG failed while wrapping database master key")
    })?;
    let mut ciphertext = SecureBytes::<KEY_LEN>::try_zeroed().map_err(secure_memory_error)?;
    ciphertext.with_secret_mut(|destination| {
        master_key.with_bytes(|bytes| destination.copy_from_slice(bytes));
    });
    let aad = dmk_wrap_aad(
        database_salt,
        provider_key_id,
        provider_key_epoch,
        key_epoch,
        publication_sequence,
    );
    let tag = ciphertext
        .with_secret_mut(|bytes| cipher.seal_detached(&nonce, &aad, bytes))
        .map_err(|_| crypto_protocol_error("database master key wrapping failed"))?;
    let mut ciphertext_and_tag = [0_u8; KEY_LEN + TAG_LEN];
    ciphertext.with_secret(|bytes| ciphertext_and_tag[..KEY_LEN].copy_from_slice(bytes));
    ciphertext_and_tag[KEY_LEN..].copy_from_slice(&tag);
    Ok(WrappedDatabaseMasterKey {
        nonce,
        ciphertext_and_tag,
    })
}

pub(crate) fn unwrap_database_master_key(
    wrapping_key: &StorageEncryptionKey,
    database_salt: &[u8; 32],
    provider_key_id: &[u8; 16],
    provider_key_epoch: u64,
    key_epoch: u64,
    publication_sequence: u64,
    wrapped: &WrappedDatabaseMasterKey,
) -> Result<StorageEncryptionKey, DurabilityError> {
    let cipher = dmk_wrap_cipher(wrapping_key, database_salt)?;
    let mut plaintext = SecureBytes::<KEY_LEN>::try_zeroed().map_err(secure_memory_error)?;
    plaintext.with_secret_mut(|bytes| {
        bytes.copy_from_slice(&wrapped.ciphertext_and_tag[..KEY_LEN]);
    });
    let mut tag = [0_u8; TAG_LEN];
    tag.copy_from_slice(&wrapped.ciphertext_and_tag[KEY_LEN..]);
    let aad = dmk_wrap_aad(
        database_salt,
        provider_key_id,
        provider_key_epoch,
        key_epoch,
        publication_sequence,
    );
    plaintext
        .with_secret_mut(|bytes| cipher.open_detached(&wrapped.nonce, &aad, bytes, &tag))
        .map_err(|_| crypto_protocol_error("database master key unwrap authentication failed"))?;
    Ok(StorageEncryptionKey::from_secure(plaintext))
}

fn dmk_wrap_cipher(
    wrapping_key: &StorageEncryptionKey,
    database_salt: &[u8; 32],
) -> Result<Aes256GcmSivEphemeralContext, DurabilityError> {
    let mut hkdf = wrapping_key
        .with_bytes(|bytes| SecureHkdfSha256::extract(database_salt, bytes))
        .map_err(secure_memory_error)?;
    let mut key = SecureBytes::<KEY_LEN>::try_zeroed().map_err(secure_memory_error)?;
    derive_key_into(&mut hkdf, DMK_WRAP_KEY_DOMAIN, &mut key);
    Aes256GcmSivEphemeralContext::initialize(&key).map_err(secure_memory_error)
}

fn dmk_wrap_aad(
    database_salt: &[u8; 32],
    provider_key_id: &[u8; 16],
    provider_key_epoch: u64,
    key_epoch: u64,
    publication_sequence: u64,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(DMK_WRAP_AAD_DOMAIN.len() + 32 + 16 + 8 * 3);
    aad.extend_from_slice(DMK_WRAP_AAD_DOMAIN);
    aad.extend_from_slice(database_salt);
    aad.extend_from_slice(provider_key_id);
    aad.extend_from_slice(&provider_key_epoch.to_le_bytes());
    aad.extend_from_slice(&key_epoch.to_le_bytes());
    aad.extend_from_slice(&publication_sequence.to_le_bytes());
    aad
}

pub(crate) struct StorageNonceSequence {
    prefix: [u8; NONCE_NAMESPACE_PREFIX_LEN],
    counter: u32,
    namespaces: u64,
}

impl fmt::Debug for StorageNonceSequence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageNonceSequence")
            .field("counter", &self.counter)
            .field("namespaces", &self.namespaces)
            .finish_non_exhaustive()
    }
}

impl StorageNonceSequence {
    pub(crate) fn random() -> Result<Self, DurabilityError> {
        let mut prefix = [0_u8; NONCE_NAMESPACE_PREFIX_LEN];
        getrandom::fill(&mut prefix).map_err(|_| {
            crypto_protocol_error("OS CSPRNG failed while creating storage AEAD nonce namespace")
        })?;
        Ok(Self {
            prefix,
            counter: 0,
            namespaces: 1,
        })
    }

    pub(crate) fn next_nonce(&mut self) -> Result<[u8; NONCE_LEN], DurabilityError> {
        if self.counter == NONCES_PER_NAMESPACE {
            getrandom::fill(&mut self.prefix).map_err(|_| {
                crypto_protocol_error(
                    "OS CSPRNG failed while rotating storage AEAD nonce namespace",
                )
            })?;
            self.counter = 0;
            self.namespaces = self
                .namespaces
                .checked_add(1)
                .ok_or_else(|| crypto_protocol_error("storage AEAD nonce namespace exhausted"))?;
        }
        let counter = u16::try_from(self.counter)
            .map_err(|_| crypto_protocol_error("storage AEAD nonce counter overflow"))?;
        let mut nonce = [0_u8; NONCE_LEN];
        nonce[..NONCE_NAMESPACE_PREFIX_LEN].copy_from_slice(&self.prefix);
        nonce[NONCE_NAMESPACE_PREFIX_LEN..].copy_from_slice(&counter.to_le_bytes());
        self.counter += 1;
        Ok(nonce)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageEncryptionDomain {
    Section,
    Wal,
    ImmutableObject,
}

impl StorageEncryptionDomain {
    const fn hkdf_info(self) -> &'static [u8] {
        match self {
            Self::Section => b"CFMD-AE-v1/section-key\0",
            Self::Wal => b"CFMD-AE-v1/wal-key\0",
            Self::ImmutableObject => b"CFMD-AE-v1/immutable-object-key\0",
        }
    }

    const fn aad_domain(self) -> &'static [u8] {
        match self {
            Self::Section => b"CFMD-AE-v1/section\0",
            Self::Wal => b"CFMD-AE-v1/wal\0",
            Self::ImmutableObject => b"CFMD-AE-v1/immutable-object\0",
        }
    }
}

#[derive(Clone)]
pub struct StorageAeadCodec {
    backend: BackendState,
    key_commitment: [u8; 16],
}

impl fmt::Debug for StorageAeadCodec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageAeadCodec")
            .field("algorithm", &self.algorithm())
            .field("backend_capabilities", &self.backend_capabilities())
            .field("key_commitment", &self.key_commitment)
            .finish_non_exhaustive()
    }
}

impl StorageAeadCodec {
    pub fn from_config(config: &StorageEncryption) -> Result<Option<Self>, DurabilityError> {
        match config {
            StorageEncryption::None => Ok(None),
            StorageEncryption::Direct { .. } | StorageEncryption::Wrapped { .. } => Err(
                crypto_protocol_error("storage AEAD codec requires persisted database salt"),
            ),
        }
    }

    pub(crate) fn with_salt(
        algorithm: StorageAeadAlgorithm,
        key: &StorageEncryptionKey,
        database_salt: &[u8; 32],
    ) -> Result<Self, DurabilityError> {
        match algorithm {
            StorageAeadAlgorithm::Aes256GcmSiv => Self::with_backend::<RustCryptoAes256GcmSiv>(
                key,
                database_salt,
                BackendState::aes256_gcm_siv,
            ),
        }
    }

    fn with_backend<B: StorageAeadBackend>(
        key: &StorageEncryptionKey,
        database_salt: &[u8; 32],
        assemble: impl FnOnce(
            cfmd_secure_memory::SecretHandle<B::Context>,
            cfmd_secure_memory::SecretHandle<B::Context>,
            cfmd_secure_memory::SecretHandle<B::Context>,
        ) -> BackendState,
    ) -> Result<Self, DurabilityError> {
        let mut hkdf = key
            .with_bytes(|bytes| SecureHkdfSha256::extract(database_salt, bytes))
            .map_err(secure_memory_error)?;
        let mut derived_key = SecureBytes::<KEY_LEN>::try_zeroed().map_err(secure_memory_error)?;

        derive_key_into(
            &mut hkdf,
            StorageEncryptionDomain::Section.hkdf_info(),
            &mut derived_key,
        );
        let section = B::initialize(&derived_key).map_err(secure_memory_error)?;

        derive_key_into(
            &mut hkdf,
            StorageEncryptionDomain::Wal.hkdf_info(),
            &mut derived_key,
        );
        let wal = B::initialize(&derived_key).map_err(secure_memory_error)?;

        derive_key_into(
            &mut hkdf,
            StorageEncryptionDomain::ImmutableObject.hkdf_info(),
            &mut derived_key,
        );
        let immutable_object = B::initialize(&derived_key).map_err(secure_memory_error)?;

        derive_key_into(&mut hkdf, KEY_COMMITMENT_DOMAIN, &mut derived_key);
        let key_commitment = derive_key_commitment(&derived_key)?;
        Ok(Self {
            backend: assemble(section, wal, immutable_object),
            key_commitment,
        })
    }

    #[must_use]
    pub const fn algorithm(&self) -> StorageAeadAlgorithm {
        self.backend.algorithm()
    }

    #[must_use]
    pub const fn backend_capabilities(&self) -> StorageAeadBackendCapabilities {
        self.backend.capabilities()
    }

    #[must_use]
    pub const fn key_commitment(&self) -> [u8; 16] {
        self.key_commitment
    }

    pub(crate) fn sealed_len(plaintext_len: usize) -> Result<usize, DurabilityError> {
        ENVELOPE_PREFIX_LEN
            .checked_add(plaintext_len)
            .and_then(|len| len.checked_add(TAG_LEN))
            .ok_or(DurabilityError::PayloadTooLarge)
    }

    pub fn seal(
        &self,
        domain: StorageEncryptionDomain,
        nonce: [u8; NONCE_LEN],
        context: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, DurabilityError> {
        let mut ciphertext = plaintext.to_vec();
        let aad = domain_aad(domain, context)?;
        let tag = self
            .backend
            .seal_detached(domain, &nonce, &aad, &mut ciphertext)
            .map_err(|_| crypto_protocol_error("storage AEAD encryption failed"))?;
        let capacity = ENVELOPE_PREFIX_LEN
            .checked_add(ciphertext.len())
            .and_then(|len| len.checked_add(TAG_LEN))
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let mut envelope = Vec::with_capacity(capacity);
        envelope.extend_from_slice(&AE_MAGIC);
        envelope.push(AE_VERSION);
        envelope.push(self.algorithm() as u8);
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&ciphertext);
        envelope.extend_from_slice(&tag);
        Ok(envelope)
    }

    pub fn open(
        &self,
        domain: StorageEncryptionDomain,
        context: &[u8],
        envelope: &[u8],
    ) -> Result<Vec<u8>, DurabilityError> {
        if envelope.len() < ENVELOPE_PREFIX_LEN + TAG_LEN {
            return Err(crypto_corruption("storage AEAD envelope is truncated"));
        }
        if envelope[..4] != AE_MAGIC || envelope[4] != AE_VERSION {
            return Err(crypto_corruption("storage AEAD envelope header mismatch"));
        }
        let algorithm = StorageAeadAlgorithm::decode(envelope[5])?;
        if algorithm != self.algorithm() {
            return Err(crypto_corruption("storage AEAD algorithm mismatch"));
        }
        let mut nonce = [0_u8; NONCE_LEN];
        nonce.copy_from_slice(&envelope[6..ENVELOPE_PREFIX_LEN]);
        let tag_offset = envelope.len() - TAG_LEN;
        let mut ciphertext = envelope[ENVELOPE_PREFIX_LEN..tag_offset].to_vec();
        let mut tag = [0_u8; TAG_LEN];
        tag.copy_from_slice(&envelope[tag_offset..]);
        let aad = domain_aad(domain, context)?;
        self.backend
            .open_detached(domain, &nonce, &aad, &mut ciphertext, &tag)
            .map_err(|_| crypto_corruption("storage AEAD authentication failed"))?;
        Ok(ciphertext)
    }
}

fn derive_key_commitment(key: &SecureBytes<KEY_LEN>) -> Result<[u8; 16], DurabilityError> {
    let mut state = SecureBox::try_new_with(Sha256::new).map_err(secure_memory_error)?;
    key.with_secret(|bytes| {
        state.with_secret_mut(|digest| Digest::update(digest, bytes));
    });
    let mut digest =
        SecureBox::<Output<Sha256>>::try_new_with(Default::default).map_err(secure_memory_error)?;
    state.with_secret_mut(|state| {
        digest.with_secret_mut(|output| Digest::finalize_into_reset(state, output));
    });
    let mut commitment = [0_u8; 16];
    digest.with_secret(|output| commitment.copy_from_slice(&output[..16]));
    Ok(commitment)
}

fn derive_key_into(hkdf: &mut SecureHkdfSha256, info: &[u8], output: &mut SecureBytes<KEY_LEN>) {
    hkdf.expand_one_block_into(info, output);
}

fn domain_aad(domain: StorageEncryptionDomain, context: &[u8]) -> Result<Vec<u8>, DurabilityError> {
    let prefix = domain.aad_domain();
    let capacity = prefix
        .len()
        .checked_add(context.len())
        .ok_or(DurabilityError::PayloadTooLarge)?;
    let mut aad = Vec::with_capacity(capacity);
    aad.extend_from_slice(prefix);
    aad.extend_from_slice(context);
    Ok(aad)
}

fn secure_memory_error(error: SecureMemoryError) -> DurabilityError {
    match error {
        SecureMemoryError::Platform { operation, source } => {
            DurabilityError::Io(std::io::Error::new(
                source.kind(),
                format!("secure memory {operation}: {source}"),
            ))
        }
        SecureMemoryError::UnsupportedPlatform => {
            crypto_protocol_error("hardened secret memory is unsupported on this platform")
        }
        SecureMemoryError::EmptySecret => {
            crypto_protocol_error("hardened secret memory rejected an empty secret")
        }
    }
}

const fn crypto_protocol_error(reason: &'static str) -> DurabilityError {
    DurabilityError::Protocol { offset: 0, reason }
}

const fn crypto_corruption(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

#[cfg(test)]
mod tests {
    use aes_gcm_siv::{Aes256GcmSiv, aead::KeyInit};

    use super::*;

    #[test]
    #[ignore = "manual R&D lifecycle microbenchmark"]
    fn secure_codec_initialization_microbench() {
        use std::{hint::black_box, time::Instant};

        use hkdf::Hkdf;

        const ITERATIONS: u32 = 1_000;
        let raw_key = [0x51_u8; KEY_LEN];
        let secure_key = StorageEncryptionKey::try_new(raw_key).unwrap();
        let salt = [0xA5_u8; 32];

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let hkdf = Hkdf::<Sha256>::new(Some(&salt), &raw_key);
            let mut section = [0_u8; KEY_LEN];
            let mut wal = [0_u8; KEY_LEN];
            let mut immutable = [0_u8; KEY_LEN];
            let mut commitment = [0_u8; KEY_LEN];
            hkdf.expand(StorageEncryptionDomain::Section.hkdf_info(), &mut section)
                .unwrap();
            hkdf.expand(StorageEncryptionDomain::Wal.hkdf_info(), &mut wal)
                .unwrap();
            hkdf.expand(
                StorageEncryptionDomain::ImmutableObject.hkdf_info(),
                &mut immutable,
            )
            .unwrap();
            hkdf.expand(KEY_COMMITMENT_DOMAIN, &mut commitment).unwrap();
            black_box(Aes256GcmSiv::new((&section).into()));
            black_box(Aes256GcmSiv::new((&wal).into()));
            black_box(Aes256GcmSiv::new((&immutable).into()));
            black_box(Sha256::digest(commitment));
        }
        let raw = started.elapsed();

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(
                StorageAeadCodec::with_salt(StorageAeadAlgorithm::Aes256GcmSiv, &secure_key, &salt)
                    .unwrap(),
            );
        }
        let secure = started.elapsed();

        println!(
            "codec_init raw={:.2} us/op secure={:.2} us/op ratio={:.2}",
            raw.as_secs_f64() * 1_000_000.0 / f64::from(ITERATIONS),
            secure.as_secs_f64() * 1_000_000.0 / f64::from(ITERATIONS),
            secure.as_secs_f64() / raw.as_secs_f64(),
        );
    }

    #[test]
    fn storage_encryption_key_clone_shares_protected_slot() {
        let key = StorageEncryptionKey::try_new([0x39; KEY_LEN]).unwrap();
        let clone = key.clone();
        assert!(Arc::ptr_eq(&key.0, &clone.0));
    }

    #[test]
    fn backend_security_capabilities_are_explicit_and_not_overclaimed() {
        let codec = StorageAeadCodec::with_salt(
            StorageAeadAlgorithm::Aes256GcmSiv,
            &StorageEncryptionKey::try_new([0x31; KEY_LEN]).unwrap(),
            &[0x42; 32],
        )
        .unwrap();
        let capabilities = codec.backend_capabilities();
        assert_eq!(
            capabilities.persistent_state(),
            PersistentSecretStateProtection::HardenedProcessMemory
        );
        assert_eq!(
            capabilities.initialization(),
            SecretStateInitializationProtection::ConstructorTransientsMayExist
        );
        assert!(!capabilities.is_strict_in_place());
    }

    #[test]
    fn encryption_mode_is_orthogonal_to_algorithm() {
        let key = StorageEncryptionKey::try_new([0x27; KEY_LEN]).unwrap();
        let direct = StorageEncryption::direct(StorageAeadAlgorithm::Aes256GcmSiv, key.clone());
        let wrapped =
            StorageEncryption::wrapped(StorageAeadAlgorithm::Aes256GcmSiv, key, [0x19; 16], 7, 3);
        assert_eq!(direct.algorithm(), Some(StorageAeadAlgorithm::Aes256GcmSiv));
        assert_eq!(
            wrapped.algorithm(),
            Some(StorageAeadAlgorithm::Aes256GcmSiv)
        );
    }

    #[test]
    fn aes_schedule_has_zeroizing_drop() {
        fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}
        assert_zeroize_on_drop::<aes::Aes256>();
    }

    fn codec(byte: u8) -> StorageAeadCodec {
        StorageAeadCodec::with_salt(
            StorageAeadAlgorithm::Aes256GcmSiv,
            &StorageEncryptionKey::try_new([byte; 32]).unwrap(),
            &[0xA5; 32],
        )
        .unwrap()
    }

    #[test]
    fn aes256_gcm_siv_round_trip_and_tamper_detection() {
        let codec = codec(7);
        let nonce = [3_u8; 12];
        let context = b"generation=12;kind=metadata;ordinal=0";
        let plaintext = b"classified metadata";
        let envelope = codec
            .seal(StorageEncryptionDomain::Section, nonce, context, plaintext)
            .unwrap();
        assert_ne!(envelope.as_slice(), plaintext);
        assert_eq!(
            codec
                .open(StorageEncryptionDomain::Section, context, &envelope)
                .unwrap(),
            plaintext
        );

        let mut tampered = envelope.clone();
        let index = ENVELOPE_PREFIX_LEN + 2;
        tampered[index] ^= 0x40;
        assert!(matches!(
            codec.open(StorageEncryptionDomain::Section, context, &tampered),
            Err(DurabilityError::Corruption { .. })
        ));
    }

    #[test]
    fn aad_domain_and_key_are_fail_closed() {
        let active_codec = codec(9);
        let wrong_key = codec(10);
        let nonce = [11_u8; 12];
        let envelope = active_codec
            .seal(
                StorageEncryptionDomain::Wal,
                nonce,
                b"lsn=41;revision=8;kind=prepare",
                b"payload",
            )
            .unwrap();

        assert!(
            active_codec
                .open(
                    StorageEncryptionDomain::Wal,
                    b"lsn=42;revision=8;kind=prepare",
                    &envelope
                )
                .is_err()
        );
        assert!(
            active_codec
                .open(
                    StorageEncryptionDomain::Section,
                    b"lsn=41;revision=8;kind=prepare",
                    &envelope
                )
                .is_err()
        );
        assert!(
            wrong_key
                .open(
                    StorageEncryptionDomain::Wal,
                    b"lsn=41;revision=8;kind=prepare",
                    &envelope
                )
                .is_err()
        );
    }

    #[test]
    fn wrapped_database_master_key_binds_provider_identity_epoch_and_publication() {
        let wrapping_key = StorageEncryptionKey::try_new([0x21; 32]).unwrap();
        let wrong_key = StorageEncryptionKey::try_new([0x22; 32]).unwrap();
        let master_key = StorageEncryptionKey::try_new([0x51; 32]).unwrap();
        let salt = [0xA7; 32];
        let provider_id = [0x19; 16];
        let wrapped =
            wrap_database_master_key(&wrapping_key, &salt, &provider_id, 7, 3, 9, &master_key)
                .unwrap();
        let unwrapped =
            unwrap_database_master_key(&wrapping_key, &salt, &provider_id, 7, 3, 9, &wrapped)
                .unwrap();
        assert!(unwrapped.with_bytes(|left| master_key.with_bytes(|right| left == right)));
        assert!(
            unwrap_database_master_key(&wrong_key, &salt, &provider_id, 7, 3, 9, &wrapped,)
                .is_err()
        );
        assert!(
            unwrap_database_master_key(&wrapping_key, &salt, &provider_id, 8, 3, 9, &wrapped,)
                .is_err()
        );
        assert!(
            unwrap_database_master_key(&wrapping_key, &salt, &provider_id, 7, 3, 10, &wrapped,)
                .is_err()
        );
    }

    #[test]
    fn nonce_sequence_is_exact_inside_namespace_and_rotates_at_bound() {
        let mut sequence = StorageNonceSequence::random().unwrap();
        let first = sequence.next_nonce().unwrap();
        let second = sequence.next_nonce().unwrap();
        assert_eq!(
            &first[..NONCE_NAMESPACE_PREFIX_LEN],
            &second[..NONCE_NAMESPACE_PREFIX_LEN]
        );
        assert_eq!(u16::from_le_bytes(first[10..12].try_into().unwrap()), 0);
        assert_eq!(u16::from_le_bytes(second[10..12].try_into().unwrap()), 1);

        sequence.counter = NONCES_PER_NAMESPACE;
        let namespaces_before = sequence.namespaces;
        let rotated = sequence.next_nonce().unwrap();
        assert_eq!(sequence.namespaces, namespaces_before + 1);
        assert_eq!(sequence.counter, 1);
        assert_eq!(u16::from_le_bytes(rotated[10..12].try_into().unwrap()), 0);
    }

    #[test]
    fn hkdf_separates_section_wal_and_immutable_object_keys() {
        let codec = codec(13);
        let nonce = [17_u8; 12];
        let context = b"same physical identity";
        let plaintext = b"same plaintext";
        let section = codec
            .seal(StorageEncryptionDomain::Section, nonce, context, plaintext)
            .unwrap();
        let wal = codec
            .seal(StorageEncryptionDomain::Wal, nonce, context, plaintext)
            .unwrap();
        let immutable_object = codec
            .seal(
                StorageEncryptionDomain::ImmutableObject,
                nonce,
                context,
                plaintext,
            )
            .unwrap();
        assert_ne!(section, wal);
        assert_ne!(section, immutable_object);
        assert_ne!(wal, immutable_object);
    }
}
