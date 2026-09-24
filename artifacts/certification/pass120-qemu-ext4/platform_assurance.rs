use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use kernel_auth::{KeyId, Sha256Digest, TrustRootSet, sha256};

use super::DurabilityError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedDurabilityProfile {
    LinuxExt4Ordered,
    LinuxXfs,
}

impl SupportedDurabilityProfile {
    #[must_use]
    pub const fn filesystem(self) -> &'static str {
        match self {
            Self::LinuxExt4Ordered => "ext4",
            Self::LinuxXfs => "xfs",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurabilityPlatformEvidence {
    pub profile: SupportedDurabilityProfile,
    pub kernel_release: String,
    pub mount_source: String,
    pub filesystem: String,
    pub mount_options: Vec<String>,
    pub super_options: Vec<String>,
    pub primitive_probe_completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestructiveDurabilityCampaignEvidence {
    pub profile: SupportedDurabilityProfile,
    pub campaign_id: String,
    pub platform_fingerprint: Sha256Digest,
    pub completed_fault_cases: u32,
    pub expected_fault_cases: u32,
    pub evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedDestructiveDurabilityCampaignEvidence {
    pub trust_root_epoch: u64,
    pub evidence: DestructiveDurabilityCampaignEvidence,
    pub signer: KeyId,
    pub signature: [u8; 64],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedDestructiveDurabilityCampaignEvidence {
    evidence: DestructiveDurabilityCampaignEvidence,
}

impl VerifiedDestructiveDurabilityCampaignEvidence {
    #[must_use]
    pub const fn evidence(&self) -> &DestructiveDurabilityCampaignEvidence {
        &self.evidence
    }
}

const DURABILITY_CAMPAIGN_AUTH_DOMAIN: &[u8] = b"CFMD-DURABILITY-CAMPAIGN-AUTH-v1\0";
const MAX_CAMPAIGN_ID_LEN: usize = 256;

#[must_use]
pub fn durability_campaign_signing_message(
    trust_root_epoch: u64,
    evidence: &DestructiveDurabilityCampaignEvidence,
) -> Vec<u8> {
    let mut bytes = DURABILITY_CAMPAIGN_AUTH_DOMAIN.to_vec();
    bytes.extend_from_slice(&trust_root_epoch.to_le_bytes());
    bytes.push(match evidence.profile {
        SupportedDurabilityProfile::LinuxExt4Ordered => 1,
        SupportedDurabilityProfile::LinuxXfs => 2,
    });
    bytes.extend_from_slice(&(evidence.campaign_id.len() as u64).to_le_bytes());
    bytes.extend_from_slice(evidence.campaign_id.as_bytes());
    bytes.extend_from_slice(&evidence.platform_fingerprint.0);
    bytes.extend_from_slice(&evidence.completed_fault_cases.to_le_bytes());
    bytes.extend_from_slice(&evidence.expected_fault_cases.to_le_bytes());
    bytes.extend_from_slice(&evidence.evidence_digest.0);
    bytes
}

pub fn verify_signed_destructive_durability_campaign(
    trust_roots: &TrustRootSet,
    signed: &SignedDestructiveDurabilityCampaignEvidence,
) -> Result<VerifiedDestructiveDurabilityCampaignEvidence, DurabilityError> {
    if signed.trust_root_epoch != trust_roots.epoch() {
        return Err(protocol("destructive campaign trust-root epoch mismatch"));
    }
    signed.evidence.validate_structure()?;
    let message = durability_campaign_signing_message(signed.trust_root_epoch, &signed.evidence);
    trust_roots
        .verify_message(signed.signer, &message, &signed.signature)
        .map_err(|_| protocol("destructive campaign signature authentication failed"))?;
    Ok(VerifiedDestructiveDurabilityCampaignEvidence {
        evidence: signed.evidence.clone(),
    })
}

impl DestructiveDurabilityCampaignEvidence {
    fn validate_structure(&self) -> Result<(), DurabilityError> {
        if self.campaign_id.is_empty() || self.campaign_id.len() > MAX_CAMPAIGN_ID_LEN {
            return Err(protocol(
                "destructive campaign identity is incomplete or oversized",
            ));
        }
        if self.expected_fault_cases != REQUIRED_DESTRUCTIVE_CASES
            || self.completed_fault_cases != REQUIRED_DESTRUCTIVE_CASES
        {
            return Err(protocol(
                "destructive durability campaign does not cover every required cut",
            ));
        }
        if self.evidence_digest == Sha256Digest([0; 32]) {
            return Err(protocol("destructive campaign evidence digest is empty"));
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        runtime: &DurabilityPlatformEvidence,
    ) -> Result<(), DurabilityError> {
        self.validate_structure()?;
        if self.profile != runtime.profile {
            return Err(protocol(
                "destructive campaign profile does not match runtime profile",
            ));
        }
        if self.platform_fingerprint != durability_platform_fingerprint(runtime) {
            return Err(protocol(
                "destructive campaign belongs to another platform fingerprint",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MountInfo {
    mount_point: PathBuf,
    source: String,
    filesystem: String,
    mount_options: Vec<String>,
    super_options: Vec<String>,
}

pub fn verify_supported_durability_platform(
    directory: impl AsRef<Path>,
    profile: SupportedDurabilityProfile,
) -> Result<DurabilityPlatformEvidence, DurabilityError> {
    if std::env::consts::OS != "linux" {
        return Err(protocol(
            "supported durability profiles currently require Linux",
        ));
    }
    let directory = directory.as_ref();
    fs::create_dir_all(directory)?;
    let canonical = directory.canonicalize()?;
    let mount = mount_for_path(&canonical)?;
    validate_mount(&mount, profile)?;
    probe_durability_primitives(&canonical)?;
    let kernel_release = fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_else(|_| "unknown".to_owned())
        .trim()
        .to_owned();
    Ok(DurabilityPlatformEvidence {
        profile,
        kernel_release,
        mount_source: mount.source,
        filesystem: mount.filesystem,
        mount_options: mount.mount_options,
        super_options: mount.super_options,
        primitive_probe_completed: true,
    })
}

#[must_use]
pub fn durability_platform_fingerprint(evidence: &DurabilityPlatformEvidence) -> Sha256Digest {
    let mut bytes = b"CFMD-DURABILITY-PLATFORM-v1\0".to_vec();
    bytes.extend_from_slice(match evidence.profile {
        SupportedDurabilityProfile::LinuxExt4Ordered => b"linux-ext4-ordered\0",
        SupportedDurabilityProfile::LinuxXfs => b"linux-xfs\0",
    });
    bytes.extend_from_slice(std::env::consts::ARCH.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(evidence.kernel_release.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(evidence.mount_source.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(evidence.filesystem.as_bytes());
    bytes.push(0);
    let mut mount_options = evidence.mount_options.clone();
    mount_options.sort();
    let mut super_options = evidence.super_options.clone();
    super_options.sort();
    for option in mount_options.into_iter().chain(super_options) {
        bytes.extend_from_slice(option.as_bytes());
        bytes.push(0);
    }
    sha256(&bytes)
}

pub fn certify_supported_durability_platform(
    directory: impl AsRef<Path>,
    profile: SupportedDurabilityProfile,
    campaign: &VerifiedDestructiveDurabilityCampaignEvidence,
) -> Result<DurabilityPlatformEvidence, DurabilityError> {
    let evidence = verify_supported_durability_platform(directory, profile)?;
    campaign.evidence.validate_for(&evidence)?;
    Ok(evidence)
}

pub fn probe_durability_primitives(directory: impl AsRef<Path>) -> Result<(), DurabilityError> {
    let directory = directory.as_ref();
    fs::create_dir_all(directory)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| protocol("system clock precedes Unix epoch"))?
        .as_nanos();
    let pending = directory.join(format!(".cfmd-platform-probe-{nonce}.pending"));
    let published = directory.join(format!(".cfmd-platform-probe-{nonce}.published"));
    let payload = b"cfmd-platform-durability-probe-v1";

    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&pending)?;
        file.write_all(payload)?;
        file.sync_all()?;
        File::open(directory)?.sync_all()?;
        fs::rename(&pending, &published)?;
        File::open(directory)?.sync_all()?;

        let mut observed = Vec::new();
        File::open(&published)?.read_to_end(&mut observed)?;
        if observed != payload {
            return Err(protocol("durability primitive probe read-back mismatch"));
        }
        fs::remove_file(&published)?;
        File::open(directory)?.sync_all()?;
        Ok(())
    })();

    let _ = fs::remove_file(&pending);
    let _ = fs::remove_file(&published);
    result
}

#[cfg(target_os = "linux")]
pub fn current_mount_durability_evidence(
    directory: impl AsRef<Path>,
) -> Result<(String, String, Vec<String>, Vec<String>), DurabilityError> {
    let directory = directory.as_ref();
    fs::create_dir_all(directory)?;
    let mount = mount_for_path(&directory.canonicalize()?)?;
    Ok((
        mount.source,
        mount.filesystem,
        mount.mount_options,
        mount.super_options,
    ))
}

fn validate_mount(
    mount: &MountInfo,
    profile: SupportedDurabilityProfile,
) -> Result<(), DurabilityError> {
    if mount.filesystem != profile.filesystem() {
        return Err(protocol(
            "filesystem does not match requested durability profile",
        ));
    }
    if !mount.source.starts_with("/dev/") {
        return Err(protocol(
            "supported durability profile requires a local block-device mount",
        ));
    }
    let has_option = |needle: &str| {
        mount
            .mount_options
            .iter()
            .chain(&mount.super_options)
            .any(|option| option == needle)
    };
    if has_option("fsync=volatile") {
        return Err(protocol("filesystem advertises volatile fsync semantics"));
    }
    if has_option("nobarrier") {
        return Err(protocol("filesystem barriers are disabled"));
    }
    if profile == SupportedDurabilityProfile::LinuxExt4Ordered && has_option("data=writeback") {
        return Err(protocol(
            "ext4 data=writeback is outside the supported durability profile",
        ));
    }
    Ok(())
}

fn mount_for_path(path: &Path) -> Result<MountInfo, DurabilityError> {
    let text = fs::read_to_string("/proc/self/mountinfo")?;
    mount_for_path_from(&text, path)
        .ok_or_else(|| protocol("cannot resolve filesystem mount for durability directory"))
}

fn mount_for_path_from(text: &str, path: &Path) -> Option<MountInfo> {
    text.lines()
        .filter_map(parse_mountinfo_line)
        .filter(|mount| path.starts_with(&mount.mount_point))
        .max_by_key(|mount| mount.mount_point.as_os_str().len())
}

fn parse_mountinfo_line(line: &str) -> Option<MountInfo> {
    let (left, right) = line.split_once(" - ")?;
    let left_fields = left.split_whitespace().collect::<Vec<_>>();
    let right_fields = right.split_whitespace().collect::<Vec<_>>();
    if left_fields.len() < 6 || right_fields.len() < 3 {
        return None;
    }
    Some(MountInfo {
        mount_point: PathBuf::from(unescape_mountinfo(left_fields[4])),
        source: unescape_mountinfo(right_fields[1]),
        filesystem: right_fields[0].to_owned(),
        mount_options: split_options(left_fields[5]),
        super_options: split_options(right_fields[2]),
    })
}

fn split_options(options: &str) -> Vec<String> {
    options.split(',').map(str::to_owned).collect()
}

fn unescape_mountinfo(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn protocol(reason: &'static str) -> DurabilityError {
    DurabilityError::Protocol { offset: 0, reason }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_longest_mount_and_decodes_mountinfo_escapes() {
        let text = concat!(
            "1 0 8:1 / / rw,relatime - ext4 /dev/vda1 rw,data=ordered\n",
            "2 1 8:2 / /var/lib/cfmd\\040data rw,relatime - xfs /dev/vdb1 rw\n",
        );
        let mount = mount_for_path_from(text, Path::new("/var/lib/cfmd data/db")).unwrap();
        assert_eq!(mount.filesystem, "xfs");
        assert_eq!(mount.source, "/dev/vdb1");
    }

    #[test]
    fn supported_profiles_reject_wrong_or_unsafe_mounts() {
        let mut ext4 = MountInfo {
            mount_point: PathBuf::from("/data"),
            source: "/dev/nvme0n1p1".to_owned(),
            filesystem: "ext4".to_owned(),
            mount_options: vec!["rw".to_owned()],
            super_options: vec!["rw".to_owned(), "data=ordered".to_owned()],
        };
        assert!(validate_mount(&ext4, SupportedDurabilityProfile::LinuxExt4Ordered).is_ok());
        ext4.super_options.push("nobarrier".to_owned());
        assert!(validate_mount(&ext4, SupportedDurabilityProfile::LinuxExt4Ordered).is_err());
        ext4.super_options.pop();
        ext4.super_options.push("data=writeback".to_owned());
        assert!(validate_mount(&ext4, SupportedDurabilityProfile::LinuxExt4Ordered).is_err());
        ext4.super_options.pop();
        ext4.source = "overlay".to_owned();
        assert!(validate_mount(&ext4, SupportedDurabilityProfile::LinuxExt4Ordered).is_err());
    }

    #[test]
    fn destructive_campaign_requires_complete_matching_evidence() {
        let runtime = DurabilityPlatformEvidence {
            profile: SupportedDurabilityProfile::LinuxXfs,
            kernel_release: "6.x".to_owned(),
            mount_source: "/dev/vdb1".to_owned(),
            filesystem: "xfs".to_owned(),
            mount_options: vec!["rw".to_owned()],
            super_options: vec!["rw".to_owned()],
            primitive_probe_completed: true,
        };
        let good = DestructiveDurabilityCampaignEvidence {
            profile: SupportedDurabilityProfile::LinuxXfs,
            campaign_id: "lab-2026-09-23".to_owned(),
            platform_fingerprint: durability_platform_fingerprint(&runtime),
            completed_fault_cases: REQUIRED_DESTRUCTIVE_CASES,
            expected_fault_cases: REQUIRED_DESTRUCTIVE_CASES,
            evidence_digest: Sha256Digest([0xAA; 32]),
        };
        assert!(good.validate_for(&runtime).is_ok());
        let mut incomplete = good.clone();
        incomplete.completed_fault_cases -= 1;
        assert!(incomplete.validate_for(&runtime).is_err());
    }

    #[test]
    fn destructive_campaign_certificate_is_strictly_authenticated() {
        use ed25519_dalek::{Signer, SigningKey};
        use kernel_auth::key_id;

        let signer = SigningKey::from_bytes(&[0x31; 32]);
        let trust_roots = TrustRootSet::bootstrap(7, &[signer.verifying_key().to_bytes()]).unwrap();
        let runtime = DurabilityPlatformEvidence {
            profile: SupportedDurabilityProfile::LinuxXfs,
            kernel_release: "6.x".to_owned(),
            mount_source: "/dev/vdb1".to_owned(),
            filesystem: "xfs".to_owned(),
            mount_options: vec!["rw".to_owned()],
            super_options: vec!["rw".to_owned()],
            primitive_probe_completed: true,
        };
        let evidence = DestructiveDurabilityCampaignEvidence {
            profile: runtime.profile,
            campaign_id: "lab-run-1".to_owned(),
            platform_fingerprint: durability_platform_fingerprint(&runtime),
            completed_fault_cases: REQUIRED_DESTRUCTIVE_CASES,
            expected_fault_cases: REQUIRED_DESTRUCTIVE_CASES,
            evidence_digest: Sha256Digest([0xAB; 32]),
        };
        let message = durability_campaign_signing_message(7, &evidence);
        let mut certificate = SignedDestructiveDurabilityCampaignEvidence {
            trust_root_epoch: 7,
            evidence,
            signer: key_id(&signer.verifying_key().to_bytes()),
            signature: signer.sign(&message).to_bytes(),
        };
        let verified =
            verify_signed_destructive_durability_campaign(&trust_roots, &certificate).unwrap();
        assert_eq!(verified.evidence(), &certificate.evidence);
        certificate.evidence.campaign_id.push('x');
        assert!(verify_signed_destructive_durability_campaign(&trust_roots, &certificate).is_err());
    }

    #[test]
    fn real_runtime_primitive_probe_crosses_file_and_directory_barriers() {
        let directory =
            std::env::temp_dir().join(format!("cfmd-platform-probe-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        probe_durability_primitives(&directory).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn current_mount_evidence_is_resolvable_and_supported_profiles_fail_closed() {
        let directory =
            std::env::temp_dir().join(format!("cfmd-platform-profile-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let (_, filesystem, _, _) = current_mount_durability_evidence(&directory).unwrap();
        if filesystem != "ext4" {
            assert!(
                verify_supported_durability_platform(
                    &directory,
                    SupportedDurabilityProfile::LinuxExt4Ordered
                )
                .is_err()
            );
        }
        if filesystem != "xfs" {
            assert!(
                verify_supported_durability_platform(
                    &directory,
                    SupportedDurabilityProfile::LinuxXfs
                )
                .is_err()
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructiveDurabilityCut {
    AfterCandidateFileSync,
    AfterPrerequisiteDirectorySync,
    AfterPendingManifestSync,
    AfterManifestRename,
    AfterManifestDirectorySync,
    AfterObsoleteRemove,
    AfterObsoleteDirectorySync,
}

impl DestructiveDurabilityCut {
    pub const ALL: [Self; 7] = [
        Self::AfterCandidateFileSync,
        Self::AfterPrerequisiteDirectorySync,
        Self::AfterPendingManifestSync,
        Self::AfterManifestRename,
        Self::AfterManifestDirectorySync,
        Self::AfterObsoleteRemove,
        Self::AfterObsoleteDirectorySync,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AfterCandidateFileSync => "after-candidate-file-sync",
            Self::AfterPrerequisiteDirectorySync => "after-prerequisite-directory-sync",
            Self::AfterPendingManifestSync => "after-pending-manifest-sync",
            Self::AfterManifestRename => "after-manifest-rename",
            Self::AfterManifestDirectorySync => "after-manifest-directory-sync",
            Self::AfterObsoleteRemove => "after-obsolete-remove",
            Self::AfterObsoleteDirectorySync => "after-obsolete-directory-sync",
        }
    }
}

const REQUIRED_DESTRUCTIVE_CASES: u32 = 7;
const CERT_OLD: &[u8] = b"CFMD-CERT-OLD-v1";
const CERT_NEW: &[u8] = b"CFMD-CERT-NEW-v1";

/// Prepares one real-filesystem destructive campaign cut. The caller must hard
/// power-cut the machine/VM while it remains stopped at this cut; a normal
/// process exit is not evidence for historical #13.
pub fn prepare_destructive_durability_case(
    directory: impl AsRef<Path>,
    cut: DestructiveDurabilityCut,
) -> Result<(), DurabilityError> {
    let directory = directory.as_ref();
    if directory.exists() {
        let mut entries = fs::read_dir(directory)?;
        if entries.next().is_some() {
            return Err(protocol("destructive campaign directory must be empty"));
        }
    } else {
        fs::create_dir_all(directory)?;
    }

    let old = directory.join("old.data");
    let new = directory.join("new.data");
    let manifest = directory.join("manifest");
    let pending = directory.join("manifest.pending");

    write_synced(&old, CERT_OLD)?;
    write_synced(&manifest, CERT_OLD)?;
    sync_dir(directory)?;

    write_synced(&new, CERT_NEW)?;
    if cut == DestructiveDurabilityCut::AfterCandidateFileSync {
        return Ok(());
    }
    sync_dir(directory)?;
    if cut == DestructiveDurabilityCut::AfterPrerequisiteDirectorySync {
        return Ok(());
    }

    write_synced(&pending, CERT_NEW)?;
    if cut == DestructiveDurabilityCut::AfterPendingManifestSync {
        return Ok(());
    }
    fs::rename(&pending, &manifest)?;
    if cut == DestructiveDurabilityCut::AfterManifestRename {
        return Ok(());
    }
    sync_dir(directory)?;
    if cut == DestructiveDurabilityCut::AfterManifestDirectorySync {
        return Ok(());
    }

    fs::remove_file(&old)?;
    if cut == DestructiveDurabilityCut::AfterObsoleteRemove {
        return Ok(());
    }
    sync_dir(directory)?;
    Ok(())
}

/// Verifies the post-power-loss image for one destructive campaign cut against
/// the exact namespace uncertainty allowed by the #18 publication model.
pub fn verify_destructive_durability_case(
    directory: impl AsRef<Path>,
    cut: DestructiveDurabilityCut,
) -> Result<(), DurabilityError> {
    let directory = directory.as_ref();
    let old = directory.join("old.data");
    let new = directory.join("new.data");
    let manifest = directory.join("manifest");

    let manifest_bytes = fs::read(&manifest)?;
    let old_exists = old.exists();
    let new_bytes = fs::read(&new).ok();
    let manifest_is_old = manifest_bytes == CERT_OLD;
    let manifest_is_new = manifest_bytes == CERT_NEW;
    if !manifest_is_old && !manifest_is_new {
        return Err(protocol(
            "destructive campaign observed torn manifest contents",
        ));
    }
    if manifest_is_new && new_bytes.as_deref() != Some(CERT_NEW) {
        return Err(protocol(
            "published new authority is missing its durable prerequisite",
        ));
    }

    match cut {
        DestructiveDurabilityCut::AfterCandidateFileSync => {
            if !manifest_is_old || !old_exists {
                return Err(protocol("pre-publication crash lost old authority"));
            }
        }
        DestructiveDurabilityCut::AfterPrerequisiteDirectorySync
        | DestructiveDurabilityCut::AfterPendingManifestSync => {
            if !manifest_is_old || !old_exists || new_bytes.as_deref() != Some(CERT_NEW) {
                return Err(protocol(
                    "pre-rename crash violated durable prerequisite closure",
                ));
            }
        }
        DestructiveDurabilityCut::AfterManifestRename => {
            if !old_exists {
                return Err(protocol("rename-cut crash removed old payload before GC"));
            }
            if !manifest_is_old && !manifest_is_new {
                return Err(protocol("rename-cut crash has no valid authority"));
            }
        }
        DestructiveDurabilityCut::AfterManifestDirectorySync => {
            if !manifest_is_new || new_bytes.as_deref() != Some(CERT_NEW) || !old_exists {
                return Err(protocol(
                    "post-publish crash did not preserve published authority",
                ));
            }
        }
        DestructiveDurabilityCut::AfterObsoleteRemove => {
            if !manifest_is_new || new_bytes.as_deref() != Some(CERT_NEW) {
                return Err(protocol("GC crash lost published authority"));
            }
        }
        DestructiveDurabilityCut::AfterObsoleteDirectorySync => {
            if !manifest_is_new || new_bytes.as_deref() != Some(CERT_NEW) || old_exists {
                return Err(protocol("completed GC image violates durability contract"));
            }
        }
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), DurabilityError> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn sync_dir(directory: &Path) -> Result<(), DurabilityError> {
    File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod destructive_tests {
    use super::*;

    #[test]
    fn clean_execution_of_each_destructive_cut_satisfies_its_post_crash_predicate() {
        for (index, cut) in DestructiveDurabilityCut::ALL.into_iter().enumerate() {
            let directory = std::env::temp_dir().join(format!(
                "cfmd-destructive-cut-{}-{index}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&directory);
            prepare_destructive_durability_case(&directory, cut).unwrap();
            verify_destructive_durability_case(&directory, cut).unwrap();
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn new_manifest_without_new_payload_is_rejected() {
        let directory =
            std::env::temp_dir().join(format!("cfmd-destructive-torn-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("old.data"), CERT_OLD).unwrap();
        fs::write(directory.join("manifest"), CERT_NEW).unwrap();
        assert!(
            verify_destructive_durability_case(
                &directory,
                DestructiveDurabilityCut::AfterManifestRename
            )
            .is_err()
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
