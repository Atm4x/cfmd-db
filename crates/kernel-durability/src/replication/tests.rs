use kernel_auth::{KeyId, sha256};
use kernel_change::RevisionEffectId;

use super::codec::{
    encode_peer_evidence, encode_signed_peer_evidence, peer_evidence_kind_and_payload_digest,
    signed_peer_evidence_proof_digest,
};
use super::*;

#[test]
fn membership_vote_streaming_digests_match_legacy_canonical_encoding() {
    let evidence = ReplicationPeerEvidence::MembershipVote(ReplicationMembershipVote {
        voter: ReplicaId::new(7),
        previous_membership_epoch: 3,
        term: 19,
        next: ReplicationMembership {
            epoch: 4,
            members: (1_u64..=256).map(ReplicaId::new).collect(),
            quorum_size: 129,
        },
    });
    let signed = SignedReplicationPeerEvidence {
        trust_epoch: 5,
        signer: KeyId([0xC3; 32]),
        evidence,
        signature: [0x3C; 64],
    };

    let (kind, payload) = encode_peer_evidence(&signed.evidence).unwrap();
    let (stream_kind, stream_payload_digest) =
        peer_evidence_kind_and_payload_digest(&signed.evidence).unwrap();
    assert_eq!(stream_kind, kind);
    assert_eq!(stream_payload_digest, sha256(&payload));
    assert_eq!(
        signed_peer_evidence_proof_digest(&signed).unwrap(),
        sha256(&encode_signed_peer_evidence(&signed).unwrap())
    );
}

#[test]
fn recovery_ack_streaming_digests_match_legacy_canonical_encoding() {
    let evidence = ReplicationPeerEvidence::RecoveryAck(ReplicationRecoveryAck {
        voter: ReplicaId::new(7),
        membership_epoch: 3,
        recovery_term: 19,
        leader: ReplicaId::new(8),
        locks: (1_u64..=64)
            .map(|position| ReplicationLockSummary {
                position,
                term: 11,
                effect: RevisionEffectId(u128::from(position) << 32),
            })
            .collect(),
    });
    let signed = SignedReplicationPeerEvidence {
        trust_epoch: 5,
        signer: KeyId([0xA5; 32]),
        evidence,
        signature: [0x5A; 64],
    };

    let (kind, payload) = encode_peer_evidence(&signed.evidence).unwrap();
    let (stream_kind, stream_payload_digest) =
        peer_evidence_kind_and_payload_digest(&signed.evidence).unwrap();
    assert_eq!(stream_kind, kind);
    assert_eq!(stream_payload_digest, sha256(&payload));
    assert_eq!(
        signed_peer_evidence_proof_digest(&signed).unwrap(),
        sha256(&encode_signed_peer_evidence(&signed).unwrap())
    );
}
