mod file;
mod recovery;
mod simulated;

pub use file::FileRevisionWal;
pub(crate) use file::WalRegionRecovery;
pub(crate) use recovery::WalRegionScanSpec;
pub use recovery::scan_wal;
pub use simulated::SimulatedRevisionWal;

pub(crate) const WAL_FRESHNESS_PREFIX_DOMAIN: &[u8] = b"CFMD-WAL-FRESHNESS-PREFIX-v1\0";
pub(crate) fn wal_aad_context(
    kind: crate::wal_frame::RecordKind,
    lsn: u64,
    revision: kernel_types::RevisionId,
) -> [u8; 17] {
    let mut context = [0_u8; 17];
    context[0] = kind as u8;
    context[1..9].copy_from_slice(&lsn.to_le_bytes());
    context[9..17].copy_from_slice(&revision.raw().to_le_bytes());
    context
}
