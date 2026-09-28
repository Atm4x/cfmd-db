#![forbid(unsafe_code)]

mod artifact;
mod core;
mod durable;
mod freshness;

pub use artifact::*;
pub use core::*;
pub use durable::*;
pub use freshness::*;

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use kernel_semantics::{
        ArtifactAuthenticationSet, RuntimeProfileDigest, SemanticContractIdentity,
        SemanticDeploymentError, SemanticDeploymentRegistry, SemanticExecutionPolicy,
        SemanticImplementationPackage,
    };
    use std::cell::Cell;
    use std::collections::BTreeSet;

    fn signing(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn trust(signers: &[&SigningKey]) -> TrustRootSet {
        let keys = signers
            .iter()
            .map(|key| key.verifying_key().to_bytes())
            .collect::<Vec<_>>();
        TrustRootSet::bootstrap(1, &keys).unwrap()
    }

    #[test]
    fn semantic_artifact_authentication_bridges_into_existing_deployment_boundary() {
        let signer = signing(7);
        let trust = trust(&[&signer]);
        let runtime = RuntimeProfileDigest([0xA5; 32]);
        let artifact = b"external semantic implementation bytes";
        let descriptor = b"opaque-contract-descriptor-v1";
        let manifest = sign_artifact_manifest(&signer, artifact, runtime, descriptor);
        let verified = verify_artifact_manifest(&trust, &manifest, artifact, descriptor).unwrap();

        let contract = SemanticContractIdentity::OpaqueArtifact {
            artifact: verified.artifact_digest,
            runtime,
        };
        let package = SemanticImplementationPackage {
            contract,
            artifact_digest: verified.artifact_digest,
            runtime_profile: runtime,
            refinement: None,
            executable: None,
        };
        let mut deployment = SemanticDeploymentRegistry::default();
        deployment.register(package);
        let policy = SemanticExecutionPolicy {
            allowed_runtime_profiles: BTreeSet::from([runtime]),
            revoked_artifacts: BTreeSet::new(),
            require_authentication: true,
        };

        let unauthenticated = deployment.authorize_artifact(
            verified.artifact_digest,
            &policy,
            &ArtifactAuthenticationSet::default(),
        );
        assert_eq!(
            unauthenticated,
            Err(SemanticDeploymentError::UnauthenticatedArtifact)
        );

        let mut authentications = ArtifactAuthenticationSet::default();
        verified.mark_authenticated(&mut authentications);
        assert!(
            deployment
                .authorize_artifact(verified.artifact_digest, &policy, &authentications)
                .is_ok()
        );
    }

    #[test]
    fn artifact_tamper_descriptor_tamper_and_signature_tamper_are_rejected() {
        let signer = signing(9);
        let trust = trust(&[&signer]);
        let runtime = RuntimeProfileDigest([2; 32]);
        let artifact = b"artifact";
        let descriptor = b"descriptor";
        let manifest = sign_artifact_manifest(&signer, artifact, runtime, descriptor);

        assert_eq!(
            verify_artifact_manifest(&trust, &manifest, b"artifacU", descriptor),
            Err(AuthError::ArtifactDigestMismatch)
        );
        assert_eq!(
            verify_artifact_manifest(&trust, &manifest, artifact, b"descriptoS"),
            Err(AuthError::SemanticDescriptorDigestMismatch)
        );
        let mut forged = manifest.clone();
        forged.signature[5] ^= 0x80;
        assert_eq!(
            verify_artifact_manifest(&trust, &forged, artifact, descriptor),
            Err(AuthError::InvalidSignature)
        );
    }

    struct CountingCas {
        calls: Cell<u32>,
    }

    impl ArtifactCas for CountingCas {
        fn load_exact(
            &self,
            _digest: Sha256Digest,
            _expected_len: u64,
        ) -> Result<Option<Vec<u8>>, AuthError> {
            self.calls.set(self.calls.get() + 1);
            Ok(None)
        }
    }

    #[test]
    fn unauthenticated_manifest_never_reaches_cas() {
        let signer = signing(10);
        let trust = trust(&[&signer]);
        let runtime = RuntimeProfileDigest([3; 32]);
        let descriptor = b"descriptor";
        let mut manifest = sign_artifact_manifest(&signer, b"artifact", runtime, descriptor);
        manifest.artifact_len = u64::MAX;
        let cas = CountingCas {
            calls: Cell::new(0),
        };

        assert_eq!(
            verify_artifact_from_cas(&trust, &cas, &manifest, descriptor),
            Err(AuthError::InvalidSignature)
        );
        assert_eq!(cas.calls.get(), 0);
    }

    #[test]
    fn cas_detects_missing_and_corrupted_objects() {
        let signer = signing(10);
        let trust = trust(&[&signer]);
        let runtime = RuntimeProfileDigest([3; 32]);
        let artifact = b"artifact".to_vec();
        let descriptor = b"descriptor";
        let manifest = sign_artifact_manifest(&signer, &artifact, runtime, descriptor);
        let empty = MemoryArtifactCas::default();
        assert_eq!(
            verify_artifact_from_cas(&trust, &empty, &manifest, descriptor),
            Err(AuthError::CasMiss)
        );
        let mut corrupt = MemoryArtifactCas::default();
        corrupt.insert_unchecked_for_test(manifest.artifact_digest, b"wrong".to_vec());
        assert_eq!(
            verify_artifact_from_cas(&trust, &corrupt, &manifest, descriptor),
            Err(AuthError::CasCorruption)
        );
        let mut valid = MemoryArtifactCas::default();
        assert_eq!(valid.insert(artifact), manifest.artifact_digest);
        assert!(verify_artifact_from_cas(&trust, &valid, &manifest, descriptor).is_ok());
    }

    #[test]
    fn signed_key_rotation_replaces_trust_without_backdoor_accepting_retired_key() {
        let old = signing(11);
        let replacement = signing(12);
        let initial = trust(&[&old]);
        let add_replacement = sign_trust_rotation(
            &old,
            1,
            2,
            vec![
                old.verifying_key().to_bytes(),
                replacement.verifying_key().to_bytes(),
            ],
        )
        .unwrap();
        let dual = initial.apply_rotation(&add_replacement).unwrap();
        assert!(dual.contains(key_id(old.verifying_key().as_bytes())));
        assert!(dual.contains(key_id(replacement.verifying_key().as_bytes())));

        let retire_old = sign_trust_rotation(
            &replacement,
            2,
            3,
            vec![replacement.verifying_key().to_bytes()],
        )
        .unwrap();
        let current = dual.apply_rotation(&retire_old).unwrap();
        assert!(!current.contains(key_id(old.verifying_key().as_bytes())));
        assert!(current.contains(key_id(replacement.verifying_key().as_bytes())));

        let runtime = RuntimeProfileDigest([4; 32]);
        let old_manifest = sign_artifact_manifest(&old, b"old", runtime, b"descriptor");
        assert_eq!(
            verify_artifact_manifest(&current, &old_manifest, b"old", b"descriptor"),
            Err(AuthError::UnknownSigner)
        );
        let new_manifest = sign_artifact_manifest(&replacement, b"new", runtime, b"descriptor");
        assert!(verify_artifact_manifest(&current, &new_manifest, b"new", b"descriptor").is_ok());
    }

    #[test]
    fn stale_or_wrongly_signed_rotation_is_rejected() {
        let trusted = signing(13);
        let outsider = signing(14);
        let initial = trust(&[&trusted]);
        let stale =
            sign_trust_rotation(&trusted, 0, 1, vec![trusted.verifying_key().to_bytes()]).unwrap();
        assert_eq!(
            initial.apply_rotation(&stale),
            Err(AuthError::InvalidRotationEpoch)
        );
        let outsider_update =
            sign_trust_rotation(&outsider, 1, 2, vec![outsider.verifying_key().to_bytes()])
                .unwrap();
        assert_eq!(
            initial.apply_rotation(&outsider_update),
            Err(AuthError::UnknownSigner)
        );
    }

    #[test]
    fn weak_root_key_is_rejected() {
        let identity_point = {
            let mut bytes = [0_u8; 32];
            bytes[0] = 1;
            bytes
        };
        assert_eq!(
            TrustRootSet::bootstrap(1, &[identity_point]),
            Err(AuthError::WeakVerifyingKey)
        );
    }

    fn components<'a>(
        manifest: &'a [u8],
        checkpoint: &'a [u8],
        metadata: &'a [u8],
    ) -> GenerationComponents<'a> {
        GenerationComponents {
            manifest,
            checkpoint,
            metadata,
            prepared_capsule: None,
        }
    }

    #[test]
    fn durable_generation_authentication_binds_all_published_components() {
        let signer = signing(20);
        let trust = trust(&[&signer]);
        let store_id = [0x55; 32];
        let record = sign_generation_record(
            &signer,
            store_id,
            8,
            None,
            components(b"manifest", b"checkpoint", b"metadata"),
        );
        let verified = verify_generation_record(
            &trust,
            &record,
            components(b"manifest", b"checkpoint", b"metadata"),
        )
        .unwrap();
        assert_eq!(verified.generation, 8);
        assert_eq!(
            verify_generation_record(
                &trust,
                &record,
                components(b"manifest", b"CHECKPOINT", b"metadata"),
            ),
            Err(AuthError::CheckpointDigestMismatch)
        );
    }

    #[test]
    fn generation_signature_prevents_cross_store_transplant() {
        let signer = signing(21);
        let trust = trust(&[&signer]);
        let mut record =
            sign_generation_record(&signer, [1; 32], 1, None, components(b"m", b"c", b"d"));
        record.store_id = [2; 32];
        assert_eq!(
            verify_generation_record(&trust, &record, components(b"m", b"c", b"d")),
            Err(AuthError::InvalidSignature)
        );
    }

    #[test]
    fn external_anchor_detects_rollback_fork_and_accepts_one_generation_catchup() {
        let signer = signing(22);
        let trust = trust(&[&signer]);
        let store_id = [0xA0; 32];
        let first_record = sign_generation_record(
            &signer,
            store_id,
            10,
            None,
            components(b"m10", b"c10", b"d10"),
        );
        let first =
            verify_generation_record(&trust, &first_record, components(b"m10", b"c10", b"d10"))
                .unwrap();
        let anchor = AnchoredGeneration::from(first);

        let second_record = sign_generation_record(
            &signer,
            store_id,
            11,
            Some(first.auth_digest),
            components(b"m11", b"c11", b"d11"),
        );
        let second =
            verify_generation_record(&trust, &second_record, components(b"m11", b"c11", b"d11"))
                .unwrap();
        assert_eq!(
            compare_with_anchor(Some(anchor), second),
            Ok(FreshnessDisposition::CatchUpOne)
        );
        assert_eq!(
            compare_with_anchor(Some(AnchoredGeneration::from(second)), first),
            Err(AuthError::RollbackDetected)
        );

        let fork_record = sign_generation_record(
            &signer,
            store_id,
            10,
            None,
            components(b"fork", b"c10", b"d10"),
        );
        let fork =
            verify_generation_record(&trust, &fork_record, components(b"fork", b"c10", b"d10"))
                .unwrap();
        assert_eq!(
            compare_with_anchor(Some(anchor), fork),
            Err(AuthError::AnchorForkDetected)
        );
    }

    #[test]
    fn wal_auth_chain_binds_exact_frame_bytes_and_contiguous_lsn() {
        let signer = signing(24);
        let trust = trust(&[&signer]);
        let store_id = [0xC0; 32];
        let generation = 7;
        let root = AuthorityDigest([0x11; 32]);
        let prepare_record =
            sign_wal_frame_record(&signer, store_id, generation, 41, root, b"prepare-frame");
        let prepare = verify_wal_frame_record(&trust, &prepare_record, b"prepare-frame").unwrap();
        let commit_record = sign_wal_frame_record(
            &signer,
            store_id,
            generation,
            42,
            prepare.auth_digest,
            b"commit-frame",
        );
        let commit = verify_wal_frame_record(&trust, &commit_record, b"commit-frame").unwrap();
        assert_eq!(
            verify_wal_extension(root, store_id, generation, 41, &[prepare, commit]),
            Ok(commit.auth_digest)
        );
        assert_eq!(
            verify_wal_frame_record(&trust, &prepare_record, b"PREPARE-frame"),
            Err(AuthError::WalFrameDigestMismatch)
        );
    }

    #[test]
    fn wal_auth_chain_rejects_missing_or_relinked_frame() {
        let signer = signing(25);
        let trust = trust(&[&signer]);
        let store_id = [0xD0; 32];
        let root = AuthorityDigest([0x22; 32]);
        let first_record = sign_wal_frame_record(&signer, store_id, 9, 1, root, b"frame-1");
        let first = verify_wal_frame_record(&trust, &first_record, b"frame-1").unwrap();
        let third_record =
            sign_wal_frame_record(&signer, store_id, 9, 3, first.auth_digest, b"frame-3");
        let third = verify_wal_frame_record(&trust, &third_record, b"frame-3").unwrap();
        assert_eq!(
            verify_wal_extension(root, store_id, 9, 1, &[first, third]),
            Err(AuthError::WalChainGap)
        );
        let wrong_link_record = sign_wal_frame_record(
            &signer,
            store_id,
            9,
            2,
            AuthorityDigest([0xEE; 32]),
            b"frame-2",
        );
        let wrong_link = verify_wal_frame_record(&trust, &wrong_link_record, b"frame-2").unwrap();
        assert_eq!(
            verify_wal_extension(root, store_id, 9, 1, &[first, wrong_link]),
            Err(AuthError::WalChainLinkMismatch)
        );
    }

    #[test]
    fn anchor_gap_is_not_silently_accepted() {
        let signer = signing(23);
        let trust = trust(&[&signer]);
        let store_id = [0xB0; 32];
        let anchor = AnchoredGeneration {
            store_id,
            generation: 3,
            auth_digest: AuthorityDigest([7; 32]),
        };
        let record = sign_generation_record(
            &signer,
            store_id,
            5,
            Some(anchor.auth_digest),
            components(b"m5", b"c5", b"d5"),
        );
        let local =
            verify_generation_record(&trust, &record, components(b"m5", b"c5", b"d5")).unwrap();
        assert_eq!(
            compare_with_anchor(Some(anchor), local),
            Err(AuthError::AnchorGap)
        );
    }
}
