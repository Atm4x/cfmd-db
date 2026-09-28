use crate::*;
use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signer, SigningKey};
use kernel_auth::{TrustRootSet, key_id};
use kernel_change::RevisionEffectId;

use super::*;
use crate::replication::replication_peer_evidence_signing_message;

fn signed_heartbeat() -> (
    ReplicationPeerAuthPolicy,
    TrustRootSet,
    SignedReplicationTransportFrame,
) {
    let signing = SigningKey::from_bytes(&[7_u8; 32]);
    let verifying = signing.verifying_key().to_bytes();
    let id = key_id(&verifying);
    let trust = TrustRootSet::bootstrap(3, &[verifying]).unwrap();
    let peer = ReplicaId::new(2);
    let policy = ReplicationPeerAuthPolicy {
        cluster: ReplicationClusterId([9; 32]),
        trust_epoch: 3,
        peer_keys: BTreeMap::from([(peer, id)]),
    };
    let frame = ReplicationTransportFrame {
        cluster: policy.cluster,
        trust_epoch: policy.trust_epoch,
        sender: peer,
        sequence: 1,
        payload: ReplicationTransportPayload::Heartbeat(ReplicationHeartbeat {
            membership_epoch: 4,
            term: 11,
            logical_tick: 7,
        }),
    };
    let signature = signing
        .sign(&replication_transport_signing_message(&frame).unwrap())
        .to_bytes();
    (
        policy,
        trust,
        SignedReplicationTransportFrame { frame, signature },
    )
}

#[test]
fn transport_wire_is_canonical_authenticated_and_session_replay_safe() {
    let (policy, trust, signed) = signed_heartbeat();
    let bytes = encode_signed_replication_transport_frame(&signed).unwrap();
    let decoded = decode_signed_replication_transport_frame(&bytes).unwrap();
    assert_eq!(decoded, signed);
    let mut ingress = ReplicationTransportIngress::new();
    ingress.accept(&policy, &trust, &decoded).unwrap();
    assert!(ingress.accept(&policy, &trust, &decoded).is_err());

    let mut tampered = bytes;
    tampered[60] ^= 1;
    let decoded = decode_signed_replication_transport_frame(&tampered).unwrap();
    let mut ingress = ReplicationTransportIngress::new();
    assert!(ingress.accept(&policy, &trust, &decoded).is_err());
}

#[test]
fn anti_entropy_detects_divergence_without_treating_digest_as_authority() {
    let locks = vec![
        ReplicationLockSummary {
            position: 1,
            term: 3,
            effect: RevisionEffectId(10),
        },
        ReplicationLockSummary {
            position: 2,
            term: 3,
            effect: RevisionEffectId(11),
        },
    ];
    let local = replication_anti_entropy_summary(4, 3, &locks).unwrap();
    let remote = replication_anti_entropy_summary(4, 3, &locks[..1]).unwrap();
    assert_eq!(
        compare_replication_anti_entropy(&local, &local),
        ReplicationAntiEntropyRelation::InSync
    );
    assert_eq!(
        compare_replication_anti_entropy(&local, &remote),
        ReplicationAntiEntropyRelation::ExchangeRequired
    );
    let other = replication_anti_entropy_summary(5, 3, &locks).unwrap();
    assert_eq!(
        compare_replication_anti_entropy(&local, &other),
        ReplicationAntiEntropyRelation::MembershipMismatch
    );
}

#[test]
fn recovery_frontier_wire_owner_is_shared_without_changing_semantic_signatures() {
    let signing_a = SigningKey::from_bytes(&[11_u8; 32]);
    let signing_b = SigningKey::from_bytes(&[12_u8; 32]);
    let verifying_a = signing_a.verifying_key().to_bytes();
    let verifying_b = signing_b.verifying_key().to_bytes();
    let key_a = key_id(&verifying_a);
    let key_b = key_id(&verifying_b);
    let trust = TrustRootSet::bootstrap(7, &[verifying_a, verifying_b]).unwrap();
    let voter_a = ReplicaId::new(2);
    let voter_b = ReplicaId::new(3);
    let cluster = ReplicationClusterId([21; 32]);
    let policy = ReplicationPeerAuthPolicy {
        cluster,
        trust_epoch: 7,
        peer_keys: BTreeMap::from([(voter_a, key_a), (voter_b, key_b)]),
    };
    let locks = (1_u64..=1024)
        .map(|position| ReplicationLockSummary {
            position,
            term: 9,
            effect: RevisionEffectId(u128::from(position)),
        })
        .collect::<Vec<_>>();

    let make_frame = |sequence: u64, voter: ReplicaId, signing: &SigningKey, signer| {
        let evidence = ReplicationPeerEvidence::RecoveryAck(ReplicationRecoveryAck {
            voter,
            membership_epoch: 4,
            recovery_term: 12,
            leader: ReplicaId::new(1),
            locks: locks.clone(),
        });
        let peer_signature = signing
            .sign(
                &replication_peer_evidence_signing_message(cluster, 7, signer, &evidence).unwrap(),
            )
            .to_bytes();
        let peer = SignedReplicationPeerEvidence {
            trust_epoch: 7,
            signer,
            evidence,
            signature: peer_signature,
        };
        let frame = ReplicationTransportFrame {
            cluster,
            trust_epoch: 7,
            sender: voter,
            sequence,
            payload: ReplicationTransportPayload::PeerEvidence(peer),
        };
        let signature = signing
            .sign(&replication_transport_signing_message(&frame).unwrap())
            .to_bytes();
        SignedReplicationTransportFrame { frame, signature }
    };

    let first = make_frame(1, voter_a, &signing_a, key_a);
    let second = make_frame(1, voter_b, &signing_b, key_b);
    assert!(encode_signed_replication_transport_frame(&first).is_err());
    let mut egress = ReplicationTransportEgress::new();
    let first_wire = egress.encode(&first).unwrap();
    let second_wire = egress.encode(&second).unwrap();
    assert!(first_wire.len() > 32 * 1024);
    assert!(second_wire.len() < 512);

    let mut ingress = ReplicationTransportIngress::new();
    assert_eq!(
        ingress.accept_wire(&policy, &trust, &first_wire).unwrap(),
        first
    );
    assert_eq!(
        ingress.accept_wire(&policy, &trust, &second_wire).unwrap(),
        second
    );
    assert!(decode_signed_replication_transport_frame(&second_wire).is_err());
}

#[test]
fn membership_successor_wire_owner_is_shared_without_changing_semantic_signatures() {
    let signing_a = SigningKey::from_bytes(&[13_u8; 32]);
    let signing_b = SigningKey::from_bytes(&[14_u8; 32]);
    let verifying_a = signing_a.verifying_key().to_bytes();
    let verifying_b = signing_b.verifying_key().to_bytes();
    let key_a = key_id(&verifying_a);
    let key_b = key_id(&verifying_b);
    let trust = TrustRootSet::bootstrap(8, &[verifying_a, verifying_b]).unwrap();
    let voter_a = ReplicaId::new(2);
    let voter_b = ReplicaId::new(3);
    let cluster = ReplicationClusterId([22; 32]);
    let policy = ReplicationPeerAuthPolicy {
        cluster,
        trust_epoch: 8,
        peer_keys: BTreeMap::from([(voter_a, key_a), (voter_b, key_b)]),
    };
    let successor = ReplicationMembership {
        epoch: 5,
        members: (1_u64..=1024).map(ReplicaId::new).collect(),
        quorum_size: 513,
    };

    let make_frame = |sequence: u64, voter: ReplicaId, signing: &SigningKey, signer| {
        let evidence = ReplicationPeerEvidence::MembershipVote(ReplicationMembershipVote {
            voter,
            previous_membership_epoch: 4,
            term: 12,
            next: successor.clone(),
        });
        let peer_signature = signing
            .sign(
                &replication_peer_evidence_signing_message(cluster, 8, signer, &evidence).unwrap(),
            )
            .to_bytes();
        let peer = SignedReplicationPeerEvidence {
            trust_epoch: 8,
            signer,
            evidence,
            signature: peer_signature,
        };
        let frame = ReplicationTransportFrame {
            cluster,
            trust_epoch: 8,
            sender: voter,
            sequence,
            payload: ReplicationTransportPayload::PeerEvidence(peer),
        };
        let signature = signing
            .sign(&replication_transport_signing_message(&frame).unwrap())
            .to_bytes();
        SignedReplicationTransportFrame { frame, signature }
    };

    let first = make_frame(1, voter_a, &signing_a, key_a);
    let second = make_frame(1, voter_b, &signing_b, key_b);
    assert!(encode_signed_replication_transport_frame(&first).is_err());
    let mut egress = ReplicationTransportEgress::new();
    let first_wire = egress.encode(&first).unwrap();
    let second_wire = egress.encode(&second).unwrap();
    assert!(first_wire.len() > 8 * 1024);
    assert!(second_wire.len() < 512);

    let mut ingress = ReplicationTransportIngress::new();
    assert_eq!(
        ingress.accept_wire(&policy, &trust, &first_wire).unwrap(),
        first
    );
    assert_eq!(
        ingress.accept_wire(&policy, &trust, &second_wire).unwrap(),
        second
    );
    assert!(decode_signed_replication_transport_frame(&second_wire).is_err());
}

#[test]
fn failure_detector_only_reports_loss_and_never_creates_recovery_authority() {
    let membership = ReplicationMembership {
        epoch: 4,
        members: BTreeSet::from([ReplicaId::new(1), ReplicaId::new(2), ReplicaId::new(3)]),
        quorum_size: 2,
    };
    let mut detector = ReplicationFailureDetector::new(ReplicaId::new(1), 5).unwrap();
    detector.observe_authenticated(ReplicaId::new(2));
    assert!(detector.quorum_reachable(&membership));
    detector.advance_to(6).unwrap();
    assert!(!detector.quorum_reachable(&membership));
    assert_eq!(
        detector.quorum_loss_observation(&membership, 9),
        Some(ReplicationQuorumLoss {
            membership_epoch: 4,
            observed_term: 9
        })
    );
    assert!(detector.advance_to(5).is_err());
}
