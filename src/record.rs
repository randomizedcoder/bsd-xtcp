/// Raw IP address representation from kernel data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpAddr {
    V4([u8; 4]),
    V6([u8; 16]),
}

/// Intermediate representation of a TCP socket record parsed from kernel data.
///
/// All values are in normalized units (RTT in microseconds, etc.).
/// `Option<T>` fields may be absent depending on the data source or TCP state.
#[derive(Debug, Clone, Default)]
pub struct RawSocketRecord {
    // Connection identity
    pub local_addr: Option<IpAddr>,
    pub remote_addr: Option<IpAddr>,
    pub local_port: Option<u16>,
    pub remote_port: Option<u16>,
    pub ip_version: Option<u8>, // 4 or 6
    pub socket_id: Option<u64>,

    // TCP state
    pub state: Option<i32>, // macOS TCPS_* value (0-10)
    pub tcp_flags: Option<u32>,

    // Congestion control
    pub snd_cwnd: Option<u32>,
    pub snd_ssthresh: Option<u32>,
    pub snd_wnd: Option<u32>,
    pub rcv_wnd: Option<u32>,
    pub maxseg: Option<u32>,

    // RTT (all in microseconds)
    pub rtt_us: Option<u32>,
    pub rttvar_us: Option<u32>,
    pub rto_us: Option<u32>,

    // Sequence numbers
    pub snd_nxt: Option<u32>,
    pub snd_una: Option<u32>,
    pub snd_max: Option<u32>,
    pub rcv_nxt: Option<u32>,
    pub rcv_adv: Option<u32>,

    // Window scale
    pub snd_wscale: Option<u32>,
    pub rcv_wscale: Option<u32>,

    // Counters
    pub dupacks: Option<u32>,
    pub rxt_shift: Option<u32>,

    // Buffers
    pub snd_buf_used: Option<u32>,
    pub snd_buf_hiwat: Option<u32>,
    pub rcv_buf_used: Option<u32>,
    pub rcv_buf_hiwat: Option<u32>,

    // Process attribution
    pub pid: Option<i32>,
    pub effective_pid: Option<i32>,
    pub uid: Option<u32>,

    // Platform-specific
    pub inp_gencnt: Option<u64>,
    pub start_time_secs: Option<u32>,

    // Data source tag
    pub sources: Vec<u8>,
}
