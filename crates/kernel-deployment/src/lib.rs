#![forbid(unsafe_code)]

mod abi;
mod cursor;
mod limits;
mod package;
mod policy;
mod repository;
mod runtime;
mod verification;

pub use abi::*;
pub use limits::*;
pub use package::*;
pub use policy::*;
pub use repository::*;
pub use runtime::*;
pub use verification::*;

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use kernel_auth::{MemoryArtifactCas, sign_artifact_manifest};
    use kernel_auth::{TrustRootSet, sha256};
    use kernel_semantics::{BuiltinSemanticModuleSpec, EquivalenceModule};
    use kernel_semantics::{
        ImplementationArtifactDigest, RuntimeProfileDigest, SemanticContractIdentity,
    };
    use std::collections::BTreeSet;

    const CHECKER_ID: RefinementCheckerId = RefinementCheckerId([0x33; 32]);
    const FORMAT_ID: RefinementFormatId = RefinementFormatId([0x44; 32]);

    struct ExactProofChecker;

    impl RefinementChecker for ExactProofChecker {
        fn checker_id(&self) -> RefinementCheckerId {
            CHECKER_ID
        }

        fn check(
            &self,
            _contract: SemanticContractIdentity,
            artifact: ImplementationArtifactDigest,
            runtime: RuntimeProfileDigest,
            semantic_descriptor: &[u8],
            format: RefinementFormatId,
            proof: &[u8],
        ) -> Result<(), DeploymentError> {
            let mut expected = Vec::new();
            expected.extend_from_slice(b"proof-v1\0");
            expected.extend_from_slice(&artifact.0);
            expected.extend_from_slice(&runtime.0);
            expected.extend_from_slice(&sha256(semantic_descriptor).0);
            if format == FORMAT_ID && proof == expected {
                Ok(())
            } else {
                Err(DeploymentError::RefinementRejected)
            }
        }
    }

    fn proof_bytes(
        artifact: ImplementationArtifactDigest,
        runtime: RuntimeProfileDigest,
        descriptor: &[u8],
    ) -> Vec<u8> {
        let mut proof = Vec::new();
        proof.extend_from_slice(b"proof-v1\0");
        proof.extend_from_slice(&artifact.0);
        proof.extend_from_slice(&runtime.0);
        proof.extend_from_slice(&sha256(descriptor).0);
        proof
    }

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn runtime_profile() -> RuntimeProfileSpec {
        RuntimeProfileSpec::new(IsolationClass::SandboxedProcess, 4096, 4096, 10_000).unwrap()
    }

    fn defined_contract() -> SemanticContractIdentity {
        let spec = BuiltinSemanticModuleSpec::Equivalence {
            module: EquivalenceModule::TextExact,
            implementation_revision: 1,
        };
        SemanticContractIdentity::Defined(spec.contract())
    }

    fn package_fixture() -> (
        TrustRootSet,
        DeploymentPolicy,
        MemoryArtifactCas,
        SemanticPackageEnvelope,
        MemoryCheckerRegistry,
    ) {
        let signer = signing_key(7);
        let trust = TrustRootSet::bootstrap(1, &[*signer.verifying_key().as_bytes()]).unwrap();
        let runtime_profile = runtime_profile();
        let runtime_digest = runtime_profile.digest();
        let artifact = b"opaque executable bytes".to_vec();
        let descriptor = b"canonical semantic descriptor".to_vec();
        let manifest = sign_artifact_manifest(&signer, &artifact, runtime_digest, &descriptor);
        let artifact_id = ImplementationArtifactDigest(manifest.artifact_digest.0);
        let proof = RefinementProofEnvelope {
            checker: CHECKER_ID,
            format: FORMAT_ID,
            bytes: proof_bytes(artifact_id, runtime_digest, &descriptor),
        };
        let package = SemanticPackageEnvelope {
            manifest,
            runtime_profile,
            semantic_descriptor: descriptor,
            refinement: Some(proof),
        };
        let mut cas = MemoryArtifactCas::default();
        cas.insert(artifact);
        let policy = DeploymentPolicy::bootstrap(
            1,
            BTreeSet::from([runtime_digest]),
            BTreeSet::from([CHECKER_ID]),
        )
        .unwrap();
        let mut checkers = MemoryCheckerRegistry::default();
        checkers.insert(Box::new(ExactProofChecker));
        (trust, policy, cas, package, checkers)
    }

    #[test]
    fn canonical_package_roundtrip_is_exact() {
        let (_, _, _, package, _) = package_fixture();
        let encoded = package.encode().unwrap();
        let decoded = SemanticPackageEnvelope::decode(&encoded).unwrap();
        assert_eq!(decoded, package);
        assert_eq!(decoded.encode().unwrap(), encoded);
    }

    #[test]
    fn defined_contract_requires_authenticated_artifact_and_checked_refinement() {
        let (trust, policy, cas, package, checkers) = package_fixture();
        let verified = verify_external_package(
            &trust,
            &policy,
            &cas,
            package,
            defined_contract(),
            b"canonical semantic descriptor",
            &checkers,
        )
        .unwrap();
        assert_eq!(
            verified.checked_refinement.as_ref().unwrap().checker,
            CHECKER_ID
        );
    }

    #[test]
    fn proof_tamper_and_unknown_checker_are_rejected() {
        let (trust, policy, cas, mut package, checkers) = package_fixture();
        package.refinement.as_mut().unwrap().bytes[0] ^= 1;
        assert_eq!(
            verify_external_package(
                &trust,
                &policy,
                &cas,
                package.clone(),
                defined_contract(),
                b"canonical semantic descriptor",
                &checkers,
            )
            .unwrap_err(),
            DeploymentError::RefinementRejected
        );
        package.refinement.as_mut().unwrap().checker = RefinementCheckerId([0x99; 32]);
        assert_eq!(
            verify_external_package(
                &trust,
                &policy,
                &cas,
                package,
                defined_contract(),
                b"canonical semantic descriptor",
                &checkers,
            )
            .unwrap_err(),
            DeploymentError::RefinementCheckerForbidden
        );
    }

    #[test]
    fn runtime_profile_is_cryptographically_bound_and_policy_checked() {
        let (trust, policy, cas, mut package, checkers) = package_fixture();
        package.runtime_profile.max_fuel += 1;
        assert_eq!(
            verify_external_package(
                &trust,
                &policy,
                &cas,
                package,
                defined_contract(),
                b"canonical semantic descriptor",
                &checkers,
            )
            .unwrap_err(),
            DeploymentError::RuntimeProfileForbidden
        );
    }

    #[test]
    fn signed_policy_update_is_monotone_and_can_revoke_artifact() {
        let (trust, policy, _, package, _) = package_fixture();
        let signer = signing_key(7);
        let artifact = ImplementationArtifactDigest(package.manifest.artifact_digest.0);
        let next = DeploymentPolicy {
            epoch: 2,
            allowed_runtime_profiles: policy.allowed_runtime_profiles.clone(),
            revoked_artifacts: BTreeSet::from([artifact]),
            allowed_refinement_checkers: policy.allowed_refinement_checkers.clone(),
        };
        let update = sign_deployment_policy(&signer, 1, next.clone()).unwrap();
        assert_eq!(policy.apply_signed_update(&trust, &update).unwrap(), next);

        let stale = SignedDeploymentPolicy {
            current_epoch: 0,
            ..update
        };
        assert_eq!(
            policy.apply_signed_update(&trust, &stale).unwrap_err(),
            DeploymentError::InvalidPolicyEpoch
        );
    }

    #[derive(Debug)]
    struct EchoSandbox {
        profile: RuntimeProfileSpec,
    }

    impl SandboxedRuntime for EchoSandbox {
        fn profile(&self) -> RuntimeProfileSpec {
            self.profile
        }

        fn invoke(
            &mut self,
            _artifact: &[u8],
            encoded_request: &[u8],
        ) -> Result<Vec<u8>, DeploymentError> {
            let request = AbiRequest::decode(encoded_request, self.profile)?;
            AbiResponse {
                consumed_fuel: 10,
                payload: request.payload,
            }
            .encode(self.profile)
        }
    }

    #[test]
    fn sandbox_abi_is_bounded_and_runtime_profile_pinned() {
        let (trust, policy, cas, package, checkers) = package_fixture();
        let verified = verify_external_package(
            &trust,
            &policy,
            &cas,
            package,
            defined_contract(),
            b"canonical semantic descriptor",
            &checkers,
        )
        .unwrap();
        let request = AbiRequest {
            operation: AbiOperation::Equivalence,
            fuel_limit: 100,
            payload: b"hello".to_vec(),
        };
        let mut runtime = EchoSandbox {
            profile: runtime_profile(),
        };
        assert_eq!(
            verified.invoke(&mut runtime, &request).unwrap().payload,
            b"hello"
        );

        runtime.profile.max_fuel += 1;
        assert_eq!(
            verified.invoke(&mut runtime, &request).unwrap_err(),
            DeploymentError::RuntimeProfileMismatch
        );
    }

    #[test]
    fn abi_parser_rejects_trailing_garbage_and_fuel_escape() {
        let profile = runtime_profile();
        let request = AbiRequest {
            operation: AbiOperation::Ordering,
            fuel_limit: 100,
            payload: vec![1, 2, 3],
        };
        let mut encoded = request.encode(profile).unwrap();
        encoded.push(9);
        assert_eq!(
            AbiRequest::decode(&encoded, profile).unwrap_err(),
            DeploymentError::MalformedPackage
        );

        let oversized_fuel = AbiRequest {
            operation: AbiOperation::Ordering,
            fuel_limit: profile.max_fuel + 1,
            payload: Vec::new(),
        };
        assert_eq!(
            oversized_fuel.encode(profile).unwrap_err(),
            DeploymentError::FuelLimitExceeded
        );
    }

    #[test]
    fn caller_semantic_descriptor_is_part_of_the_authority_cut() {
        let (trust, policy, cas, package, checkers) = package_fixture();
        assert_eq!(
            verify_external_package(
                &trust,
                &policy,
                &cas,
                package,
                defined_contract(),
                b"different durable descriptor",
                &checkers,
            )
            .unwrap_err(),
            DeploymentError::SemanticDescriptorMismatch
        );
    }

    #[test]
    fn untrusted_repository_cannot_swap_package_for_requested_artifact() {
        let (trust, policy, cas, package, checkers) = package_fixture();
        let requested = ImplementationArtifactDigest(package.manifest.artifact_digest.0);
        let mut repository = MemoryPackageRepository::default();
        repository.insert(
            ImplementationArtifactDigest([0xA5; 32]),
            package.encode().unwrap(),
        );
        assert_eq!(
            verify_external_package_from_repository(
                &trust,
                &policy,
                &repository,
                &cas,
                PackageLookup {
                    artifact: requested,
                    contract: defined_contract(),
                    expected_semantic_descriptor: b"canonical semantic descriptor",
                },
                &checkers,
            )
            .unwrap_err(),
            DeploymentError::PackageUnavailable
        );

        repository.insert(requested, package.encode().unwrap());
        assert!(
            verify_external_package_from_repository(
                &trust,
                &policy,
                &repository,
                &cas,
                PackageLookup {
                    artifact: requested,
                    contract: defined_contract(),
                    expected_semantic_descriptor: b"canonical semantic descriptor",
                },
                &checkers,
            )
            .is_ok()
        );
    }

    #[test]
    fn deployment_policy_encoding_rejects_noncanonical_order() {
        let (trust, policy, _, _, _) = package_fixture();
        let signer = signing_key(7);
        let mut next = DeploymentPolicy {
            epoch: 2,
            allowed_runtime_profiles: policy.allowed_runtime_profiles.clone(),
            revoked_artifacts: BTreeSet::from([
                ImplementationArtifactDigest([1; 32]),
                ImplementationArtifactDigest([2; 32]),
            ]),
            allowed_refinement_checkers: policy.allowed_refinement_checkers.clone(),
        };
        let mut update = sign_deployment_policy(&signer, 1, next.clone()).unwrap();
        update.revoked_artifacts.reverse();
        let message = deployment_policy_message(&update).unwrap();
        update.signature = signer.sign(&message).to_bytes();
        assert_eq!(
            policy.apply_signed_update(&trust, &update).unwrap_err(),
            DeploymentError::NonCanonicalPolicy
        );

        next.revoked_artifacts.clear();
        assert!(sign_deployment_policy(&signer, 1, next).is_ok());
    }

    #[test]
    fn opaque_contract_requires_exact_authenticated_artifact_runtime_pair() {
        let (trust, policy, cas, mut package, checkers) = package_fixture();
        package.refinement = None;
        let contract = SemanticContractIdentity::OpaqueArtifact {
            artifact: ImplementationArtifactDigest(package.manifest.artifact_digest.0),
            runtime: package.runtime_profile.digest(),
        };
        assert!(
            verify_external_package(
                &trust,
                &policy,
                &cas,
                package,
                contract,
                b"canonical semantic descriptor",
                &checkers,
            )
            .is_ok()
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_runtime_tests {
    use super::*;

    #[test]
    fn linux_namespace_runtime_executes_abi_out_of_process() {
        let profile = RuntimeProfileSpec::new(IsolationClass::SandboxedProcess, 4096, 4096, 5_000)
            .expect("profile");
        let mut runtime = LinuxNamespaceProcessRuntime::new(profile).expect("runtime");
        let artifact = br"#!/usr/bin/python3
import sys
sys.stdin.buffer.read()
out=(1).to_bytes(2,'little')+(1).to_bytes(8,'little')+(2).to_bytes(4,'little')+b'ok'
sys.stdout.buffer.write(out)
";
        let request = AbiRequest {
            operation: AbiOperation::Equivalence,
            fuel_limit: 100,
            payload: b"request".to_vec(),
        };
        let raw = runtime
            .invoke(artifact, &request.encode(profile).expect("request"))
            .expect("sandbox");
        let response = AbiResponse::decode(&raw, profile).expect("response");
        assert_eq!(response.payload, b"ok");
        assert_eq!(response.consumed_fuel, 1);
    }

    #[test]
    fn linux_namespace_runtime_cannot_reach_host_loopback_listener() {
        use std::net::TcpListener;
        use std::thread;
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("addr").port();
        let profile = RuntimeProfileSpec::new(IsolationClass::SandboxedProcess, 4096, 4096, 5_000)
            .expect("profile");
        let mut runtime = LinuxNamespaceProcessRuntime::new(profile).expect("runtime");
        let artifact = format!(
            "#!/usr/bin/python3\nimport socket,sys\ns=socket.socket()\ns.settimeout(0.25)\ntry:\n s.connect(('127.0.0.1',{port}))\n payload=b'bad'\nexcept Exception:\n payload=b'ok'\nout=(1).to_bytes(2,'little')+(1).to_bytes(8,'little')+len(payload).to_bytes(4,'little')+payload\nsys.stdout.buffer.write(out)\n"
        );
        let request = AbiRequest {
            operation: AbiOperation::Equivalence,
            fuel_limit: 100,
            payload: Vec::new(),
        };
        let raw = runtime
            .invoke(
                artifact.as_bytes(),
                &request.encode(profile).expect("request"),
            )
            .expect("sandbox");
        let response = AbiResponse::decode(&raw, profile).expect("response");
        assert_eq!(response.payload, b"ok");
        thread::sleep(std::time::Duration::from_millis(20));
        assert!(
            matches!(listener.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}

#[cfg(test)]
mod filesystem_adapter_tests {
    use super::*;
    use crate::repository::hex_digest;
    use kernel_auth::{ArtifactCas, AuthError, sha256};
    use kernel_semantics::ImplementationArtifactDigest;
    use std::fs;

    #[test]
    fn filesystem_repository_and_cas_are_untrusted_byte_sources_only() {
        let root = std::env::temp_dir().join(format!("cfmd-deployment-fs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");

        let artifact_bytes = b"artifact-bytes";
        let artifact_sha = sha256(artifact_bytes);
        fs::write(
            root.join(format!("{}.artifact", hex_digest(&artifact_sha.0))),
            artifact_bytes,
        )
        .expect("artifact");
        let cas = FilesystemArtifactCas::new(&root);
        assert_eq!(
            cas.load_exact(artifact_sha, artifact_bytes.len() as u64)
                .expect("cas")
                .as_deref(),
            Some(artifact_bytes.as_slice())
        );

        let artifact = ImplementationArtifactDigest([0x4a; 32]);
        let package_bytes = b"untrusted-package-bytes";
        fs::write(
            root.join(format!("{}.cfmdspk", hex_digest(&artifact.0))),
            package_bytes,
        )
        .expect("package");
        let repository = FilesystemPackageRepository::new(&root);
        assert_eq!(
            repository
                .load_package_bounded(artifact, MAX_PACKAGE_BYTES)
                .expect("repository")
                .as_deref(),
            Some(package_bytes.as_slice())
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn filesystem_adapters_enforce_bounds_while_reading() {
        let root =
            std::env::temp_dir().join(format!("cfmd-deployment-fs-bounds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");

        let oversized = vec![0x5a; 4096];
        let artifact_sha = sha256(&oversized);
        fs::write(
            root.join(format!("{}.artifact", hex_digest(&artifact_sha.0))),
            &oversized,
        )
        .expect("artifact");
        let cas = FilesystemArtifactCas::new(&root);
        assert_eq!(
            cas.load_exact(artifact_sha, 32),
            Err(AuthError::CasCorruption)
        );

        let artifact = ImplementationArtifactDigest([0x6b; 32]);
        fs::write(
            root.join(format!("{}.cfmdspk", hex_digest(&artifact.0))),
            &oversized,
        )
        .expect("package");
        let repository = FilesystemPackageRepository::new(&root);
        assert_eq!(
            repository.load_package_bounded(artifact, 32),
            Err(DeploymentError::PackageTooLarge)
        );

        fs::remove_dir_all(root).expect("cleanup");
    }
}
