use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::{Command, ExitStatus};

use ed25519_dalek::{Signer, SigningKey};
use kernel_auth::{TrustRootSet, key_id};
use kernel_durability::{
    ReplicaId, ReplicationAntiEntropySummary, ReplicationClusterId, ReplicationHeartbeat,
    ReplicationPeerAuthPolicy, ReplicationTransportFrame, ReplicationTransportIngress,
    ReplicationTransportPayload, SignedReplicationTransportFrame,
    decode_signed_replication_transport_frame, encode_signed_replication_transport_frame,
    replication_transport_signing_message,
};

const MODE_ENV: &str = "CFMD_REPLICATION_MP_MODE";
const PATH_ENV: &str = "CFMD_REPLICATION_MP_PATH";

fn fixture() -> (ReplicationPeerAuthPolicy, TrustRootSet, SigningKey) {
    let signing = SigningKey::from_bytes(&[17_u8; 32]);
    let verifying = signing.verifying_key().to_bytes();
    let signer = key_id(&verifying);
    let trust = TrustRootSet::bootstrap(9, &[verifying]).expect("trust root");
    let policy = ReplicationPeerAuthPolicy {
        cluster: ReplicationClusterId([0xA5; 32]),
        trust_epoch: 9,
        peer_keys: BTreeMap::from([(ReplicaId::new(2), signer)]),
    };
    (policy, trust, signing)
}

fn signed_frame(sequence: u64, signing: &SigningKey) -> SignedReplicationTransportFrame {
    let frame = ReplicationTransportFrame {
        cluster: ReplicationClusterId([0xA5; 32]),
        trust_epoch: 9,
        sender: ReplicaId::new(2),
        sequence,
        payload: if sequence == 1 {
            ReplicationTransportPayload::Heartbeat(ReplicationHeartbeat {
                membership_epoch: 4,
                term: 12,
                logical_tick: 100,
            })
        } else {
            ReplicationTransportPayload::AntiEntropySummary(ReplicationAntiEntropySummary {
                membership_epoch: 4,
                term: 12,
                lock_count: 0,
                highest_position: None,
                lock_digest: kernel_auth::sha256(&[]),
            })
        },
    };
    let signature = signing
        .sign(&replication_transport_signing_message(&frame).expect("signing message"))
        .to_bytes();
    SignedReplicationTransportFrame { frame, signature }
}

#[test]
fn multiprocess_worker() {
    let Ok(mode) = env::var(MODE_ENV) else {
        return;
    };
    let path = env::var(PATH_ENV).expect("wire path");
    let (policy, trust, signing) = fixture();
    match mode.as_str() {
        "send" => {
            let bytes = encode_signed_replication_transport_frame(&signed_frame(1, &signing))
                .expect("encode");
            fs::write(path, bytes).expect("write full frame");
        }
        "send_torn" => {
            let bytes = encode_signed_replication_transport_frame(&signed_frame(1, &signing))
                .expect("encode");
            fs::write(path, &bytes[..bytes.len() / 2]).expect("write torn frame");
            std::process::exit(17);
        }
        "receive" => {
            let bytes = fs::read(path).expect("read frame");
            let frame = decode_signed_replication_transport_frame(&bytes).expect("decode");
            let mut ingress = ReplicationTransportIngress::new();
            ingress
                .accept(&policy, &trust, &frame)
                .expect("authenticate");
            assert!(ingress.accept(&policy, &trust, &frame).is_err());
        }
        "reject" => {
            let bytes = fs::read(path).expect("read invalid frame");
            if let Ok(frame) = decode_signed_replication_transport_frame(&bytes) {
                let mut ingress = ReplicationTransportIngress::new();
                assert!(ingress.accept(&policy, &trust, &frame).is_err());
            }
        }
        other => panic!("unknown multiprocess mode {other}"),
    }
}

fn run_worker(mode: &str, path: &std::path::Path) -> ExitStatus {
    Command::new(env::current_exe().expect("test exe"))
        .arg("--exact")
        .arg("multiprocess_worker")
        .arg("--nocapture")
        .env(MODE_ENV, mode)
        .env(PATH_ENV, path)
        .status()
        .expect("spawn replication worker")
}

#[test]
fn distributed_transport_fault_matrix_rejects_tamper_torn_and_replay() {
    let root = env::temp_dir().join(format!(
        "cfmd-replication-mp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("temp root");
    let wire = root.join("peer.frame");

    assert!(run_worker("send", &wire).success());
    assert!(run_worker("receive", &wire).success());

    let mut tampered = fs::read(&wire).expect("read for tamper");
    let signature_byte = tampered.len() - 1;
    tampered[signature_byte] ^= 0x40;
    fs::write(&wire, tampered).expect("write tamper");
    assert!(run_worker("reject", &wire).success());

    let torn = run_worker("send_torn", &wire);
    assert_eq!(torn.code(), Some(17));
    assert!(run_worker("reject", &wire).success());

    fs::remove_dir_all(root).expect("cleanup");
}
