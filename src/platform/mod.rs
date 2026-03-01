pub mod macos;
pub mod macos_layout;

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
/// On macOS/FreeBSD, reads `net.inet.tcp.pcblist_n` and parses the tagged binary stream.
/// On other platforms, returns `Err(CollectError::UnsupportedPlatform)`.
pub fn collect_tcp_sockets() -> Result<CollectionResult, CollectError> {
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    {
        macos::collect()
    }
    #[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
    {
        stub::collect()
    }
}
