use thiserror::Error;

#[derive(Debug, Error)]
pub enum SysctlError {
    #[error("sysctl name->MIB failed for {name}: {source}")]
    NameToMib {
        name: String,
        source: std::io::Error,
    },

    #[error("sysctl read failed for {name}: {source}")]
    ReadFailed {
        name: String,
        source: std::io::Error,
    },

    #[error("generation mismatch after {retries} retries for {name}")]
    GenerationMismatch { name: String, retries: u32 },

    #[error("buffer too small for header (got {got} bytes, need {need})")]
    TooSmall { got: usize, need: usize },

    #[error("platform not supported for sysctl")]
    UnsupportedPlatform,
}

/// Reads a raw sysctl value by name. Two-call pattern: get size, allocate +25%, read.
///
/// # Errors
/// Returns `SysctlError` if the sysctl call fails or the platform is unsupported.
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
pub fn read_sysctl(name: &str) -> Result<Vec<u8>, SysctlError> {
    use std::ffi::CString;

    let cname = CString::new(name).map_err(|e| SysctlError::ReadFailed {
        name: name.to_string(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, e),
    })?;

    // First call: get the size
    let mut size: libc::size_t = 0;
    let ret = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret != 0 {
        return Err(SysctlError::ReadFailed {
            name: name.to_string(),
            source: std::io::Error::last_os_error(),
        });
    }

    // Allocate with 25% headroom for concurrent changes
    let alloc_size = size + size / 4;
    let mut buf = vec![0u8; alloc_size];
    let mut actual_size = alloc_size;

    // Second call: read the data
    let ret = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut actual_size,
            std::ptr::null_mut(),
            0,
        )
    };
    if ret != 0 {
        return Err(SysctlError::ReadFailed {
            name: name.to_string(),
            source: std::io::Error::last_os_error(),
        });
    }

    buf.truncate(actual_size);
    Ok(buf)
}

#[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
pub fn read_sysctl(_name: &str) -> Result<Vec<u8>, SysctlError> {
    Err(SysctlError::UnsupportedPlatform)
}

/// Reads `net.inet.tcp.pcblist_n`, validates header/trailer generation match.
/// Retries up to `max_retries` times on generation mismatch.
///
/// Returns `(raw_buf, generation)`.
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
pub fn read_pcblist_validated(name: &str, max_retries: u32) -> Result<(Vec<u8>, u64), SysctlError> {
    const XINPGEN_MIN_SIZE: usize = 24;

    for attempt in 0..=max_retries {
        let buf = read_sysctl(name)?;

        if buf.len() < XINPGEN_MIN_SIZE {
            return Err(SysctlError::TooSmall {
                got: buf.len(),
                need: XINPGEN_MIN_SIZE,
            });
        }

        // Header: first 4 bytes = xig_len (u32), then 4 bytes count, then 8 bytes xig_gen
        let header_len = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        let header_gen = u64::from_ne_bytes([
            buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
        ]);

        // Trailer: last `header_len` bytes contain the same struct
        if buf.len() < header_len {
            return Err(SysctlError::TooSmall {
                got: buf.len(),
                need: header_len,
            });
        }
        let trailer_offset = buf.len() - header_len;
        if trailer_offset + 16 > buf.len() {
            return Err(SysctlError::TooSmall {
                got: buf.len(),
                need: trailer_offset + 16,
            });
        }
        let trailer_gen = u64::from_ne_bytes([
            buf[trailer_offset + 8],
            buf[trailer_offset + 9],
            buf[trailer_offset + 10],
            buf[trailer_offset + 11],
            buf[trailer_offset + 12],
            buf[trailer_offset + 13],
            buf[trailer_offset + 14],
            buf[trailer_offset + 15],
        ]);

        if header_gen == trailer_gen {
            return Ok((buf, header_gen));
        }

        if attempt == max_retries {
            return Err(SysctlError::GenerationMismatch {
                name: name.to_string(),
                retries: max_retries,
            });
        }
    }

    unreachable!()
}

#[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
pub fn read_pcblist_validated(
    _name: &str,
    _max_retries: u32,
) -> Result<(Vec<u8>, u64), SysctlError> {
    Err(SysctlError::UnsupportedPlatform)
}

/// Reads `kern.clockrate` and returns the `hz` value (ticks per second).
/// Typically 100 on macOS.
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
pub fn read_clock_hz() -> Result<i32, SysctlError> {
    let buf = read_sysctl("kern.clockrate")?;
    // struct clockinfo { int hz; int tick; int tickadj; int stathz; int profhz; }
    // hz is the first i32
    if buf.len() < 4 {
        return Err(SysctlError::TooSmall {
            got: buf.len(),
            need: 4,
        });
    }
    Ok(i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]))
}

#[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
pub fn read_clock_hz() -> Result<i32, SysctlError> {
    Err(SysctlError::UnsupportedPlatform)
}

/// Reads the OS version string.
///
/// - macOS: reads `kern.osproductversion` -> e.g. "15.2"
/// - FreeBSD: reads `kern.osrelease` -> e.g. "14.3-RELEASE"
#[cfg(target_os = "macos")]
pub fn read_os_version() -> Result<String, SysctlError> {
    let buf = read_sysctl("kern.osproductversion")?;
    let s = String::from_utf8_lossy(&buf);
    Ok(s.trim_end_matches('\0').to_string())
}

#[cfg(target_os = "freebsd")]
pub fn read_os_version() -> Result<String, SysctlError> {
    let buf = read_sysctl("kern.osrelease")?;
    let s = String::from_utf8_lossy(&buf);
    Ok(format!("FreeBSD {}", s.trim_end_matches('\0')))
}

#[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
pub fn read_os_version() -> Result<String, SysctlError> {
    Err(SysctlError::UnsupportedPlatform)
}

/// System-wide TCP statistics from `sysctl net.inet.tcp.stats`.
///
/// FreeBSD's `struct tcpstat` is an array of `uint64_t` counters.
/// We extract the counters relevant for delta computation.
#[derive(Debug, Clone, Default)]
pub struct TcpSysStats {
    pub connattempt: u64,
    pub accepts: u64,
    pub connects: u64,
    pub drops: u64,
    pub sndtotal: u64,
    pub sndbyte: u64,
    pub sndrexmitpack: u64,
    pub sndrexmitbyte: u64,
    pub rcvtotal: u64,
    pub rcvbyte: u64,
    pub rcvduppack: u64,
    pub rcvbadsum: u64,
}

/// Read system-wide TCP statistics from `net.inet.tcp.stats`.
///
/// The FreeBSD `struct tcpstat` fields are uint64_t counters at known offsets.
/// Field offsets (in u64 units, i.e. multiply by 8 for byte offsets):
///   0: tcps_connattempt
///   1: tcps_accepts
///   2: tcps_connects
///   3: tcps_drops
///   ...
///   11: tcps_sndtotal
///   12: tcps_sndpack (data packets)
///   13: tcps_sndbyte
///   ...
///   16: tcps_sndrexmitpack
///   17: tcps_sndrexmitbyte
///   ...
///   24: tcps_rcvtotal
///   25: tcps_rcvpack
///   26: tcps_rcvbyte
///   ...
///   28: tcps_rcvduppack
///   ...
///   33: tcps_rcvbadsum
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
pub fn read_tcp_stats() -> Result<TcpSysStats, SysctlError> {
    let buf = read_sysctl("net.inet.tcp.stats")?;

    let read_u64_at = |idx: usize| -> u64 {
        let off = idx * 8;
        if off + 8 > buf.len() {
            return 0;
        }
        u64::from_ne_bytes([
            buf[off],
            buf[off + 1],
            buf[off + 2],
            buf[off + 3],
            buf[off + 4],
            buf[off + 5],
            buf[off + 6],
            buf[off + 7],
        ])
    };

    Ok(TcpSysStats {
        connattempt: read_u64_at(0),
        accepts: read_u64_at(1),
        connects: read_u64_at(2),
        drops: read_u64_at(3),
        sndtotal: read_u64_at(11),
        sndbyte: read_u64_at(13),
        sndrexmitpack: read_u64_at(16),
        sndrexmitbyte: read_u64_at(17),
        rcvtotal: read_u64_at(24),
        rcvbyte: read_u64_at(26),
        rcvduppack: read_u64_at(28),
        rcvbadsum: read_u64_at(33),
    })
}

#[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
pub fn read_tcp_stats() -> Result<TcpSysStats, SysctlError> {
    Err(SysctlError::UnsupportedPlatform)
}
