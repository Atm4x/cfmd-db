use std::{hint::black_box, time::Instant};

use cfmd_secure_memory::SecureBytes;

fn ns_per(elapsed: std::time::Duration, iterations: u32) -> f64 {
    elapsed.as_secs_f64() * 1_000_000_000.0 / f64::from(iterations)
}

fn main() {
    const ALLOCATIONS: u32 = 50_000;
    const READS: u32 = 20_000_000;

    let started = Instant::now();
    for index in 0..ALLOCATIONS {
        let secret = SecureBytes::<32>::try_from_array([index.to_le_bytes()[0]; 32]).unwrap();
        black_box(secret.with_secret(|bytes| bytes[0]));
    }
    let secure_alloc = started.elapsed();

    let started = Instant::now();
    for index in 0..ALLOCATIONS {
        let secret = Box::new([index.to_le_bytes()[0]; 32]);
        black_box(secret[0]);
    }
    let box_alloc = started.elapsed();

    let secret = SecureBytes::<32>::try_from_array([7; 32]).unwrap();
    let started = Instant::now();
    let mut accumulator = 0_u8;
    for index in 0..READS {
        accumulator ^= secret.with_secret(|bytes| bytes[usize::try_from(index & 31).unwrap()]);
        black_box(accumulator);
    }
    let secure_read = started.elapsed();

    let ordinary = [7_u8; 32];
    let started = Instant::now();
    let mut ordinary_accumulator = 0_u8;
    for index in 0..READS {
        ordinary_accumulator ^= ordinary[usize::try_from(index & 31).unwrap()];
        black_box(ordinary_accumulator);
    }
    let ordinary_read = started.elapsed();

    println!(
        "secure_alloc_drop_ns={:.2}",
        ns_per(secure_alloc, ALLOCATIONS)
    );
    println!("box_alloc_drop_ns={:.2}", ns_per(box_alloc, ALLOCATIONS));
    println!("secure_hot_read_ns={:.3}", ns_per(secure_read, READS));
    println!("ordinary_hot_read_ns={:.3}", ns_per(ordinary_read, READS));
}
