#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableFormatCompatibility {
    pub version: u16,
    pub readable: bool,
    pub writable: bool,
}

/// Current pre-release CFMD durable storage format discriminator.
///
/// `1` is the selected release candidate, but compatibility is not frozen until
/// the project declares its first release. PASS-era formats are never retained.
pub const FORMAT_VERSION: u16 = 1;

pub const FORMAT_COMPATIBILITY: [DurableFormatCompatibility; 1] = [DurableFormatCompatibility {
    version: FORMAT_VERSION,
    readable: true,
    writable: true,
}];

/// No pre-release historical format is an upgrade source.
pub const FORMAT_UPGRADE_SOURCES: [u16; 0] = [];
/// Pre-release builds never emit an obsolete historical durable format.
pub const FORMAT_DOWNGRADE_TARGETS: [u16; 0] = [];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableFormatSupport {
    ReadWrite,
    UnsupportedOlder,
    UnsupportedNewer,
}

#[must_use]
pub const fn durable_format_support(version: u16) -> DurableFormatSupport {
    if version == FORMAT_VERSION {
        DurableFormatSupport::ReadWrite
    } else if version < FORMAT_VERSION {
        DurableFormatSupport::UnsupportedOlder
    } else {
        DurableFormatSupport::UnsupportedNewer
    }
}
