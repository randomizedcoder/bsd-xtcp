#[cfg(target_os = "macos")]
pub mod macos;
pub mod macos_layout;

pub mod freebsd;
pub mod freebsd_layout;

#[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
pub mod stub;

use crate::record::RawSocketRecord;
use crate::sysctl::SysctlError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CollectError {
    #[error("sysctl error: {0}")]
    Sysctl(#[from] SysctlError),

    #[error("parse error at offset {offset}: {message}")]
    Parse { offset: usize, message: String },

    #[error("buffer truncated: need {need} bytes at offset {offset}, have {have}")]
    Truncated {
        offset: usize,
        need: usize,
        have: usize,
    },

    #[error("unknown record kind {kind:#x} at offset {offset}")]
    UnknownKind { offset: usize, kind: u32 },

    #[error("failed to open device {path}: {source}")]
    DeviceOpen {
        path: String,
        source: std::io::Error,
    },

    #[error("device read failed: {source}")]
    DeviceRead { source: std::io::Error },

    #[error("ioctl {cmd:#x} failed: {source}")]
    Ioctl { cmd: u64, source: std::io::Error },

    #[error("version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u32, got: u32 },

    #[error("platform not supported")]
    UnsupportedPlatform,
}

/// Result of a single collection pass.
#[derive(Debug)]
pub struct CollectionResult {
    pub records: Vec<RawSocketRecord>,
    pub generation: u64,
    pub collection_duration_ns: u64,
}

/// Collect TCP socket records from the kernel.
///
/// On macOS, reads `net.inet.tcp.pcblist_n` and parses the tagged binary stream.
/// On FreeBSD, reads `/dev/tcpstats` kernel module and enriches with kern.file PID mapping.
/// On other platforms, returns `Err(CollectError::UnsupportedPlatform)`.
pub fn collect_tcp_sockets() -> Result<CollectionResult, CollectError> {
    #[cfg(target_os = "macos")]
    {
        macos::collect()
    }
    #[cfg(target_os = "freebsd")]
    {
        freebsd::collect()
    }
    #[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
    {
        stub::collect()
    }
}
