use crate::binary_codec::{push_len, read_u16};
use crate::single_file::compaction_io::{
    SingleFileCompactionIo, SingleFileCompactionIoStep, SingleFileCompactionPrimitive,
};
use std::io::Read;
use std::process::Command;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signer, SigningKey};
use kernel_auth::{
    SignedFreshnessCut, TrustRootSet, freshness_record_digest, key_id, sign_freshness_cut,
};
use kernel_model::{DatabaseState, Value};
use kernel_realization::realize_database_state_factorized;
use kernel_revision::Revision;
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{BuiltinSemanticModuleSpec, EquivalenceModule, SemanticRegistry};
use kernel_types::{ClientTransactionId, RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};

use super::*;
use crate::{
    DurableRelationMutation, DurableRevisionChange, DurableRevisionDescriptor,
    DurableSequencerOrder, ReplicaId, ReplicatedEffectEnvelope, ReplicationAntiEntropyRequest,
    ReplicationBranchId, ReplicationDecisionLock, ReplicationDecisionVote, ReplicationEffectStage,
    ReplicationEffectVote, ReplicationFailureDetector, ReplicationHeartbeat,
    ReplicationIngestOutcome, ReplicationJointMembershipAck, ReplicationJointMembershipCertificate,
    ReplicationLeaderCertificate, ReplicationLeaderVote, ReplicationLockSummary,
    ReplicationMembership, ReplicationMembershipChange, ReplicationMembershipVote,
    ReplicationPeerAuthPolicy, ReplicationPeerEvidence, ReplicationQuorumAvailability,
    ReplicationQuorumCertificate, ReplicationQuorumLoss, ReplicationRecoveryAck,
    ReplicationRecoveryCertificate, ReplicationTermPromise, ReplicationTransportFrame,
    ReplicationTransportIngress, ReplicationTransportPayload, SignedReplicationPeerEvidence,
    SignedReplicationTransportFrame, SingleFileSectionKind, replicated_effect_id,
    replication_membership_digest, replication_peer_evidence_signing_message,
    replication_transport_signing_message,
};

static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(1);
const CRASH_WORKER_ENV: &str = "CFMD_CRASH_WORKER";
const CRASH_DIR_ENV: &str = "CFMD_CRASH_DIR";
const CRASH_POINT_ENV: &str = "CFMD_CRASH_POINT";
const CRASH_SINGLE_FILE_ENCRYPTED_ENV: &str = "CFMD_CRASH_SINGLE_FILE_ENCRYPTED";
const CRASH_READY_FILE: &str = ".cfmd-crash-ready";

#[derive(Debug)]
struct TestFreshnessAuthority {
    signing: SigningKey,
    current: Option<SignedFreshnessCut>,
}

impl ExternalFreshnessAuthority for TestFreshnessAuthority {
    fn read_signed(
        &mut self,
        _store_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessCut>, DurabilityError> {
        Ok(self.current.clone())
    }

    fn compare_and_advance_signed(
        &mut self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessCut,
    ) -> Result<SignedFreshnessCut, DurabilityError> {
        let current = self.current.as_ref().map(freshness_record_digest);
        if current != expected_record {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "test freshness CAS mismatch",
            });
        }
        let signed = sign_freshness_cut(&self.signing, next);
        self.current = Some(signed.clone());
        Ok(signed)
    }
}

const FRESHNESS_OK: u8 = 0;
const FRESHNESS_FAIL_READ: u8 = 1;
const FRESHNESS_FAIL_BEFORE_APPLY: u8 = 2;
const FRESHNESS_FAIL_AFTER_APPLY: u8 = 3;

#[derive(Debug, Clone)]
struct SharedFreshnessAuthority {
    signing: SigningKey,
    current: Arc<Mutex<Option<SignedFreshnessCut>>>,
    failure_mode: Arc<AtomicU8>,
}

impl SharedFreshnessAuthority {
    fn new(signing: SigningKey) -> Self {
        Self {
            signing,
            current: Arc::new(Mutex::new(None)),
            failure_mode: Arc::new(AtomicU8::new(FRESHNESS_OK)),
        }
    }

    fn boxed(&self) -> Box<dyn ExternalFreshnessAuthority> {
        Box::new(self.clone())
    }

    fn fail_once(&self, mode: u8) {
        self.failure_mode.store(mode, Ordering::SeqCst);
    }
}

impl ExternalFreshnessAuthority for SharedFreshnessAuthority {
    fn read_signed(
        &mut self,
        _store_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessCut>, DurabilityError> {
        if self.failure_mode.swap(FRESHNESS_OK, Ordering::SeqCst) == FRESHNESS_FAIL_READ {
            return Err(DurabilityError::Io(std::io::Error::other(
                "external freshness authority unavailable",
            )));
        }
        Ok(self.current.lock().unwrap().clone())
    }

    fn compare_and_advance_signed(
        &mut self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessCut,
    ) -> Result<SignedFreshnessCut, DurabilityError> {
        let mode = self.failure_mode.swap(FRESHNESS_OK, Ordering::SeqCst);
        if mode == FRESHNESS_FAIL_BEFORE_APPLY {
            return Err(DurabilityError::Io(std::io::Error::other(
                "external freshness authority failed before CAS apply",
            )));
        }
        let mut current = self.current.lock().unwrap();
        if current.as_ref().map(freshness_record_digest) != expected_record {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "shared freshness CAS mismatch",
            });
        }
        let signed = sign_freshness_cut(&self.signing, next);
        *current = Some(signed.clone());
        drop(current);
        if mode == FRESHNESS_FAIL_AFTER_APPLY {
            return Err(DurabilityError::Io(std::io::Error::other(
                "external freshness response lost after CAS apply",
            )));
        }
        Ok(signed)
    }
}

fn external_freshness_fixture(
    store_id: [u8; 32],
) -> (ExternalFreshnessConfig, SharedFreshnessAuthority) {
    let signing = SigningKey::from_bytes(&[92; 32]);
    let trust = TrustRootSet::bootstrap(13, &[signing.verifying_key().to_bytes()]).unwrap();
    (
        ExternalFreshnessConfig {
            store_id,
            trust_roots: trust,
            deployment_policy_epoch: 8,
        },
        SharedFreshnessAuthority::new(signing),
    )
}

struct BlockingKillFault {
    target: StoreFaultPoint,
    directory: PathBuf,
}

impl StoreFaultHook for BlockingKillFault {
    fn hit(&mut self, point: StoreFaultPoint) -> Result<(), DurabilityError> {
        if point == self.target {
            signal_crash_ready(&self.directory);
        }
        Ok(())
    }
}

struct ErrorFault {
    target: StoreFaultPoint,
}

impl StoreFaultHook for ErrorFault {
    fn hit(&mut self, point: StoreFaultPoint) -> Result<(), DurabilityError> {
        if point == self.target {
            return Err(DurabilityError::Io(std::io::Error::other(
                "injected checkpoint publication failure",
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrimitiveIoFaultMode {
    Fail,
    ShortThenFail,
    ShortOnce,
    ZeroProgress,
    InterruptedOnce,
}

struct FaultingSingleFileCompactionIo {
    target: SingleFileCompactionIoStep,
    mode: PrimitiveIoFaultMode,
    target_calls: u8,
}

impl FaultingSingleFileCompactionIo {
    const fn new(target: SingleFileCompactionIoStep, mode: PrimitiveIoFaultMode) -> Self {
        Self {
            target,
            mode,
            target_calls: 0,
        }
    }

    fn fail_now(&mut self, op: SingleFileCompactionIoStep) -> bool {
        if op != self.target {
            return false;
        }
        match self.mode {
            PrimitiveIoFaultMode::Fail if self.target_calls == 0 => {
                self.target_calls = 1;
                true
            }
            PrimitiveIoFaultMode::ShortThenFail if self.target_calls == 1 => {
                self.target_calls = 2;
                true
            }
            _ => false,
        }
    }

    fn short_len(&mut self, op: SingleFileCompactionIoStep, len: usize) -> Option<usize> {
        if op != self.target || len <= 1 || self.target_calls != 0 {
            return None;
        }
        if matches!(
            self.mode,
            PrimitiveIoFaultMode::ShortThenFail | PrimitiveIoFaultMode::ShortOnce
        ) {
            self.target_calls = 1;
            Some((len / 2).max(1))
        } else {
            None
        }
    }

    fn inject_zero_progress(&mut self, op: SingleFileCompactionIoStep) -> bool {
        if op == self.target
            && self.mode == PrimitiveIoFaultMode::ZeroProgress
            && self.target_calls == 0
        {
            self.target_calls = 1;
            true
        } else {
            false
        }
    }

    fn inject_interrupted(&mut self, op: SingleFileCompactionIoStep) -> bool {
        if op == self.target
            && self.mode == PrimitiveIoFaultMode::InterruptedOnce
            && self.target_calls == 0
        {
            self.target_calls = 1;
            true
        } else {
            false
        }
    }

    fn injected_error() -> std::io::Error {
        std::io::Error::other("injected single-file compaction primitive I/O failure")
    }
}

impl SingleFileCompactionIo for FaultingSingleFileCompactionIo {
    fn open_read(
        &mut self,
        step: SingleFileCompactionIoStep,
        path: &std::path::Path,
    ) -> std::io::Result<File> {
        if self.fail_now(step) {
            return Err(Self::injected_error());
        }
        File::open(path)
    }

    fn open_read_write(
        &mut self,
        step: SingleFileCompactionIoStep,
        path: &std::path::Path,
    ) -> std::io::Result<File> {
        if self.fail_now(step) {
            return Err(Self::injected_error());
        }
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
    }

    fn metadata_len(
        &mut self,
        step: SingleFileCompactionIoStep,
        file: &File,
    ) -> std::io::Result<u64> {
        if self.fail_now(step) {
            return Err(Self::injected_error());
        }
        Ok(file.metadata()?.len())
    }

    fn seek(
        &mut self,
        step: SingleFileCompactionIoStep,
        file: &mut File,
        position: std::io::SeekFrom,
    ) -> std::io::Result<u64> {
        if self.fail_now(step) {
            return Err(Self::injected_error());
        }
        std::io::Seek::seek(file, position)
    }

    fn read(
        &mut self,
        op: SingleFileCompactionIoStep,
        file: &mut File,
        bytes: &mut [u8],
    ) -> std::io::Result<usize> {
        if self.inject_interrupted(op) {
            return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
        }
        if self.fail_now(op) {
            return Err(Self::injected_error());
        }
        if self.inject_zero_progress(op) {
            return Ok(0);
        }
        if let Some(len) = self.short_len(op, bytes.len()) {
            return file.read(&mut bytes[..len]);
        }
        file.read(bytes)
    }

    fn write(
        &mut self,
        op: SingleFileCompactionIoStep,
        file: &mut File,
        bytes: &[u8],
    ) -> std::io::Result<usize> {
        if self.inject_interrupted(op) {
            return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
        }
        if self.fail_now(op) {
            return Err(Self::injected_error());
        }
        if self.inject_zero_progress(op) {
            return Ok(0);
        }
        if let Some(len) = self.short_len(op, bytes.len()) {
            return file.write(&bytes[..len]);
        }
        file.write(bytes)
    }

    fn sync_data(&mut self, op: SingleFileCompactionIoStep, file: &File) -> std::io::Result<()> {
        if self.fail_now(op) {
            return Err(Self::injected_error());
        }
        file.sync_data()
    }

    fn sync_all(&mut self, op: SingleFileCompactionIoStep, file: &File) -> std::io::Result<()> {
        if self.fail_now(op) {
            return Err(Self::injected_error());
        }
        file.sync_all()
    }

    fn set_len(
        &mut self,
        op: SingleFileCompactionIoStep,
        file: &File,
        len: u64,
    ) -> std::io::Result<()> {
        if self.fail_now(op) {
            return Err(Self::injected_error());
        }
        file.set_len(len)
    }
}

struct TornSingleFileRootFault {
    target: StoreFaultPoint,
    path: PathBuf,
}

impl StoreFaultHook for TornSingleFileRootFault {
    fn hit(&mut self, point: StoreFaultPoint) -> Result<(), DurabilityError> {
        if point == self.target {
            crate::single_file::test_corrupt_newest_root_slot(&self.path)?;
            return Err(DurabilityError::Io(std::io::Error::other(
                "injected torn single-file root publication",
            )));
        }
        Ok(())
    }
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cfmd-durability-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn setup_revision(id: u64, values: &[i64]) -> (Revision, SemanticRegistry, SemanticId) {
    let relation = SemanticId::new(1);
    let equivalence = SemanticId::new(2);
    let mut registry = SemanticRegistry::default();
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
    environment.pin_module(
        equivalence,
        registry.install_equivalence(EquivalenceModule::I64Exact),
    );
    let mut schema = Schema::new(SchemaRevisionId::new(1));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let mut state = DatabaseState::default();
    state.model.relations.insert(
        relation,
        values
            .iter()
            .map(|value| vec![Value::I64(*value)])
            .collect(),
    );
    (
        Revision::build(RevisionId::new(id), &context, &registry, state).unwrap(),
        registry,
        relation,
    )
}

#[test]
fn failed_prepare_validation_does_not_publish_semantic_modules() {
    let dir = test_dir("prepare-validation-semantic-registry-atomicity");
    let (base, registry, _) = setup_revision(10, &[1]);
    let (target, _, _) = setup_revision(11, &[1, 2]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let before = store.semantic_registry.clone();
    let mut descriptor = DurableRevisionDescriptor::full_revision(
        ClientTransactionId::new(0x0BAD_5EED),
        base.id(),
        &target,
        &registry,
    )
    .unwrap();
    let DurableTransactionIntent::FullRevision {
        encoded_target_revision,
        semantic_modules,
        ..
    } = &mut descriptor.intent
    else {
        unreachable!("full revision descriptor carries exact intent")
    };
    semantic_modules.push(BuiltinSemanticModuleSpec::Equivalence {
        module: EquivalenceModule::TextExact,
        implementation_revision: 1,
    });
    encoded_target_revision.clear();

    assert!(store.durably_prepare(&descriptor).is_err());
    assert_eq!(store.semantic_registry, before);
    assert!(!store.requires_recovery());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_replicated_ingest_does_not_publish_semantic_modules() {
    let dir = test_dir("replicated-ingest-semantic-registry-atomicity");
    let (base, registry, _) = setup_revision(12, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let before = store.semantic_registry.clone();
    let origin = ReplicaId::new(77);
    let id = replicated_effect_id(origin, 1);
    let envelope = ReplicatedEffectEnvelope {
        origin,
        origin_sequence: 1,
        branch: ReplicationBranchId::new(0),
        effect: DurableRevisionEffectRecord {
            id,
            prerequisites: BTreeSet::new(),
            transaction_epoch: IdempotencyEpoch::ZERO,
            transaction_id: ClientTransactionId::new(id.0),
            intent: DurableTransactionIntent::RelationData {
                source_revision: base.id(),
                target_revision: RevisionId::new(13),
                semantic_revision: base.semantic_revision(),
                relation_mutations: Vec::new(),
                client_guard_digest: None,
                causal_observations: Vec::new(),
                causal_observation_groups: Vec::new(),
                relational_causal_observations: Vec::new(),
                semantic_modules: vec![BuiltinSemanticModuleSpec::Equivalence {
                    module: EquivalenceModule::TextExact,
                    implementation_revision: 1,
                }],
            },
            change: DurableRevisionChange::RelationData {
                semantic_revision: base.semantic_revision(),
                relation_mutations: Vec::new(),
            },
            source_revision: base.id(),
            target_revision: RevisionId::new(13),
        },
        ordered_by: DurableSequencerOrder {
            sequencer: ReplicaId::new(99),
            epoch: 7,
            position: 1,
        },
    };

    assert!(matches!(
        store.durably_ingest_replicated_effect(envelope),
        Err(DurabilityError::Protocol {
            reason: "replication branch identity is zero",
            ..
        })
    ));
    assert_eq!(store.semantic_registry, before);
    assert!(!store.requires_recovery());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn external_freshness_blocks_plain_open_and_detects_generation_rollback() {
    let dir = test_dir("external-freshness-rollback");
    let (revision, registry, _) = setup_revision(990, &[1]);
    let signing = SigningKey::from_bytes(&[91; 32]);
    let trust = TrustRootSet::bootstrap(7, &[signing.verifying_key().to_bytes()]).unwrap();
    let config = ExternalFreshnessConfig {
        store_id: [17; 32],
        trust_roots: trust,
        deployment_policy_epoch: 4,
    };
    let authority = Box::new(TestFreshnessAuthority {
        signing,
        current: None,
    });
    let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
    let receipt = store
        .adopt_external_freshness(config.clone(), authority)
        .unwrap();
    assert_eq!(receipt.generation, 2);
    drop(store);

    assert!(matches!(
        DurableRevisionStore::open(&dir),
        Err(DurabilityError::Protocol {
            reason: "externally anchored store requires freshness-aware open",
            ..
        })
    ));
}

#[test]
fn external_freshness_recovers_cas_response_loss_before_and_after_apply() {
    let before_dir = test_dir("external-freshness-response-loss-before");
    let (revision, registry, relation) = setup_revision(1_100, &[1]);
    let (config, authority) = external_freshness_fixture([0x41; 32]);
    let mut store = DurableRevisionStore::create(&before_dir, &revision, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    let descriptor = committed_descriptor(&revision, &registry, relation, 1_101, 2);
    authority.fail_once(FRESHNESS_FAIL_BEFORE_APPLY);
    assert!(store.durably_prepare(&descriptor).is_err());
    assert!(store.requires_recovery());
    drop(store);
    let (recovered, _) =
        DurableRevisionStore::open_with_external_freshness(&before_dir, config, authority.boxed())
            .unwrap();
    assert!(!recovered.requires_recovery());
    drop(recovered);
    fs::remove_dir_all(before_dir).unwrap();

    let after_dir = test_dir("external-freshness-response-loss-after");
    let (revision, registry, relation) = setup_revision(1_200, &[1]);
    let (config, authority) = external_freshness_fixture([0x46; 32]);
    let mut store = DurableRevisionStore::create(&after_dir, &revision, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    let descriptor = committed_descriptor(&revision, &registry, relation, 1_201, 3);
    authority.fail_once(FRESHNESS_FAIL_AFTER_APPLY);
    assert!(store.durably_prepare(&descriptor).is_err());
    assert!(store.requires_recovery());
    drop(store);
    let (recovered, _) =
        DurableRevisionStore::open_with_external_freshness(&after_dir, config, authority.boxed())
            .unwrap();
    assert!(!recovered.requires_recovery());
    drop(recovered);
    fs::remove_dir_all(after_dir).unwrap();
}

#[test]
fn external_freshness_rejects_unavailable_stale_and_rolled_back_state_before_recovery() {
    let dir = test_dir("external-freshness-unavailable-rollback");
    let (revision, registry, _) = setup_revision(1_200, &[1]);
    let (config, authority) = external_freshness_fixture([0x42; 32]);
    let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    store.rotate_checkpoint(&revision).unwrap();
    drop(store);

    authority.fail_once(FRESHNESS_FAIL_READ);
    assert!(matches!(
        DurableRevisionStore::open_with_external_freshness(&dir, config.clone(), authority.boxed()),
        Err(DurabilityError::Io(_))
    ));

    let mut stale = config.clone();
    stale.deployment_policy_epoch += 1;
    assert!(matches!(
        DurableRevisionStore::open_with_external_freshness(&dir, stale, authority.boxed()),
        Err(DurabilityError::Protocol {
            reason: "external freshness policy identity mismatch",
            ..
        })
    ));

    let generation = 3_u64;
    for path in [
        manifest_path(&dir, generation),
        checkpoint_path(&dir, generation),
        metadata_path(&dir, generation),
        wal_path(&dir, generation),
        prepared_capsule_path(&dir, generation),
    ] {
        let _ = fs::remove_file(path);
    }
    assert!(matches!(
        DurableRevisionStore::open_with_external_freshness(&dir, config, authority.boxed()),
        Err(DurabilityError::Protocol {
            reason: "local durable generation was rolled back behind external freshness authority",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn external_freshness_rejects_same_generation_fork_and_wal_truncation() {
    let fork_dir = test_dir("external-freshness-generation-fork");
    let (revision, registry, _) = setup_revision(1_300, &[1]);
    let (config, authority) = external_freshness_fixture([0x43; 32]);
    let mut store = DurableRevisionStore::create(&fork_dir, &revision, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    drop(store);
    let checkpoint = checkpoint_chunk_path(&fork_dir, 2, 0);
    let mut bytes = fs::read(&checkpoint).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    fs::write(&checkpoint, bytes).unwrap();
    assert!(matches!(
        DurableRevisionStore::open_with_external_freshness(&fork_dir, config, authority.boxed()),
        Err(DurabilityError::Protocol {
            reason: "same-generation durable store fork detected by external freshness authority",
            ..
        })
    ));
    fs::remove_dir_all(fork_dir).unwrap();

    let wal_dir = test_dir("external-freshness-wal-truncation");
    let (revision, registry, relation) = setup_revision(1_400, &[1]);
    let (config, authority) = external_freshness_fixture([0x44; 32]);
    let mut store = DurableRevisionStore::create(&wal_dir, &revision, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    let descriptor = committed_descriptor(&revision, &registry, relation, 1_401, 2);
    store.durably_prepare(&descriptor).unwrap();
    drop(store);
    File::create(wal_path(&wal_dir, 2))
        .unwrap()
        .sync_all()
        .unwrap();
    assert!(matches!(
        DurableRevisionStore::open_with_external_freshness(&wal_dir, config, authority.boxed()),
        Err(DurabilityError::Protocol {
            reason: "local WAL was truncated behind external freshness authority",
            ..
        })
    ));
    fs::remove_dir_all(wal_dir).unwrap();
}

#[test]
fn external_freshness_generation_digest_ignores_unreferenced_chunk_files() {
    let dir = test_dir("external-freshness-unreferenced-chunk");
    let (revision, registry, _) = setup_revision(1_450, &[1]);
    let (config, authority) = external_freshness_fixture([0x47; 32]);
    let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    let generation = store.generation();
    drop(store);

    fs::write(
        checkpoint_chunk_path(&dir, generation, 99_999_999),
        b"orphan",
    )
    .unwrap();
    let (recovered, _) =
        DurableRevisionStore::open_with_external_freshness(&dir, config, authority.boxed())
            .unwrap();
    assert_eq!(recovered.generation(), generation);
    drop(recovered);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_checkpoint_chunk_names_are_not_generation_artifacts() {
    assert_eq!(
        parse_checkpoint_chunk_generation(
            "checkpoint-00000000000000000042-chunk-not-an-ordinal.cfck"
        ),
        None
    );
    assert_eq!(
        parse_checkpoint_chunk_generation("checkpoint-00000000000000000042-chunk-00000001.cfck"),
        Some(42)
    );
}

#[test]
fn external_freshness_rejects_authenticated_wal_prefix_fork() {
    let dir = test_dir("external-freshness-wal-prefix-fork");
    let (revision, registry, relation) = setup_revision(1_500, &[1]);
    let (config, authority) = external_freshness_fixture([0x45; 32]);
    let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    let descriptor = committed_descriptor(&revision, &registry, relation, 1_501, 2);
    store.durably_prepare(&descriptor).unwrap();
    drop(store);

    {
        let mut current = authority.current.lock().unwrap();
        let mut cut = current.as_ref().unwrap().cut;
        cut.wal_digest = AuthorityDigest([0xEE; 32]);
        *current = Some(sign_freshness_cut(&authority.signing, cut));
    }
    assert!(matches!(
        DurableRevisionStore::open_with_external_freshness(&dir, config, authority.boxed()),
        Err(DurabilityError::Protocol {
            reason: "local WAL prefix forks from external freshness authority",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn migration_complement_payload_survives_restart_and_release_is_persisted() {
    let dir = test_dir("migration-complement-retention");
    let (revision, registry, _) = setup_revision(901, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
    let durable = crate::DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: revision.semantic_revision().schema,
            target_schema: SchemaRevisionId::new(2),
            lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_001)),
            semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_002)),
            encoding_version: 1,
            complement: Value::I64(77),
        },
        kernel_lens::ComplementRetention::UntilEpoch(10),
    );
    store
        .stage_migration_complement(&revision, durable)
        .unwrap();
    drop(store);

    let (mut reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.migration_complements().len(), 1);
    assert_eq!(
        reopened.migration_complements()[0]
            .local_capsule()
            .unwrap()
            .complement,
        Value::I64(77)
    );
    assert!(
        reopened
            .release_due_migration_complements(&revision, 9)
            .unwrap()
            .is_none()
    );
    assert!(
        reopened
            .release_due_migration_complements(&revision, 10)
            .unwrap()
            .is_some()
    );
    drop(reopened);

    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert!(reopened.migration_complements()[0].released);
    assert!(
        reopened.migration_complements()[0]
            .local_capsule()
            .is_none()
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn external_archive_and_forget_never_persist_local_complement_authority() {
    let capsule = kernel_lens::ComplementCapsule {
        source_schema: SchemaRevisionId::new(1),
        target_schema: SchemaRevisionId::new(2),
        lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_101)),
        semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_102)),
        encoding_version: 1,
        complement: Value::I64(88),
    };
    let archive = crate::DurableMigrationComplement::from_capsule(
        capsule.clone(),
        kernel_lens::ComplementRetention::ExternalArchive(kernel_lens::ArchiveProofId(
            SemanticId::new(9_103),
        )),
    );
    let forgotten = crate::DurableMigrationComplement::from_capsule(
        capsule,
        kernel_lens::ComplementRetention::Forget,
    );
    assert!(archive.released && archive.local_complement.is_none());
    assert!(archive.archive_proof().is_some());
    assert!(forgotten.released && forgotten.local_complement.is_none());
}

fn durable_complement(
    source: u64,
    target: u64,
    payload: i64,
    retention: kernel_lens::ComplementRetention,
) -> crate::DurableMigrationComplement {
    crate::DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: SchemaRevisionId::new(source),
            target_schema: SchemaRevisionId::new(target),
            lens_spec: kernel_lens::LensSpecId(SemanticId::new(u128::from(9_300 + target))),
            semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(u128::from(
                9_400 + target,
            ))),
            encoding_version: 1,
            complement: Value::I64(payload),
        },
        retention,
    )
}

#[test]
fn hostile_prepared_capsule_count_cannot_preallocate_beyond_payload_structure() {
    let mut bytes = vec![0_u8; PREPARED_CAPSULE_HEADER_LEN];
    bytes[..4].copy_from_slice(&PREPARED_CAPSULE_MAGIC);
    bytes[4..6].copy_from_slice(&PREPARED_CAPSULE_VERSION.to_le_bytes());
    bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[12..16].copy_from_slice(&crc32c(&[]).to_le_bytes());
    assert!(matches!(
        decode_prepared_cut_capsule(&bytes),
        Err(DurabilityError::Corruption {
            reason: "prepared cut capsule entry count exceeds payload structure",
            ..
        })
    ));
}

#[test]
fn hostile_chunked_checkpoint_rejects_oversized_logical_length_before_allocation() {
    let dir = test_dir("hostile-checkpoint-logical-bound");
    let (revision, registry, _) = setup_revision(9_701, &[1]);
    let generation = 7;
    let mut root = vec![0_u8; CHECKPOINT_HEADER_LEN];
    root[..4].copy_from_slice(&CHECKPOINT_MAGIC);
    root[4..6].copy_from_slice(&CHECKPOINT_FORMAT_TAG.to_le_bytes());
    root[6..8].copy_from_slice(&0_u16.to_le_bytes());
    root[8..16].copy_from_slice(&(u64::try_from(MAX_CHECKPOINT_LEN).unwrap() + 1).to_le_bytes());
    root[16..24].copy_from_slice(&revision.id().raw().to_le_bytes());
    root[24..28].copy_from_slice(&crc32c(&[]).to_le_bytes());
    let header_crc = crc32c(&root[..28]);
    root[28..32].copy_from_slice(&header_crc.to_le_bytes());
    fs::write(checkpoint_path(&dir, generation), &root).unwrap();
    let manifest = ManifestRecord {
        generation,
        base_revision: revision.id(),
        published_head: revision.id(),
        wal_first_lsn: 1,
        published_tail_lsn: 0,
        checkpoint_crc32c: crc32c(&root),
        metadata_crc32c: 0,
        prepared_capsule_crc32c: 0,
    };
    assert!(matches!(
        read_checkpoint_generation(&dir, manifest, &registry),
        Err(DurabilityError::Corruption {
            reason: "checkpoint logical stream exceeds configured bound",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hostile_sparse_metadata_file_is_rejected_without_reading_physical_length() {
    let dir = test_dir("hostile-sparse-metadata-bound");
    let path = dir.join("hostile.cfdm");
    let mut header = [0_u8; METADATA_HEADER_LEN];
    header[..4].copy_from_slice(&METADATA_MAGIC);
    header[4..6].copy_from_slice(&METADATA_FILE_TAG.to_le_bytes());
    header[16..20].copy_from_slice(&crc32c(&[]).to_le_bytes());
    let mut file = File::create(&path).unwrap();
    file.write_all(&header).unwrap();
    file.set_len(16 * 1024 * 1024 * 1024).unwrap();
    drop(file);
    assert!(matches!(
        read_metadata_bytes_bounded(&path),
        Err(DurabilityError::Corruption {
            reason: "durable metadata file has trailing bytes",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hostile_sparse_checkpoint_root_is_rejected_without_reading_physical_length() {
    let dir = test_dir("hostile-sparse-checkpoint-root-bound");
    let path = dir.join("hostile.cfcp");
    let mut header = [0_u8; CHECKPOINT_HEADER_LEN];
    header[..4].copy_from_slice(&CHECKPOINT_MAGIC);
    header[4..6].copy_from_slice(&CHECKPOINT_FORMAT_TAG.to_le_bytes());
    header[24..28].copy_from_slice(&crc32c(&[]).to_le_bytes());
    let header_crc = crc32c(&header[..28]);
    header[28..32].copy_from_slice(&header_crc.to_le_bytes());
    let mut file = File::create(&path).unwrap();
    file.write_all(&header).unwrap();
    file.set_len(16 * 1024 * 1024 * 1024).unwrap();
    drop(file);
    assert!(matches!(
        read_checkpoint_root_bounded(&path),
        Err(DurabilityError::Corruption {
            reason: "checkpoint file has trailing bytes",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hostile_sparse_prepared_capsule_is_streamed_not_materialized() {
    let dir = test_dir("hostile-sparse-prepared-capsule-bound");
    let path = dir.join("hostile.cfpc");
    let mut header = [0_u8; PREPARED_CAPSULE_HEADER_LEN];
    header[..4].copy_from_slice(&PREPARED_CAPSULE_MAGIC);
    header[4..6].copy_from_slice(&PREPARED_CAPSULE_VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&crc32c(&[]).to_le_bytes());
    let mut file = File::create(&path).unwrap();
    file.write_all(&header).unwrap();
    file.set_len(16 * 1024 * 1024 * 1024).unwrap();
    drop(file);
    assert!(matches!(
        read_prepared_cut_capsule_file(&path, crc32c(&header)),
        Err(DurabilityError::Corruption {
            reason: "prepared cut capsule has trailing bytes",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn staged_complement_rolls_back_in_memory_before_manifest_publication() {
    let dir = test_dir("staged-complement-prepublish-rollback");
    let (revision, registry, _) = setup_revision(9_711, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
    let complement = durable_complement(1, 2, 51, kernel_lens::ComplementRetention::Forever);
    assert!(
        store
            .test_stage_migration_complement_with_hook(
                &revision,
                complement,
                &mut ErrorFault {
                    target: StoreFaultPoint::AfterCheckpointSync,
                },
            )
            .is_err()
    );
    assert!(!store.requires_recovery());
    assert!(store.migration_complements().is_empty());
    drop(store);
    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert!(reopened.migration_complements().is_empty());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn complement_release_rolls_back_in_memory_before_manifest_publication() {
    let dir = test_dir("complement-release-prepublish-rollback");
    let (revision, registry, _) = setup_revision(9_721, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
    let complement = durable_complement(1, 2, 61, kernel_lens::ComplementRetention::UntilEpoch(10));
    store
        .stage_migration_complement(&revision, complement.clone())
        .unwrap();
    assert!(
        store
            .test_release_due_migration_complements_with_hook(
                &revision,
                10,
                &mut ErrorFault {
                    target: StoreFaultPoint::AfterCheckpointSync,
                },
            )
            .is_err()
    );
    assert!(!store.requires_recovery());
    assert_eq!(
        store.migration_complements(),
        std::slice::from_ref(&complement)
    );
    drop(store);
    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(
        reopened.migration_complements(),
        std::slice::from_ref(&complement)
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn historical_chain_enforces_forget_archive_and_released_payload_boundaries() {
    let dir = test_dir("historical-chain-retention-boundary");
    let (revision, registry, _) = setup_revision(905, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
    store.migration_complements = vec![
        durable_complement(1, 2, 10, kernel_lens::ComplementRetention::Forever),
        durable_complement(2, 3, 20, kernel_lens::ComplementRetention::UntilEpoch(10)),
    ];
    store.migration_complement_index = migration_history::migration_complement_index(
        &store.migration_complements,
        store.checkpoint.semantic_revision().schema,
    )
    .unwrap();
    let chain = store
        .local_historical_complement_chain(SchemaRevisionId::new(1), SchemaRevisionId::new(3))
        .unwrap();
    assert_eq!(chain.steps().len(), 2);
    assert_eq!(
        chain.steps()[0].local_capsule().unwrap().complement,
        Value::I64(10)
    );
    assert_eq!(
        chain.steps()[1].local_capsule().unwrap().complement,
        Value::I64(20)
    );

    assert!(store.migration_complements[1].release_if_due(revision.id(), 10));
    assert_eq!(
        store
            .local_historical_complement_chain(SchemaRevisionId::new(1), SchemaRevisionId::new(3),),
        Err(crate::HistoricalComplementError::LocalPayloadReleased {
            source: SchemaRevisionId::new(2),
            target: SchemaRevisionId::new(3),
        })
    );

    let proof = kernel_lens::ArchiveProofId(SemanticId::new(9_999));
    store.migration_complements[1] = durable_complement(
        2,
        3,
        20,
        kernel_lens::ComplementRetention::ExternalArchive(proof),
    );
    assert_eq!(
        store
            .local_historical_complement_chain(SchemaRevisionId::new(1), SchemaRevisionId::new(3),),
        Err(crate::HistoricalComplementError::ExternalArchiveRequired(
            proof
        ))
    );

    store.migration_complements[1] =
        durable_complement(2, 3, 20, kernel_lens::ComplementRetention::Forget);
    assert_eq!(
        store
            .local_historical_complement_chain(SchemaRevisionId::new(1), SchemaRevisionId::new(3),),
        Err(crate::HistoricalComplementError::ExplicitlyForgotten {
            source: SchemaRevisionId::new(2),
            target: SchemaRevisionId::new(3),
        })
    );
    assert_eq!(
        store
            .local_historical_complement_chain(SchemaRevisionId::new(3), SchemaRevisionId::new(4),),
        Err(crate::HistoricalComplementError::PathNotFound {
            source: SchemaRevisionId::new(3),
            target: SchemaRevisionId::new(4),
        })
    );
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn schema_migration_wal_atomically_recovers_complement_authority() {
    let dir = test_dir("schema-migration-atomic-complement");
    let (base, registry, _) = setup_revision(911, &[1]);
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(912),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let complement = crate::DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: base.semantic_revision().schema,
            target_schema: target.semantic_revision().schema,
            lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_201)),
            semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_202)),
            encoding_version: 1,
            complement: Value::I64(99),
        },
        kernel_lens::ComplementRetention::Forever,
    );
    let descriptor = DurableRevisionDescriptor::schema_migration(
        kernel_types::ClientTransactionId::new(9_203),
        base.id(),
        &target,
        complement.clone(),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    assert_eq!(
        store.migration_complements(),
        std::slice::from_ref(&complement)
    );
    let anchor = store
        .historical_epoch_anchors()
        .values()
        .next()
        .copied()
        .unwrap();
    assert_eq!(anchor.source_revision, base.id());
    assert_eq!(anchor.source_schema, base.semantic_revision().schema);
    assert_eq!(anchor.generation, 1);
    drop(store);

    // No checkpoint rotation occurred after COMMIT. Reopen must recover
    // both the schema transition and complement from the same WAL tail.
    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(
        reopened.migration_complements(),
        std::slice::from_ref(&complement)
    );
    let recovered_anchor = reopened
        .historical_epoch_anchors()
        .get(&anchor.effect_id)
        .copied()
        .unwrap();
    assert_eq!(recovered_anchor, anchor);
    assert!(matches!(
        scan.transaction_intent(kernel_types::ClientTransactionId::new(9_203)),
        Some(crate::DurableCommittedTransaction {
            intent: crate::DurableClientIntent::SchemaMigration { .. },
            ..
        })
    ));
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn schema_migration_semantic_cutover_is_derived_from_checkpoint_frontier() {
    let dir = test_dir("schema-migration-physical-frontier");
    let (base, registry, _) = setup_revision(9_181, &[1]);
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(9_182),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_183),
        base.id(),
        &target,
        crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: base.semantic_revision().schema,
                target_schema: target.semantic_revision().schema,
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_184)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_185)),
                encoding_version: 1,
                complement: Value::Unit,
            },
            kernel_lens::ComplementRetention::Forget,
        ),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    let receipt = store.durably_commit(prepared).unwrap();
    assert_eq!(receipt.target_revision(), target.id());
    let effect_id = store
        .historical_epoch_anchors()
        .values()
        .next()
        .expect("migration source anchor")
        .effect_id;
    let state = store.schema_migration_physical_states().unwrap();
    assert_eq!(state.len(), 1);
    assert_eq!(state[0].effect_id, effect_id);
    assert_eq!(state[0].source_revision, base.id());
    assert_eq!(state[0].target_revision, target.id());
    assert!(matches!(
        state[0].authority,
        crate::MigrationPhysicalAuthority::WalForwardCutover {
            source_generation: 1,
            checkpoint_revision
        } if checkpoint_revision == base.id()
    ));
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert!(matches!(
        reopened.schema_migration_physical_states().unwrap()[0].authority,
        crate::MigrationPhysicalAuthority::WalForwardCutover { .. }
    ));
    let before_effect = reopened.revision_effect_record(effect_id).cloned().unwrap();
    let generation = reopened
        .materialize_pending_schema_migrations(&target)
        .unwrap()
        .expect("pending cutover must publish a native checkpoint")
        .generation;
    assert_eq!(generation, 2);
    assert!(matches!(
        reopened.schema_migration_physical_states().unwrap()[0].authority,
        crate::MigrationPhysicalAuthority::NativeCheckpoint {
            generation: 2,
            checkpoint_revision
        } if checkpoint_revision == target.id()
    ));
    assert_eq!(
        reopened.revision_effect_record(effect_id),
        Some(&before_effect),
        "physical materialization must not append or rewrite semantic history"
    );
    assert!(
        reopened
            .materialize_pending_schema_migrations(&target)
            .unwrap()
            .is_none(),
        "native checkpoint state must not churn generations"
    );
    drop(reopened);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.base_revision(), target.id());
    assert!(matches!(
        reopened.schema_migration_physical_states().unwrap()[0].authority,
        crate::MigrationPhysicalAuthority::NativeCheckpoint {
            generation: 2,
            checkpoint_revision
        } if checkpoint_revision == target.id()
    ));
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn schema_migration_epoch_anchor_pins_source_generation_across_checkpoint_compaction() {
    let dir = test_dir("schema-migration-epoch-anchor-compaction");
    let (base, registry, _) = setup_revision(913, &[1]);
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(914),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let complement = crate::DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: base.semantic_revision().schema,
            target_schema: target.semantic_revision().schema,
            lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_211)),
            semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_212)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    let descriptor = DurableRevisionDescriptor::schema_migration(
        kernel_types::ClientTransactionId::new(9_213),
        base.id(),
        &target,
        complement,
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    let anchor = *store.historical_epoch_anchors().values().next().unwrap();
    assert_eq!(anchor.generation, 1);

    store.rotate_checkpoint(&target).unwrap();
    assert_eq!(store.generation(), 2);
    store.compact_obsolete_generations().unwrap();

    assert!(super::generation_layout::manifest_path(&dir, 1).exists());
    assert!(super::generation_layout::checkpoint_path(&dir, 1).exists());
    assert!(super::generation_layout::wal_path(&dir, 1).exists());
    assert!(super::generation_layout::metadata_path(&dir, 1).exists());
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(
        reopened.historical_epoch_anchors().get(&anchor.effect_id),
        Some(&anchor)
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_active_historical_epoch_survives_native_compaction() {
    let dir = test_dir("single-file-active-migration-epoch-compaction");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(9_170, &[1]);
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(9_171),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_172),
        base.id(),
        &target,
        crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: base.semantic_revision().schema,
                target_schema: target.semantic_revision().schema,
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_173)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_174)),
                encoding_version: 1,
                complement: Value::Unit,
            },
            kernel_lens::ComplementRetention::Forget,
        ),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    let anchor = *store.historical_epoch_anchors().values().next().unwrap();
    assert_eq!(anchor.generation, store.generation());

    let material = store
        .historical_epoch_material(anchor.effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(material.generation(), 1);
    assert_eq!(material.checkpoint().id(), base.id());
    assert_eq!(material.recovery_scan().durable_revision(), target.id());

    store.compact_obsolete_generations().unwrap();
    let material = store
        .historical_epoch_material(anchor.effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(material.checkpoint().id(), base.id());
    assert_eq!(material.recovery_scan().durable_revision(), target.id());
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    let material = reopened
        .historical_epoch_material(anchor.effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(material.checkpoint().id(), base.id());
    assert_eq!(material.recovery_scan().durable_revision(), target.id());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_historical_epoch_survives_checkpoint_rotations_and_compaction() {
    let dir = test_dir("single-file-migration-epoch-archive");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(917, &[1]);
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(918),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_218),
        base.id(),
        &target,
        crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: base.semantic_revision().schema,
                target_schema: target.semantic_revision().schema,
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_216)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_217)),
                encoding_version: 1,
                complement: Value::Unit,
            },
            kernel_lens::ComplementRetention::Forget,
        ),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    let anchor = *store.historical_epoch_anchors().values().next().unwrap();
    assert_eq!(anchor.generation, 1);

    store.rotate_checkpoint(&target).unwrap();
    assert_eq!(store.generation(), 2);
    let material = store
        .historical_epoch_material(anchor.effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(material.generation(), 1);
    assert_eq!(material.checkpoint().id(), base.id());
    assert_eq!(material.recovery_scan().durable_revision(), target.id());

    store.rotate_checkpoint(&target).unwrap();
    assert_eq!(store.generation(), 3);
    store.compact_obsolete_generations().unwrap();
    let material = store
        .historical_epoch_material(anchor.effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(material.generation(), 1);
    assert_eq!(material.checkpoint().id(), base.id());
    assert_eq!(material.recovery_scan().durable_revision(), target.id());
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    let material = reopened
        .historical_epoch_material(anchor.effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(material.generation(), 1);
    assert_eq!(material.checkpoint().id(), base.id());
    assert_eq!(material.recovery_scan().durable_revision(), target.id());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn encrypted_single_file_historical_epoch_survives_rotations_compaction_and_reopen() {
    let dir = test_dir("encrypted-single-file-migration-epoch-archive");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(9_319, &[1]);
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(9_320),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_321),
        base.id(),
        &target,
        crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: base.semantic_revision().schema,
                target_schema: target.semantic_revision().schema,
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_322)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_323)),
                encoding_version: 1,
                complement: Value::Unit,
            },
            kernel_lens::ComplementRetention::Forget,
        ),
        &registry,
    )
    .unwrap();
    let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
        crate::storage_encryption::StorageEncryptionKey::try_new([0x86; 32]).unwrap(),
    );

    let mut store = DurableRevisionStore::create_single_file_with_encryption(
        &path,
        &encryption,
        &base,
        &registry,
    )
    .unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    let anchor = *store.historical_epoch_anchors().values().next().unwrap();

    store.rotate_checkpoint(&target).unwrap();
    store.rotate_checkpoint(&target).unwrap();
    store.compact_obsolete_generations().unwrap();
    let material = store
        .historical_epoch_material(anchor.effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(material.generation(), 1);
    assert_eq!(material.checkpoint().id(), base.id());
    assert_eq!(material.recovery_scan().durable_revision(), target.id());
    drop(store);

    let (mut reopened, scan) =
        DurableRevisionStore::open_single_file_with_encryption(&path, &encryption).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    let material = reopened
        .historical_epoch_material(anchor.effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(material.generation(), 1);
    assert_eq!(material.checkpoint().id(), base.id());
    assert_eq!(material.recovery_scan().durable_revision(), target.id());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn historical_epoch_closure_survives_torn_compaction_root_plaintext_and_encrypted() {
    for encrypted in [false, true] {
        for point in single_file_compaction_uncertain_root_points() {
            let dir = test_dir(&format!("historical-epoch-torn-root-{encrypted}-{point:?}"));
            let path = dir.join("database.cfmd");
            let (base, registry, _) = setup_revision(9_324, &[1]);
            let mut target_context = base.semantic_context().clone();
            target_context.schema.revision = SchemaRevisionId::new(2);
            let target = Revision::build(
                RevisionId::new(9_325),
                &target_context,
                &registry,
                base.state().clone(),
            )
            .unwrap();
            let descriptor = DurableRevisionDescriptor::schema_migration(
                ClientTransactionId::new(9_326),
                base.id(),
                &target,
                crate::DurableMigrationComplement::from_capsule(
                    kernel_lens::ComplementCapsule {
                        source_schema: base.semantic_revision().schema,
                        target_schema: target.semantic_revision().schema,
                        lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_327)),
                        semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_328)),
                        encoding_version: 1,
                        complement: Value::Unit,
                    },
                    kernel_lens::ComplementRetention::Forget,
                ),
                &registry,
            )
            .unwrap();
            let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
                crate::storage_encryption::StorageEncryptionKey::try_new([0x87; 32]).unwrap(),
            );
            let mut store = if encrypted {
                DurableRevisionStore::create_single_file_with_encryption(
                    &path,
                    &encryption,
                    &base,
                    &registry,
                )
                .unwrap()
            } else {
                DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap()
            };
            let prepared = store.durably_prepare(&descriptor).unwrap();
            store.durably_commit(prepared).unwrap();
            let anchor = *store.historical_epoch_anchors().values().next().unwrap();
            store.rotate_checkpoint(&target).unwrap();
            store.rotate_checkpoint(&target).unwrap();

            let mut hook = TornSingleFileRootFault {
                target: point,
                path: path.clone(),
            };
            assert!(
                store
                    .test_compact_obsolete_generations_with_hook(&mut hook)
                    .is_err(),
                "{point:?} encrypted={encrypted}"
            );
            assert!(store.poisoned, "{point:?} encrypted={encrypted}");
            drop(store);

            let (mut reopened, scan) = if encrypted {
                DurableRevisionStore::open_single_file_with_encryption(&path, &encryption).unwrap()
            } else {
                DurableRevisionStore::open_single_file(&path).unwrap()
            };
            assert_eq!(scan.durable_revision(), target.id());
            let material = reopened
                .historical_epoch_material(anchor.effect_id)
                .unwrap()
                .unwrap();
            assert_eq!(material.generation(), 1);
            assert_eq!(material.checkpoint().id(), base.id());
            assert_eq!(material.recovery_scan().durable_revision(), target.id());
            drop(reopened);
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn releasing_historical_epoch_authority_drops_single_file_archive_on_publication() {
    let dir = test_dir("single-file-release-migration-epoch-archive");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(9_330, &[1]);
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(9_331),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_332),
        base.id(),
        &target,
        crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: base.semantic_revision().schema,
                target_schema: target.semantic_revision().schema,
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_333)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_334)),
                encoding_version: 1,
                complement: Value::Unit,
            },
            kernel_lens::ComplementRetention::Forget,
        ),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    let anchor = *store.historical_epoch_anchors().values().next().unwrap();
    store.rotate_checkpoint(&target).unwrap();
    assert!(
        store
            .backend
            .single_file_container()
            .unwrap()
            .has_historical_epoch_archive(anchor.generation)
            .unwrap()
    );

    let receipt = store
        .release_historical_epoch_authority(&target, anchor.effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.generation, 3);
    assert!(
        !store
            .historical_epoch_anchors()
            .contains_key(&anchor.effect_id)
    );
    assert!(
        !store
            .backend
            .single_file_container()
            .unwrap()
            .has_historical_epoch_archive(anchor.generation)
            .unwrap()
    );
    assert!(
        store
            .historical_epoch_material(anchor.effect_id)
            .unwrap()
            .is_none()
    );
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert!(
        !reopened
            .historical_epoch_anchors()
            .contains_key(&anchor.effect_id)
    );
    assert!(
        reopened
            .historical_epoch_material(anchor.effect_id)
            .unwrap()
            .is_none()
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unpublished_historical_epoch_release_restores_anchor_in_memory() {
    let dir = test_dir("historical-epoch-release-unpublished");
    let (base, registry, _) = setup_revision(9_340, &[1]);
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(9_341),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_342),
        base.id(),
        &target,
        crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: base.semantic_revision().schema,
                target_schema: target.semantic_revision().schema,
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_343)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_344)),
                encoding_version: 1,
                complement: Value::Unit,
            },
            kernel_lens::ComplementRetention::Forget,
        ),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    let anchor = *store.historical_epoch_anchors().values().next().unwrap();
    let mut hook = ErrorFault {
        target: StoreFaultPoint::AfterCheckpointSync,
    };
    assert!(
        store
            .test_release_historical_epoch_authority_with_hook(
                &target,
                anchor.effect_id,
                &mut hook,
            )
            .is_err()
    );
    assert!(!store.poisoned);
    assert_eq!(
        store.historical_epoch_anchors().get(&anchor.effect_id),
        Some(&anchor)
    );
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(
        reopened.historical_epoch_anchors().get(&anchor.effect_id),
        Some(&anchor)
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn uncertain_historical_epoch_release_requires_recovery_and_does_not_resurrect_anchor() {
    for point in [
        StoreFaultPoint::AfterManifestRename,
        StoreFaultPoint::AfterManifestDirectorySync,
    ] {
        let dir = test_dir(&format!("historical-epoch-release-uncertain-{point:?}"));
        let (base, registry, _) = setup_revision(9_350, &[1]);
        let mut target_context = base.semantic_context().clone();
        target_context.schema.revision = SchemaRevisionId::new(2);
        let target = Revision::build(
            RevisionId::new(9_351),
            &target_context,
            &registry,
            base.state().clone(),
        )
        .unwrap();
        let descriptor = DurableRevisionDescriptor::schema_migration(
            ClientTransactionId::new(9_352),
            base.id(),
            &target,
            crate::DurableMigrationComplement::from_capsule(
                kernel_lens::ComplementCapsule {
                    source_schema: base.semantic_revision().schema,
                    target_schema: target.semantic_revision().schema,
                    lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_353)),
                    semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_354)),
                    encoding_version: 1,
                    complement: Value::Unit,
                },
                kernel_lens::ComplementRetention::Forget,
            ),
            &registry,
        )
        .unwrap();

        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let prepared = store.durably_prepare(&descriptor).unwrap();
        store.durably_commit(prepared).unwrap();
        let anchor = *store.historical_epoch_anchors().values().next().unwrap();
        let mut hook = ErrorFault { target: point };
        assert!(
            store
                .test_release_historical_epoch_authority_with_hook(
                    &target,
                    anchor.effect_id,
                    &mut hook,
                )
                .is_err(),
            "{point:?}"
        );
        assert!(store.poisoned, "{point:?}");
        drop(store);

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(scan.durable_revision(), target.id(), "{point:?}");
        assert!(
            !reopened
                .historical_epoch_anchors()
                .contains_key(&anchor.effect_id),
            "{point:?}"
        );
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn staged_schema_migration_complement_is_consumed_without_resurrection_or_conflict() {
    let dir = test_dir("schema-migration-staged-complement");
    let (base, registry, _) = setup_revision(915, &[1]);
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(916),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let complement = durable_complement(1, 2, 41, kernel_lens::ComplementRetention::Forever);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .stage_migration_complement(&base, complement.clone())
        .unwrap();

    let descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_151),
        base.id(),
        &target,
        complement.clone(),
        &registry,
    )
    .unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    assert_eq!(
        store.migration_complements(),
        std::slice::from_ref(&complement)
    );

    let conflicting = durable_complement(1, 2, 99, kernel_lens::ComplementRetention::Forever);
    let conflicting_target = Revision::build(
        RevisionId::new(917),
        &target_context,
        &registry,
        target.state().clone(),
    )
    .unwrap();
    let conflicting_descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_152),
        target.id(),
        &conflicting_target,
        conflicting,
        &registry,
    )
    .unwrap();
    assert!(matches!(
        store.durably_prepare(&conflicting_descriptor),
        Err(DurabilityError::Protocol {
            reason: "schema migration complement conflicts with staged durable authority",
            ..
        })
    ));
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(
        reopened.migration_complements(),
        std::slice::from_ref(&complement)
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn group_commit_advances_schema_migration_frontier_within_the_group() {
    let dir = test_dir("schema-migration-group-frontier");
    let (base, registry, _) = setup_revision(921, &[1]);

    let mut context_two = base.semantic_context().clone();
    context_two.schema.revision = SchemaRevisionId::new(2);
    let revision_two = Revision::build(
        RevisionId::new(922),
        &context_two,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let complement_one = durable_complement(1, 2, 10, kernel_lens::ComplementRetention::Forever);
    let descriptor_one = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_221),
        base.id(),
        &revision_two,
        complement_one.clone(),
        &registry,
    )
    .unwrap();

    let mut context_three = revision_two.semantic_context().clone();
    context_three.schema.revision = SchemaRevisionId::new(3);
    let revision_three = Revision::build(
        RevisionId::new(923),
        &context_three,
        &registry,
        revision_two.state().clone(),
    )
    .unwrap();
    let complement_two = durable_complement(2, 3, 20, kernel_lens::ComplementRetention::Forever);
    let descriptor_two = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_222),
        revision_two.id(),
        &revision_three,
        complement_two.clone(),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let receipts = store
        .durably_commit_group(&[descriptor_one, descriptor_two])
        .unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(store.durable_head, revision_three.id());
    assert_eq!(
        store.migration_complements(),
        &[complement_one.clone(), complement_two.clone()]
    );
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), revision_three.id());
    assert_eq!(
        reopened.migration_complements(),
        &[complement_one, complement_two]
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

fn crash_point_name(point: StoreFaultPoint) -> &'static str {
    match point {
        StoreFaultPoint::AfterCheckpointSync => "after-checkpoint-sync",
        StoreFaultPoint::AfterWalSync => "after-wal-sync",
        StoreFaultPoint::AfterMetadataSync => "after-metadata-sync",
        StoreFaultPoint::AfterPrerequisiteDirectorySync => "after-prerequisite-directory-sync",
        StoreFaultPoint::AfterPendingManifestSync => "after-pending-manifest-sync",
        StoreFaultPoint::AfterManifestRename => "after-manifest-rename",
        StoreFaultPoint::AfterManifestDirectorySync => "after-manifest-directory-sync",
        StoreFaultPoint::BeforeCompactionRemove => "before-compaction-remove",
        StoreFaultPoint::AfterCompactionRemove => "after-compaction-remove",
        StoreFaultPoint::AfterCompactionDirectorySync => "after-compaction-directory-sync",
        StoreFaultPoint::SingleFileCompaction(step) => step
            .crash_name()
            .expect("non-publication compaction step has no crash-point name"),
    }
}

fn parse_crash_point(raw: &str) -> StoreFaultPoint {
    let mut points = vec![
        StoreFaultPoint::AfterCheckpointSync,
        StoreFaultPoint::AfterWalSync,
        StoreFaultPoint::AfterMetadataSync,
        StoreFaultPoint::AfterPrerequisiteDirectorySync,
        StoreFaultPoint::AfterPendingManifestSync,
        StoreFaultPoint::AfterManifestRename,
        StoreFaultPoint::AfterManifestDirectorySync,
        StoreFaultPoint::BeforeCompactionRemove,
        StoreFaultPoint::AfterCompactionRemove,
        StoreFaultPoint::AfterCompactionDirectorySync,
    ];
    points.extend(
        SingleFileCompactionIoStep::ALL
            .iter()
            .copied()
            .filter(|step| step.crash_name().is_some())
            .map(StoreFaultPoint::SingleFileCompaction),
    );
    points
        .into_iter()
        .find(|point| crash_point_name(*point) == raw)
        .unwrap_or_else(|| panic!("unknown crash point {raw}"))
}

fn signal_crash_ready(dir: &Path) -> ! {
    let marker = dir.join(CRASH_READY_FILE);
    let file = File::create(marker).unwrap();
    file.sync_all().unwrap();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

fn run_crash_worker(test_name: &str, dir: &Path, point: &str) {
    let marker = dir.join(CRASH_READY_FILE);
    let _ = fs::remove_file(&marker);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .env(CRASH_WORKER_ENV, "1")
        .env(CRASH_DIR_ENV, dir)
        .env(CRASH_POINT_ENV, point)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.is_file() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("crash worker exited before killpoint: {status}");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "crash worker did not reach killpoint {point}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    child.kill().unwrap();
    let _ = child.wait().unwrap();
    let _ = fs::remove_file(marker);
}

fn run_single_file_compaction_crash_worker(dir: &Path, point: StoreFaultPoint, encrypted: bool) {
    let marker = dir.join(CRASH_READY_FILE);
    let _ = fs::remove_file(&marker);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("store::tests::crash_worker_single_file_compaction")
        .arg("--nocapture")
        .env(CRASH_WORKER_ENV, "1")
        .env(CRASH_DIR_ENV, dir)
        .env(CRASH_POINT_ENV, crash_point_name(point))
        .env(
            CRASH_SINGLE_FILE_ENCRYPTED_ENV,
            if encrypted { "1" } else { "0" },
        )
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.is_file() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("single-file compaction crash worker exited before killpoint: {status}");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "single-file compaction crash worker did not reach killpoint {point:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    child.kill().unwrap();
    let _ = child.wait().unwrap();
    let _ = fs::remove_file(marker);
}

fn single_file_compaction_fault_points() -> Vec<StoreFaultPoint> {
    SingleFileCompactionIoStep::ALL
        .iter()
        .copied()
        .filter(|step| step.is_publication_sync_boundary())
        .map(StoreFaultPoint::SingleFileCompaction)
        .collect()
}

fn single_file_compaction_uncertain_root_points() -> Vec<StoreFaultPoint> {
    SingleFileCompactionIoStep::ALL
        .iter()
        .copied()
        .filter(|step| step.is_uncertain_root_write())
        .map(StoreFaultPoint::SingleFileCompaction)
        .collect()
}

fn single_file_compaction_primitive_failure_cases()
-> Vec<(SingleFileCompactionIoStep, PrimitiveIoFaultMode)> {
    let mut cases = Vec::new();
    for &step in SingleFileCompactionIoStep::ALL {
        match step.primitive() {
            SingleFileCompactionPrimitive::Read => {
                cases.push((step, PrimitiveIoFaultMode::Fail));
                cases.push((step, PrimitiveIoFaultMode::ShortThenFail));
            }
            SingleFileCompactionPrimitive::Write => {
                cases.push((step, PrimitiveIoFaultMode::Fail));
                cases.push((step, PrimitiveIoFaultMode::ShortThenFail));
                if step.supports_zero_progress() {
                    cases.push((step, PrimitiveIoFaultMode::ZeroProgress));
                }
            }
            SingleFileCompactionPrimitive::OpenRead
            | SingleFileCompactionPrimitive::OpenReadWrite
            | SingleFileCompactionPrimitive::MetadataLen
            | SingleFileCompactionPrimitive::Seek
            | SingleFileCompactionPrimitive::SyncData
            | SingleFileCompactionPrimitive::SyncAll
            | SingleFileCompactionPrimitive::SetLen => {
                cases.push((step, PrimitiveIoFaultMode::Fail));
            }
        }
    }
    cases
}

fn single_file_compaction_short_progress_cases() -> Vec<SingleFileCompactionIoStep> {
    SingleFileCompactionIoStep::ALL
        .iter()
        .copied()
        .filter(|step| step.supports_short_progress())
        .collect()
}

fn committed_descriptor(
    base: &Revision,
    registry: &SemanticRegistry,
    relation: SemanticId,
    target_revision: u64,
    inserted: i64,
) -> DurableRevisionDescriptor {
    let mut state = base.state().clone();
    state
        .model
        .relations
        .entry(relation)
        .or_default()
        .push(vec![Value::I64(inserted)]);
    let target = Revision::build(
        RevisionId::new(target_revision),
        base.semantic_context(),
        registry,
        state,
    )
    .unwrap();
    DurableRevisionDescriptor::relation_data(
        ClientTransactionId::new(u128::from(target_revision)),
        base.id(),
        &target,
        base.semantic_revision(),
        vec![DurableRelationMutation {
            relation,
            inserted: vec![vec![Value::I64(inserted)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        registry,
    )
    .unwrap()
}

fn transition_from(
    base: &Revision,
    registry: &SemanticRegistry,
    relation: SemanticId,
    target_revision: u64,
    inserted: i64,
) -> (Revision, DurableRevisionDescriptor) {
    let mut state = base.state().clone();
    state
        .model
        .relations
        .entry(relation)
        .or_default()
        .push(vec![Value::I64(inserted)]);
    let target = Revision::build(
        RevisionId::new(target_revision),
        base.semantic_context(),
        registry,
        state,
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::relation_data(
        ClientTransactionId::new(u128::from(target_revision)),
        base.id(),
        &target,
        base.semantic_revision(),
        vec![DurableRelationMutation {
            relation,
            inserted: vec![vec![Value::I64(inserted)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        registry,
    )
    .unwrap();
    (target, descriptor)
}

#[test]
fn group_commit_rejects_migration_history_source_revisit_before_wal_mutation() {
    let dir = test_dir("schema-migration-group-source-revisit");
    let (base, registry, _) = setup_revision(925, &[1]);

    let mut context_two = base.semantic_context().clone();
    context_two.schema.revision = SchemaRevisionId::new(2);
    let revision_two = Revision::build(
        RevisionId::new(926),
        &context_two,
        &registry,
        base.state().clone(),
    )
    .unwrap();

    let mut context_back_to_one = revision_two.semantic_context().clone();
    context_back_to_one.schema.revision = SchemaRevisionId::new(1);
    let revision_back_to_one = Revision::build(
        RevisionId::new(927),
        &context_back_to_one,
        &registry,
        revision_two.state().clone(),
    )
    .unwrap();

    let mut context_three = revision_back_to_one.semantic_context().clone();
    context_three.schema.revision = SchemaRevisionId::new(3);
    let revision_three = Revision::build(
        RevisionId::new(928),
        &context_three,
        &registry,
        revision_back_to_one.state().clone(),
    )
    .unwrap();

    let descriptor_one = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_225),
        base.id(),
        &revision_two,
        durable_complement(1, 2, 10, kernel_lens::ComplementRetention::Forever),
        &registry,
    )
    .unwrap();
    let descriptor_two = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_226),
        revision_two.id(),
        &revision_back_to_one,
        durable_complement(2, 1, 20, kernel_lens::ComplementRetention::Forever),
        &registry,
    )
    .unwrap();
    let descriptor_three = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(9_227),
        revision_back_to_one.id(),
        &revision_three,
        durable_complement(1, 3, 30, kernel_lens::ComplementRetention::Forever),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let before_lsn = store.wal.last_lsn();
    assert!(matches!(
        store.durably_commit_group(&[descriptor_one, descriptor_two, descriptor_three]),
        Err(DurabilityError::Protocol {
            reason: "migration complement chain revisits a schema revision",
            ..
        })
    ));
    assert_eq!(store.wal.last_lsn(), before_lsn);
    assert_eq!(store.durable_head, base.id());
    assert!(store.migration_complements().is_empty());
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    assert!(reopened.migration_complements().is_empty());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_checkpoint_rejects_unencodable_chunk_count_before_sidecar_publication() {
    let dir = test_dir("streaming-chunk-count-bound");
    let values: Vec<i64> = (0..20_000).map(i64::from).collect();
    let (base, registry, _) = setup_revision(1, &values);
    assert!(checkpoint::encode_revision(&base).unwrap().len() > usize::from(u16::MAX));
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();

    assert!(matches!(
        store.begin_streaming_checkpoint_with_chunk_size(&base, 1),
        Err(DurabilityError::PayloadTooLarge)
    ));
    assert!(!store.has_streaming_checkpoint());
    assert!(!prepared_capsule_path(&dir, 2).exists());
    assert!(!metadata_path(&dir, 2).exists());
    assert!(!wal_path(&dir, 2).exists());

    drop(store);
    let (_reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn directory_streaming_checkpoint_uses_bounded_canonical_spool_and_cleans_it() {
    let dir = test_dir("streaming-canonical-spool");
    let values: Vec<i64> = (0..50_000).map(i64::from).collect();
    let (base, registry, _) = setup_revision(1, &values);
    let encoded_len = checkpoint::encoded_revision_len(&base).unwrap();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();

    let started = store
        .begin_streaming_checkpoint_with_chunk_size(&base, 64 * 1024)
        .unwrap();
    let spool = super::generation_layout::checkpoint_stream_spool_path(&dir, started.generation);
    assert!(spool.exists());
    assert_eq!(fs::metadata(&spool).unwrap().len(), encoded_len);
    assert!(!checkpoint_chunk_path(&dir, started.generation, 0).exists());

    let progress = store.write_streaming_checkpoint_chunks(1).unwrap();
    assert_eq!(progress.chunks_written, 1);
    assert!(checkpoint_chunk_path(&dir, started.generation, 0).exists());
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    store.finalize_streaming_checkpoint().unwrap();
    assert!(!spool.exists());

    drop(store);
    let (_reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn directory_recovery_removes_orphan_checkpoint_stream_spool() {
    let dir = test_dir("streaming-orphan-spool-recovery");
    let (base, registry, _) = setup_revision(1, &[1, 2, 3]);
    let store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    drop(store);

    let spool = super::generation_layout::checkpoint_stream_spool_path(&dir, 2);
    fs::write(&spool, b"unpublished scratch").unwrap();
    assert!(spool.exists());

    let (_reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    assert!(!spool.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn directory_streaming_checkpoint_rejects_canonical_spool_tamper() {
    let dir = test_dir("streaming-canonical-spool-tamper");
    let values: Vec<i64> = (0..20_000).map(i64::from).collect();
    let (base, registry, _) = setup_revision(1, &values);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let started = store
        .begin_streaming_checkpoint_with_chunk_size(&base, 64 * 1024)
        .unwrap();
    let spool = super::generation_layout::checkpoint_stream_spool_path(&dir, started.generation);

    let mut bytes = fs::read(&spool).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x80;
    fs::write(&spool, bytes).unwrap();

    assert!(matches!(
        store.write_streaming_checkpoint_chunks(usize::MAX),
        Err(DurabilityError::Corruption {
            reason: "checkpoint canonical spool changed during resumable publication",
            ..
        })
    ));
    drop(store);
    assert!(!spool.exists());

    let (_reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn premature_streaming_checkpoint_finalize_preserves_resumable_job() {
    let dir = test_dir("streaming-premature-finalize");
    let (base, registry, _) = setup_revision(1, &[1, 2, 3]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let started = store
        .begin_streaming_checkpoint_with_chunk_size(&base, 16)
        .unwrap();
    assert!(!started.ready_to_publish);
    assert!(matches!(
        store.finalize_streaming_checkpoint(),
        Err(DurabilityError::Protocol { .. })
    ));
    assert!(store.has_streaming_checkpoint());
    let progress = store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    assert!(progress.ready_to_publish);
    let receipt = store.finalize_streaming_checkpoint().unwrap();
    assert_eq!(receipt.base_revision, base.id());
    drop(store);
    let (_reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compaction_during_streaming_checkpoint_fails_closed_and_preserves_job() {
    let dir = test_dir("streaming-compaction-interlock");
    let (base, registry, _) = setup_revision(1, &[1, 2, 3]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let started = store
        .begin_streaming_checkpoint_with_chunk_size(&base, 16)
        .unwrap();
    store.write_streaming_checkpoint_chunks(1).unwrap();
    assert!(checkpoint_chunk_path(&dir, started.generation, 0).is_file());

    assert!(matches!(
        store.compact_obsolete_generations(),
        Err(DurabilityError::Protocol {
            reason: "generation compaction is blocked by streaming checkpoint job",
            ..
        })
    ));
    assert!(store.has_streaming_checkpoint());
    assert!(checkpoint_chunk_path(&dir, started.generation, 0).is_file());

    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    store.finalize_streaming_checkpoint().unwrap();
    drop(store);
    let (_reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_checkpoint_capsule_recovers_prepare_before_cut_commit_after_cut() {
    let dir = test_dir("streaming-cross-cut-prepare");
    let (base, registry, relation) = setup_revision(1, &[1]);
    let (_target, descriptor) = transition_from(&base, &registry, relation, 2, 2);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();

    let started = store
        .begin_streaming_checkpoint_with_chunk_size(&base, 32)
        .unwrap();
    assert_eq!(started.mirrored_lsn, prepared.prepare_lsn());
    assert!(store.has_streaming_checkpoint());
    store.write_streaming_checkpoint_chunks(1).unwrap();
    store.durably_commit(prepared).unwrap();
    let progress = store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    assert!(progress.ready_to_publish);
    let receipt = store.finalize_streaming_checkpoint().unwrap();
    assert_eq!(receipt.base_revision, base.id());
    assert_eq!(store.durable_head(), RevisionId::new(2));
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.checkpoint_revision().id(), base.id());
    assert_eq!(scan.durable_revision(), RevisionId::new(2));
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_checkpoint_allows_commits_during_build_and_after_publication() {
    let dir = test_dir("streaming-live-tail");
    let (base, registry, relation) = setup_revision(10, &[1]);
    let (r11, d11) = transition_from(&base, &registry, relation, 11, 2);
    let (r12, d12) = transition_from(&r11, &registry, relation, 12, 3);
    let (_r13, d13) = transition_from(&r12, &registry, relation, 13, 4);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .begin_streaming_checkpoint_with_chunk_size(&base, 24)
        .unwrap();
    store.write_streaming_checkpoint_chunks(1).unwrap();
    let p11 = store.durably_prepare(&d11).unwrap();
    store.durably_commit(p11).unwrap();
    store.write_streaming_checkpoint_chunks(1).unwrap();
    let p12 = store.durably_prepare(&d12).unwrap();
    store.durably_commit(p12).unwrap();
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    store.finalize_streaming_checkpoint().unwrap();
    assert_eq!(store.durable_head(), r12.id());

    // The manifest certifies publication at R12, but the same shadow WAL
    // becomes active and may legally grow beyond that certificate.
    let p13 = store.durably_prepare(&d13).unwrap();
    store.durably_commit(p13).unwrap();
    drop(store);
    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.checkpoint_revision().id(), base.id());
    assert_eq!(scan.durable_revision(), RevisionId::new(13));
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_checkpoint_rejects_unverified_shadow_wal_tail_before_publication() {
    let dir = test_dir("streaming-shadow-tail");
    let (base, registry, relation) = setup_revision(15, &[1]);
    let (_target, descriptor) = transition_from(&base, &registry, relation, 16, 2);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let started = store
        .begin_streaming_checkpoint_with_chunk_size(&base, 24)
        .unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    let shadow_path = wal_path(&dir, started.generation);
    let mut shadow = OpenOptions::new().append(true).open(shadow_path).unwrap();
    shadow.write_all(b"CF").unwrap();
    shadow.sync_all().unwrap();
    drop(shadow);

    assert!(matches!(
        store.finalize_streaming_checkpoint(),
        Err(DurabilityError::Protocol {
            reason: "checkpoint cut plus shadow WAL does not recover exact publish endpoint",
            ..
        })
    ));
    assert!(!store.requires_recovery());
    drop(store);
    let (_reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(16));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_checkpoint_chunk_corruption_blocks_publish_and_old_authority_survives() {
    let dir = test_dir("streaming-corrupt-chunk");
    let (base, registry, _) = setup_revision(20, &[1, 2, 3]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let progress = store
        .begin_streaming_checkpoint_with_chunk_size(&base, 16)
        .unwrap();
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    let chunk = checkpoint_chunk_path(&dir, progress.generation, 0);
    let mut bytes = fs::read(&chunk).unwrap();
    bytes[0] ^= 0x5a;
    fs::write(&chunk, bytes).unwrap();
    assert!(matches!(
        store.finalize_streaming_checkpoint(),
        Err(DurabilityError::Corruption { .. })
    ));
    assert!(!store.requires_recovery());
    drop(store);
    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_unpublished_shadow_job_does_not_poison_published_store() {
    let dir = test_dir("streaming-shadow-abort");
    let (base, registry, relation) = setup_revision(30, &[1]);
    let (_target, descriptor) = transition_from(&base, &registry, relation, 31, 2);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store.begin_streaming_checkpoint(&base).unwrap();
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    store.test_mark_streaming_checkpoint_failed();
    assert!(matches!(
        store.finalize_streaming_checkpoint(),
        Err(DurabilityError::Protocol { .. })
    ));
    assert!(!store.requires_recovery());
    drop(store);
    let (_reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(31));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_checkpoint_freshness_failure_keeps_live_authority_at_published_generation() {
    let dir = test_dir("streaming-freshness-live-authority-first");
    let (base, registry, _) = setup_revision(40, &[1, 2, 3]);
    let (config, authority) = external_freshness_fixture([0x5c; 32]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();

    let started = store
        .begin_streaming_checkpoint_with_chunk_size(&base, 32)
        .unwrap();
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();

    authority.fail_once(FRESHNESS_FAIL_BEFORE_APPLY);
    assert!(matches!(
        store.finalize_streaming_checkpoint(),
        Err(DurabilityError::Io(_))
    ));
    assert!(store.requires_recovery());
    assert!(!store.has_streaming_checkpoint());
    assert_eq!(store.generation, started.generation);
    assert_eq!(store.checkpoint.id(), base.id());
    assert_eq!(store.wal.last_lsn(), started.mirrored_lsn);
    drop(store);

    let (reopened, scan) =
        DurableRevisionStore::open_with_external_freshness(&dir, config, authority.boxed())
            .unwrap();
    assert_eq!(reopened.generation, started.generation);
    assert_eq!(reopened.checkpoint.id(), base.id());
    assert_eq!(scan.durable_revision(), base.id());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn synchronous_checkpoint_freshness_failure_keeps_live_authority_at_published_generation() {
    let dir = test_dir("checkpoint-freshness-live-authority-first");
    let (base, registry, _) = setup_revision(45, &[1, 2, 3]);
    let (config, authority) = external_freshness_fixture([0x5d; 32]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    let previous_generation = store.generation;

    authority.fail_once(FRESHNESS_FAIL_BEFORE_APPLY);
    assert!(matches!(
        store.rotate_checkpoint(&base),
        Err(DurabilityError::Io(_))
    ));
    assert!(store.requires_recovery());
    assert_eq!(store.generation, previous_generation + 1);
    assert_eq!(store.checkpoint.id(), base.id());
    assert_eq!(store.wal.last_lsn(), 0);
    drop(store);

    let (reopened, scan) =
        DurableRevisionStore::open_with_external_freshness(&dir, config, authority.boxed())
            .unwrap();
    assert_eq!(reopened.generation, previous_generation + 1);
    assert_eq!(reopened.checkpoint.id(), base.id());
    assert_eq!(scan.durable_revision(), base.id());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn group_commit_publishes_contiguous_chain_only_after_shared_barriers() {
    let dir = test_dir("group-commit-chain");
    let (base, registry, relation) = setup_revision(20, &[1]);
    let (middle, _, _) = setup_revision(21, &[1, 2]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let first = committed_descriptor(&base, &registry, relation, 21, 2);
    let second = committed_descriptor(&middle, &registry, relation, 22, 3);

    let receipts = store
        .durably_commit_group(&[first.clone(), second.clone()])
        .unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(store.durable_head(), RevisionId::new(22));
    assert_eq!(
        store.transaction_outcome(ClientTransactionId::new(21)),
        DurableTransactionOutcome::Committed {
            target_revision: RevisionId::new(21)
        }
    );
    assert_eq!(
        store.transaction_outcome(ClientTransactionId::new(22)),
        DurableTransactionOutcome::Committed {
            target_revision: RevisionId::new(22)
        }
    );
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(22));
    assert_eq!(scan.committed().len(), 2);
    assert_eq!(
        reopened.transaction_outcome(ClientTransactionId::new(21)),
        DurableTransactionOutcome::Committed {
            target_revision: RevisionId::new(21)
        }
    );
    assert_eq!(
        reopened.transaction_outcome(ClientTransactionId::new(22)),
        DurableTransactionOutcome::Committed {
            target_revision: RevisionId::new(22)
        }
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn group_commit_rejects_noncontiguous_or_duplicate_transaction_chain() {
    let dir = test_dir("group-commit-invalid");
    let (base, registry, relation) = setup_revision(30, &[1]);
    let (middle, _, _) = setup_revision(31, &[1, 2]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let first = committed_descriptor(&base, &registry, relation, 31, 2);
    let noncontiguous = committed_descriptor(&base, &registry, relation, 33, 3);
    assert!(matches!(
        store.durably_commit_group(&[first.clone(), noncontiguous]),
        Err(DurabilityError::Protocol {
            reason: "group commit descriptors are not a contiguous revision chain",
            ..
        })
    ));
    assert_eq!(store.durable_head(), RevisionId::new(30));

    let mut duplicate = committed_descriptor(&middle, &registry, relation, 32, 3);
    duplicate.transaction_id = first.transaction_id;
    assert!(matches!(
        store.durably_commit_group(&[first, duplicate]),
        Err(DurabilityError::Protocol {
            reason: "group commit repeats a transaction id",
            ..
        })
    ));
    assert_eq!(store.durable_head(), RevisionId::new(30));
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn async_commit_batcher_never_acknowledges_before_flush_and_retains_on_failure() {
    let dir = test_dir("group-commit-batcher");
    let (base, registry, relation) = setup_revision(40, &[1]);
    let (middle, _, _) = setup_revision(41, &[1, 2]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let first = committed_descriptor(&base, &registry, relation, 41, 2);
    let second = committed_descriptor(&middle, &registry, relation, 42, 3);
    let mut batcher = DurableCommitBatcher::new(DurableCommitBatchPolicy::new(2).unwrap());

    assert_eq!(
        batcher.enqueue(&store, first.clone()).unwrap(),
        DurableBatchEnqueueOutcome::Queued
    );
    assert_eq!(
        store.transaction_outcome(first.transaction_id),
        DurableTransactionOutcome::Unknown
    );
    assert_eq!(
        batcher.enqueue(&store, second.clone()).unwrap(),
        DurableBatchEnqueueOutcome::FlushRequired
    );
    assert_eq!(batcher.pending_len(), 2);

    // A failed flush does not discard exact pending descriptors.
    store.poisoned = true;
    assert!(matches!(
        batcher.flush(&mut store),
        Err(DurabilityError::Poisoned)
    ));
    assert_eq!(batcher.pending_len(), 2);
    store.poisoned = false;

    let receipts = batcher.flush(&mut store).unwrap();
    assert_eq!(receipts.len(), 2);
    assert!(batcher.is_empty());
    assert_eq!(store.durable_head(), RevisionId::new(42));
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(42));
    assert_eq!(scan.committed().len(), 2);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn commit_batcher_consumes_locally_committed_group_when_freshness_ack_fails() {
    let dir = test_dir("group-commit-batcher-freshness-outcome");
    let (base, registry, relation) = setup_revision(43, &[1]);
    let (middle, _, _) = setup_revision(44, &[1, 2]);
    let (config, authority) = external_freshness_fixture([0x5b; 32]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    let first = committed_descriptor(&base, &registry, relation, 44, 4);
    let second = committed_descriptor(&middle, &registry, relation, 45, 5);
    let mut batcher = DurableCommitBatcher::new(DurableCommitBatchPolicy::new(2).unwrap());
    batcher.enqueue(&store, first.clone()).unwrap();
    batcher.enqueue(&store, second.clone()).unwrap();

    authority.fail_once(FRESHNESS_FAIL_BEFORE_APPLY);
    assert!(matches!(
        batcher.flush(&mut store),
        Err(DurabilityError::Io(_))
    ));
    assert!(batcher.is_empty());
    assert!(store.requires_recovery());
    assert_eq!(store.durable_head(), second.target_revision);
    assert!(matches!(
        store.transaction_outcome(first.transaction_id),
        DurableTransactionOutcome::Committed { target_revision }
            if target_revision == first.target_revision
    ));
    assert!(matches!(
        store.transaction_outcome(second.transaction_id),
        DurableTransactionOutcome::Committed { target_revision }
            if target_revision == second.target_revision
    ));
    drop(store);

    let (reopened, scan) =
        DurableRevisionStore::open_with_external_freshness(&dir, config, authority.boxed())
            .unwrap();
    assert_eq!(scan.durable_revision(), second.target_revision);
    assert_eq!(reopened.durable_head(), second.target_revision);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn store_reopens_exact_checkpoint_and_committed_wal_tail() {
    let dir = test_dir("reopen");
    let (base, registry, relation) = setup_revision(10, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let (target, _, _) = setup_revision(11, &[1, 2]);
    let descriptor = DurableRevisionDescriptor::relation_data(
        ClientTransactionId::new(11),
        RevisionId::new(10),
        &target,
        base.semantic_revision(),
        vec![DurableRelationMutation {
            relation,
            inserted: vec![vec![Value::I64(2)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        &registry,
    )
    .unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    assert_eq!(store.durable_head(), RevisionId::new(11));
    assert_eq!(store.causal_coverage_root(), RevisionId::new(10));
    let effect_11 = *store
        .revision_effect_frontier(RevisionId::new(11))
        .unwrap()
        .iter()
        .next()
        .unwrap();
    assert_eq!(
        store.revision_effect_frontier(RevisionId::new(11)),
        Some(&BTreeSet::from([effect_11]))
    );
    let ideal = store
        .revision_effect_ideal(RevisionId::new(11))
        .unwrap()
        .unwrap();
    assert_eq!(ideal.events().len(), 1);
    assert_eq!(ideal.events()[&effect_11].payload, descriptor.intent);
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.checkpoint_revision(), &base);
    assert_eq!(reopened.durable_head(), RevisionId::new(11));
    assert_eq!(scan.durable_revision(), RevisionId::new(11));
    assert_eq!(
        reopened.revision_effect_frontier(RevisionId::new(11)),
        Some(&BTreeSet::from([effect_11]))
    );
    assert_eq!(
        reopened
            .revision_effect_record(effect_11)
            .unwrap()
            .transaction_id,
        ClientTransactionId::new(11)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn multi_parent_resolution_derives_exact_causal_cut_and_recovers_it() {
    let dir = test_dir("multi-parent-resolution-cut");
    let (base, registry, relation) = setup_revision(30, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();

    for (source, target, inserted) in [(30, 31, 2), (31, 32, 3)] {
        let (target_revision, _, _) = setup_revision(target, &[1, 2, 3]);
        let descriptor = DurableRevisionDescriptor::relation_data(
            ClientTransactionId::new(u128::from(target)),
            RevisionId::new(source),
            &target_revision,
            base.semantic_revision(),
            vec![DurableRelationMutation {
                relation,
                inserted: vec![vec![Value::I64(inserted)]],
                removed: Vec::new(),
                object_field_writes: Vec::new(),
                authorization: crate::DurableRelationAuthorization::default(),
            }],
            &registry,
        )
        .unwrap();
        let prepared = store.durably_prepare(&descriptor).unwrap();
        store.durably_commit(prepared).unwrap();
    }

    let (resolved, _, _) = setup_revision(33, &[1, 2, 3, 4]);
    let descriptor = DurableRevisionDescriptor::relation_resolution(
        ClientTransactionId::new(0x3033),
        RevisionId::new(32),
        &resolved,
        base.semantic_revision(),
        crate::DurableRelationResolution {
            relation_mutations: vec![DurableRelationMutation {
                relation,
                inserted: vec![vec![Value::I64(4)]],
                removed: Vec::new(),
                object_field_writes: Vec::new(),
                authorization: crate::DurableRelationAuthorization::default(),
            }],
            rewrite_intents: vec![crate::DurableRelationRewriteIntent {
                relation,
                rewrite_spec: SemanticId::new(0x301),
                law_set: SemanticId::new(0x302),
            }],
            causal_parents: vec![RevisionId::new(31), RevisionId::new(32)],
        },
        &registry,
    )
    .unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();

    let effect_31 = *store
        .revision_effect_frontier(RevisionId::new(31))
        .unwrap()
        .iter()
        .next()
        .unwrap();
    let effect_32 = *store
        .revision_effect_frontier(RevisionId::new(32))
        .unwrap()
        .iter()
        .next()
        .unwrap();
    let resolution_effect = *store
        .revision_effect_frontier(RevisionId::new(33))
        .unwrap()
        .iter()
        .next()
        .unwrap();
    let effect = store.revision_effect_record(resolution_effect).unwrap();
    assert_eq!(effect.kind(), crate::DurableEffectKind::RelationResolution);
    assert_eq!(
        effect.coordination_class(),
        crate::DurableEffectCoordinationClass::OpaqueNonConfluent
    );
    assert_eq!(effect.prerequisites, BTreeSet::from([effect_31, effect_32]));
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(33));
    assert!(matches!(
        reopened.transaction_intent(ClientTransactionId::new(0x3033)),
        Some(crate::DurableCommittedTransaction {
            intent: crate::DurableClientIntent::RelationResolution { causal_parents, .. },
            ..
        }) if causal_parents == &vec![RevisionId::new(31), RevisionId::new(32)]
    ));
    assert_eq!(
        reopened
            .revision_effect_record(resolution_effect)
            .unwrap()
            .prerequisites,
        BTreeSet::from([effect_31, effect_32])
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unpublished_checkpoint_failure_keeps_previous_authority_usable() {
    let safe_points = [
        StoreFaultPoint::AfterCheckpointSync,
        StoreFaultPoint::AfterWalSync,
        StoreFaultPoint::AfterMetadataSync,
        StoreFaultPoint::AfterPrerequisiteDirectorySync,
        StoreFaultPoint::AfterPendingManifestSync,
    ];

    for point in safe_points {
        let dir = test_dir(&format!("checkpoint-unpublished-{point:?}"));
        let (base, registry, relation) = setup_revision(20, &[1]);
        let (next, _, _) = setup_revision(21, &[1, 2]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let descriptor = committed_descriptor(&base, &registry, relation, 21, 2);
        let prepared = store.durably_prepare(&descriptor).unwrap();
        store.durably_commit(prepared).unwrap();
        let specs = store.materialization_specs().to_vec();
        let mut hook = ErrorFault { target: point };

        assert!(
            store
                .test_rotate_checkpoint_with_fault_policy(&next, &specs, &[], &[], &mut hook)
                .is_err(),
            "{point:?}"
        );
        assert!(!store.requires_recovery(), "{point:?}");
        assert_eq!(store.durable_head(), RevisionId::new(21), "{point:?}");
        assert_eq!(
            store.checkpoint_revision().id(),
            RevisionId::new(20),
            "{point:?}"
        );

        // The failed generation was never authoritative. A fresh rotation
        // may skip its orphan generation number and still publish normally.
        let receipt = store.rotate_checkpoint(&next).unwrap();
        assert!(receipt.generation >= 2, "{point:?}");
        assert_eq!(store.checkpoint_revision().id(), RevisionId::new(21));
        drop(store);
        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.durable_head(), RevisionId::new(21));
        assert!(scan.committed().is_empty());
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn manifest_publication_failure_requires_recovery() {
    let uncertain_points = [
        StoreFaultPoint::AfterManifestRename,
        StoreFaultPoint::AfterManifestDirectorySync,
    ];

    for point in uncertain_points {
        let dir = test_dir(&format!("checkpoint-uncertain-{point:?}"));
        let (base, registry, relation) = setup_revision(30, &[1]);
        let (next, _, _) = setup_revision(31, &[1, 2]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let descriptor = committed_descriptor(&base, &registry, relation, 31, 2);
        let prepared = store.durably_prepare(&descriptor).unwrap();
        store.durably_commit(prepared).unwrap();
        let specs = store.materialization_specs().to_vec();
        let mut hook = ErrorFault { target: point };

        assert!(
            store
                .test_rotate_checkpoint_with_fault_policy(&next, &specs, &[], &[], &mut hook)
                .is_err(),
            "{point:?}"
        );
        assert!(store.requires_recovery(), "{point:?}");
        assert!(matches!(
            store.rotate_checkpoint(&next),
            Err(DurabilityError::Poisoned)
        ));

        // Reopen is the only authority-resolution path. Depending on where
        // the injected acknowledgement failure occurred, generation 2 is
        // already visible and must be accepted as authoritative.
        drop(store);
        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.durable_head(), RevisionId::new(31), "{point:?}");
        assert_eq!(
            reopened.checkpoint_revision().id(),
            RevisionId::new(31),
            "{point:?}"
        );
        assert!(scan.committed().is_empty(), "{point:?}");
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn synchronous_directory_checkpoints_use_only_current_chunked_representation() {
    let dir = test_dir("synchronous-current-chunked-checkpoint");
    let (base, registry, _) = setup_revision(39_001, &[1, 2, 3]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();

    for generation in [1_u64, 2_u64] {
        if generation == 2 {
            store.rotate_checkpoint(&base).unwrap();
        }
        let root = read_checkpoint_root_bounded(&checkpoint_path(&dir, generation)).unwrap();
        assert_eq!(read_u16(&root[4..6]), CHECKPOINT_FORMAT_TAG);
        let chunk_count = usize::from(read_u16(&root[6..8]));
        assert!(chunk_count > 0);
        assert_eq!(
            root.len(),
            CHECKPOINT_HEADER_LEN
                + chunk_count * checkpoint_storage::CHECKPOINT_CHUNK_DESCRIPTOR_LEN
        );
        for ordinal in 0..chunk_count {
            assert!(checkpoint_chunk_path(&dir, generation, ordinal).is_file());
        }
    }

    drop(store);
    let (_, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn checkpoint_rotation_publishes_new_generation_and_resets_wal_base() {
    let dir = test_dir("rotate");
    let (base, registry, relation) = setup_revision(20, &[1]);
    let (next, _, _) = setup_revision(21, &[1, 2]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let descriptor = DurableRevisionDescriptor::relation_data(
        ClientTransactionId::new(21),
        RevisionId::new(20),
        &next,
        base.semantic_revision(),
        vec![DurableRelationMutation {
            relation,
            inserted: vec![vec![Value::I64(2)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        &registry,
    )
    .unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    let receipt = store.rotate_checkpoint(&next).unwrap();
    assert_eq!(receipt.generation, 2);
    assert_eq!(store.checkpoint_revision(), &next);
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.generation(), 2);
    assert_eq!(reopened.checkpoint_revision(), &next);
    assert_eq!(scan.base_revision(), RevisionId::new(21));
    assert!(scan.committed().is_empty());
    assert_eq!(reopened.causal_coverage_root(), RevisionId::new(20));
    let effect_21 = reopened
        .revision_effect_frontier(RevisionId::new(21))
        .unwrap();
    assert_eq!(effect_21.len(), 1);
    assert!(
        reopened
            .revision_effect_ideal(RevisionId::new(21))
            .unwrap()
            .is_some_and(|ideal| ideal.events().len() == 1)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unpublished_orphan_generation_is_ignored_and_next_rotation_skips_it() {
    let dir = test_dir("orphan");
    let (base, registry, _) = setup_revision(30, &[1]);
    let store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    drop(store);
    fs::copy(checkpoint_path(&dir, 1), checkpoint_path(&dir, 2)).unwrap();
    File::create(wal_path(&dir, 2)).unwrap().sync_all().unwrap();
    fs::write(
        dir.join("pending-manifest-00000000000000000002.tmp"),
        b"torn",
    )
    .unwrap();

    let (mut reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.generation(), 1);
    assert_eq!(scan.durable_revision(), RevisionId::new(30));
    let receipt = reopened.rotate_checkpoint(&base).unwrap();
    assert_eq!(receipt.generation, 3);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn published_manifest_does_not_fallback_when_checkpoint_is_corrupt() {
    let dir = test_dir("corrupt");
    let (base, registry, _) = setup_revision(40, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store.rotate_checkpoint(&base).unwrap();
    drop(store);
    let checkpoint = checkpoint_path(&dir, 2);
    let mut bytes = fs::read(&checkpoint).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x80;
    fs::write(checkpoint, bytes).unwrap();

    assert!(matches!(
        DurableRevisionStore::open(&dir),
        Err(DurabilityError::Corruption { .. })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unsupported_highest_manifest_format_never_falls_back() {
    let dir = test_dir("unsupported-highest-format");
    let (base, registry, _) = setup_revision(41, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store.rotate_checkpoint(&base).unwrap();
    drop(store);

    let path = manifest_path(&dir, 2);
    let mut bytes = fs::read(&path).unwrap();
    bytes[4..6].copy_from_slice(&99_u16.to_le_bytes());
    let checksum = crc32c(&bytes[..60]);
    bytes[60..64].copy_from_slice(&checksum.to_le_bytes());
    fs::write(path, bytes).unwrap();

    assert!(matches!(
        DurableRevisionStore::open(&dir),
        Err(DurabilityError::UnsupportedDurableFormat {
            component: crate::DurableFormatComponent::Manifest,
            version: 99,
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pre_release_historical_metadata_is_rejected_instead_of_migrated() {
    let dir = test_dir("pre-release-format-rejected");
    let (base, registry, _) = setup_revision(42, &[1]);
    let store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    drop(store);

    let mut payload = Vec::new();
    payload.extend_from_slice(&6_u16.to_le_bytes());
    metadata::encode_materialization_specs(&mut payload, &[]).unwrap();
    payload.extend_from_slice(&crate::PHYSICAL_ARTIFACT_RECIPE_TAG.to_le_bytes());
    push_len(&mut payload, 0).unwrap();
    push_len(&mut payload, 0).unwrap();
    let modules = registry
        .builtin_modules_for_context(base.semantic_context())
        .unwrap();
    metadata::encode_semantic_module_specs(&mut payload, &modules).unwrap();

    let payload_crc = crc32c(&payload);
    let mut header = [0_u8; METADATA_HEADER_LEN];
    header[0..4].copy_from_slice(&METADATA_MAGIC);
    header[4..6].copy_from_slice(&METADATA_FILE_TAG.to_le_bytes());
    header[8..16].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    header[16..20].copy_from_slice(&payload_crc.to_le_bytes());
    let mut historical_file = Vec::new();
    historical_file.extend_from_slice(&header);
    historical_file.extend_from_slice(&payload);
    fs::write(metadata_path(&dir, 1), &historical_file).unwrap();

    let manifest_path = manifest_path(&dir, 1);
    let mut manifest = decode_manifest(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest.metadata_crc32c = crc32c(&historical_file);
    fs::write(&manifest_path, encode_manifest(manifest)).unwrap();

    assert!(DurableRevisionStore::open(&dir).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compaction_keeps_only_active_generation_artifacts() {
    let dir = test_dir("compact");
    let (base, registry, _) = setup_revision(50, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store.rotate_checkpoint(&base).unwrap();
    store.compact_obsolete_generations().unwrap();
    assert!(!checkpoint_path(&dir, 1).exists());
    assert!(!wal_path(&dir, 1).exists());
    assert!(!metadata_path(&dir, 1).exists());
    assert!(!manifest_path(&dir, 1).exists());
    assert!(checkpoint_path(&dir, 2).exists());
    assert!(wal_path(&dir, 2).exists());
    assert!(metadata_path(&dir, 2).exists());
    assert!(manifest_path(&dir, 2).exists());
    drop(store);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn published_manifest_requires_exact_durable_metadata_sidecar() {
    let dir = test_dir("metadata-authority");
    let (base, registry, _) = setup_revision(59, &[1]);
    let store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    drop(store);
    let path = metadata_path(&dir, 1);
    let original = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(matches!(
        DurableRevisionStore::open(&dir),
        Err(DurabilityError::Corruption {
            reason: "published durable metadata file is missing",
            ..
        })
    ));
    fs::write(&path, &original).unwrap();
    let mut corrupt = original;
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0x40;
    fs::write(&path, corrupt).unwrap();
    assert!(matches!(
        DurableRevisionStore::open(&dir),
        Err(DurabilityError::Corruption { .. })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn published_manifest_with_missing_wal_is_corruption_not_empty_segment() {
    let dir = test_dir("missing-wal");
    let (base, registry, _) = setup_revision(60, &[1]);
    let store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    drop(store);
    fs::remove_file(wal_path(&dir, 1)).unwrap();
    assert!(matches!(
        DurableRevisionStore::open(&dir),
        Err(DurabilityError::Corruption {
            reason: "published WAL segment is missing",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn subprocess_kill_after_prepare_recovers_previous_committed_head() {
    let dir = test_dir("kill-after-prepare");
    let (base, registry, _) = setup_revision(80, &[1]);
    drop(DurableRevisionStore::create(&dir, &base, &registry).unwrap());

    run_crash_worker(
        "store::tests::crash_worker_wal_boundary",
        &dir,
        "after-prepare",
    );

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.durable_head(), RevisionId::new(80));
    assert!(scan.committed().is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn subprocess_kill_after_commit_recovers_committed_head_without_ack() {
    let dir = test_dir("kill-after-commit");
    let (base, registry, _) = setup_revision(80, &[1]);
    drop(DurableRevisionStore::create(&dir, &base, &registry).unwrap());

    run_crash_worker(
        "store::tests::crash_worker_wal_boundary",
        &dir,
        "after-commit",
    );

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.durable_head(), RevisionId::new(81));
    assert_eq!(scan.committed().len(), 1);
    assert_eq!(
        scan.committed()[0].descriptor.target_revision,
        RevisionId::new(81)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn subprocess_kill_checkpoint_manifest_matrix_preserves_authority_boundary() {
    let points = [
        (StoreFaultPoint::AfterCheckpointSync, 1),
        (StoreFaultPoint::AfterWalSync, 1),
        (StoreFaultPoint::AfterMetadataSync, 1),
        (StoreFaultPoint::AfterPrerequisiteDirectorySync, 1),
        (StoreFaultPoint::AfterPendingManifestSync, 1),
        (StoreFaultPoint::AfterManifestRename, 2),
        (StoreFaultPoint::AfterManifestDirectorySync, 2),
    ];

    for (point, expected_generation) in points {
        let dir = test_dir(crash_point_name(point));
        let (base, registry, relation) = setup_revision(90, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let descriptor = committed_descriptor(&base, &registry, relation, 91, 2);
        let prepared = store.durably_prepare(&descriptor).unwrap();
        store.durably_commit(prepared).unwrap();
        drop(store);

        run_crash_worker(
            "store::tests::crash_worker_checkpoint_rotation",
            &dir,
            crash_point_name(point),
        );

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.generation(), expected_generation, "{point:?}");
        assert_eq!(reopened.durable_head(), RevisionId::new(91), "{point:?}");
        if expected_generation == 1 {
            assert_eq!(reopened.checkpoint_revision().id(), RevisionId::new(90));
            assert_eq!(scan.committed().len(), 1);
        } else {
            assert_eq!(reopened.checkpoint_revision().id(), RevisionId::new(91));
            assert!(scan.committed().is_empty());
        }
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn subprocess_kill_during_compaction_never_removes_active_generation() {
    for point in [
        StoreFaultPoint::BeforeCompactionRemove,
        StoreFaultPoint::AfterCompactionRemove,
        StoreFaultPoint::AfterCompactionDirectorySync,
    ] {
        let dir = test_dir(crash_point_name(point));
        let (base, registry, _) = setup_revision(100, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store.rotate_checkpoint(&base).unwrap();
        drop(store);

        run_crash_worker(
            "store::tests::crash_worker_compaction",
            &dir,
            crash_point_name(point),
        );

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.generation(), 2, "{point:?}");
        assert_eq!(reopened.durable_head(), RevisionId::new(100), "{point:?}");
        assert!(scan.committed().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn single_file_compaction_primitive_io_failure_matrix_reopens_plaintext_and_encrypted() {
    for encrypted in [false, true] {
        for (op, mode) in single_file_compaction_primitive_failure_cases() {
            let dir = test_dir(&format!("primitive-{op:?}-{encrypted}"));
            let path = dir.join("database.cfmd");
            let (base, registry, relation) = setup_revision(40_180, &[1]);
            let (_, descriptor) = transition_from(&base, &registry, relation, 40_182, 182);
            let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
                crate::storage_encryption::StorageEncryptionKey::try_new([0x62; 32]).unwrap(),
            );
            let mut store = if encrypted {
                DurableRevisionStore::create_single_file_with_encryption(
                    &path,
                    &encryption,
                    &base,
                    &registry,
                )
                .unwrap()
            } else {
                DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap()
            };
            store
                .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
                .unwrap();
            store.rotate_checkpoint(&base).unwrap();
            store.rotate_checkpoint(&base).unwrap();
            store.durably_prepare(&descriptor).unwrap();

            let mut io = FaultingSingleFileCompactionIo::new(op, mode);
            let result = store.test_compact_obsolete_generations_with_io(&mut io);
            assert!(
                matches!(result, Err(DurabilityError::Io(_))),
                "{op:?} encrypted={encrypted}: {result:?}"
            );
            assert_ne!(io.target_calls, 0, "fault was not injected for {op:?}");
            assert!(store.poisoned, "{op:?} encrypted={encrypted}");
            assert_eq!(
                store.compact_obsolete_generations(),
                Err(DurabilityError::Poisoned),
                "{op:?} encrypted={encrypted}"
            );
            drop(store);

            let (reopened, scan) = if encrypted {
                DurableRevisionStore::open_single_file_with_encryption(&path, &encryption).unwrap()
            } else {
                DurableRevisionStore::open_single_file(&path).unwrap()
            };
            assert_eq!(scan.durable_revision(), base.id(), "{op:?}");
            assert_eq!(reopened.generation(), 3, "{op:?}");
            assert_eq!(
                reopened.current_replication_membership().unwrap().epoch,
                1,
                "{op:?} encrypted={encrypted}"
            );
            drop(reopened);
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn single_file_compaction_short_and_interrupted_io_make_forward_progress_plaintext_and_encrypted() {
    for encrypted in [false, true] {
        for op in single_file_compaction_short_progress_cases() {
            for mode in [
                PrimitiveIoFaultMode::ShortOnce,
                PrimitiveIoFaultMode::InterruptedOnce,
            ] {
                let dir = test_dir(&format!("progress-{op:?}-{mode:?}-{encrypted}"));
                let path = dir.join("database.cfmd");
                let (base, registry, relation) = setup_revision(40_181, &[1]);
                let (_, descriptor) = transition_from(&base, &registry, relation, 40_183, 183);
                let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
                    crate::storage_encryption::StorageEncryptionKey::try_new([0x63; 32]).unwrap(),
                );
                let mut store = if encrypted {
                    DurableRevisionStore::create_single_file_with_encryption(
                        &path,
                        &encryption,
                        &base,
                        &registry,
                    )
                    .unwrap()
                } else {
                    DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap()
                };
                store
                    .durably_install_replication_membership(membership_change(
                        1,
                        &[1, 2, 3],
                        2,
                        &[],
                    ))
                    .unwrap();
                store.rotate_checkpoint(&base).unwrap();
                store.rotate_checkpoint(&base).unwrap();
                store.durably_prepare(&descriptor).unwrap();

                let mut io = FaultingSingleFileCompactionIo::new(op, mode);
                store
                    .test_compact_obsolete_generations_with_io(&mut io)
                    .unwrap();
                assert_ne!(
                    io.target_calls, 0,
                    "I/O perturbation was not injected for {op:?}"
                );
                assert!(!store.poisoned, "{op:?} encrypted={encrypted}");
                drop(store);

                let (reopened, scan) = if encrypted {
                    DurableRevisionStore::open_single_file_with_encryption(&path, &encryption)
                        .unwrap()
                } else {
                    DurableRevisionStore::open_single_file(&path).unwrap()
                };
                assert_eq!(scan.durable_revision(), base.id(), "{op:?}");
                assert_eq!(reopened.generation(), 3, "{op:?}");
                assert_eq!(reopened.current_replication_membership().unwrap().epoch, 1);
                drop(reopened);
                fs::remove_dir_all(dir).unwrap();
            }
        }
    }
}

#[test]
fn single_file_compaction_wal_premature_eof_is_structural_corruption_plaintext_and_encrypted() {
    let wal_reads = [
        SingleFileCompactionIoStep::StagingWalRead,
        SingleFileCompactionIoStep::FrontWalRead,
        SingleFileCompactionIoStep::JournalReopenWalRead,
    ];
    for encrypted in [false, true] {
        for op in wal_reads {
            let dir = test_dir(&format!("wal-eof-{op:?}-{encrypted}"));
            let path = dir.join("database.cfmd");
            let (base, registry, relation) = setup_revision(40_184, &[1]);
            let (_, descriptor) = transition_from(&base, &registry, relation, 40_185, 185);
            let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
                crate::storage_encryption::StorageEncryptionKey::try_new([0x64; 32]).unwrap(),
            );
            let mut store = if encrypted {
                DurableRevisionStore::create_single_file_with_encryption(
                    &path,
                    &encryption,
                    &base,
                    &registry,
                )
                .unwrap()
            } else {
                DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap()
            };
            store
                .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
                .unwrap();
            store.rotate_checkpoint(&base).unwrap();
            store.rotate_checkpoint(&base).unwrap();
            store.durably_prepare(&descriptor).unwrap();

            let mut io =
                FaultingSingleFileCompactionIo::new(op, PrimitiveIoFaultMode::ZeroProgress);
            assert!(matches!(
                store.test_compact_obsolete_generations_with_io(&mut io),
                Err(DurabilityError::Corruption { .. })
            ));
            assert_ne!(
                io.target_calls, 0,
                "premature EOF was not injected for {op:?}"
            );
            assert!(store.poisoned, "{op:?} encrypted={encrypted}");
            drop(store);

            let (reopened, scan) = if encrypted {
                DurableRevisionStore::open_single_file_with_encryption(&path, &encryption).unwrap()
            } else {
                DurableRevisionStore::open_single_file(&path).unwrap()
            };
            assert_eq!(scan.durable_revision(), base.id(), "{op:?}");
            assert_eq!(reopened.generation(), 3, "{op:?}");
            assert_eq!(reopened.current_replication_membership().unwrap().epoch, 1);
            drop(reopened);
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn single_file_compaction_error_fault_matrix_reopens_plaintext_and_encrypted() {
    for encrypted in [false, true] {
        for point in single_file_compaction_fault_points() {
            let dir = test_dir(crash_point_name(point));
            let path = dir.join("database.cfmd");
            let (base, registry, _) = setup_revision(40_176, &[1]);
            let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
                crate::storage_encryption::StorageEncryptionKey::try_new([0x61; 32]).unwrap(),
            );
            let mut store = if encrypted {
                DurableRevisionStore::create_single_file_with_encryption(
                    &path,
                    &encryption,
                    &base,
                    &registry,
                )
                .unwrap()
            } else {
                DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap()
            };
            store
                .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
                .unwrap();
            store.rotate_checkpoint(&base).unwrap();
            store.rotate_checkpoint(&base).unwrap();

            let mut hook = ErrorFault { target: point };
            assert!(matches!(
                store.test_compact_obsolete_generations_with_hook(&mut hook),
                Err(DurabilityError::Io(_))
            ));
            assert!(store.poisoned, "{point:?} encrypted={encrypted}");
            assert_eq!(
                store.compact_obsolete_generations(),
                Err(DurabilityError::Poisoned),
                "{point:?} encrypted={encrypted}"
            );
            drop(store);

            let (reopened, scan) = if encrypted {
                DurableRevisionStore::open_single_file_with_encryption(&path, &encryption).unwrap()
            } else {
                DurableRevisionStore::open_single_file(&path).unwrap()
            };
            assert_eq!(scan.durable_revision(), base.id(), "{point:?}");
            assert_eq!(reopened.generation(), 3, "{point:?}");
            assert_eq!(
                reopened.current_replication_membership().unwrap().epoch,
                1,
                "{point:?} encrypted={encrypted}"
            );
            drop(reopened);
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn single_file_compaction_torn_root_matrix_falls_back_plaintext_and_encrypted() {
    for encrypted in [false, true] {
        for point in single_file_compaction_uncertain_root_points() {
            let dir = test_dir(crash_point_name(point));
            let path = dir.join("database.cfmd");
            let (base, registry, _) = setup_revision(40_178, &[1]);
            let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
                crate::storage_encryption::StorageEncryptionKey::try_new([0x61; 32]).unwrap(),
            );
            let mut store = if encrypted {
                DurableRevisionStore::create_single_file_with_encryption(
                    &path,
                    &encryption,
                    &base,
                    &registry,
                )
                .unwrap()
            } else {
                DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap()
            };
            store
                .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
                .unwrap();
            store.rotate_checkpoint(&base).unwrap();
            store.rotate_checkpoint(&base).unwrap();

            let mut hook = TornSingleFileRootFault {
                target: point,
                path: path.clone(),
            };
            assert!(matches!(
                store.test_compact_obsolete_generations_with_hook(&mut hook),
                Err(DurabilityError::Io(_))
            ));
            assert!(store.poisoned, "{point:?} encrypted={encrypted}");
            drop(store);

            let (reopened, scan) = if encrypted {
                DurableRevisionStore::open_single_file_with_encryption(&path, &encryption).unwrap()
            } else {
                DurableRevisionStore::open_single_file(&path).unwrap()
            };
            assert_eq!(scan.durable_revision(), base.id(), "{point:?}");
            assert_eq!(reopened.generation(), 3, "{point:?}");
            assert_eq!(
                reopened.current_replication_membership().unwrap().epoch,
                1,
                "{point:?} encrypted={encrypted}"
            );
            drop(reopened);
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn subprocess_kill_at_every_single_file_compaction_boundary_reopens_plaintext_and_encrypted() {
    for encrypted in [false, true] {
        for point in single_file_compaction_fault_points()
            .into_iter()
            .chain(single_file_compaction_uncertain_root_points())
        {
            let dir = test_dir(crash_point_name(point));
            let path = dir.join("database.cfmd");
            let (base, registry, _) = setup_revision(40_177, &[1]);
            let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
                crate::storage_encryption::StorageEncryptionKey::try_new([0x61; 32]).unwrap(),
            );
            let mut store = if encrypted {
                DurableRevisionStore::create_single_file_with_encryption(
                    &path,
                    &encryption,
                    &base,
                    &registry,
                )
                .unwrap()
            } else {
                DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap()
            };
            store
                .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
                .unwrap();
            store.rotate_checkpoint(&base).unwrap();
            store.rotate_checkpoint(&base).unwrap();
            drop(store);

            run_single_file_compaction_crash_worker(&dir, point, encrypted);

            let (reopened, scan) = if encrypted {
                DurableRevisionStore::open_single_file_with_encryption(&path, &encryption).unwrap()
            } else {
                DurableRevisionStore::open_single_file(&path).unwrap()
            };
            assert_eq!(scan.durable_revision(), base.id(), "{point:?}");
            assert_eq!(reopened.generation(), 3, "{point:?}");
            assert_eq!(
                reopened.current_replication_membership().unwrap().epoch,
                1,
                "{point:?} encrypted={encrypted}"
            );
            drop(reopened);
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn crash_worker_wal_boundary() {
    if std::env::var_os(CRASH_WORKER_ENV).is_none() {
        return;
    }
    let dir = PathBuf::from(std::env::var_os(CRASH_DIR_ENV).unwrap());
    let point = std::env::var(CRASH_POINT_ENV).unwrap();
    let (base, registry, relation) = setup_revision(80, &[1]);
    let (mut store, _) = DurableRevisionStore::open(&dir).unwrap();
    let descriptor = committed_descriptor(&base, &registry, relation, 81, 2);
    let prepared = store.durably_prepare(&descriptor).unwrap();
    if point == "after-prepare" {
        signal_crash_ready(&dir);
    }
    store.durably_commit(prepared).unwrap();
    if point == "after-commit" {
        signal_crash_ready(&dir);
    }
    panic!("unknown WAL crash point {point}");
}

#[test]
fn crash_worker_checkpoint_rotation() {
    if std::env::var_os(CRASH_WORKER_ENV).is_none() {
        return;
    }
    let dir = PathBuf::from(std::env::var_os(CRASH_DIR_ENV).unwrap());
    let point = parse_crash_point(&std::env::var(CRASH_POINT_ENV).unwrap());
    let (next, _, _) = setup_revision(91, &[1, 2]);
    let (mut store, _) = DurableRevisionStore::open(&dir).unwrap();
    let mut hook = BlockingKillFault {
        target: point,
        directory: dir.clone(),
    };
    let specs = store.materialization_specs().to_vec();
    store
        .test_rotate_checkpoint_with_fault_policy(&next, &specs, &[], &[], &mut hook)
        .unwrap();
    panic!("checkpoint crash point was not reached: {point:?}");
}

#[test]
fn crash_worker_compaction() {
    if std::env::var_os(CRASH_WORKER_ENV).is_none() {
        return;
    }
    let dir = PathBuf::from(std::env::var_os(CRASH_DIR_ENV).unwrap());
    let point = parse_crash_point(&std::env::var(CRASH_POINT_ENV).unwrap());
    let (mut store, _) = DurableRevisionStore::open(&dir).unwrap();
    let mut hook = BlockingKillFault {
        target: point,
        directory: dir.clone(),
    };
    store
        .test_compact_obsolete_generations_with_hook(&mut hook)
        .unwrap();
    panic!("compaction crash point was not reached: {point:?}");
}

#[test]
fn crash_worker_single_file_compaction() {
    if std::env::var_os(CRASH_WORKER_ENV).is_none() {
        return;
    }
    let dir = PathBuf::from(std::env::var_os(CRASH_DIR_ENV).unwrap());
    let path = dir.join("database.cfmd");
    let point = parse_crash_point(&std::env::var(CRASH_POINT_ENV).unwrap());
    let encrypted = std::env::var(CRASH_SINGLE_FILE_ENCRYPTED_ENV).as_deref() == Ok("1");
    let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
        crate::storage_encryption::StorageEncryptionKey::try_new([0x61; 32]).unwrap(),
    );
    let (mut store, _) = if encrypted {
        DurableRevisionStore::open_single_file_with_encryption(&path, &encryption).unwrap()
    } else {
        DurableRevisionStore::open_single_file(&path).unwrap()
    };
    let mut hook = BlockingKillFault {
        target: point,
        directory: dir.clone(),
    };
    store
        .test_compact_obsolete_generations_with_hook(&mut hook)
        .unwrap();
    panic!("single-file compaction crash point was not reached: {point:?}");
}

fn assert_committed_at(
    store: &DurableRevisionStore,
    epoch: IdempotencyEpoch,
    transaction_id: ClientTransactionId,
    target_revision: u64,
) {
    assert_eq!(
        store.transaction_outcome_at(epoch, transaction_id),
        DurableTransactionOutcome::Committed {
            target_revision: RevisionId::new(target_revision),
        }
    );
}

#[test]
fn idempotency_epoch_reuse_survives_crash_before_checkpoint_without_causal_alias() {
    let dir = test_dir("epoch-reuse-before-checkpoint");
    let (base, registry, relation) = setup_revision(300, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let transaction_id = ClientTransactionId::new(0xDEAD);

    let (rev_301, _, _) = setup_revision(301, &[1, 2]);
    let first = DurableRevisionDescriptor::relation_data(
        transaction_id,
        RevisionId::new(300),
        &rev_301,
        base.semantic_revision(),
        vec![DurableRelationMutation {
            relation,
            inserted: vec![vec![Value::I64(2)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        &registry,
    )
    .unwrap();
    let prepared = store.durably_prepare(&first).unwrap();
    store.durably_commit(prepared).unwrap();
    let first_effect = *store
        .revision_effect_frontier(RevisionId::new(301))
        .unwrap()
        .iter()
        .next()
        .unwrap();

    store
        .advance_idempotency_epoch(IdempotencyEpoch::new(1))
        .unwrap();
    let (rev_302, _, _) = setup_revision(302, &[1, 2, 3]);
    let second = DurableRevisionDescriptor::relation_data(
        transaction_id,
        RevisionId::new(301),
        &rev_302,
        base.semantic_revision(),
        vec![DurableRelationMutation {
            relation,
            inserted: vec![vec![Value::I64(3)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        &registry,
    )
    .unwrap();
    let prepared = store.durably_prepare(&second).unwrap();
    store.durably_commit(prepared).unwrap();
    let second_effect = *store
        .revision_effect_frontier(RevisionId::new(302))
        .unwrap()
        .iter()
        .next()
        .unwrap();
    assert_ne!(first_effect, second_effect);
    assert_committed_at(&store, IdempotencyEpoch::ZERO, transaction_id, 301);
    assert_committed_at(&store, IdempotencyEpoch::new(1), transaction_id, 302);
    drop(store); // no checkpoint after epoch advance/reuse

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(302));
    assert_eq!(
        reopened.current_idempotency_epoch(),
        IdempotencyEpoch::new(1)
    );
    assert_committed_at(&reopened, IdempotencyEpoch::ZERO, transaction_id, 301);
    assert_committed_at(&reopened, IdempotencyEpoch::new(1), transaction_id, 302);
    assert_eq!(
        reopened
            .revision_effect_record(first_effect)
            .unwrap()
            .transaction_epoch,
        IdempotencyEpoch::ZERO
    );
    assert_eq!(
        reopened
            .revision_effect_record(second_effect)
            .unwrap()
            .transaction_epoch,
        IdempotencyEpoch::new(1)
    );
    assert!(
        reopened
            .revision_effect_ideal(RevisionId::new(302))
            .unwrap()
            .is_some()
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn retry_gc_persists_watermark_and_keeps_causal_history_self_contained() {
    let dir = test_dir("retry-gc-causal-independence");
    let (base, registry, relation) = setup_revision(400, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let old_id = ClientTransactionId::new(0xA1);
    let new_id = ClientTransactionId::new(0xA2);

    let (rev_401, _, _) = setup_revision(401, &[1, 2]);
    let old = DurableRevisionDescriptor::relation_data(
        old_id,
        RevisionId::new(400),
        &rev_401,
        base.semantic_revision(),
        vec![DurableRelationMutation {
            relation,
            inserted: vec![vec![Value::I64(2)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        &registry,
    )
    .unwrap();
    let prepared = store.durably_prepare(&old).unwrap();
    store.durably_commit(prepared).unwrap();
    let old_effect = *store
        .revision_effect_frontier(RevisionId::new(401))
        .unwrap()
        .iter()
        .next()
        .unwrap();

    store
        .advance_idempotency_epoch(IdempotencyEpoch::new(1))
        .unwrap();
    let (rev_402, _, _) = setup_revision(402, &[1, 2, 3]);
    let new = DurableRevisionDescriptor::relation_data(
        new_id,
        RevisionId::new(401),
        &rev_402,
        base.semantic_revision(),
        vec![DurableRelationMutation {
            relation,
            inserted: vec![vec![Value::I64(3)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        &registry,
    )
    .unwrap();
    let prepared = store.durably_prepare(&new).unwrap();
    store.durably_commit(prepared).unwrap();

    assert_eq!(
        store
            .expire_retry_history_before(IdempotencyEpoch::new(1))
            .unwrap(),
        1
    );
    assert_eq!(
        store.transaction_outcome_at(IdempotencyEpoch::ZERO, old_id),
        DurableTransactionOutcome::RetryHistoryExpired
    );
    assert!(
        store
            .transaction_intent_at(IdempotencyEpoch::ZERO, old_id)
            .is_none()
    );
    let old_ideal = store
        .revision_effect_ideal(RevisionId::new(401))
        .unwrap()
        .unwrap();
    assert_eq!(old_ideal.events()[&old_effect].payload, old.intent);

    store.rotate_checkpoint(&rev_402).unwrap();
    drop(store);
    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert!(scan.committed().is_empty());
    assert_eq!(
        reopened.current_idempotency_epoch(),
        IdempotencyEpoch::new(1)
    );
    assert_eq!(reopened.minimum_retry_epoch(), IdempotencyEpoch::new(1));
    assert_eq!(
        reopened.transaction_outcome_at(IdempotencyEpoch::ZERO, old_id),
        DurableTransactionOutcome::RetryHistoryExpired
    );
    assert_eq!(
        reopened.transaction_outcome_at(IdempotencyEpoch::new(1), new_id),
        DurableTransactionOutcome::Committed {
            target_revision: RevisionId::new(402)
        }
    );
    assert_eq!(
        reopened
            .revision_effect_ideal(RevisionId::new(401))
            .unwrap()
            .unwrap()
            .events()[&old_effect]
            .payload,
        old.intent
    );
    fs::remove_dir_all(dir).unwrap();
}

macro_rules! replicated_relation_effect {
    ($origin:expr, $sequence:expr, $branch:expr, $source:expr, $target:expr, $deps:expr, $semantic:expr, $position:expr) => {{
        let origin = ReplicaId::new($origin);
        let id = replicated_effect_id(origin, $sequence);
        ReplicatedEffectEnvelope {
            origin,
            origin_sequence: $sequence,
            branch: ReplicationBranchId::new($branch),
            effect: DurableRevisionEffectRecord {
                id,
                prerequisites: $deps,
                transaction_epoch: IdempotencyEpoch::ZERO,
                transaction_id: ClientTransactionId::new(id.0),
                intent: DurableTransactionIntent::RelationData {
                    source_revision: $source,
                    target_revision: $target,
                    semantic_revision: $semantic,
                    relation_mutations: Vec::new(),
                    client_guard_digest: None,
                    causal_observations: Vec::new(),
                    causal_observation_groups: Vec::new(),
                    relational_causal_observations: Vec::new(),
                    semantic_modules: Vec::new(),
                },
                change: DurableRevisionChange::RelationData {
                    semantic_revision: $semantic,
                    relation_mutations: Vec::new(),
                },
                source_revision: $source,
                target_revision: $target,
            },
            ordered_by: DurableSequencerOrder {
                sequencer: ReplicaId::new(99),
                epoch: 7,
                position: $position,
            },
        }
    }};
}

#[test]
fn local_prepare_rejects_target_already_owned_by_local_causal_authority_before_wal_mutation() {
    let dir = test_dir("local-prepare-local-target-collision");
    let (base, registry, relation) = setup_revision(2_280, &[1]);
    let (current, first) = transition_from(&base, &registry, relation, 2_281, 1);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let prepared = store.durably_prepare(&first).unwrap();
    store.durably_commit(prepared).unwrap();

    let collision = committed_descriptor(&current, &registry, relation, base.id().raw(), 2);
    let before_lsn = store.wal.last_lsn();
    assert!(matches!(
        store.durably_prepare(&collision),
        Err(DurabilityError::Protocol {
            reason: "local target revision already belongs to local causal authority",
            ..
        })
    ));
    assert_eq!(store.wal.last_lsn(), before_lsn);
    assert_eq!(store.durable_head, current.id());
    assert!(!store.requires_recovery());
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), current.id());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn group_commit_rejects_provisional_local_target_reuse_before_wal_mutation() {
    let dir = test_dir("group-local-target-reuse");
    let (base, registry, relation) = setup_revision(2_290, &[1]);
    let (first_target, first) = transition_from(&base, &registry, relation, 2_291, 1);

    let mut state = first_target.state().clone();
    state
        .model
        .relations
        .entry(relation)
        .or_default()
        .push(vec![Value::I64(2)]);
    let repeated_target = Revision::build(
        first_target.id(),
        first_target.semantic_context(),
        &registry,
        state,
    )
    .unwrap();
    let second = DurableRevisionDescriptor::relation_data(
        ClientTransactionId::new(22_292),
        first_target.id(),
        &repeated_target,
        first_target.semantic_revision(),
        vec![DurableRelationMutation {
            relation,
            inserted: vec![vec![Value::I64(2)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let before_lsn = store.wal.last_lsn();
    assert!(matches!(
        store.durably_commit_group(&[first, second]),
        Err(DurabilityError::Protocol {
            reason: "local target revision already belongs to local causal authority",
            ..
        })
    ));
    assert_eq!(store.wal.last_lsn(), before_lsn);
    assert_eq!(store.durable_head, base.id());
    assert!(!store.requires_recovery());
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn local_prepare_rejects_target_already_owned_by_replicated_authority() {
    let dir = test_dir("local-prepare-replicated-target-collision");
    let (base, registry, relation) = setup_revision(2_300, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let target = RevisionId::new(2_301);
    let replicated = replicated_relation_effect!(
        230,
        1,
        230,
        base.id(),
        target,
        BTreeSet::new(),
        base.semantic_revision(),
        1
    );
    store.durably_ingest_replicated_effect(replicated).unwrap();
    let descriptor = committed_descriptor(&base, &registry, relation, target.raw(), 9);
    let before_lsn = store.wal.last_lsn();

    assert!(matches!(
        store.durably_prepare(&descriptor),
        Err(DurabilityError::Protocol {
            reason: "local target revision already belongs to replicated causal authority",
            ..
        })
    ));
    assert_eq!(store.wal.last_lsn(), before_lsn);
    assert!(!store.requires_recovery());
    drop(store);
    DurableRevisionStore::open(&dir).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_recovery_does_not_advance_external_freshness_authority() {
    let dir = test_dir("recovery-freshness-deferred-until-authority-valid");
    let (base, registry, relation) = setup_revision(2_400, &[1]);
    let (config, authority) = external_freshness_fixture([0x58; 32]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();

    let target = RevisionId::new(2_401);
    let replicated = replicated_relation_effect!(
        240,
        1,
        240,
        base.id(),
        target,
        BTreeSet::new(),
        base.semantic_revision(),
        1
    );
    store.replication.ingest(replicated).unwrap();

    let mut descriptor = committed_descriptor(&base, &registry, relation, target.raw(), 10);
    descriptor.revision_effect_id = Some(RevisionEffectId(1));
    let prepared = store.wal.append_prepare_unflushed(&descriptor).unwrap();
    store.wal.append_commit_unflushed(prepared).unwrap();
    store.wal.durability_barrier().unwrap();
    drop(store);

    let before = authority.current.lock().unwrap().clone().unwrap();
    assert!(matches!(
        DurableRevisionStore::open_with_external_freshness(&dir, config, authority.boxed()),
        Err(DurabilityError::Protocol {
            reason: "replicated causal authority collides with local authority",
            ..
        })
    ));
    assert_eq!(authority.current.lock().unwrap().as_ref(), Some(&before));
    fs::remove_dir_all(dir).unwrap();
}

fn membership_change(
    epoch: u64,
    members: &[u64],
    quorum_size: usize,
    acknowledged_by_previous: &[u64],
) -> ReplicationMembershipChange {
    ReplicationMembershipChange {
        next: ReplicationMembership {
            epoch,
            members: members.iter().copied().map(ReplicaId::new).collect(),
            quorum_size,
        },
        acknowledged_by_previous: acknowledged_by_previous
            .iter()
            .copied()
            .map(ReplicaId::new)
            .collect(),
    }
}

fn quorum_certificate(
    effect: RevisionEffectId,
    membership_epoch: u64,
    acknowledged_by: &[u64],
) -> ReplicationQuorumCertificate {
    ReplicationQuorumCertificate {
        effect,
        membership_epoch,
        acknowledged_by: acknowledged_by
            .iter()
            .copied()
            .map(ReplicaId::new)
            .collect(),
    }
}

fn replica_signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn replication_auth_policy(
    trust_epoch: u64,
    entries: &[(u64, &SigningKey)],
) -> ReplicationPeerAuthPolicy {
    ReplicationPeerAuthPolicy {
        cluster: crate::ReplicationClusterId([0xA5; 32]),
        trust_epoch,
        peer_keys: entries
            .iter()
            .map(|(replica, key)| {
                (
                    ReplicaId::new(*replica),
                    key_id(key.verifying_key().as_bytes()),
                )
            })
            .collect(),
    }
}

fn replication_trust(epoch: u64, keys: &[&SigningKey]) -> TrustRootSet {
    let verifying_keys: Vec<_> = keys
        .iter()
        .map(|key| *key.verifying_key().as_bytes())
        .collect();
    TrustRootSet::bootstrap(epoch, &verifying_keys).unwrap()
}

fn sign_replication_evidence(
    policy: &ReplicationPeerAuthPolicy,
    key: &SigningKey,
    evidence: ReplicationPeerEvidence,
) -> SignedReplicationPeerEvidence {
    let signer = key_id(key.verifying_key().as_bytes());
    let message = replication_peer_evidence_signing_message(
        policy.cluster,
        policy.trust_epoch,
        signer,
        &evidence,
    )
    .unwrap();
    SignedReplicationPeerEvidence {
        trust_epoch: policy.trust_epoch,
        signer,
        evidence,
        signature: key.sign(&message).to_bytes(),
    }
}

fn record_signed_replication_evidence(
    store: &mut DurableRevisionStore,
    trust: &TrustRootSet,
    policy: &ReplicationPeerAuthPolicy,
    key: &SigningKey,
    evidence: ReplicationPeerEvidence,
) -> ReplicationAuthenticationReceipt {
    store
        .durably_record_authenticated_replication_peer_evidence(
            trust,
            sign_replication_evidence(policy, key, evidence),
        )
        .unwrap()
}

fn certify_authenticated_replication_leader(
    store: &mut DurableRevisionStore,
    trust: &TrustRootSet,
    policy: &ReplicationPeerAuthPolicy,
    term: u64,
    leader: u64,
    voters: &[(u64, &SigningKey)],
) {
    for (voter, key) in voters {
        record_signed_replication_evidence(
            store,
            trust,
            policy,
            key,
            ReplicationPeerEvidence::LeaderVote(ReplicationLeaderVote {
                voter: ReplicaId::new(*voter),
                membership_epoch: 1,
                term,
                candidate: ReplicaId::new(leader),
            }),
        );
    }
    store
        .durably_certify_replication_leader(ReplicationLeaderCertificate {
            membership_epoch: 1,
            term,
            leader: ReplicaId::new(leader),
            acknowledged_by: voters
                .iter()
                .map(|(voter, _)| ReplicaId::new(*voter))
                .collect(),
        })
        .unwrap();
}

#[test]
#[allow(clippy::too_many_lines)]
fn authenticated_replication_evidence_is_cluster_key_and_epoch_bound_across_restart() {
    let dir = test_dir("replication-auth-evidence");
    let (base, registry, _) = setup_revision(700, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    let key1 = replica_signing_key(11);
    let key2 = replica_signing_key(12);
    let key3 = replica_signing_key(13);
    let trust1 = replication_trust(1, &[&key1, &key2, &key3]);
    let policy1 = replication_auth_policy(1, &[(1, &key1), (2, &key2), (3, &key3)]);
    store
        .durably_install_replication_peer_auth_policy(policy1.clone(), &trust1)
        .unwrap();

    let vote = ReplicationLeaderVote {
        voter: ReplicaId::new(1),
        membership_epoch: 1,
        term: 5,
        candidate: ReplicaId::new(1),
    };
    assert_eq!(
        store.durably_record_replication_leader_vote(vote),
        Err(DurabilityError::Protocol {
            offset: 0,
            reason: "replication peer evidence is not authenticated in current trust epoch",
        })
    );

    let receipt = record_signed_replication_evidence(
        &mut store,
        &trust1,
        &policy1,
        &key1,
        ReplicationPeerEvidence::LeaderVote(vote),
    );
    assert_eq!(
        store.replication_authentication_receipt(receipt.proof_digest),
        Some(receipt)
    );

    let mut forged = sign_replication_evidence(
        &policy1,
        &key2,
        ReplicationPeerEvidence::LeaderVote(ReplicationLeaderVote {
            voter: ReplicaId::new(2),
            membership_epoch: 1,
            term: 5,
            candidate: ReplicaId::new(1),
        }),
    );
    forged.signature[0] ^= 0x80;
    assert_eq!(
        store.durably_record_authenticated_replication_peer_evidence(&trust1, forged),
        Err(DurabilityError::Protocol {
            offset: 0,
            reason: "replication peer evidence signature verification failed",
        })
    );

    let key1v2 = replica_signing_key(21);
    let key2v2 = replica_signing_key(22);
    let key3v2 = replica_signing_key(23);
    let trust2 = replication_trust(2, &[&key1v2, &key2v2, &key3v2]);
    let policy2 = replication_auth_policy(2, &[(1, &key1v2), (2, &key2v2), (3, &key3v2)]);
    store
        .durably_install_replication_peer_auth_policy(policy2.clone(), &trust2)
        .unwrap();

    let stale = sign_replication_evidence(
        &policy1,
        &key2,
        ReplicationPeerEvidence::TermPromise(ReplicationTermPromise {
            voter: ReplicaId::new(2),
            membership_epoch: 1,
            term: 6,
        }),
    );
    assert_eq!(
        store.durably_record_authenticated_replication_peer_evidence(&trust2, stale),
        Err(DurabilityError::Protocol {
            offset: 0,
            reason: "replication peer evidence uses a stale trust epoch",
        })
    );

    record_signed_replication_evidence(
        &mut store,
        &trust2,
        &policy2,
        &key2v2,
        ReplicationPeerEvidence::TermPromise(ReplicationTermPromise {
            voter: ReplicaId::new(2),
            membership_epoch: 1,
            term: 6,
        }),
    );
    drop(store);

    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.replication_peer_auth_policy(), Some(&policy2));
    assert_eq!(
        reopened.replication_authentication_receipt(receipt.proof_digest),
        Some(receipt)
    );
    assert_eq!(
        reopened.replication_promised_term(1, ReplicaId::new(2)),
        Some(6)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(clippy::too_many_lines)]
fn quorum_loss_fences_authority_until_authenticated_lock_recovery_and_survives_restart() {
    let dir = test_dir("replication-quorum-loss-recovery");
    let (base, registry, _) = setup_revision(710, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    let key1 = replica_signing_key(31);
    let key2 = replica_signing_key(32);
    let key3 = replica_signing_key(33);
    let trust = replication_trust(1, &[&key1, &key2, &key3]);
    let policy = replication_auth_policy(1, &[(1, &key1), (2, &key2), (3, &key3)]);
    store
        .durably_install_replication_peer_auth_policy(policy.clone(), &trust)
        .unwrap();

    let effect = replicated_relation_effect!(
        211,
        1,
        211,
        base.id(),
        RevisionId::new(711),
        BTreeSet::new(),
        semantic_revision,
        80
    );
    let effect_id = effect.effect.id;
    store.durably_ingest_replicated_effect(effect).unwrap();

    certify_authenticated_replication_leader(
        &mut store,
        &trust,
        &policy,
        5,
        1,
        &[(1, &key1), (2, &key2)],
    );
    assert_eq!(
        store.replication_quorum_availability(),
        Some(ReplicationQuorumAvailability::Available {
            membership_epoch: 1,
            term: 5,
        })
    );
    for (voter, key) in [(1, &key1), (2, &key2)] {
        record_signed_replication_evidence(
            &mut store,
            &trust,
            &policy,
            key,
            ReplicationPeerEvidence::DecisionVote(ReplicationDecisionVote {
                voter: ReplicaId::new(voter),
                membership_epoch: 1,
                term: 5,
                leader: ReplicaId::new(1),
                position: 80,
                effect: effect_id,
                carried_from_term: None,
            }),
        );
    }
    store
        .durably_lock_replication_decision(ReplicationDecisionLock {
            membership_epoch: 1,
            term: 5,
            leader: ReplicaId::new(1),
            position: 80,
            effect: effect_id,
            acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
            carried_from_term: None,
        })
        .unwrap();

    store
        .durably_mark_replication_quorum_lost(ReplicationQuorumLoss {
            membership_epoch: 1,
            observed_term: 5,
        })
        .unwrap();
    assert_eq!(
        store.replication_quorum_availability(),
        Some(ReplicationQuorumAvailability::Lost(ReplicationQuorumLoss {
            membership_epoch: 1,
            observed_term: 5,
        }))
    );

    for (voter, key) in [(1, &key1), (2, &key2)] {
        record_signed_replication_evidence(
            &mut store,
            &trust,
            &policy,
            key,
            ReplicationPeerEvidence::LeaderVote(ReplicationLeaderVote {
                voter: ReplicaId::new(voter),
                membership_epoch: 1,
                term: 6,
                candidate: ReplicaId::new(2),
            }),
        );
    }
    assert_eq!(
        store.durably_certify_replication_leader(ReplicationLeaderCertificate {
            membership_epoch: 1,
            term: 6,
            leader: ReplicaId::new(2),
            acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
        }),
        Err(DurabilityError::Protocol {
            offset: 0,
            reason: "replication consensus authority is fenced by quorum loss",
        })
    );

    let local_lock = ReplicationLockSummary {
        position: 80,
        term: 5,
        effect: effect_id,
    };
    for (voter, key) in [(1, &key1), (2, &key2)] {
        record_signed_replication_evidence(
            &mut store,
            &trust,
            &policy,
            key,
            ReplicationPeerEvidence::RecoveryAck(ReplicationRecoveryAck {
                voter: ReplicaId::new(voter),
                membership_epoch: 1,
                recovery_term: 7,
                leader: ReplicaId::new(2),
                locks: vec![local_lock],
            }),
        );
    }
    store
        .durably_recover_replication_quorum(&ReplicationRecoveryCertificate {
            membership_epoch: 1,
            recovery_term: 7,
            leader: ReplicaId::new(2),
            acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
            reconciled_locks: vec![local_lock],
        })
        .unwrap();
    assert_eq!(
        store.replication_quorum_availability(),
        Some(ReplicationQuorumAvailability::Available {
            membership_epoch: 1,
            term: 7,
        })
    );
    assert_eq!(
        store.replication_leader_certificate(1, 7).unwrap().leader,
        ReplicaId::new(2)
    );
    drop(store);

    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(
        reopened.replication_quorum_availability(),
        Some(ReplicationQuorumAvailability::Available {
            membership_epoch: 1,
            term: 7,
        })
    );
    assert_eq!(
        reopened.replication_decision_lock(80).unwrap().effect,
        effect_id
    );
    assert_eq!(
        reopened
            .replication_leader_certificate(1, 7)
            .unwrap()
            .leader,
        ReplicaId::new(2)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn authenticated_joint_membership_ack_is_single_vote_per_successor() {
    let dir = test_dir("replication-joint-ack-single-vote");
    let (base, registry, _) = setup_revision(714, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    let key1 = replica_signing_key(34);
    let key2 = replica_signing_key(35);
    let key3 = replica_signing_key(36);
    let trust = replication_trust(1, &[&key1, &key2, &key3]);
    let policy = replication_auth_policy(1, &[(1, &key1), (2, &key2), (3, &key3)]);
    store
        .durably_install_replication_peer_auth_policy(policy.clone(), &trust)
        .unwrap();
    let next = ReplicationMembership {
        epoch: 2,
        members: [ReplicaId::new(1), ReplicaId::new(2), ReplicaId::new(3)]
            .into_iter()
            .collect(),
        quorum_size: 2,
    };
    let next_digest = replication_membership_digest(&next).unwrap();
    record_signed_replication_evidence(
        &mut store,
        &trust,
        &policy,
        &key2,
        ReplicationPeerEvidence::JointMembershipAck(ReplicationJointMembershipAck {
            voter: ReplicaId::new(2),
            previous_membership_epoch: 1,
            next_membership_epoch: 2,
            term: 10,
            leader: ReplicaId::new(1),
            next_membership_digest: next_digest,
        }),
    );

    let conflicting = sign_replication_evidence(
        &policy,
        &key2,
        ReplicationPeerEvidence::JointMembershipAck(ReplicationJointMembershipAck {
            voter: ReplicaId::new(2),
            previous_membership_epoch: 1,
            next_membership_epoch: 2,
            term: 10,
            leader: ReplicaId::new(3),
            next_membership_digest: next_digest,
        }),
    );
    assert_eq!(
        store.durably_record_authenticated_replication_peer_evidence(&trust, conflicting),
        Err(DurabilityError::Protocol {
            offset: 0,
            reason: "replication voter published conflicting joint-membership acknowledgements",
        })
    );
    drop(store);
    DurableRevisionStore::open(&dir).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn recovery_ack_evidence_is_single_vote_and_shares_lock_frontier_owner() {
    let dir = test_dir("replication-recovery-ack-single-vote");
    let (base, registry, _) = setup_revision(713, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    let key1 = replica_signing_key(31);
    let key2 = replica_signing_key(32);
    let key3 = replica_signing_key(33);
    let trust = replication_trust(1, &[&key1, &key2, &key3]);
    let policy = replication_auth_policy(1, &[(1, &key1), (2, &key2), (3, &key3)]);
    store
        .durably_install_replication_peer_auth_policy(policy.clone(), &trust)
        .unwrap();
    store
        .durably_mark_replication_quorum_lost(ReplicationQuorumLoss {
            membership_epoch: 1,
            observed_term: 5,
        })
        .unwrap();

    let shared_locks = vec![ReplicationLockSummary {
        position: 80,
        term: 5,
        effect: RevisionEffectId(0xA1),
    }];
    for (voter, key) in [(1, &key1), (2, &key2)] {
        record_signed_replication_evidence(
            &mut store,
            &trust,
            &policy,
            key,
            ReplicationPeerEvidence::RecoveryAck(ReplicationRecoveryAck {
                voter: ReplicaId::new(voter),
                membership_epoch: 1,
                recovery_term: 7,
                leader: ReplicaId::new(2),
                locks: shared_locks.clone(),
            }),
        );
    }
    assert_eq!(store.replication.recovery_ack_storage_shape(), (2, 1));

    let conflicting = sign_replication_evidence(
        &policy,
        &key1,
        ReplicationPeerEvidence::RecoveryAck(ReplicationRecoveryAck {
            voter: ReplicaId::new(1),
            membership_epoch: 1,
            recovery_term: 7,
            leader: ReplicaId::new(3),
            locks: shared_locks,
        }),
    );
    assert_eq!(
        store.durably_record_authenticated_replication_peer_evidence(&trust, conflicting),
        Err(DurabilityError::Protocol {
            offset: 0,
            reason: "replication voter published conflicting recovery acknowledgements",
        })
    );
    assert_eq!(store.replication.recovery_ack_storage_shape(), (2, 1));

    drop(store);
    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.replication.recovery_ack_storage_shape(), (2, 1));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn authenticated_recovery_acks_persist_lock_frontier_once_not_per_voter() {
    let dir = test_dir("replication-recovery-ack-durable-owner-dedup");
    let (base, registry, _) = setup_revision(7131, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    let key1 = replica_signing_key(41);
    let key2 = replica_signing_key(42);
    let key3 = replica_signing_key(43);
    let trust = replication_trust(1, &[&key1, &key2, &key3]);
    let policy = replication_auth_policy(1, &[(1, &key1), (2, &key2), (3, &key3)]);
    store
        .durably_install_replication_peer_auth_policy(policy.clone(), &trust)
        .unwrap();
    store
        .durably_mark_replication_quorum_lost(ReplicationQuorumLoss {
            membership_epoch: 1,
            observed_term: 5,
        })
        .unwrap();

    let locks: Vec<_> = (1_u64..=1024)
        .map(|position| ReplicationLockSummary {
            position,
            term: 5,
            effect: RevisionEffectId(u128::from(position)),
        })
        .collect();
    let before = fs::metadata(store.replication_journal_path())
        .unwrap()
        .len();
    record_signed_replication_evidence(
        &mut store,
        &trust,
        &policy,
        &key1,
        ReplicationPeerEvidence::RecoveryAck(ReplicationRecoveryAck {
            voter: ReplicaId::new(1),
            membership_epoch: 1,
            recovery_term: 7,
            leader: ReplicaId::new(2),
            locks: locks.clone(),
        }),
    );
    let after_first = fs::metadata(store.replication_journal_path())
        .unwrap()
        .len();
    record_signed_replication_evidence(
        &mut store,
        &trust,
        &policy,
        &key2,
        ReplicationPeerEvidence::RecoveryAck(ReplicationRecoveryAck {
            voter: ReplicaId::new(2),
            membership_epoch: 1,
            recovery_term: 7,
            leader: ReplicaId::new(2),
            locks,
        }),
    );
    let after_second = fs::metadata(store.replication_journal_path())
        .unwrap()
        .len();

    let first_ack_bytes = after_first - before;
    let second_ack_bytes = after_second - after_first;
    assert!(first_ack_bytes > 32_000);
    assert!(second_ack_bytes < 512);
    assert!(first_ack_bytes > second_ack_bytes * 20);
    assert_eq!(store.replication.recovery_ack_storage_shape(), (2, 1));
    drop(store);

    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.replication.recovery_ack_storage_shape(), (2, 1));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn quorum_loss_observed_term_cannot_regress() {
    let dir = test_dir("replication-quorum-loss-term-monotone");
    let (base, registry, _) = setup_revision(712, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    store
        .durably_mark_replication_quorum_lost(ReplicationQuorumLoss {
            membership_epoch: 1,
            observed_term: 5,
        })
        .unwrap();
    assert_eq!(
        store.durably_mark_replication_quorum_lost(ReplicationQuorumLoss {
            membership_epoch: 1,
            observed_term: 4,
        }),
        Err(DurabilityError::Protocol {
            offset: 0,
            reason: "replication quorum loss observed term regressed",
        })
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(clippy::too_many_lines)]
fn joint_membership_requires_authenticated_successor_quorum_when_peer_auth_is_active() {
    let dir = test_dir("replication-auth-joint-membership");
    let (base, registry, _) = setup_revision(720, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    let key1 = replica_signing_key(41);
    let key2 = replica_signing_key(42);
    let key3 = replica_signing_key(43);
    let key4 = replica_signing_key(44);
    let key5 = replica_signing_key(45);
    let key6 = replica_signing_key(46);
    let trust = replication_trust(1, &[&key1, &key2, &key3, &key4, &key5, &key6]);
    let policy = replication_auth_policy(
        1,
        &[
            (1, &key1),
            (2, &key2),
            (3, &key3),
            (4, &key4),
            (5, &key5),
            (6, &key6),
        ],
    );
    store
        .durably_install_replication_peer_auth_policy(policy.clone(), &trust)
        .unwrap();
    certify_authenticated_replication_leader(
        &mut store,
        &trust,
        &policy,
        10,
        1,
        &[(1, &key1), (2, &key2)],
    );

    let next = ReplicationMembership {
        epoch: 2,
        members: [ReplicaId::new(4), ReplicaId::new(5), ReplicaId::new(6)]
            .into_iter()
            .collect(),
        quorum_size: 2,
    };
    for (voter, key) in [(1, &key1), (2, &key2)] {
        record_signed_replication_evidence(
            &mut store,
            &trust,
            &policy,
            key,
            ReplicationPeerEvidence::MembershipVote(ReplicationMembershipVote {
                voter: ReplicaId::new(voter),
                previous_membership_epoch: 1,
                term: 10,
                next: next.clone(),
            }),
        );
    }
    let certificate = ReplicationJointMembershipCertificate {
        previous_membership_epoch: 1,
        term: 10,
        leader: ReplicaId::new(1),
        next: next.clone(),
        acknowledged_by_previous: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
        acknowledged_by_next: [ReplicaId::new(4), ReplicaId::new(5)].into_iter().collect(),
    };
    assert_eq!(
        store.durably_certify_replication_joint_membership(certificate.clone()),
        Err(DurabilityError::Protocol {
            offset: 0,
            reason: "joint membership certificate lacks authenticated successor evidence",
        })
    );

    let next_digest = replication_membership_digest(&next).unwrap();
    for (voter, key) in [(4, &key4), (5, &key5)] {
        record_signed_replication_evidence(
            &mut store,
            &trust,
            &policy,
            key,
            ReplicationPeerEvidence::JointMembershipAck(ReplicationJointMembershipAck {
                voter: ReplicaId::new(voter),
                previous_membership_epoch: 1,
                next_membership_epoch: 2,
                term: 10,
                leader: ReplicaId::new(1),
                next_membership_digest: next_digest,
            }),
        );
    }
    store
        .durably_certify_replication_joint_membership(certificate)
        .unwrap();
    store
        .durably_install_replication_membership(ReplicationMembershipChange {
            next,
            acknowledged_by_previous: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
        })
        .unwrap();
    assert_eq!(store.current_replication_membership().unwrap().epoch, 2);
    drop(store);

    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.current_replication_membership().unwrap().epoch, 2);
    assert_eq!(reopened.replication_peer_auth_policy(), Some(&policy));
    fs::remove_dir_all(dir).unwrap();
}

fn certify_replication_leader(
    store: &mut DurableRevisionStore,
    membership_epoch: u64,
    term: u64,
    leader: u64,
    voters: &[u64],
) {
    for voter in voters {
        store
            .durably_record_replication_leader_vote(ReplicationLeaderVote {
                voter: ReplicaId::new(*voter),
                membership_epoch,
                term,
                candidate: ReplicaId::new(leader),
            })
            .unwrap();
    }
    store
        .durably_certify_replication_leader(ReplicationLeaderCertificate {
            membership_epoch,
            term,
            leader: ReplicaId::new(leader),
            acknowledged_by: voters.iter().copied().map(ReplicaId::new).collect(),
        })
        .unwrap();
}

fn record_consensus_decision_votes(
    store: &mut DurableRevisionStore,
    template: ReplicationDecisionVote,
    voters: &[u64],
) {
    for voter in voters {
        store
            .durably_record_replication_decision_vote(ReplicationDecisionVote {
                voter: ReplicaId::new(*voter),
                ..template
            })
            .unwrap();
    }
}

fn record_effect_votes(
    store: &mut DurableRevisionStore,
    effect: RevisionEffectId,
    membership_epoch: u64,
    voters: &[u64],
) {
    for voter in voters {
        store
            .durably_record_replicated_effect_vote(ReplicationEffectVote {
                voter: ReplicaId::new(*voter),
                effect,
                membership_epoch,
            })
            .unwrap();
    }
}

fn record_membership_votes(
    store: &mut DurableRevisionStore,
    previous_membership_epoch: u64,
    term: u64,
    next: &ReplicationMembership,
    voters: &[u64],
) {
    for voter in voters {
        store
            .durably_record_replication_membership_vote(ReplicationMembershipVote {
                voter: ReplicaId::new(*voter),
                previous_membership_epoch,
                term,
                next: next.clone(),
            })
            .unwrap();
    }
}

#[test]
fn replication_effect_lifecycle_requires_quorum_before_publication_and_survives_restart() {
    let dir = test_dir("replication-quorum-publication-restart");
    let (base, registry, _) = setup_revision(545, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    let branch = ReplicationBranchId::new(50);
    let envelope = replicated_relation_effect!(
        50,
        1,
        50,
        base.id(),
        RevisionId::new(546),
        BTreeSet::new(),
        semantic_revision,
        1
    );
    let effect = envelope.effect.id;
    store.durably_ingest_replicated_effect(envelope).unwrap();
    assert_eq!(
        store.replication_effect_stage(effect),
        Some(ReplicationEffectStage::LocalDurable)
    );
    assert!(store.replication_published_branch_head(branch).is_none());
    assert!(matches!(
        store.durably_publish_replicated_effect(effect),
        Err(DurabilityError::Protocol {
            reason: "replicated effect cannot publish before quorum durability",
            ..
        })
    ));
    assert!(matches!(
        store.durably_certify_replicated_effect_quorum(quorum_certificate(effect, 1, &[1])),
        Err(DurabilityError::Protocol {
            reason: "replication quorum certificate lacks configured quorum",
            ..
        })
    ));
    record_effect_votes(&mut store, effect, 1, &[1, 2]);
    store
        .durably_certify_replicated_effect_quorum(quorum_certificate(effect, 1, &[1, 2]))
        .unwrap();
    assert_eq!(
        store.replication_effect_stage(effect),
        Some(ReplicationEffectStage::QuorumDurable)
    );
    store.durably_publish_replicated_effect(effect).unwrap();
    assert_eq!(
        store.replication_effect_stage(effect),
        Some(ReplicationEffectStage::Published)
    );
    assert_eq!(
        store
            .replication_published_branch_head(branch)
            .unwrap()
            .head_revision,
        RevisionId::new(546)
    );
    assert_eq!(store.durable_head(), base.id());

    store
        .replication
        .test_semantic_snapshot_roundtrip()
        .unwrap();
    assert_eq!(
        store
            .replication
            .test_semantic_snapshot_retained_history_shape(),
        (1, 1, 0, 1),
    );

    drop(store);
    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.current_replication_membership().unwrap().epoch, 1);
    assert_eq!(
        reopened.replication_effect_stage(effect),
        Some(ReplicationEffectStage::Published)
    );
    assert_eq!(
        reopened
            .replication_published_branch_head(branch)
            .unwrap()
            .head_effect,
        effect
    );
    assert_eq!(reopened.durable_head(), base.id());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replication_membership_rotation_requires_previous_quorum_and_fences_stale_certificates() {
    let dir = test_dir("replication-membership-rotation");
    let (base, registry, _) = setup_revision(550, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    assert!(matches!(
        store.durably_install_replication_membership(membership_change(2, &[2, 3, 4], 2, &[1])),
        Err(DurabilityError::Protocol {
            reason: "replication membership change lacks previous-epoch quorum",
            ..
        })
    ));
    let epoch2 = membership_change(2, &[2, 3, 4], 2, &[1, 2]);
    record_membership_votes(&mut store, 1, 10, &epoch2.next, &[1, 2]);
    store
        .durably_install_replication_membership(epoch2)
        .unwrap();

    let envelope = replicated_relation_effect!(
        51,
        1,
        51,
        base.id(),
        RevisionId::new(551),
        BTreeSet::new(),
        semantic_revision,
        2
    );
    let effect = envelope.effect.id;
    store.durably_ingest_replicated_effect(envelope).unwrap();
    assert!(matches!(
        store.durably_certify_replicated_effect_quorum(quorum_certificate(effect, 1, &[1, 2])),
        Err(DurabilityError::Protocol {
            reason: "replication quorum certificate uses a stale membership epoch",
            ..
        })
    ));
    record_effect_votes(&mut store, effect, 2, &[2, 4]);
    store
        .durably_certify_replicated_effect_quorum(quorum_certificate(effect, 2, &[2, 4]))
        .unwrap();
    assert!(matches!(
        store.durably_install_replication_membership(membership_change(3, &[3, 4, 5], 2, &[1, 2])),
        Err(DurabilityError::Protocol {
            reason: "replication acknowledgement references a non-member replica",
            ..
        })
    ));
    let epoch3 = membership_change(3, &[3, 4, 5], 2, &[2, 3]);
    record_membership_votes(&mut store, 2, 11, &epoch3.next, &[2, 3]);
    store
        .durably_install_replication_membership(epoch3)
        .unwrap();
    drop(store);

    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.current_replication_membership().unwrap().epoch, 3);
    assert_eq!(
        reopened.replication_effect_stage(effect),
        Some(ReplicationEffectStage::QuorumDurable)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replication_vote_once_rejects_conflicting_effects_across_leaders_and_survives_restart() {
    let dir = test_dir("replication-effect-vote-once");
    let (base, registry, _) = setup_revision(552, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    let first = replicated_relation_effect!(
        61,
        1,
        61,
        base.id(),
        RevisionId::new(553),
        BTreeSet::new(),
        semantic_revision,
        40
    );
    let first_id = first.effect.id;
    store.durably_ingest_replicated_effect(first).unwrap();

    let mut second = replicated_relation_effect!(
        62,
        1,
        62,
        base.id(),
        RevisionId::new(554),
        BTreeSet::new(),
        semantic_revision,
        40
    );
    second.ordered_by.sequencer = ReplicaId::new(100);
    second.ordered_by.epoch = 8;
    let second_id = second.effect.id;
    store.durably_ingest_replicated_effect(second).unwrap();

    store
        .durably_record_replicated_effect_vote(ReplicationEffectVote {
            voter: ReplicaId::new(1),
            effect: first_id,
            membership_epoch: 1,
        })
        .unwrap();
    assert!(matches!(
        store.durably_record_replicated_effect_vote(ReplicationEffectVote {
            voter: ReplicaId::new(1),
            effect: second_id,
            membership_epoch: 1,
        }),
        Err(DurabilityError::Protocol {
            reason: "replication voter already voted for another effect in this decision slot",
            ..
        })
    ));
    drop(store);

    let (mut reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert!(matches!(
        reopened.durably_record_replicated_effect_vote(ReplicationEffectVote {
            voter: ReplicaId::new(1),
            effect: second_id,
            membership_epoch: 1,
        }),
        Err(DurabilityError::Protocol {
            reason: "replication voter already voted for another effect in this decision slot",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replication_term_and_leader_authority_fences_stale_leader_and_survives_restart() {
    let dir = test_dir("replication-term-leader-authority");
    let (base, registry, _) = setup_revision(2_601, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    certify_replication_leader(&mut store, 1, 5, 1, &[1, 2]);
    assert!(store.replication_leader_certificate(1, 5).is_some());

    assert!(matches!(
        store.durably_record_replication_leader_vote(ReplicationLeaderVote {
            voter: ReplicaId::new(1),
            membership_epoch: 1,
            term: 5,
            candidate: ReplicaId::new(2),
        }),
        Err(DurabilityError::Protocol {
            reason: "replication voter already voted for another leader in this term",
            ..
        })
    ));
    store
        .durably_record_replication_term_promise(ReplicationTermPromise {
            voter: ReplicaId::new(2),
            membership_epoch: 1,
            term: 6,
        })
        .unwrap();
    assert!(matches!(
        store.durably_record_replication_leader_vote(ReplicationLeaderVote {
            voter: ReplicaId::new(2),
            membership_epoch: 1,
            term: 5,
            candidate: ReplicaId::new(1),
        }),
        Err(DurabilityError::Protocol {
            reason: "replication leader vote uses a stale term",
            ..
        })
    ));
    assert!(matches!(
        store.durably_record_replication_term_promise(ReplicationTermPromise {
            voter: ReplicaId::new(2),
            membership_epoch: 1,
            term: 5,
        }),
        Err(DurabilityError::Protocol {
            reason: "replication term promise regressed",
            ..
        })
    ));
    assert!(matches!(
        store.durably_certify_replication_leader(ReplicationLeaderCertificate {
            membership_epoch: 1,
            term: 5,
            leader: ReplicaId::new(1),
            acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
        }),
        Ok(())
    ));
    drop(store);

    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(
        reopened.replication_promised_term(1, ReplicaId::new(2)),
        Some(6)
    );
    assert_eq!(
        reopened
            .replication_leader_certificate(1, 5)
            .unwrap()
            .leader,
        ReplicaId::new(1)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(clippy::too_many_lines)]
fn replication_decision_lock_requires_leader_quorum_and_safe_carry_forward() {
    let dir = test_dir("replication-decision-lock-carry-forward");
    let (base, registry, _) = setup_revision(2_610, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    let first = replicated_relation_effect!(
        201,
        1,
        201,
        base.id(),
        RevisionId::new(2_611),
        BTreeSet::new(),
        semantic_revision,
        70
    );
    let first_id = first.effect.id;
    store.durably_ingest_replicated_effect(first).unwrap();
    let mut conflicting = replicated_relation_effect!(
        202,
        1,
        202,
        base.id(),
        RevisionId::new(2_612),
        BTreeSet::new(),
        semantic_revision,
        70
    );
    conflicting.ordered_by.sequencer = ReplicaId::new(100);
    conflicting.ordered_by.epoch = 8;
    let conflicting_id = conflicting.effect.id;
    store.durably_ingest_replicated_effect(conflicting).unwrap();

    certify_replication_leader(&mut store, 1, 5, 1, &[1, 2]);
    record_consensus_decision_votes(
        &mut store,
        ReplicationDecisionVote {
            voter: ReplicaId::new(1),
            membership_epoch: 1,
            term: 5,
            leader: ReplicaId::new(1),
            position: 70,
            effect: first_id,
            carried_from_term: None,
        },
        &[1, 2],
    );
    record_effect_votes(&mut store, first_id, 1, &[1, 2]);
    assert!(matches!(
        store.durably_certify_replicated_effect_quorum(quorum_certificate(first_id, 1, &[1, 2])),
        Err(DurabilityError::Protocol {
            reason: "replication quorum lacks consensus decision lock",
            ..
        })
    ));
    store
        .durably_lock_replication_decision(ReplicationDecisionLock {
            membership_epoch: 1,
            term: 5,
            leader: ReplicaId::new(1),
            position: 70,
            effect: first_id,
            acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
            carried_from_term: None,
        })
        .unwrap();
    store
        .durably_certify_replicated_effect_quorum(quorum_certificate(first_id, 1, &[1, 2]))
        .unwrap();

    certify_replication_leader(&mut store, 1, 6, 2, &[1, 2]);

    assert!(matches!(
        store.durably_record_replication_decision_vote(ReplicationDecisionVote {
            voter: ReplicaId::new(1),
            membership_epoch: 1,
            term: 6,
            leader: ReplicaId::new(2),
            position: 70,
            effect: conflicting_id,
            carried_from_term: Some(5),
        }),
        Err(DurabilityError::Protocol {
            reason: "replication decision conflicts with a durable locked value",
            ..
        })
    ));
    assert!(matches!(
        store.durably_record_replication_decision_vote(ReplicationDecisionVote {
            voter: ReplicaId::new(1),
            membership_epoch: 1,
            term: 6,
            leader: ReplicaId::new(2),
            position: 70,
            effect: first_id,
            carried_from_term: None,
        }),
        Err(DurabilityError::Protocol {
            reason: "replication later-term decision did not carry forward the durable lock",
            ..
        })
    ));
    record_consensus_decision_votes(
        &mut store,
        ReplicationDecisionVote {
            voter: ReplicaId::new(1),
            membership_epoch: 1,
            term: 6,
            leader: ReplicaId::new(2),
            position: 70,
            effect: first_id,
            carried_from_term: Some(5),
        },
        &[1, 2],
    );
    store
        .durably_lock_replication_decision(ReplicationDecisionLock {
            membership_epoch: 1,
            term: 6,
            leader: ReplicaId::new(2),
            position: 70,
            effect: first_id,
            acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
            carried_from_term: Some(5),
        })
        .unwrap();
    assert_eq!(store.replication_decision_lock(70).unwrap().term, 6);
    drop(store);

    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    let lock = reopened.replication_decision_lock(70).unwrap();
    assert_eq!(lock.effect, first_id);
    assert_eq!(lock.term, 6);
    assert_eq!(lock.carried_from_term, Some(5));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replication_joint_membership_requires_old_and_new_quorums_in_certified_term() {
    let dir = test_dir("replication-joint-membership");
    let (base, registry, _) = setup_revision(2_620, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    certify_replication_leader(&mut store, 1, 10, 1, &[1, 2]);

    let next = membership_change(2, &[4, 5, 6], 2, &[1, 2]);
    record_membership_votes(&mut store, 1, 10, &next.next, &[1, 2]);
    assert!(matches!(
        store.durably_install_replication_membership(next.clone()),
        Err(DurabilityError::Protocol {
            reason: "replication membership change lacks joint quorum certificate",
            ..
        })
    ));
    assert!(matches!(
        store.durably_certify_replication_joint_membership(ReplicationJointMembershipCertificate {
            previous_membership_epoch: 1,
            term: 10,
            leader: ReplicaId::new(1),
            next: next.next.clone(),
            acknowledged_by_previous: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
            acknowledged_by_next: [ReplicaId::new(4)].into_iter().collect(),
        }),
        Err(DurabilityError::Protocol {
            reason: "joint membership certificate lacks successor-membership quorum",
            ..
        })
    ));
    store
        .durably_certify_replication_joint_membership(ReplicationJointMembershipCertificate {
            previous_membership_epoch: 1,
            term: 10,
            leader: ReplicaId::new(1),
            next: next.next.clone(),
            acknowledged_by_previous: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
            acknowledged_by_next: [ReplicaId::new(4), ReplicaId::new(5)].into_iter().collect(),
        })
        .unwrap();
    store.durably_install_replication_membership(next).unwrap();
    assert_eq!(store.current_replication_membership().unwrap().epoch, 2);
    drop(store);

    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.current_replication_membership().unwrap().epoch, 2);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replication_joint_membership_is_fenced_by_later_term_promise() {
    let dir = test_dir("replication-joint-membership-stale-term");
    let (base, registry, _) = setup_revision(2_630, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    certify_replication_leader(&mut store, 1, 10, 1, &[1, 2]);
    let next = membership_change(2, &[4, 5, 6], 2, &[1, 2]);
    record_membership_votes(&mut store, 1, 10, &next.next, &[1, 2]);
    store
        .durably_certify_replication_joint_membership(ReplicationJointMembershipCertificate {
            previous_membership_epoch: 1,
            term: 10,
            leader: ReplicaId::new(1),
            next: next.next.clone(),
            acknowledged_by_previous: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
            acknowledged_by_next: [ReplicaId::new(4), ReplicaId::new(5)].into_iter().collect(),
        })
        .unwrap();
    store
        .durably_record_replication_term_promise(ReplicationTermPromise {
            voter: ReplicaId::new(3),
            membership_epoch: 1,
            term: 11,
        })
        .unwrap();
    assert!(matches!(
        store.durably_install_replication_membership(next),
        Err(DurabilityError::Protocol {
            reason: "replication membership change uses a stale consensus term",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn authenticated_transport_routes_peer_evidence_but_keeps_heartbeat_advisory() {
    let dir = test_dir("replication-authenticated-transport-route");
    let (revision, registry, _) = setup_revision(1, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[2], 1, &[]))
        .unwrap();

    let signing = SigningKey::from_bytes(&[42_u8; 32]);
    let verifying = signing.verifying_key().to_bytes();
    let signer_id = key_id(&verifying);
    let trust = TrustRootSet::bootstrap(1, &[verifying]).unwrap();
    let policy = ReplicationPeerAuthPolicy {
        cluster: crate::ReplicationClusterId([3; 32]),
        trust_epoch: 1,
        peer_keys: BTreeMap::from([(ReplicaId::new(2), signer_id)]),
    };
    store
        .durably_install_replication_peer_auth_policy(policy.clone(), &trust)
        .unwrap();

    let promise = ReplicationPeerEvidence::TermPromise(ReplicationTermPromise {
        voter: ReplicaId::new(2),
        membership_epoch: 1,
        term: 4,
    });
    let inner = sign_replication_evidence(&policy, &signing, promise);
    let frame = ReplicationTransportFrame {
        cluster: policy.cluster,
        trust_epoch: 1,
        sender: ReplicaId::new(2),
        sequence: 1,
        payload: ReplicationTransportPayload::PeerEvidence(inner),
    };
    let transport_signed = SignedReplicationTransportFrame {
        signature: signing
            .sign(&replication_transport_signing_message(&frame).unwrap())
            .to_bytes(),
        frame,
    };
    let transport_bytes =
        crate::encode_signed_replication_transport_frame(&transport_signed).unwrap();
    let mut ingress = ReplicationTransportIngress::new();
    let receipt = store
        .durably_accept_replication_transport_bytes(&mut ingress, &trust, &transport_bytes)
        .unwrap()
        .expect("authority evidence receipt");
    assert_eq!(receipt.voter, ReplicaId::new(2));
    assert_eq!(
        store.replication_promised_term(1, ReplicaId::new(2)),
        Some(4)
    );

    let before = store.replication_quorum_availability();
    let heartbeat = ReplicationTransportFrame {
        cluster: policy.cluster,
        trust_epoch: 1,
        sender: ReplicaId::new(2),
        sequence: 2,
        payload: ReplicationTransportPayload::Heartbeat(ReplicationHeartbeat {
            membership_epoch: 1,
            term: 4,
            logical_tick: 10,
        }),
    };
    let signed_heartbeat = SignedReplicationTransportFrame {
        signature: signing
            .sign(&replication_transport_signing_message(&heartbeat).unwrap())
            .to_bytes(),
        frame: heartbeat,
    };
    assert!(
        store
            .durably_accept_replication_transport_frame(&mut ingress, &trust, signed_heartbeat,)
            .unwrap()
            .is_none()
    );
    assert_eq!(store.replication_quorum_availability(), before);
}

#[test]
fn anti_entropy_summary_chunk_and_failure_detector_are_non_authoritative_until_fenced() {
    let dir = test_dir("replication-anti-entropy-failure-detector");
    let (revision, registry, _) = setup_revision(1, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    let summary = store
        .replication_anti_entropy_summary()
        .unwrap()
        .expect("summary");
    assert_eq!(summary.membership_epoch, 1);
    assert_eq!(summary.lock_count, 0);
    let chunk = store
        .replication_anti_entropy_chunk(ReplicationAntiEntropyRequest {
            membership_epoch: 1,
            from_position: 0,
            max_locks: 8,
        })
        .unwrap();
    assert!(chunk.complete);
    assert!(chunk.locks.is_empty());
    let before_advisory = store.replication_quorum_availability();
    assert_eq!(
        before_advisory,
        Some(ReplicationQuorumAvailability::Available {
            membership_epoch: 1,
            term: 0,
        })
    );
    assert_eq!(store.replication_quorum_availability(), before_advisory);

    let mut detector = ReplicationFailureDetector::new(ReplicaId::new(1), 2).unwrap();
    detector.observe_authenticated(ReplicaId::new(2));
    assert!(
        !store
            .durably_fence_replication_if_quorum_unreachable(&detector, 1)
            .unwrap()
    );
    detector.advance_to(3).unwrap();
    assert!(
        store
            .durably_fence_replication_if_quorum_unreachable(&detector, 1)
            .unwrap()
    );
    assert!(matches!(
        store.replication_quorum_availability(),
        Some(ReplicationQuorumAvailability::Lost(_))
    ));
}

#[test]
fn hostile_sparse_replication_frame_is_rejected_before_payload_allocation() {
    let dir = test_dir("replication-sparse-frame-bound");
    let (base, registry, _) = setup_revision(5579, &[1]);
    let store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let journal_path = store.replication_journal_path().to_path_buf();
    drop(store);

    let declared_len = u32::try_from(crate::MAX_PAYLOAD_LEN + 1).unwrap();
    let mut header = [0_u8; 16];
    header[..4].copy_from_slice(b"CFRP");
    header[4..6].copy_from_slice(&1_u16.to_le_bytes());
    header[6] = 1;
    header[8..12].copy_from_slice(&declared_len.to_le_bytes());
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&journal_path)
        .unwrap();
    file.write_all(&header).unwrap();
    file.set_len(16 + u64::from(declared_len)).unwrap();
    file.sync_all().unwrap();
    drop(file);

    assert!(matches!(
        DurableRevisionStore::open(&dir),
        Err(DurabilityError::PayloadTooLarge)
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replication_membership_vote_once_blocks_conflicting_successors_across_restart() {
    let dir = test_dir("replication-membership-vote-once");
    let (base, registry, _) = setup_revision(558, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    let first = membership_change(2, &[2, 3, 4], 2, &[1, 2]).next;
    let conflicting = membership_change(2, &[1, 3, 4], 2, &[1, 3]).next;
    store
        .durably_record_replication_membership_vote(ReplicationMembershipVote {
            voter: ReplicaId::new(1),
            previous_membership_epoch: 1,
            term: 10,
            next: first,
        })
        .unwrap();
    assert!(matches!(
        store.durably_record_replication_membership_vote(ReplicationMembershipVote {
            voter: ReplicaId::new(1),
            previous_membership_epoch: 1,
            term: 11,
            next: conflicting.clone(),
        }),
        Err(DurabilityError::Protocol {
            reason: "replication voter already voted for another successor membership",
            ..
        })
    ));
    drop(store);

    let (mut reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert!(matches!(
        reopened.durably_record_replication_membership_vote(ReplicationMembershipVote {
            voter: ReplicaId::new(1),
            previous_membership_epoch: 1,
            term: 12,
            next: conflicting,
        }),
        Err(DurabilityError::Protocol {
            reason: "replication voter already voted for another successor membership",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replication_membership_votes_share_one_successor_owner_across_restart() {
    let dir = test_dir("replication-membership-vote-owner-dedup");
    let (base, registry, _) = setup_revision(5581, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    let successor = membership_change(2, &[2, 3, 4], 2, &[1, 2]).next;
    for voter in [1, 2] {
        store
            .durably_record_replication_membership_vote(ReplicationMembershipVote {
                voter: ReplicaId::new(voter),
                previous_membership_epoch: 1,
                term: 10,
                next: successor.clone(),
            })
            .unwrap();
    }
    assert_eq!(store.replication.membership_vote_storage_shape(), (2, 1));
    drop(store);

    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.replication.membership_vote_storage_shape(), (2, 1));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn authenticated_membership_votes_persist_successor_once_not_per_voter() {
    let dir = test_dir("replication-membership-vote-durable-owner-dedup");
    let (base, registry, _) = setup_revision(5582, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();

    let key1 = replica_signing_key(71);
    let key2 = replica_signing_key(72);
    let key3 = replica_signing_key(73);
    let trust = replication_trust(1, &[&key1, &key2, &key3]);
    let policy = replication_auth_policy(1, &[(1, &key1), (2, &key2), (3, &key3)]);
    store
        .durably_install_replication_peer_auth_policy(policy.clone(), &trust)
        .unwrap();

    let successor = ReplicationMembership {
        epoch: 2,
        members: (100_u64..1124_u64).map(ReplicaId::new).collect(),
        quorum_size: 513,
    };
    let before = fs::metadata(store.replication_journal_path())
        .unwrap()
        .len();
    record_signed_replication_evidence(
        &mut store,
        &trust,
        &policy,
        &key1,
        ReplicationPeerEvidence::MembershipVote(ReplicationMembershipVote {
            voter: ReplicaId::new(1),
            previous_membership_epoch: 1,
            term: 10,
            next: successor.clone(),
        }),
    );
    let after_first = fs::metadata(store.replication_journal_path())
        .unwrap()
        .len();
    record_signed_replication_evidence(
        &mut store,
        &trust,
        &policy,
        &key2,
        ReplicationPeerEvidence::MembershipVote(ReplicationMembershipVote {
            voter: ReplicaId::new(2),
            previous_membership_epoch: 1,
            term: 10,
            next: successor,
        }),
    );
    let after_second = fs::metadata(store.replication_journal_path())
        .unwrap()
        .len();

    let first_vote_bytes = after_first - before;
    let second_vote_bytes = after_second - after_first;
    assert!(first_vote_bytes > 8_000);
    assert!(second_vote_bytes < 512);
    assert!(first_vote_bytes > second_vote_bytes * 10);
    assert_eq!(store.replication.membership_vote_storage_shape(), (2, 1));
    drop(store);

    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.replication.membership_vote_storage_shape(), (2, 1));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replicated_branch_publication_is_contiguous_even_when_later_effect_is_quorum_durable() {
    let dir = test_dir("replication-publication-contiguous");
    let (base, registry, _) = setup_revision(555, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    let first = replicated_relation_effect!(
        52,
        1,
        52,
        base.id(),
        RevisionId::new(556),
        BTreeSet::new(),
        semantic_revision,
        3
    );
    let first_id = first.effect.id;
    store.durably_ingest_replicated_effect(first).unwrap();
    let second = replicated_relation_effect!(
        52,
        2,
        52,
        RevisionId::new(556),
        RevisionId::new(557),
        BTreeSet::from([first_id]),
        semantic_revision,
        4
    );
    let second_id = second.effect.id;
    store.durably_ingest_replicated_effect(second).unwrap();
    record_effect_votes(&mut store, first_id, 1, &[1, 2]);
    store
        .durably_certify_replicated_effect_quorum(quorum_certificate(first_id, 1, &[1, 2]))
        .unwrap();
    record_effect_votes(&mut store, second_id, 1, &[2, 3]);
    store
        .durably_certify_replicated_effect_quorum(quorum_certificate(second_id, 1, &[2, 3]))
        .unwrap();
    assert!(matches!(
        store.durably_publish_replicated_effect(second_id),
        Err(DurabilityError::Protocol {
            reason: "replicated branch publication skipped an earlier local-durable effect",
            ..
        })
    ));
    store.durably_publish_replicated_effect(first_id).unwrap();
    store.durably_publish_replicated_effect(second_id).unwrap();
    assert_eq!(
        store
            .replication_published_branch_head(ReplicationBranchId::new(52))
            .unwrap()
            .head_effect,
        second_id
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replicated_publication_cannot_outrun_quorum_durability_of_remote_causal_prefix() {
    let dir = test_dir("replication-publication-causal-quorum");
    let (base, registry, _) = setup_revision(560, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    let prerequisite = replicated_relation_effect!(
        53,
        1,
        53,
        base.id(),
        RevisionId::new(561),
        BTreeSet::new(),
        semantic_revision,
        5
    );
    let prerequisite_id = prerequisite.effect.id;
    store
        .durably_ingest_replicated_effect(prerequisite)
        .unwrap();
    let dependent = replicated_relation_effect!(
        54,
        1,
        54,
        RevisionId::new(561),
        RevisionId::new(562),
        BTreeSet::from([prerequisite_id]),
        semantic_revision,
        6
    );
    let dependent_id = dependent.effect.id;
    store.durably_ingest_replicated_effect(dependent).unwrap();
    record_effect_votes(&mut store, dependent_id, 1, &[1, 2]);
    store
        .durably_certify_replicated_effect_quorum(quorum_certificate(dependent_id, 1, &[1, 2]))
        .unwrap();
    assert!(matches!(
        store.durably_publish_replicated_effect(dependent_id),
        Err(DurabilityError::Protocol {
            reason: "replicated effect cannot publish before replicated prerequisites are quorum durable",
            ..
        })
    ));
    record_effect_votes(&mut store, prerequisite_id, 1, &[2, 3]);
    store
        .durably_certify_replicated_effect_quorum(quorum_certificate(prerequisite_id, 1, &[2, 3]))
        .unwrap();
    store
        .durably_publish_replicated_effect(dependent_id)
        .unwrap();
    assert_eq!(
        store.replication_effect_stage(dependent_id),
        Some(ReplicationEffectStage::Published)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn independent_replication_branches_survive_restart_and_retirement() {
    let dir = test_dir("replication-branches-restart");
    let (base, registry, _) = setup_revision(500, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();

    let branch_a = ReplicationBranchId::new(1);
    let branch_b = ReplicationBranchId::new(2);
    let a1 = replicated_relation_effect!(
        10,
        1,
        1,
        base.id(),
        RevisionId::new(501),
        BTreeSet::new(),
        semantic_revision,
        1
    );
    let a1_id = a1.effect.id;
    assert_eq!(
        store.durably_ingest_replicated_effect(a1).unwrap(),
        ReplicationIngestOutcome::Inserted
    );
    let a2 = replicated_relation_effect!(
        10,
        2,
        1,
        RevisionId::new(501),
        RevisionId::new(502),
        BTreeSet::from([a1_id]),
        semantic_revision,
        2
    );
    let a2_id = a2.effect.id;
    store.durably_ingest_replicated_effect(a2).unwrap();
    let b1 = replicated_relation_effect!(
        11,
        1,
        2,
        base.id(),
        RevisionId::new(510),
        BTreeSet::new(),
        semantic_revision,
        3
    );
    let b1_id = b1.effect.id;
    store.durably_ingest_replicated_effect(b1).unwrap();

    assert_eq!(store.durable_head(), base.id());
    let head_a = store.replication_branch_head(branch_a).unwrap();
    assert_eq!(head_a.head_revision, RevisionId::new(502));
    assert_eq!(
        store.replication_branch_head(branch_b).unwrap().head_effect,
        b1_id
    );
    let ideal_a = store
        .replicated_branch_effect_ideal(branch_a)
        .unwrap()
        .unwrap();
    assert_eq!(ideal_a.events().len(), 2);

    drop(store);
    let (mut reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.durable_head(), base.id());
    assert_eq!(
        reopened
            .replication_branch_head(branch_a)
            .unwrap()
            .head_effect,
        a2_id
    );
    let ideal_b = reopened
        .replicated_branch_effect_ideal(branch_b)
        .unwrap()
        .unwrap();
    assert_eq!(ideal_b.events().len(), 1);
    reopened
        .durably_retire_replication_branch(branch_a, a2_id)
        .unwrap();
    drop(reopened);

    let (mut reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert!(reopened.replication_branch_head(branch_a).unwrap().retired);
    let after_retirement = replicated_relation_effect!(
        10,
        3,
        1,
        RevisionId::new(502),
        RevisionId::new(503),
        BTreeSet::from([a2_id]),
        semantic_revision,
        4
    );
    assert!(matches!(
        reopened.durably_ingest_replicated_effect(after_retirement),
        Err(DurabilityError::Protocol {
            reason: "retired replication branch cannot be advanced",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replicated_admission_rejects_stale_causal_cut_and_conflicting_order_slot() {
    let dir = test_dir("replication-admission-hostile");
    let (base, registry, _) = setup_revision(520, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let bad = replicated_relation_effect!(
        20,
        1,
        20,
        base.id(),
        RevisionId::new(521),
        BTreeSet::from([RevisionEffectId(77)]),
        semantic_revision,
        1
    );
    let before = fs::metadata(store.replication_journal_path())
        .unwrap()
        .len();
    assert!(matches!(
        store.durably_ingest_replicated_effect(bad),
        Err(DurabilityError::Protocol {
            reason: "replicated effect prerequisites do not equal the authoritative causal cut",
            ..
        })
    ));
    assert_eq!(
        fs::metadata(store.replication_journal_path())
            .unwrap()
            .len(),
        before
    );

    let first = replicated_relation_effect!(
        20,
        1,
        20,
        base.id(),
        RevisionId::new(521),
        BTreeSet::new(),
        semantic_revision,
        1
    );
    store.durably_ingest_replicated_effect(first).unwrap();
    let second = replicated_relation_effect!(
        21,
        1,
        21,
        base.id(),
        RevisionId::new(522),
        BTreeSet::new(),
        semantic_revision,
        1
    );
    assert!(matches!(
        store.durably_ingest_replicated_effect(second),
        Err(DurabilityError::Protocol {
            reason: "sequencer order slot is already bound to another effect",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replicated_admission_fences_stale_sequencer_epochs() {
    let dir = test_dir("replication-epoch-fence");
    let (base, registry, _) = setup_revision(525, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let newer_epoch = ReplicatedEffectEnvelope {
        ordered_by: DurableSequencerOrder {
            sequencer: ReplicaId::new(99),
            epoch: 8,
            position: 1,
        },
        ..replicated_relation_effect!(
            22,
            1,
            22,
            base.id(),
            RevisionId::new(526),
            BTreeSet::new(),
            semantic_revision,
            2
        )
    };
    store.durably_ingest_replicated_effect(newer_epoch).unwrap();
    let stale_epoch = ReplicatedEffectEnvelope {
        ordered_by: DurableSequencerOrder {
            sequencer: ReplicaId::new(99),
            epoch: 7,
            position: 9,
        },
        ..replicated_relation_effect!(
            23,
            1,
            23,
            base.id(),
            RevisionId::new(527),
            BTreeSet::new(),
            semantic_revision,
            9
        )
    };
    assert!(matches!(
        store.durably_ingest_replicated_effect(stale_epoch),
        Err(DurabilityError::Protocol {
            reason: "replicated effect uses a stale sequencer epoch",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replication_journal_truncates_only_an_incomplete_tail_on_reopen() {
    let dir = test_dir("replication-truncated-tail");
    let (base, registry, _) = setup_revision(530, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let branch = ReplicationBranchId::new(30);
    let effect = replicated_relation_effect!(
        30,
        1,
        30,
        base.id(),
        RevisionId::new(531),
        BTreeSet::new(),
        semantic_revision,
        1
    );
    let effect_id = effect.effect.id;
    store.durably_ingest_replicated_effect(effect).unwrap();
    let journal = store.replication_journal_path().to_path_buf();
    let good_len = fs::metadata(&journal).unwrap().len();
    drop(store);

    OpenOptions::new()
        .append(true)
        .open(&journal)
        .unwrap()
        .write_all(b"CFRP\x01")
        .unwrap();
    let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(fs::metadata(&journal).unwrap().len(), good_len);
    assert_eq!(
        reopened
            .replication_branch_head(branch)
            .unwrap()
            .head_effect,
        effect_id
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replication_journal_rejects_nonzero_reserved_header_byte() {
    let dir = test_dir("replication-reserved-header-byte");
    let (base, registry, _) = setup_revision(535, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let effect = replicated_relation_effect!(
        35,
        1,
        35,
        base.id(),
        RevisionId::new(536),
        BTreeSet::new(),
        semantic_revision,
        1
    );
    store.durably_ingest_replicated_effect(effect).unwrap();
    let journal = store.replication_journal_path().to_path_buf();
    drop(store);

    let mut bytes = fs::read(&journal).unwrap();
    assert!(bytes.len() >= 16);
    bytes[7] = 1;
    fs::write(&journal, bytes).unwrap();
    assert!(matches!(
        DurableRevisionStore::open(&dir),
        Err(DurabilityError::Corruption {
            reason: "replication journal reserved header byte is non-zero",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn replication_journal_checksum_corruption_is_not_silently_truncated() {
    let dir = test_dir("replication-checksum-corruption");
    let (base, registry, _) = setup_revision(540, &[1]);
    let semantic_revision = base.semantic_revision();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let effect = replicated_relation_effect!(
        40,
        1,
        40,
        base.id(),
        RevisionId::new(541),
        BTreeSet::new(),
        semantic_revision,
        1
    );
    let same = effect.clone();
    assert_eq!(
        store.durably_ingest_replicated_effect(effect).unwrap(),
        ReplicationIngestOutcome::Inserted
    );
    let size = fs::metadata(store.replication_journal_path())
        .unwrap()
        .len();
    assert_eq!(
        store.durably_ingest_replicated_effect(same).unwrap(),
        ReplicationIngestOutcome::AlreadyPresent
    );
    assert_eq!(
        fs::metadata(store.replication_journal_path())
            .unwrap()
            .len(),
        size
    );
    let journal = store.replication_journal_path().to_path_buf();
    drop(store);

    let mut bytes = fs::read(&journal).unwrap();
    *bytes.last_mut().unwrap() ^= 0x55;
    fs::write(&journal, bytes).unwrap();
    assert!(matches!(
        DurableRevisionStore::open(&dir),
        Err(DurabilityError::Corruption {
            reason: "replication journal payload checksum mismatch",
            ..
        })
    ));
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn prepare_rejects_retry_key_conflict_before_wal_mutation() {
    let dir = test_dir("prepare-retry-key-conflict-pre-wal");
    let (base, registry, relation) = setup_revision(2_500, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let first = committed_descriptor(&base, &registry, relation, 2_501, 11);
    let mut conflicting = committed_descriptor(&base, &registry, relation, 2_502, 12);
    conflicting.transaction_id = first.transaction_id;

    store.durably_prepare(&first).unwrap();
    let before_lsn = store.wal.last_lsn();
    assert!(matches!(
        store.durably_prepare(&conflicting),
        Err(DurabilityError::Protocol {
            reason: "transaction retry key already belongs to prepared descriptor",
            ..
        })
    ));
    assert_eq!(store.wal.last_lsn(), before_lsn);
    assert!(!store.requires_recovery());
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    assert_eq!(reopened.durable_head, base.id());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prepare_rejects_target_collision_with_existing_prepare_before_wal_mutation() {
    let dir = test_dir("prepare-target-conflict-pre-wal");
    let (base, registry, relation) = setup_revision(2_550, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let first = committed_descriptor(&base, &registry, relation, 2_551, 21);
    let mut conflicting = committed_descriptor(&base, &registry, relation, 2_551, 22);
    conflicting.transaction_id = ClientTransactionId::new(9_999);

    store.durably_prepare(&first).unwrap();
    let before_lsn = store.wal.last_lsn();
    assert!(matches!(
        store.durably_prepare(&conflicting),
        Err(DurabilityError::Protocol {
            reason: "target revision already belongs to prepared descriptor",
            ..
        })
    ));
    assert_eq!(store.wal.last_lsn(), before_lsn);
    assert!(!store.requires_recovery());
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    assert_eq!(reopened.durable_head, base.id());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn commit_rejects_stale_prepared_source_before_wal_mutation() {
    let dir = test_dir("commit-stale-prepared-source-pre-wal");
    let (base, registry, relation) = setup_revision(2_575, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let first = committed_descriptor(&base, &registry, relation, 2_576, 31);
    let stale = committed_descriptor(&base, &registry, relation, 2_577, 32);

    let first_token = store.durably_prepare(&first).unwrap();
    let stale_token = store.durably_prepare(&stale).unwrap();
    store.durably_commit(first_token).unwrap();
    let before_lsn = store.wal.last_lsn();

    assert!(matches!(
        store.durably_commit(stale_token),
        Err(DurabilityError::Protocol {
            reason: "commit prepared source no longer matches durable store head",
            ..
        })
    ));
    assert_eq!(store.wal.last_lsn(), before_lsn);
    assert_eq!(store.durable_head, first.target_revision);
    assert!(!store.requires_recovery());
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), first.target_revision);
    assert_eq!(reopened.durable_head, first.target_revision);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_commit_freshness_advance_keeps_live_authority_at_durable_commit() {
    let dir = test_dir("commit-freshness-live-authority-first");
    let (base, registry, relation) = setup_revision(2_600, &[1]);
    let (config, authority) = external_freshness_fixture([0x5a; 32]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    let descriptor = committed_descriptor(&base, &registry, relation, 2_601, 13);
    let transaction_id = descriptor.transaction_id;
    let token = store.durably_prepare(&descriptor).unwrap();

    authority.fail_once(FRESHNESS_FAIL_BEFORE_APPLY);
    assert!(matches!(
        store.durably_commit(token),
        Err(DurabilityError::Io(_))
    ));
    assert!(store.requires_recovery());
    assert_eq!(store.durable_head, descriptor.target_revision);
    assert!(
        store
            .revision_effect_frontier(descriptor.target_revision)
            .is_some()
    );
    assert!(matches!(
        store.transaction_outcome(transaction_id),
        DurableTransactionOutcome::Committed { target_revision }
            if target_revision == descriptor.target_revision
    ));
    drop(store);

    let (reopened, scan) =
        DurableRevisionStore::open_with_external_freshness(&dir, config, authority.boxed())
            .unwrap();
    assert_eq!(scan.durable_revision(), descriptor.target_revision);
    assert_eq!(reopened.durable_head, descriptor.target_revision);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_reopen_restores_unresolved_prepare_identity_before_wal_mutation() {
    let dir = test_dir("streaming-reopen-unresolved-prepare-identity");
    let (base, registry, relation) = setup_revision(2_700, &[1]);
    let (_target, first) = transition_from(&base, &registry, relation, 2_701, 41);
    let (_other_target, mut conflicting) = transition_from(&base, &registry, relation, 2_702, 42);
    conflicting.transaction_id = first.transaction_id;
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();

    store.durably_prepare(&first).unwrap();
    store
        .begin_streaming_checkpoint_with_chunk_size(&base, 32)
        .unwrap();
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    store.finalize_streaming_checkpoint().unwrap();
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    assert_eq!(reopened.prepared_transactions.len(), 1);
    let before_lsn = reopened.wal.last_lsn();
    assert!(matches!(
        reopened.durably_prepare(&conflicting),
        Err(DurabilityError::Protocol {
            reason: "transaction retry key already belongs to prepared descriptor",
            ..
        })
    ));
    assert_eq!(reopened.wal.last_lsn(), before_lsn);
    drop(reopened);

    let (_reopened_again, scan_again) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan_again.durable_revision(), base.id());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn committed_prepare_retires_before_future_checkpoint_capsule() {
    let dir = test_dir("committed-prepare-retired-from-capsule");
    let (base, registry, relation) = setup_revision(2_750, &[1]);
    let (target, descriptor) = transition_from(&base, &registry, relation, 2_751, 51);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();

    let token = store.durably_prepare(&descriptor).unwrap();
    assert_eq!(store.prepared_transactions.len(), 1);
    store.durably_commit(token).unwrap();
    assert_eq!(store.prepared_transactions.len(), 0);

    let started = store
        .begin_streaming_checkpoint_with_chunk_size(&target, 32)
        .unwrap();
    let capsule = decode_prepared_cut_capsule(
        &fs::read(prepared_capsule_path(&dir, started.generation)).unwrap(),
    )
    .unwrap();
    assert_eq!(capsule.len(), 0);
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    store.finalize_streaming_checkpoint().unwrap();
    drop(store);

    let (_reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_publication_discards_pre_cut_stale_prepare_tombstone() {
    let dir = test_dir("streaming-prunes-pre-cut-stale-prepare");
    let (base, registry, relation) = setup_revision(2_800, &[1]);
    let (_stale_target, stale) = transition_from(&base, &registry, relation, 2_801, 61);
    let (winner_target, winner) = transition_from(&base, &registry, relation, 2_802, 62);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();

    store.durably_prepare(&stale).unwrap();
    let winner_token = store.durably_prepare(&winner).unwrap();
    store.durably_commit(winner_token).unwrap();
    assert_eq!(store.prepared_transactions.len(), 1);

    let started = store
        .begin_streaming_checkpoint_with_chunk_size(&winner_target, 32)
        .unwrap();
    let capsule = decode_prepared_cut_capsule(
        &fs::read(prepared_capsule_path(&dir, started.generation)).unwrap(),
    )
    .unwrap();
    assert_eq!(capsule.len(), 0);
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    store.finalize_streaming_checkpoint().unwrap();
    assert_eq!(store.prepared_transactions.len(), 0);
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), winner_target.id());
    assert_eq!(reopened.prepared_transactions.len(), 0);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn recovered_prepare_identity_is_tombstone_not_commit_authority() {
    let dir = test_dir("recovered-prepare-is-tombstone");
    let (base, registry, relation) = setup_revision(2_725, &[1]);
    let (_target, descriptor) = transition_from(&base, &registry, relation, 2_726, 46);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let token = store.durably_prepare(&descriptor).unwrap();
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    assert_eq!(reopened.prepared_transactions.len(), 1);
    let before_lsn = reopened.wal.last_lsn();
    assert!(matches!(
        reopened.durably_commit(token),
        Err(DurabilityError::Protocol {
            reason: "commit token has no prepared descriptor in this store",
            ..
        })
    ));
    assert_eq!(reopened.wal.last_lsn(), before_lsn);
    assert_eq!(reopened.durable_head(), base.id());
    drop(reopened);

    let (_reopened_again, scan_again) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan_again.durable_revision(), base.id());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_store_commits_reopens_rotates_and_reopens_without_sidecars() {
    let dir = test_dir("single-file-store-live-roundtrip");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(40_000, &[1]);
    let (revision_two, _, _) = setup_revision(40_001, &[1, 2]);
    let descriptor_two = DurableRevisionDescriptor::full_revision(
        ClientTransactionId::new(40_001),
        base.id(),
        &revision_two,
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();
    let prepared = store.durably_prepare(&descriptor_two).unwrap();
    store.durably_commit(prepared).unwrap();
    assert_eq!(store.durable_head(), revision_two.id());
    drop(store);

    let (mut store, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(scan.durable_revision(), revision_two.id());
    assert_eq!(store.durable_head(), revision_two.id());
    store.rotate_checkpoint(&revision_two).unwrap();
    assert_eq!(store.generation(), 2);

    let (revision_three, _, _) = setup_revision(40_002, &[1, 2, 3]);
    let descriptor_three = DurableRevisionDescriptor::full_revision(
        ClientTransactionId::new(40_002),
        revision_two.id(),
        &revision_three,
        &registry,
    )
    .unwrap();
    let prepared = store.durably_prepare(&descriptor_three).unwrap();
    store.durably_commit(prepared).unwrap();
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(scan.durable_revision(), revision_three.id());
    assert_eq!(reopened.checkpoint_revision().id(), revision_two.id());
    assert_eq!(reopened.durable_head(), revision_three.id());
    assert_eq!(reopened.generation(), 2);
    let names = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["database.cfmd"]);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_replication_authority_replays_live_wal_and_survives_rotation() {
    let dir = test_dir("single-file-replication-authority-roundtrip");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(40_100, &[1]);

    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    assert_eq!(store.current_replication_membership().unwrap().epoch, 1);
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    assert_eq!(reopened.current_replication_membership().unwrap().epoch, 1);
    reopened.rotate_checkpoint(&base).unwrap();
    assert!(
        reopened
            .backend
            .single_file_container()
            .unwrap()
            .read_section(SingleFileSectionKind::ReplicationAuthority, 0)
            .unwrap()
            .is_none()
    );
    drop(reopened);

    let (reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    assert_eq!(reopened.current_replication_membership().unwrap().epoch, 1);
    assert_eq!(reopened.generation(), 2);
    let names = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["database.cfmd"]);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn encrypted_single_file_replication_segment_chain_survives_rotations() {
    let dir = test_dir("encrypted-single-file-replication-streaming-rotation");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(40_150, &[1]);
    let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
        crate::storage_encryption::StorageEncryptionKey::try_new([0x51; 32]).unwrap(),
    );

    let mut store = DurableRevisionStore::create_single_file_with_encryption(
        &path,
        &encryption,
        &base,
        &registry,
    )
    .unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    store.rotate_checkpoint(&base).unwrap();
    store.rotate_checkpoint(&base).unwrap();
    drop(store);

    let (reopened, scan) =
        DurableRevisionStore::open_single_file_with_encryption(&path, &encryption).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    assert_eq!(reopened.generation(), 3);
    assert_eq!(reopened.current_replication_membership().unwrap().epoch, 1);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn encrypted_single_file_compaction_relocates_authenticated_authority_closure() {
    let dir = test_dir("encrypted-single-file-authority-compaction");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(40_175, &[1]);
    let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
        crate::storage_encryption::StorageEncryptionKey::try_new([0x52; 32]).unwrap(),
    );

    let mut store = DurableRevisionStore::create_single_file_with_encryption(
        &path,
        &encryption,
        &base,
        &registry,
    )
    .unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    store.rotate_checkpoint(&base).unwrap();
    store.rotate_checkpoint(&base).unwrap();
    let size_before = fs::metadata(&path).unwrap().len();

    store.compact_obsolete_generations().unwrap();
    let size_after = fs::metadata(&path).unwrap().len();
    assert!(size_after < size_before);
    assert_eq!(store.current_replication_membership().unwrap().epoch, 1);
    drop(store);

    let (reopened, scan) =
        DurableRevisionStore::open_single_file_with_encryption(&path, &encryption).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    assert_eq!(reopened.generation(), 3);
    assert_eq!(reopened.current_replication_membership().unwrap().epoch, 1);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_external_freshness_survives_commit_rotation_and_rejects_rollback() {
    let dir = test_dir("single-file-external-freshness");
    let path = dir.join("database.cfmd");
    let rollback = dir.join("rollback.cfmd");
    let (base, registry, relation) = setup_revision(40_200, &[1]);
    let (next, descriptor) = transition_from(&base, &registry, relation, 40_201, 201);
    let (config, authority) = external_freshness_fixture([41; 32]);
    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();

    let adopted = store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    assert_eq!(adopted.generation, 2);
    assert!(store.external_freshness.is_some());
    fs::copy(&path, &rollback).unwrap();

    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    assert_eq!(store.durable_head(), next.id());
    drop(store);

    assert!(matches!(
        DurableRevisionStore::open_single_file(&path),
        Err(DurabilityError::Protocol {
            reason: "externally anchored store requires freshness-aware open",
            ..
        })
    ));

    let (mut reopened, scan) = DurableRevisionStore::open_single_file_with_external_freshness(
        &path,
        config.clone(),
        authority.boxed(),
    )
    .unwrap();
    assert_eq!(scan.durable_revision(), next.id());
    assert_eq!(reopened.durable_head(), next.id());
    reopened.rotate_checkpoint(&next).unwrap();
    assert_eq!(reopened.generation(), 3);
    drop(reopened);

    let (mut reopened, scan) = DurableRevisionStore::open_single_file_with_external_freshness(
        &path,
        config.clone(),
        authority.boxed(),
    )
    .unwrap();
    assert_eq!(scan.durable_revision(), next.id());
    assert_eq!(reopened.generation(), 3);
    let size_before_compaction = fs::metadata(&path).unwrap().len();
    reopened.compact_obsolete_generations().unwrap();
    assert!(fs::metadata(&path).unwrap().len() <= size_before_compaction);
    drop(reopened);

    let (reopened, scan) = DurableRevisionStore::open_single_file_with_external_freshness(
        &path,
        config.clone(),
        authority.boxed(),
    )
    .unwrap();
    assert_eq!(scan.durable_revision(), next.id());
    assert_eq!(reopened.generation(), 3);
    drop(reopened);

    fs::copy(&rollback, &path).unwrap();
    assert!(matches!(
        DurableRevisionStore::open_single_file_with_external_freshness(
            &path,
            config,
            authority.boxed(),
        ),
        Err(DurabilityError::Protocol {
            reason: "local durable generation was rolled back behind external freshness authority",
            ..
        } | DurabilityError::Protocol {
            reason: "local WAL was truncated behind external freshness authority",
            ..
        })
    ));

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn encrypted_single_file_external_freshness_uses_the_same_keyed_open_boundary() {
    let dir = test_dir("encrypted-single-file-external-freshness");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(40_240, &[1]);
    let (config, authority) = external_freshness_fixture([42; 32]);
    let encryption = crate::storage_encryption::StorageEncryption::aes256_gcm_siv(
        crate::storage_encryption::StorageEncryptionKey::try_new([0x44; 32]).unwrap(),
    );
    let mut store = DurableRevisionStore::create_single_file_with_encryption(
        &path,
        &encryption,
        &base,
        &registry,
    )
    .unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();
    drop(store);

    assert!(
        DurableRevisionStore::open_single_file_with_external_freshness(
            &path,
            config.clone(),
            authority.boxed(),
        )
        .is_err()
    );
    assert!(matches!(
        DurableRevisionStore::open_single_file_with_encryption(&path, &encryption),
        Err(DurabilityError::Protocol {
            reason: "externally anchored store requires freshness-aware open",
            ..
        })
    ));

    let (reopened, scan) =
        DurableRevisionStore::open_single_file_with_external_freshness_and_encryption(
            &path,
            config,
            authority.boxed(),
            &encryption,
        )
        .unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    assert!(reopened.external_freshness.is_some());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_compaction_relocates_authority_without_losing_live_prepare_or_wal() {
    let dir = test_dir("single-file-in-place-compaction");
    let path = dir.join("database.cfmd");
    let (base, registry, relation) = setup_revision(40_250, &[1]);
    let (target, descriptor) = transition_from(&base, &registry, relation, 40_251, 251);
    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();

    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    store.rotate_checkpoint(&base).unwrap();
    store.rotate_checkpoint(&base).unwrap();
    let generation_before = store.generation();
    let token = store.durably_prepare(&descriptor).unwrap();
    let size_before = fs::metadata(&path).unwrap().len();

    store.compact_obsolete_generations().unwrap();
    let size_after = fs::metadata(&path).unwrap().len();
    assert!(size_after < size_before);
    assert_eq!(store.generation(), generation_before);
    assert_eq!(store.durable_head(), base.id());

    store.durably_commit(token).unwrap();
    assert_eq!(store.durable_head(), target.id());
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(reopened.generation(), generation_before);
    assert_eq!(reopened.durable_head(), target.id());
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(reopened.current_replication_membership().unwrap().epoch, 1);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_external_freshness_tracks_streaming_carry_forward() {
    let dir = test_dir("single-file-freshness-streaming");
    let path = dir.join("database.cfmd");
    let (base, registry, relation) = setup_revision(40_260, &[1]);
    let (target, descriptor) = transition_from(&base, &registry, relation, 40_261, 261);
    let (config, authority) = external_freshness_fixture([42; 32]);
    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();
    store
        .adopt_external_freshness(config.clone(), authority.boxed())
        .unwrap();

    store
        .begin_streaming_checkpoint_with_chunk_size(&base, 16)
        .unwrap();
    store.write_streaming_checkpoint_chunks(1).unwrap();
    let token = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(token).unwrap();
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    let receipt = store.finalize_streaming_checkpoint().unwrap();
    assert_eq!(receipt.base_revision, base.id());
    assert_eq!(store.durable_head(), target.id());
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open_single_file_with_external_freshness(
        &path,
        config,
        authority.boxed(),
    )
    .unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(reopened.durable_head(), target.id());
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_streaming_checkpoint_archives_pinned_historical_epoch() {
    let dir = test_dir("single-file-streaming-historical-epoch");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(40_320, &[1]);
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(40_321),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(40_322),
        base.id(),
        &target,
        crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: base.semantic_revision().schema,
                target_schema: target.semantic_revision().schema,
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(40_323)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(40_324)),
                encoding_version: 1,
                complement: Value::Unit,
            },
            kernel_lens::ComplementRetention::Forget,
        ),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    let anchor = *store.historical_epoch_anchors().values().next().unwrap();
    store
        .begin_streaming_checkpoint_with_chunk_size(&target, 32)
        .unwrap();
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    store.finalize_streaming_checkpoint().unwrap();
    assert_eq!(store.generation(), 2);
    store.compact_obsolete_generations().unwrap();
    let material = store
        .historical_epoch_material(anchor.effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(material.generation(), 1);
    assert_eq!(material.checkpoint().id(), base.id());
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(
        reopened
            .historical_epoch_material(anchor.effect_id)
            .unwrap()
            .unwrap()
            .checkpoint()
            .id(),
        base.id()
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn directory_root_backed_history_releases_generation_pin_and_reopens_exact_source() {
    let dir = test_dir("directory-root-backed-history");
    let (base, registry, _) = setup_revision(40_360, &[1, 2, 3]);
    let (base_atoms, base_root) =
        realize_database_state_factorized(base.state(), base.semantic_context()).unwrap();

    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(40_361),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(40_362),
        base.id(),
        &target,
        crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: base.semantic_revision().schema,
                target_schema: target.semantic_revision().schema,
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(40_363)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(40_364)),
                encoding_version: 1,
                complement: Value::Unit,
            },
            kernel_lens::ComplementRetention::Forget,
        ),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .rotate_checkpoint_with_factorized_realization(&base, &base_atoms, &base_root)
        .unwrap();
    let historical_generation = store.generation();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    let anchor = *store.historical_epoch_anchors().values().next().unwrap();
    assert_eq!(anchor.generation, historical_generation);

    let (target_atoms, target_root) =
        realize_database_state_factorized(target.state(), target.semantic_context()).unwrap();
    store
        .rotate_checkpoint_with_factorized_realization(&target, &target_atoms, &target_root)
        .unwrap();
    assert!(
        !store
            .pinned_historical_generations()
            .contains(&historical_generation)
    );
    assert_eq!(
        store
            .historical_revision_from_realization(anchor.effect_id)
            .unwrap()
            .unwrap(),
        base
    );

    store.compact_obsolete_generations().unwrap();
    assert!(!super::generation_layout::checkpoint_path(&dir, historical_generation).exists());
    assert!(!super::generation_layout::manifest_path(&dir, historical_generation).exists());
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(
        reopened
            .historical_revision_from_realization(anchor.effect_id)
            .unwrap()
            .unwrap(),
        base
    );
    reopened
        .release_historical_epoch_authority(&target, anchor.effect_id)
        .unwrap()
        .unwrap();
    assert!(
        reopened
            .historical_revision_from_realization(anchor.effect_id)
            .unwrap()
            .is_none()
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn aborted_streaming_complete_historical_root_never_supersedes_old_generation_authority() {
    let dir = test_dir("aborted-streaming-complete-historical-root");
    let (base, registry, _) = setup_revision(40_365, &[1, 2, 3]);
    let (base_atoms, base_root) =
        realize_database_state_factorized(base.state(), base.semantic_context()).unwrap();
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(40_366),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(40_367),
        base.id(),
        &target,
        crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: base.semantic_revision().schema,
                target_schema: target.semantic_revision().schema,
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(40_368)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(40_369)),
                encoding_version: 1,
                complement: Value::Unit,
            },
            kernel_lens::ComplementRetention::Forget,
        ),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .rotate_checkpoint_with_factorized_realization(&base, &base_atoms, &base_root)
        .unwrap();
    let historical_generation = store.generation();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    let anchor = *store.historical_epoch_anchors().values().next().unwrap();
    let (target_atoms, target_root) =
        realize_database_state_factorized(target.state(), target.semantic_context()).unwrap();

    store
        .begin_streaming_checkpoint_with_factorized_realization_and_chunk_size(
            &target,
            &target_atoms,
            &target_root,
            32,
        )
        .unwrap();
    store.write_streaming_checkpoint_chunks(1).unwrap();
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(reopened.durable_head(), target.id());
    assert_eq!(reopened.checkpoint_revision().id(), base.id());
    assert!(
        reopened
            .historical_revision_from_realization(anchor.effect_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reopened
            .historical_epoch_material(anchor.effect_id)
            .unwrap()
            .unwrap()
            .checkpoint()
            .id(),
        base.id()
    );
    assert!(
        reopened
            .pinned_historical_generations()
            .contains(&historical_generation)
    );

    reopened
        .begin_streaming_checkpoint_with_factorized_realization_and_chunk_size(
            &target,
            &target_atoms,
            &target_root,
            32,
        )
        .unwrap();
    reopened
        .write_streaming_checkpoint_chunks(usize::MAX)
        .unwrap();
    reopened.finalize_streaming_checkpoint().unwrap();
    assert_eq!(
        reopened
            .historical_revision_from_realization(anchor.effect_id)
            .unwrap()
            .unwrap(),
        base
    );
    assert!(
        !reopened
            .pinned_historical_generations()
            .contains(&historical_generation)
    );
    reopened.compact_obsolete_generations().unwrap();
    assert!(!super::generation_layout::checkpoint_path(&dir, historical_generation).exists());
    drop(reopened);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(
        reopened
            .historical_revision_from_realization(anchor.effect_id)
            .unwrap()
            .unwrap(),
        base
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_root_backed_history_does_not_retain_generation_archive() {
    let dir = test_dir("single-file-root-backed-history");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(40_370, &[1, 2, 3]);
    let (base_atoms, base_root) =
        realize_database_state_factorized(base.state(), base.semantic_context()).unwrap();
    let mut target_context = base.semantic_context().clone();
    target_context.schema.revision = SchemaRevisionId::new(2);
    let target = Revision::build(
        RevisionId::new(40_371),
        &target_context,
        &registry,
        base.state().clone(),
    )
    .unwrap();
    let descriptor = DurableRevisionDescriptor::schema_migration(
        ClientTransactionId::new(40_372),
        base.id(),
        &target,
        crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: base.semantic_revision().schema,
                target_schema: target.semantic_revision().schema,
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(40_373)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(40_374)),
                encoding_version: 1,
                complement: Value::Unit,
            },
            kernel_lens::ComplementRetention::Forget,
        ),
        &registry,
    )
    .unwrap();

    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();
    store
        .rotate_checkpoint_with_factorized_realization(&base, &base_atoms, &base_root)
        .unwrap();
    let historical_generation = store.generation();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    let anchor = *store.historical_epoch_anchors().values().next().unwrap();
    let (target_atoms, target_root) =
        realize_database_state_factorized(target.state(), target.semantic_context()).unwrap();
    store
        .rotate_checkpoint_with_factorized_realization(&target, &target_atoms, &target_root)
        .unwrap();
    store.compact_obsolete_generations().unwrap();
    assert!(
        !store
            .backend
            .single_file_container()
            .unwrap()
            .has_historical_epoch_archive(historical_generation)
            .unwrap()
    );
    assert_eq!(
        store
            .historical_revision_from_realization(anchor.effect_id)
            .unwrap()
            .unwrap(),
        base
    );
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(
        reopened
            .historical_revision_from_realization(anchor.effect_id)
            .unwrap()
            .unwrap(),
        base
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_checkpoint_owns_and_reopens_factorized_realization() {
    let dir = test_dir("single-file-checkpoint-factorized-realization");
    let path = dir.join("database.cfmd");
    let (base, registry, _) = setup_revision(40_280, &[1, 2, 3]);
    let (atoms, root) =
        realize_database_state_factorized(base.state(), base.semantic_context()).unwrap();
    let expected_dependencies = root.dependencies();

    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();
    store
        .rotate_checkpoint_with_factorized_realization(&base, &atoms, &root)
        .unwrap();
    let physical = store.checkpoint_factorized_realization().unwrap();
    assert_eq!(physical.revision(), base.id());
    assert_eq!(physical.root().dependencies(), expected_dependencies);
    store.compact_obsolete_generations().unwrap();
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    let physical = reopened.checkpoint_factorized_realization().unwrap();
    assert_eq!(physical.revision(), base.id());
    assert_eq!(physical.root().dependencies(), expected_dependencies);
    reopened.rotate_checkpoint(&base).unwrap();
    assert_eq!(reopened.generation(), 3);
    assert_eq!(
        reopened
            .checkpoint_factorized_realization()
            .unwrap()
            .root()
            .dependencies(),
        expected_dependencies
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_streaming_factorized_cut_carries_newer_wal_exactly() {
    let dir = test_dir("single-file-streaming-factorized-carry-forward");
    let path = dir.join("database.cfmd");
    let (base, registry, relation) = setup_revision(40_290, &[1]);
    let (r1, d1) = transition_from(&base, &registry, relation, 40_291, 291);
    let (r2, d2) = transition_from(&r1, &registry, relation, 40_292, 292);
    let (atoms, root) =
        realize_database_state_factorized(base.state(), base.semantic_context()).unwrap();
    let expected_dependencies = root.dependencies();
    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();

    store
        .begin_streaming_checkpoint_with_factorized_realization_and_chunk_size(
            &base, &atoms, &root, 16,
        )
        .unwrap();
    store.write_streaming_checkpoint_chunks(1).unwrap();
    let p1 = store.durably_prepare(&d1).unwrap();
    store.durably_commit(p1).unwrap();
    let p2 = store.durably_prepare(&d2).unwrap();
    store.durably_commit(p2).unwrap();
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    let receipt = store.finalize_streaming_checkpoint().unwrap();

    assert_eq!(receipt.base_revision, base.id());
    assert_eq!(store.durable_head(), r2.id());
    assert_eq!(
        store
            .checkpoint_factorized_realization()
            .unwrap()
            .revision(),
        base.id()
    );
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(reopened.checkpoint_revision().id(), base.id());
    assert_eq!(reopened.durable_head(), r2.id());
    assert_eq!(scan.durable_revision(), r2.id());
    let physical = reopened.checkpoint_factorized_realization().unwrap();
    assert_eq!(physical.revision(), base.id());
    assert_eq!(physical.root().dependencies(), expected_dependencies);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_file_streaming_checkpoint_carries_exact_wal_and_replication_suffix() {
    let dir = test_dir("single-file-streaming-carry-forward");
    let path = dir.join("database.cfmd");
    let (base, registry, relation) = setup_revision(40_300, &[1]);
    let (r1, d1) = transition_from(&base, &registry, relation, 40_301, 301);
    let (r2, d2) = transition_from(&r1, &registry, relation, 40_302, 302);
    let mut store = DurableRevisionStore::create_single_file(&path, &base, &registry).unwrap();

    store
        .begin_streaming_checkpoint_with_chunk_size(&base, 16)
        .unwrap();
    store.write_streaming_checkpoint_chunks(1).unwrap();
    let p1 = store.durably_prepare(&d1).unwrap();
    store.durably_commit(p1).unwrap();
    store
        .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
        .unwrap();
    store.write_streaming_checkpoint_chunks(1).unwrap();
    let p2 = store.durably_prepare(&d2).unwrap();
    store.durably_commit(p2).unwrap();
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    let receipt = store.finalize_streaming_checkpoint().unwrap();

    assert_eq!(receipt.base_revision, base.id());
    assert_eq!(store.durable_head(), r2.id());
    assert_eq!(store.current_replication_membership().unwrap().epoch, 1);
    assert_eq!(store.generation(), 2);
    assert!(
        store
            .backend
            .single_file_container()
            .unwrap()
            .read_section(SingleFileSectionKind::ReplicationAuthority, 0)
            .unwrap()
            .is_none()
    );

    // A subsequent ordinary rotation must segment only the post-cut live
    // replication suffix once, while preserving the same semantic endpoint.
    store.rotate_checkpoint(&r2).unwrap();
    assert_eq!(store.generation(), 3);
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open_single_file(&path).unwrap();
    assert_eq!(reopened.checkpoint_revision().id(), r2.id());
    assert_eq!(reopened.durable_head(), r2.id());
    assert_eq!(scan.durable_revision(), r2.id());
    assert_eq!(reopened.current_replication_membership().unwrap().epoch, 1);
    let names = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["database.cfmd"]);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn directory_checkpoint_owns_and_reopens_factorized_realization() {
    let dir = test_dir("directory-checkpoint-factorized-realization");
    let (base, registry, _) = setup_revision(40_310, &[1, 2, 3]);
    let (atoms, root) =
        realize_database_state_factorized(base.state(), base.semantic_context()).unwrap();
    let expected_dependencies = root.dependencies();

    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .rotate_checkpoint_with_factorized_realization(&base, &atoms, &root)
        .unwrap();
    assert_eq!(store.generation(), 2);
    let physical = store.checkpoint_factorized_realization().unwrap();
    assert_eq!(physical.revision(), base.id());
    assert_eq!(physical.root().dependencies(), expected_dependencies);
    drop(store);

    let (mut reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), base.id());
    let physical = reopened.checkpoint_factorized_realization().unwrap();
    assert_eq!(physical.revision(), base.id());
    assert_eq!(physical.root().dependencies(), expected_dependencies);
    reopened.rotate_checkpoint(&base).unwrap();
    assert_eq!(reopened.generation(), 3);
    assert_eq!(
        reopened
            .checkpoint_factorized_realization()
            .unwrap()
            .root()
            .dependencies(),
        expected_dependencies
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn directory_streaming_factorized_cut_carries_newer_wal_exactly() {
    let dir = test_dir("directory-streaming-factorized-carry-forward");
    let (base, registry, relation) = setup_revision(40_320, &[1]);
    let (r1, d1) = transition_from(&base, &registry, relation, 40_321, 321);
    let (r2, d2) = transition_from(&r1, &registry, relation, 40_322, 322);
    let (atoms, root) =
        realize_database_state_factorized(base.state(), base.semantic_context()).unwrap();
    let expected_dependencies = root.dependencies();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();

    store
        .begin_streaming_checkpoint_with_factorized_realization_and_chunk_size(
            &base, &atoms, &root, 16,
        )
        .unwrap();
    store.write_streaming_checkpoint_chunks(1).unwrap();
    let p1 = store.durably_prepare(&d1).unwrap();
    store.durably_commit(p1).unwrap();
    let p2 = store.durably_prepare(&d2).unwrap();
    store.durably_commit(p2).unwrap();
    store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
    let receipt = store.finalize_streaming_checkpoint().unwrap();

    assert_eq!(receipt.base_revision, base.id());
    assert_eq!(store.durable_head(), r2.id());
    assert_eq!(
        store
            .checkpoint_factorized_realization()
            .unwrap()
            .revision(),
        base.id()
    );
    drop(store);

    let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.checkpoint_revision().id(), base.id());
    assert_eq!(reopened.durable_head(), r2.id());
    assert_eq!(scan.durable_revision(), r2.id());
    let physical = reopened.checkpoint_factorized_realization().unwrap();
    assert_eq!(physical.revision(), base.id());
    assert_eq!(physical.root().dependencies(), expected_dependencies);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn directory_published_realization_missing_is_corruption() {
    let dir = test_dir("directory-published-realization-missing");
    let (base, registry, _) = setup_revision(40_330, &[1]);
    let (atoms, root) =
        realize_database_state_factorized(base.state(), base.semantic_context()).unwrap();
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    store
        .rotate_checkpoint_with_factorized_realization(&base, &atoms, &root)
        .unwrap();
    let generation = store.generation();
    drop(store);
    fs::remove_file(generation_layout::realization_path(&dir, generation)).unwrap();
    assert!(matches!(
        DurableRevisionStore::open(&dir),
        Err(DurabilityError::Corruption { .. })
    ));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn causal_history_release_publishes_head_root_and_preserves_effect_id_high_watermark() {
    let dir = test_dir("causal-history-release-high-watermark");
    let (base, registry, relation) = setup_revision(10_000, &[1]);
    let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
    let (next, _, _) = setup_revision(10_001, &[1, 2]);
    let descriptor = DurableRevisionDescriptor::relation_data(
        ClientTransactionId::new(0xCA11),
        base.id(),
        &next,
        base.semantic_revision(),
        vec![DurableRelationMutation {
            relation,
            inserted: vec![vec![Value::I64(2)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        &registry,
    )
    .unwrap();
    let prepared = store.durably_prepare(&descriptor).unwrap();
    store.durably_commit(prepared).unwrap();
    assert_eq!(store.next_revision_effect_id, 2);
    assert!(
        store
            .release_causal_history_before_head(&next)
            .unwrap()
            .is_some()
    );
    assert_eq!(store.causal_coverage_root(), next.id());
    assert!(store.revision_effects.is_empty());
    assert_eq!(store.next_revision_effect_id, 2);
    drop(store);

    let (mut reopened, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(reopened.causal_coverage_root(), next.id());
    assert!(reopened.revision_effects.is_empty());
    assert_eq!(reopened.next_revision_effect_id, 2);

    let (third, _, _) = setup_revision(10_002, &[1, 2, 3]);
    let descriptor = DurableRevisionDescriptor::relation_data(
        ClientTransactionId::new(0xCA12),
        next.id(),
        &third,
        next.semantic_revision(),
        vec![DurableRelationMutation {
            relation,
            inserted: vec![vec![Value::I64(3)]],
            removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: crate::DurableRelationAuthorization::default(),
        }],
        &registry,
    )
    .unwrap();
    let prepared = reopened.durably_prepare(&descriptor).unwrap();
    reopened.durably_commit(prepared).unwrap();
    assert_eq!(
        reopened.revision_effect_frontier(third.id()),
        Some(&BTreeSet::from([RevisionEffectId(2)]))
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}
