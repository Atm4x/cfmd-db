#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use crate::{DeploymentError, IsolationClass, RuntimeProfileSpec, SandboxedRuntime};
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxNamespaceProcessRuntime {
    profile: RuntimeProfileSpec,
    unshare_program: PathBuf,
    timeout_program: PathBuf,
}

#[cfg(target_os = "linux")]
impl LinuxNamespaceProcessRuntime {
    pub fn new(profile: RuntimeProfileSpec) -> Result<Self, DeploymentError> {
        profile.validate()?;
        if profile.isolation != IsolationClass::SandboxedProcess {
            return Err(DeploymentError::InvalidRuntimeProfile);
        }
        Ok(Self {
            profile,
            unshare_program: PathBuf::from("/usr/bin/unshare"),
            timeout_program: PathBuf::from("/usr/bin/timeout"),
        })
    }

    #[must_use]
    pub fn with_programs(
        mut self,
        unshare: impl Into<PathBuf>,
        timeout: impl Into<PathBuf>,
    ) -> Self {
        self.unshare_program = unshare.into();
        self.timeout_program = timeout.into();
        self
    }

    fn invoke_file(
        &self,
        executable: &Path,
        encoded_request: &[u8],
    ) -> Result<Vec<u8>, DeploymentError> {
        let timeout_ms = self.profile.max_fuel.max(1);
        let mut child = Command::new(&self.timeout_program)
            .arg("--signal=KILL")
            .arg(format!("{}.{:03}s", timeout_ms / 1_000, timeout_ms % 1_000))
            .arg(&self.unshare_program)
            .args([
                "--user",
                "--map-root-user",
                "--mount",
                "--pid",
                "--fork",
                "--kill-child",
                "--net",
                "--ipc",
                "--uts",
                "--propagation",
                "private",
            ])
            .arg(executable)
            .env_clear()
            .current_dir(executable.parent().ok_or(DeploymentError::RuntimeFailure)?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| DeploymentError::RuntimeFailure)?;
        let mut stdin = child.stdin.take().ok_or(DeploymentError::RuntimeFailure)?;
        stdin
            .write_all(encoded_request)
            .map_err(|_| DeploymentError::RuntimeFailure)?;
        drop(stdin);
        let limit = u64::from(self.profile.max_response_bytes) + 1;
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or(DeploymentError::RuntimeFailure)?
            .take(limit)
            .read_to_end(&mut output)
            .map_err(|_| DeploymentError::RuntimeFailure)?;
        let status = child.wait().map_err(|_| DeploymentError::RuntimeFailure)?;
        if !status.success() {
            return Err(DeploymentError::RuntimeFailure);
        }
        if output.len() > usize::try_from(self.profile.max_response_bytes).unwrap_or(usize::MAX) {
            return Err(DeploymentError::AbiFrameTooLarge);
        }
        Ok(output)
    }
}

#[cfg(target_os = "linux")]
impl SandboxedRuntime for LinuxNamespaceProcessRuntime {
    fn profile(&self) -> RuntimeProfileSpec {
        self.profile
    }

    fn invoke(
        &mut self,
        artifact: &[u8],
        encoded_request: &[u8],
    ) -> Result<Vec<u8>, DeploymentError> {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let id = COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("cfmd-semantic-sandbox-{}-{id}", std::process::id()));
        fs::create_dir(&dir).map_err(|_| DeploymentError::RuntimeFailure)?;
        let executable = dir.join("artifact");
        let result = (|| {
            fs::write(&executable, artifact).map_err(|_| DeploymentError::RuntimeFailure)?;
            let mut permissions = fs::metadata(&executable)
                .map_err(|_| DeploymentError::RuntimeFailure)?
                .permissions();
            permissions.set_mode(0o500);
            fs::set_permissions(&executable, permissions)
                .map_err(|_| DeploymentError::RuntimeFailure)?;
            self.invoke_file(&executable, encoded_request)
        })();
        let _ = fs::remove_dir_all(&dir);
        result
    }
}
