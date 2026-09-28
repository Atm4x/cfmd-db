mod file;
mod recovery;
mod simulated;

pub use file::FileRevisionWal;
pub(crate) use file::WalRegionRecovery;
pub use recovery::scan_wal;
pub use simulated::SimulatedRevisionWal;

pub(crate) const WAL_FRESHNESS_PREFIX_DOMAIN: &[u8] = b"CFMD-WAL-FRESHNESS-PREFIX-v1\0";
