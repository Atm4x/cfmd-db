#![cfg(target_os = "linux")]

use std::{env, process::Command};

use cfmd_secure_memory::{SecureBytes, SecureMemoryOperation};

const CHILD_ENV: &str = "CFMD_SECURE_MEMORY_RLIMIT_CHILD";

#[test]
fn zero_memlock_limit_reports_lock_operation() {
    if env::var_os(CHILD_ENV).is_some() {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: getrlimit writes one rlimit value to the valid pointer.
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &raw mut limit) },
            0
        );
        limit.rlim_cur = 0;
        // SAFETY: setrlimit receives a valid rlimit value and affects only this
        // dedicated child process spawned by the parent branch below.
        assert_eq!(
            unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &raw const limit) },
            0
        );

        let error = SecureBytes::<32>::try_zeroed().unwrap_err();
        assert_eq!(error.operation(), Some(SecureMemoryOperation::LockPages));
        return;
    }

    let executable = env::current_exe().unwrap();
    let output = Command::new(executable)
        .arg("--exact")
        .arg("zero_memlock_limit_reports_lock_operation")
        .arg("--nocapture")
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "resource-limit child failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
