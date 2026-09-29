use std::{hint::black_box, time::Instant};

use aes_gcm_siv::{
    Aes256GcmSiv, Nonce,
    aead::{AeadInOut, KeyInit, inout::InOutBuf},
};
use cfmd_secure_memory::SecureBox;

fn bench_ordinary(cipher: &Aes256GcmSiv, size: u32, iterations: u32) -> f64 {
    let mut buffer = vec![0xA5_u8; usize::try_from(size).unwrap()];
    let aad = b"CFMD secure-memory hot-path benchmark";
    let started = Instant::now();
    for iteration in 0..iterations {
        let mut raw_nonce = [0_u8; 12];
        raw_nonce[4..].copy_from_slice(&u64::from(iteration).to_le_bytes());
        let nonce = Nonce::try_from(raw_nonce.as_slice()).unwrap();
        let tag = cipher
            .encrypt_inout_detached(&nonce, aad, InOutBuf::from(&mut buffer[..]))
            .unwrap();
        black_box(tag);
    }
    let elapsed = started.elapsed().as_secs_f64();
    f64::from(size) * f64::from(iterations) / elapsed / (1024.0 * 1024.0)
}

fn bench_secure(cipher: &SecureBox<Aes256GcmSiv>, size: u32, iterations: u32) -> f64 {
    let mut buffer = vec![0xA5_u8; usize::try_from(size).unwrap()];
    let aad = b"CFMD secure-memory hot-path benchmark";
    let started = Instant::now();
    for iteration in 0..iterations {
        let mut raw_nonce = [0_u8; 12];
        raw_nonce[4..].copy_from_slice(&u64::from(iteration).to_le_bytes());
        let nonce = Nonce::try_from(raw_nonce.as_slice()).unwrap();
        let tag = cipher.with_secret(|state| {
            state
                .encrypt_inout_detached(&nonce, aad, InOutBuf::from(&mut buffer[..]))
                .unwrap()
        });
        black_box(tag);
    }
    let elapsed = started.elapsed().as_secs_f64();
    f64::from(size) * f64::from(iterations) / elapsed / (1024.0 * 1024.0)
}

fn main() {
    let key = [0x51_u8; 32];
    let ordinary = Aes256GcmSiv::new((&key).into());
    let secure = SecureBox::try_new_with(|| Aes256GcmSiv::new((&key).into())).unwrap();

    for (size, iterations) in [(4096_u32, 100_000_u32), (65_536, 10_000)] {
        let ordinary_mib_s = bench_ordinary(&ordinary, size, iterations);
        let secure_mib_s = bench_secure(&secure, size, iterations);
        println!(
            "size={size} ordinary={ordinary_mib_s:.2} MiB/s secure={secure_mib_s:.2} MiB/s ratio={:.4}",
            secure_mib_s / ordinary_mib_s
        );
    }
}
