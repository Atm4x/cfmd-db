use std::hint::black_box;
use std::time::Instant;

use ed25519_dalek::SigningKey;
use kernel_auth::{
    AuthorityDigest, TrustRootSet, sign_artifact_manifest, sign_wal_frame_record,
    verify_artifact_manifest, verify_wal_frame_record,
};
use kernel_semantics::RuntimeProfileDigest;

fn ns_per_op(elapsed: std::time::Duration, rounds: u32) -> f64 {
    elapsed.as_secs_f64() * 1_000_000_000.0 / f64::from(rounds)
}

fn main() {
    let signer = SigningKey::from_bytes(&[77; 32]);
    let trust = TrustRootSet::bootstrap(1, &[signer.verifying_key().to_bytes()]).unwrap();
    let runtime = RuntimeProfileDigest([0xA5; 32]);

    let artifact = vec![0x5A; 16 * 1024];
    let descriptor = b"cfmd-semantic-descriptor-v1";
    let manifest = sign_artifact_manifest(&signer, &artifact, runtime, descriptor);
    let rounds = 10_000_u32;
    let start = Instant::now();
    for _ in 0..rounds {
        black_box(verify_artifact_manifest(
            &trust,
            black_box(&manifest),
            black_box(&artifact),
            descriptor,
        ))
        .unwrap();
    }
    println!(
        "artifact_verify_16k_ns_per_op={:.1}",
        ns_per_op(start.elapsed(), rounds)
    );

    let frame = vec![0xC3; 1024];
    let root = AuthorityDigest([0x11; 32]);
    let wal = sign_wal_frame_record(&signer, [0x44; 32], 9, 1, root, &frame);
    let start = Instant::now();
    for _ in 0..rounds {
        black_box(verify_wal_frame_record(
            &trust,
            black_box(&wal),
            black_box(&frame),
        ))
        .unwrap();
    }
    println!(
        "wal_verify_1k_ns_per_op={:.1}",
        ns_per_op(start.elapsed(), rounds)
    );

    let start = Instant::now();
    for lsn in 1..=rounds {
        black_box(sign_wal_frame_record(
            &signer,
            [0x44; 32],
            9,
            u64::from(lsn),
            root,
            black_box(&frame),
        ));
    }
    println!(
        "wal_sign_1k_ns_per_op={:.1}",
        ns_per_op(start.elapsed(), rounds)
    );
}
