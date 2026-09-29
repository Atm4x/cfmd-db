use cfmd_secure_memory::{SecureBox, SecureBytes, SecureMemoryError};
use sha2::{Digest, Sha256, digest::Output};

const SHA256_BLOCK_LEN: usize = 64;
const SHA256_OUTPUT_LEN: usize = 32;
const HMAC_IPAD: u8 = 0x36;
const HMAC_OPAD: u8 = 0x5c;

struct Workspace {
    prk: [u8; SHA256_OUTPUT_LEN],
    pad: [u8; SHA256_BLOCK_LEN],
    digest: Sha256,
    intermediate: Output<Sha256>,
}

/// Fixed CFMD HKDF-SHA256 whose PRK, HMAC pads, digest state and intermediate
/// output share one hardened mapping. Expansion writes into a caller-owned
/// hardened output slot, avoiding one mmap/mlock cycle per HMAC temporary.
pub(super) struct SecureHkdfSha256 {
    workspace: SecureBox<Workspace>,
}

impl SecureHkdfSha256 {
    pub(super) fn extract(
        salt: &[u8; SHA256_OUTPUT_LEN],
        ikm: &[u8; SHA256_OUTPUT_LEN],
    ) -> Result<Self, SecureMemoryError> {
        let mut workspace = SecureBox::try_new_with(|| Workspace {
            prk: [0; SHA256_OUTPUT_LEN],
            pad: [0; SHA256_BLOCK_LEN],
            digest: Sha256::new(),
            intermediate: Output::<Sha256>::default(),
        })?;
        workspace.with_secret_mut(|state| {
            hmac_sha256(state, salt, &[ikm.as_slice()]);
            state.prk.copy_from_slice(&state.intermediate);
        });
        Ok(Self { workspace })
    }

    pub(super) fn expand_one_block_into(
        &mut self,
        info: &[u8],
        output: &mut SecureBytes<SHA256_OUTPUT_LEN>,
    ) {
        self.workspace.with_secret_mut(|state| {
            hmac_sha256_with_workspace_prk(state, &[info, &[1]]);
            output.with_secret_mut(|destination| {
                destination.copy_from_slice(&state.intermediate);
            });
        });
    }
}

fn hmac_sha256(state: &mut Workspace, key: &[u8; SHA256_OUTPUT_LEN], components: &[&[u8]]) {
    state.pad.fill(0);
    state.pad[..SHA256_OUTPUT_LEN].copy_from_slice(key);
    hmac_sha256_with_prepared_key_pad(state, components);
}

fn hmac_sha256_with_workspace_prk(state: &mut Workspace, components: &[&[u8]]) {
    state.pad.fill(0);
    state.pad[..SHA256_OUTPUT_LEN].copy_from_slice(&state.prk);
    hmac_sha256_with_prepared_key_pad(state, components);
}

fn hmac_sha256_with_prepared_key_pad(state: &mut Workspace, components: &[&[u8]]) {
    for byte in &mut state.pad {
        *byte ^= HMAC_IPAD;
    }

    Digest::update(&mut state.digest, state.pad.as_slice());
    for component in components {
        Digest::update(&mut state.digest, component);
    }
    Digest::finalize_into_reset(&mut state.digest, &mut state.intermediate);

    for byte in &mut state.pad {
        *byte ^= HMAC_IPAD ^ HMAC_OPAD;
    }
    Digest::update(&mut state.digest, state.pad.as_slice());
    Digest::update(&mut state.digest, state.intermediate.as_slice());
    Digest::finalize_into_reset(&mut state.digest, &mut state.intermediate);
}

#[cfg(test)]
mod tests {
    use hkdf::Hkdf;
    use sha2::Sha256;

    use super::*;

    #[test]
    fn fixed_cfmd_hkdf_matches_rustcrypto_reference() {
        let salt = [0xA5; SHA256_OUTPUT_LEN];
        let ikm = [0x5A; SHA256_OUTPUT_LEN];
        let info = b"CFMD-AE-v1/rnd03/compatibility\0";
        let reference = Hkdf::<Sha256>::new(Some(&salt), &ikm);
        let mut expected = [0_u8; SHA256_OUTPUT_LEN];
        reference.expand(info, &mut expected).unwrap();

        let mut secure = SecureHkdfSha256::extract(&salt, &ikm).unwrap();
        let mut derived = SecureBytes::<SHA256_OUTPUT_LEN>::try_zeroed().unwrap();
        secure.expand_one_block_into(info, &mut derived);
        derived.with_secret(|actual| assert_eq!(actual, &expected));
    }

    #[test]
    fn info_domain_separation_changes_output() {
        let salt = [0x11; SHA256_OUTPUT_LEN];
        let ikm = [0x22; SHA256_OUTPUT_LEN];
        let mut secure = SecureHkdfSha256::extract(&salt, &ikm).unwrap();
        let mut left = SecureBytes::<SHA256_OUTPUT_LEN>::try_zeroed().unwrap();
        let mut right = SecureBytes::<SHA256_OUTPUT_LEN>::try_zeroed().unwrap();
        secure.expand_one_block_into(b"left", &mut left);
        secure.expand_one_block_into(b"right", &mut right);
        left.with_secret(|left_bytes| {
            right.with_secret(|right_bytes| assert_ne!(left_bytes, right_bytes));
        });
    }
}
