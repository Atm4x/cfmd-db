#[cfg(target_os = "linux")]
fn main() {
    use std::{fs::File, hint::black_box, io::Read, process};

    use cfmd_secure_memory::SecureBytes;

    let mut random = File::open("/dev/urandom").expect("open /dev/urandom");
    let mut control = vec![0_u8; 32];
    random
        .read_exact(&mut control)
        .expect("fill ordinary control marker");

    let mut secure = SecureBytes::<32>::try_zeroed().expect("allocate secure marker");
    secure.with_secret_mut(|bytes| {
        random.read_exact(bytes).expect("fill secure marker");
    });

    println!("CONTROL_MARKER_HEX={}", hex(&control));
    secure.with_secret(|bytes| println!("SECURE_MARKER_HEX={}", hex(bytes)));

    black_box(&control);
    black_box(&secure);
    process::abort();
}

#[cfg(target_os = "linux")]
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("write to String cannot fail");
    }
    encoded
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("core-dump probe currently supports Linux only");
    std::process::exit(77);
}
