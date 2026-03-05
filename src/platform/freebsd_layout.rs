// FreeBSD tcp_stats_kld record layout.
//
// Mirrors the C `struct tcp_stats_record` from kmod/tcp_stats_kld/tcp_stats_kld.h.
// This file must be kept in sync with the kernel module header.

/// Fixed record size from the kernel module.
pub const TCP_STATS_RECORD_SIZE: usize = 320;

/// Protocol version this client expects.
pub const TCP_STATS_VERSION: u32 = 1;

/// Maximum length of CC algorithm name (NUL-terminated).
pub const TCP_STATS_CC_MAXLEN: usize = 16;

/// Maximum length of TCP stack name (NUL-terminated).
pub const TCP_STATS_STACK_MAXLEN: usize = 16;

// Record flags
pub const TSR_F_IPV6: u32 = 0x0000_0001;
#[allow(dead_code)]
pub const TSR_F_LISTEN: u32 = 0x0000_0002;
#[allow(dead_code)]
pub const TSR_F_SYNCACHE: u32 = 0x0000_0004;

// Address family constants (FreeBSD)
pub const AF_INET: u8 = 2;
pub const AF_INET6: u8 = 28;

// DataSource enum values matching proto
pub const DATA_SOURCE_FREEBSD_KLD: u8 = 5;
pub const DATA_SOURCE_KERN_FILE: u8 = 6;

/// Rust mirror of `struct tcp_stats_record` (320 bytes, packed).
///
/// Field order and sizes exactly match the C definition.
/// All multi-byte fields are native-endian (read on the same FreeBSD host).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct TcpStatsRecord {
    // Record header (16 bytes)
    pub tsr_version: u32,
    pub tsr_len: u32,
    pub tsr_flags: u32,
    pub _tsr_pad0: u32,

    // Connection identity (48 bytes)
    pub tsr_af: u8,
    pub _tsr_pad1: [u8; 3],
    pub tsr_local_port: u16,
    pub tsr_remote_port: u16,
    pub tsr_local_addr: [u8; 16],  // union { in_addr v4; in6_addr v6; }
    pub tsr_remote_addr: [u8; 16], // union { in_addr v4; in6_addr v6; }

    // TCP state (8 bytes)
    pub tsr_state: i32,
    pub tsr_flags_tcp: u32,

    // Congestion control (52 bytes)
    pub tsr_snd_cwnd: u32,
    pub tsr_snd_ssthresh: u32,
    pub tsr_snd_wnd: u32,
    pub tsr_rcv_wnd: u32,
    pub tsr_maxseg: u32,
    pub tsr_cc: [u8; TCP_STATS_CC_MAXLEN],
    pub tsr_stack: [u8; TCP_STATS_STACK_MAXLEN],

    // RTT from tcp_fill_info() (16 bytes)
    pub tsr_rtt: u32,
    pub tsr_rttvar: u32,
    pub tsr_rto: u32,
    pub tsr_rttmin: u32,

    // Window scale + options (4 bytes)
    pub tsr_snd_wscale: u8,
    pub tsr_rcv_wscale: u8,
    pub tsr_options: u8,
    pub _tsr_pad2: u8,

    // Sequence numbers from tcp_fill_info() (20 bytes)
    pub tsr_snd_nxt: u32,
    pub tsr_snd_una: u32,
    pub tsr_snd_max: u32,
    pub tsr_rcv_nxt: u32,
    pub tsr_rcv_adv: u32,

    // Counters (20 bytes)
    pub tsr_snd_rexmitpack: u32,
    pub tsr_rcv_ooopack: u32,
    pub tsr_snd_zerowin: u32,
    pub tsr_dupacks: u32,
    pub tsr_rcv_numsacks: u32,

    // ECN (12 bytes)
    pub tsr_ecn: u32,
    pub tsr_delivered_ce: u32,
    pub tsr_received_ce: u32,

    // DSACK (8 bytes)
    pub tsr_dsack_bytes: u32,
    pub tsr_dsack_pack: u32,

    // TLP (12 bytes)
    pub tsr_total_tlp: u32,
    pub tsr_total_tlp_bytes: u64,

    // Timers in milliseconds, 0 = not running (24 bytes)
    pub tsr_tt_rexmt: i32,
    pub tsr_tt_persist: i32,
    pub tsr_tt_keep: i32,
    pub tsr_tt_2msl: i32,
    pub tsr_tt_delack: i32,
    pub tsr_rcvtime: i32,

    // Buffer utilization (16 bytes)
    pub tsr_snd_buf_cc: u32,
    pub tsr_snd_buf_hiwat: u32,
    pub tsr_rcv_buf_cc: u32,
    pub tsr_rcv_buf_hiwat: u32,

    // Socket metadata (20 bytes)
    pub tsr_so_addr: u64,
    pub tsr_uid: u32,
    pub tsr_inp_gencnt: u64,

    // Spare for future expansion (52 bytes)
    pub _tsr_spare: [u32; 13],
}

// Compile-time size assertion matching the C _Static_assert.
const _: () = assert!(std::mem::size_of::<TcpStatsRecord>() == TCP_STATS_RECORD_SIZE);

/// Response to TCPSTATS_VERSION_CMD ioctl.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TcpstatsVersion {
    pub protocol_version: u32,
    pub record_size: u32,
    pub record_count_hint: u32,
    pub flags: u32,
}

// FreeBSD ioctl encoding macros.
// From <sys/ioccom.h>:
//   #define _IOC(inout,group,num,len)
//     ((unsigned long)(inout) | ((len & IOCPARM_MASK) << 16) | ((group) << 8) | (num))
//   IOCPARM_MASK = 0x1FFF
//   IOC_OUT = 0x40000000
//   IOC_IN  = 0x80000000
//   IOC_VOID = 0x20000000
//   _IOR(g,n,t) = _IOC(IOC_OUT, g, n, sizeof(t))
//   _IOW(g,n,t) = _IOC(IOC_IN, g, n, sizeof(t))
//   _IO(g,n)    = _IOC(IOC_VOID, g, n, 0)

const IOC_OUT: u64 = 0x4000_0000;
const IOC_IN: u64 = 0x8000_0000;
const IOC_VOID: u64 = 0x2000_0000;

const fn freebsd_ioc(inout: u64, group: u8, num: u8, len: usize) -> u64 {
    inout | (((len as u64) & 0x1FFF) << 16) | ((group as u64) << 8) | (num as u64)
}

const fn freebsd_ior(group: u8, num: u8, len: usize) -> u64 {
    freebsd_ioc(IOC_OUT, group, num, len)
}

const fn freebsd_iow(group: u8, num: u8, len: usize) -> u64 {
    freebsd_ioc(IOC_IN, group, num, len)
}

const fn freebsd_io(group: u8, num: u8) -> u64 {
    freebsd_ioc(IOC_VOID, group, num, 0)
}

/// `TCPSTATS_VERSION_CMD = _IOR('T', 1, struct tcpstats_version)`
pub const TCPSTATS_VERSION_CMD: u64 = freebsd_ior(b'T', 1, std::mem::size_of::<TcpstatsVersion>());

/// `TCPSTATS_SET_FILTER = _IOW('T', 2, struct tcpstats_filter)`
/// Filter struct is up to 256 bytes; we use the actual struct size for the ioctl encoding.
/// For now we define the constant but don't implement the filter struct in Rust.
pub const TCPSTATS_SET_FILTER: u64 = freebsd_iow(b'T', 2, 256);

/// `TCPSTATS_RESET = _IO('T', 3)`
pub const TCPSTATS_RESET: u64 = freebsd_io(b'T', 3);

/// FreeBSD `struct xfile` field type constant for sockets.
pub const DTYPE_SOCKET: u16 = 2;

/// Extract a NUL-terminated string from a fixed-size byte array.
pub fn extract_nul_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_size() {
        assert_eq!(std::mem::size_of::<TcpStatsRecord>(), 320);
    }

    #[test]
    fn test_version_struct_size() {
        assert_eq!(std::mem::size_of::<TcpstatsVersion>(), 16);
    }

    #[test]
    fn test_ioctl_constants() {
        // TCPSTATS_VERSION_CMD = _IOR('T', 1, struct tcpstats_version)
        // = IOC_OUT | ((16 & 0x1FFF) << 16) | ('T' << 8) | 1
        // = 0x40000000 | 0x00100000 | 0x5400 | 0x01
        // = 0x40105401
        assert_eq!(TCPSTATS_VERSION_CMD, 0x4010_5401);

        // TCPSTATS_RESET = _IO('T', 3)
        // = IOC_VOID | 0 | ('T' << 8) | 3
        // = 0x20000000 | 0x5400 | 0x03
        // = 0x20005403
        assert_eq!(TCPSTATS_RESET, 0x2000_5403);
    }

    #[test]
    fn test_extract_nul_string() {
        let mut buf = [0u8; 16];
        buf[..5].copy_from_slice(b"cubic");
        assert_eq!(extract_nul_string(&buf), "cubic");

        let empty = [0u8; 16];
        assert_eq!(extract_nul_string(&empty), "");

        let full: [u8; 4] = *b"rack";
        assert_eq!(extract_nul_string(&full), "rack");
    }
}
