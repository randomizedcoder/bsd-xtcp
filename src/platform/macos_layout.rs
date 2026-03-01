// Offset constants for macOS XNU pcblist_n tagged binary records.
//
// These offsets are derived from XNU kernel headers (bsd/netinet/tcp_var.h,
// bsd/netinet/in_pcb.h, bsd/sys/socketvar.h). They must be validated on a
// real macOS host. Isolating them here makes corrections a single-file change.

// --- Record kind tags (xso_kind values) ---
pub const XSO_SOCKET: u32 = 0x001;
pub const XSO_RCVBUF: u32 = 0x002;
pub const XSO_SNDBUF: u32 = 0x004;
pub const XSO_STATS: u32 = 0x008;
pub const XSO_INPCB: u32 = 0x010;
pub const XSO_TCPCB: u32 = 0x020;

// --- Tagged record header ---
// Each record starts with an 8-byte header:
//   u32 xgn_len   (total record length including header)
//   u32 xgn_kind  (one of XSO_* above)
pub const TAG_HEADER_SIZE: usize = 8;
pub const TAG_LEN_OFFSET: usize = 0;
pub const TAG_KIND_OFFSET: usize = 4;

// --- xinpgen header/trailer ---
// struct xinpgen { u32 xig_len; u32 xig_count; u64 xig_gen; u64 xig_sogen; }
pub const XINPGEN_LEN_OFFSET: usize = 0;
pub const XINPGEN_GEN_OFFSET: usize = 8;

// --- xsocket_n offsets (from record body start, after tag header) ---
// Fields of interest within xsocket_n:
pub const XSOCKET_N_SO_LAST_PID_OFFSET: usize = 68;
pub const XSOCKET_N_SO_E_PID_OFFSET: usize = 72;
pub const XSOCKET_N_SO_UID_OFFSET: usize = 36;
// so_pcb is the kernel socket identifier
pub const XSOCKET_N_SO_PCB_OFFSET: usize = 8;

// --- xsockbuf_n offsets (receive/send buffer) ---
// cc = current byte count in buffer, hiwat = high water mark
pub const XSOCKBUF_N_CC_OFFSET: usize = 0;
pub const XSOCKBUF_N_HIWAT_OFFSET: usize = 4;

// --- xinpcb_n offsets ---
// inp_vflag determines IPv4 vs IPv6
pub const XINPCB_N_INP_VFLAG_OFFSET: usize = 44;
// IPv4 local/remote addresses (struct in_addr, 4 bytes)
pub const XINPCB_N_INP_LADDR_OFFSET: usize = 84;
pub const XINPCB_N_INP_FADDR_OFFSET: usize = 80;
// Ports (network byte order, u16)
pub const XINPCB_N_INP_LPORT_OFFSET: usize = 72;
pub const XINPCB_N_INP_FPORT_OFFSET: usize = 70;
// inp_gencnt
pub const XINPCB_N_INP_GENCNT_OFFSET: usize = 104;

// IPv6 addresses (struct in6_addr, 16 bytes) — in the inp_dep union
pub const XINPCB_N_IN6P_LADDR_OFFSET: usize = 48;
pub const XINPCB_N_IN6P_FADDR_OFFSET: usize = 32;

// --- xtcpcb_n offsets ---
pub const XTCPCB_N_T_STATE_OFFSET: usize = 0;
pub const XTCPCB_N_T_FLAGS_OFFSET: usize = 8;
pub const XTCPCB_N_SND_CWND_OFFSET: usize = 12;
pub const XTCPCB_N_SND_SSTHRESH_OFFSET: usize = 16;
pub const XTCPCB_N_T_MAXSEG_OFFSET: usize = 20;

// RTT fields (raw ticks, need TCP_RTT_SHIFT and hz conversion)
pub const XTCPCB_N_T_SRTT_OFFSET: usize = 24;
pub const XTCPCB_N_T_RTTVAR_OFFSET: usize = 28;

// Sequence numbers
pub const XTCPCB_N_SND_NXT_OFFSET: usize = 32;
pub const XTCPCB_N_SND_UNA_OFFSET: usize = 36;
pub const XTCPCB_N_SND_MAX_OFFSET: usize = 40;
pub const XTCPCB_N_RCV_NXT_OFFSET: usize = 44;
pub const XTCPCB_N_RCV_ADV_OFFSET: usize = 48;

// Windows
pub const XTCPCB_N_SND_WND_OFFSET: usize = 52;
pub const XTCPCB_N_RCV_WND_OFFSET: usize = 56;

// Window scale
pub const XTCPCB_N_SND_WSCALE_OFFSET: usize = 60;
pub const XTCPCB_N_RCV_WSCALE_OFFSET: usize = 61;

// Counters
pub const XTCPCB_N_T_DUPACKS_OFFSET: usize = 64;
pub const XTCPCB_N_T_RXTSHIFT_OFFSET: usize = 4;
pub const XTCPCB_N_T_STARTTIME_OFFSET: usize = 68;

// RTO (raw ticks)
pub const XTCPCB_N_T_RXTCUR_OFFSET: usize = 72;

// --- Constants ---

/// TCP RTT shift factor: t_srtt is stored as (srtt << 3) in ticks.
pub const TCP_RTT_SHIFT: u32 = 3;

/// TCP RTTVAR shift factor: t_rttvar is stored as (rttvar << 2) in ticks.
pub const TCP_RTTVAR_SHIFT: u32 = 2;

/// inp_vflag values
pub const INP_IPV4: u8 = 0x1;
pub const INP_IPV6: u8 = 0x2;

/// Round up to the next multiple of 8 (XNU alignment for tagged records).
pub const fn roundup64(len: u32) -> usize {
    ((len as usize) + 7) & !7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundup64() {
        assert_eq!(roundup64(0), 0);
        assert_eq!(roundup64(1), 8);
        assert_eq!(roundup64(7), 8);
        assert_eq!(roundup64(8), 8);
        assert_eq!(roundup64(9), 16);
        assert_eq!(roundup64(100), 104);
    }
}
