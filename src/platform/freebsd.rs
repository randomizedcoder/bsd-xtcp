use crate::platform::freebsd_layout::*;
use crate::platform::CollectError;
use crate::record::{IpAddr, RawSocketRecord};
use std::collections::HashMap;

#[cfg(target_os = "freebsd")]
use crate::platform::CollectionResult;

// Device paths for the KLD character devices.
#[cfg(target_os = "freebsd")]
const DEV_TCPSTATS_FULL: &str = "/dev/tcpstats-full";
#[cfg(target_os = "freebsd")]
const DEV_TCPSTATS: &str = "/dev/tcpstats";

/// Top-level FreeBSD collector entry point.
///
/// 1. Reads records from `/dev/tcpstats-full` (or `/dev/tcpstats`).
/// 2. Enriches records with PID/FD mapping from `kern.file` sysctl.
/// 3. Returns the collection result.
#[cfg(target_os = "freebsd")]
pub fn collect() -> Result<CollectionResult, CollectError> {
    let start = std::time::Instant::now();
    let (mut records, generation) = collect_from_kld()?;
    enrich_with_pid_mapping(&mut records);
    let duration = start.elapsed().as_nanos() as u64;

    Ok(CollectionResult {
        records,
        generation,
        collection_duration_ns: duration,
    })
}

/// Opens the KLD device, checks version via ioctl, reads all records.
#[cfg(target_os = "freebsd")]
fn collect_from_kld() -> Result<(Vec<RawSocketRecord>, u64), CollectError> {
    use std::fs::File;
    use std::io::Read;
    use std::os::unix::io::AsRawFd;

    // Try /dev/tcpstats-full first (includes all states), fall back to /dev/tcpstats.
    let (mut file, _path) = File::open(DEV_TCPSTATS_FULL)
        .map(|f| (f, DEV_TCPSTATS_FULL))
        .or_else(|_| File::open(DEV_TCPSTATS).map(|f| (f, DEV_TCPSTATS)))
        .map_err(|e| CollectError::DeviceOpen {
            path: format!("{} or {}", DEV_TCPSTATS_FULL, DEV_TCPSTATS),
            source: e,
        })?;

    // Version check via ioctl.
    let mut ver = TcpstatsVersion::default();
    let ret = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            TCPSTATS_VERSION_CMD as libc::c_ulong,
            &mut ver as *mut TcpstatsVersion,
        )
    };
    if ret < 0 {
        return Err(CollectError::Ioctl {
            cmd: TCPSTATS_VERSION_CMD,
            source: std::io::Error::last_os_error(),
        });
    }
    if ver.protocol_version != TCP_STATS_VERSION {
        return Err(CollectError::VersionMismatch {
            expected: TCP_STATS_VERSION,
            got: ver.protocol_version,
        });
    }

    // Size buffer from the kmod's record count hint (from version ioctl).
    // Add 20% headroom for sockets appearing between ioctl and read.
    // Minimum 64KB to handle small counts + sockets created during the gap.
    let hint = ver.record_count_hint as usize;
    let capacity = ((hint + hint / 5 + 64) * TCP_STATS_RECORD_SIZE).max(64 * 1024);
    let mut buf = vec![0u8; capacity];
    let nbytes = file.read(&mut buf).map_err(|e| CollectError::DeviceRead { source: e })?;
    buf.truncate(nbytes);

    let records = parse_kld_records(&buf)?;

    // Use record count as a pseudo-generation (KLD has no generation counter).
    let generation = records.len() as u64;

    Ok((records, generation))
}

// --- Pure parser functions (testable on any platform) ---

/// Parse a buffer of concatenated 320-byte KLD records into `RawSocketRecord`s.
///
/// This is a pure function testable on any platform with synthetic data.
pub fn parse_kld_records(buf: &[u8]) -> Result<Vec<RawSocketRecord>, CollectError> {
    if buf.is_empty() {
        return Ok(Vec::new());
    }

    if buf.len() % TCP_STATS_RECORD_SIZE != 0 {
        return Err(CollectError::Parse {
            offset: 0,
            message: format!(
                "buffer size {} is not a multiple of record size {}",
                buf.len(),
                TCP_STATS_RECORD_SIZE,
            ),
        });
    }

    let count = buf.len() / TCP_STATS_RECORD_SIZE;
    let mut records = Vec::with_capacity(count);

    for i in 0..count {
        let offset = i * TCP_STATS_RECORD_SIZE;
        let chunk = &buf[offset..offset + TCP_STATS_RECORD_SIZE];

        // Safety: chunk is exactly TCP_STATS_RECORD_SIZE bytes, and TcpStatsRecord
        // is #[repr(C, packed)] with that exact size. We copy to avoid alignment issues.
        let tsr: TcpStatsRecord = unsafe {
            let mut rec = std::mem::MaybeUninit::<TcpStatsRecord>::uninit();
            std::ptr::copy_nonoverlapping(
                chunk.as_ptr(),
                rec.as_mut_ptr() as *mut u8,
                TCP_STATS_RECORD_SIZE,
            );
            rec.assume_init()
        };

        if tsr.tsr_version != TCP_STATS_VERSION {
            return Err(CollectError::VersionMismatch {
                expected: TCP_STATS_VERSION,
                got: tsr.tsr_version,
            });
        }

        records.push(kld_record_to_raw(&tsr)?);
    }

    Ok(records)
}

/// Convert a single KLD C record to a `RawSocketRecord`.
fn kld_record_to_raw(tsr: &TcpStatsRecord) -> Result<RawSocketRecord, CollectError> {
    let is_ipv6 = (tsr.tsr_flags & TSR_F_IPV6) != 0 || tsr.tsr_af == AF_INET6;

    let (local_addr, remote_addr, ip_version) = if is_ipv6 {
        let mut la = [0u8; 16];
        la.copy_from_slice(&tsr.tsr_local_addr);
        let mut ra = [0u8; 16];
        ra.copy_from_slice(&tsr.tsr_remote_addr);
        (Some(IpAddr::V6(la)), Some(IpAddr::V6(ra)), Some(6u8))
    } else {
        let mut la = [0u8; 4];
        la.copy_from_slice(&tsr.tsr_local_addr[..4]);
        let mut ra = [0u8; 4];
        ra.copy_from_slice(&tsr.tsr_remote_addr[..4]);
        (Some(IpAddr::V4(la)), Some(IpAddr::V4(ra)), Some(4u8))
    };

    // Extract NUL-terminated strings for CC algo and TCP stack.
    let cc_algo = {
        let s = extract_nul_string(&tsr.tsr_cc);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };
    let tcp_stack = {
        let s = extract_nul_string(&tsr.tsr_stack);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };

    // Normalize timers: negative values become 0 (timer not running).
    let normalize_timer = |v: i32| -> Option<u32> { Some(if v < 0 { 0 } else { v as u32 }) };

    Ok(RawSocketRecord {
        // Connection identity
        local_addr,
        remote_addr,
        local_port: Some(tsr.tsr_local_port),
        remote_port: Some(tsr.tsr_remote_port),
        ip_version,
        socket_id: Some(tsr.tsr_so_addr),

        // TCP state (FreeBSD TCPS_* values are same as macOS: 0-10)
        state: Some(tsr.tsr_state),
        tcp_flags: Some(tsr.tsr_flags_tcp),

        // Congestion control
        snd_cwnd: Some(tsr.tsr_snd_cwnd),
        snd_ssthresh: Some(tsr.tsr_snd_ssthresh),
        snd_wnd: Some(tsr.tsr_snd_wnd),
        rcv_wnd: Some(tsr.tsr_rcv_wnd),
        maxseg: Some(tsr.tsr_maxseg),
        cc_algo,
        tcp_stack,

        // RTT (already in microseconds from tcp_fill_info in the KLD)
        rtt_us: Some(tsr.tsr_rtt),
        rttvar_us: Some(tsr.tsr_rttvar),
        rto_us: Some(tsr.tsr_rto),
        rtt_min_us: Some(tsr.tsr_rttmin),

        // Window scale
        snd_wscale: Some(tsr.tsr_snd_wscale as u32),
        rcv_wscale: Some(tsr.tsr_rcv_wscale as u32),

        // Options
        options: Some(tsr.tsr_options),

        // Sequence numbers
        snd_nxt: Some(tsr.tsr_snd_nxt),
        snd_una: Some(tsr.tsr_snd_una),
        snd_max: Some(tsr.tsr_snd_max),
        rcv_nxt: Some(tsr.tsr_rcv_nxt),
        rcv_adv: Some(tsr.tsr_rcv_adv),

        // Counters
        snd_rexmitpack: Some(tsr.tsr_snd_rexmitpack),
        rcv_ooopack: Some(tsr.tsr_rcv_ooopack),
        snd_zerowin: Some(tsr.tsr_snd_zerowin),
        dupacks: Some(tsr.tsr_dupacks),
        rcv_numsacks: Some(tsr.tsr_rcv_numsacks),

        // ECN
        ecn_flags: Some(tsr.tsr_ecn),
        delivered_ce: Some(tsr.tsr_delivered_ce),
        received_ce: Some(tsr.tsr_received_ce),

        // DSACK
        dsack_bytes: Some(tsr.tsr_dsack_bytes),
        dsack_pack: Some(tsr.tsr_dsack_pack),

        // TLP
        total_tlp: Some(tsr.tsr_total_tlp),
        total_tlp_bytes: Some(tsr.tsr_total_tlp_bytes),

        // Timers (normalized: negative -> 0)
        timer_rexmt_ms: normalize_timer(tsr.tsr_tt_rexmt),
        timer_persist_ms: normalize_timer(tsr.tsr_tt_persist),
        timer_keep_ms: normalize_timer(tsr.tsr_tt_keep),
        timer_2msl_ms: normalize_timer(tsr.tsr_tt_2msl),
        timer_delack_ms: normalize_timer(tsr.tsr_tt_delack),
        idle_time_ms: normalize_timer(tsr.tsr_rcvtime),

        // Buffers
        snd_buf_used: Some(tsr.tsr_snd_buf_cc),
        snd_buf_hiwat: Some(tsr.tsr_snd_buf_hiwat),
        rcv_buf_used: Some(tsr.tsr_rcv_buf_cc),
        rcv_buf_hiwat: Some(tsr.tsr_rcv_buf_hiwat),

        // Socket metadata
        uid: Some(tsr.tsr_uid),
        inp_gencnt: Some(tsr.tsr_inp_gencnt),

        // PID not available from KLD directly — enriched later via kern.file.
        pid: None,
        effective_pid: None,
        fd: None,

        // Not available from KLD
        rxt_shift: None,
        start_time_secs: None,

        // Data source
        sources: vec![DATA_SOURCE_FREEBSD_KLD],
    })
}

/// Enrich records with PID and FD information from `kern.file` sysctl.
///
/// Reads the `kern.file` sysctl, parses the `xfile` struct array, and joins
/// on socket_id (tsr_so_addr == xf_data for DTYPE_SOCKET entries).
#[cfg(target_os = "freebsd")]
fn enrich_with_pid_mapping(records: &mut [RawSocketRecord]) {
    let pid_map = match build_pid_map() {
        Ok(m) => m,
        Err(_) => return, // Best-effort: if kern.file fails, skip PID enrichment.
    };

    for rec in records.iter_mut() {
        if let Some(socket_id) = rec.socket_id {
            if let Some(&(pid, fd)) = pid_map.get(&socket_id) {
                rec.pid = Some(pid);
                rec.fd = Some(fd);
                if !rec.sources.contains(&DATA_SOURCE_KERN_FILE) {
                    rec.sources.push(DATA_SOURCE_KERN_FILE);
                }
            }
        }
    }
}

/// PID mapping entry: (pid, fd).
type PidMap = HashMap<u64, (i32, i32)>;

/// Build a mapping from socket kernel address -> (pid, fd) using `kern.file` sysctl.
#[cfg(target_os = "freebsd")]
fn build_pid_map() -> Result<PidMap, CollectError> {
    let buf = crate::sysctl::read_sysctl("kern.file")?;
    parse_kern_file(&buf)
}

/// Parse FreeBSD `kern.file` sysctl output (array of `struct xfile`).
///
/// The `xfile` struct is self-describing: `xf_size` at offset 0 gives the stride.
/// We filter for `xf_type == DTYPE_SOCKET` and extract (xf_data, xf_pid, xf_fd).
///
/// This is a pure function testable on any platform.
pub fn parse_kern_file(buf: &[u8]) -> Result<PidMap, CollectError> {
    let mut map = PidMap::new();

    if buf.len() < 4 {
        return Ok(map);
    }

    // First field of xfile is xf_size (u64 on FreeBSD, but we read as usize).
    // struct xfile {
    //   ksize_t  xf_size;    // offset 0: size of this struct (stride)
    //   pid_t    xf_pid;     // offset 8: owning process
    //   uid_t    xf_uid;     // offset 12: effective uid
    //   int      xf_fd;      // offset 16: descriptor number
    //   int      xf_file_flags; // offset 20
    //   short    xf_type;    // offset 24: descriptor type
    //   ...
    //   kvaddr_t xf_data;    // offset 40 (amd64): socket kernel address
    // }
    // Note: ksize_t and kvaddr_t are 8 bytes on 64-bit FreeBSD.

    // Read stride from first entry.
    if buf.len() < 8 {
        return Ok(map);
    }
    let stride = u64::from_ne_bytes([
        buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
    ]) as usize;

    if stride == 0 || stride > buf.len() {
        return Ok(map);
    }

    // Minimum offsets we need: xf_data at offset 40 + 8 = 48.
    if stride < 48 {
        return Ok(map);
    }

    let mut pos = 0;
    while pos + stride <= buf.len() {
        let entry = &buf[pos..pos + stride];

        // xf_type at offset 24 (short = i16)
        let xf_type = u16::from_ne_bytes([entry[24], entry[25]]);

        if xf_type == DTYPE_SOCKET {
            // xf_pid at offset 8 (i32)
            let xf_pid = i32::from_ne_bytes([entry[8], entry[9], entry[10], entry[11]]);
            // xf_fd at offset 16 (i32)
            let xf_fd = i32::from_ne_bytes([entry[16], entry[17], entry[18], entry[19]]);
            // xf_data at offset 40 (u64)
            let xf_data = u64::from_ne_bytes([
                entry[40], entry[41], entry[42], entry[43], entry[44], entry[45], entry[46],
                entry[47],
            ]);

            // First PID seen for this socket wins (multi-FD sharing is rare).
            map.entry(xf_data).or_insert((xf_pid, xf_fd));
        }

        pos += stride;
    }

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a minimal 320-byte KLD record buffer.
    fn make_kld_record(af: u8, local_port: u16, remote_port: u16, state: i32, cc: &str) -> Vec<u8> {
        let mut buf = vec![0u8; TCP_STATS_RECORD_SIZE];

        // Version
        buf[0..4].copy_from_slice(&TCP_STATS_VERSION.to_ne_bytes());
        // Length
        buf[4..8].copy_from_slice(&(TCP_STATS_RECORD_SIZE as u32).to_ne_bytes());

        // Flags (set TSR_F_IPV6 if AF_INET6)
        let flags: u32 = if af == AF_INET6 { TSR_F_IPV6 } else { 0 };
        buf[8..12].copy_from_slice(&flags.to_ne_bytes());

        // AF
        buf[16] = af;

        // Ports (native endian as stored by KLD)
        buf[20..22].copy_from_slice(&local_port.to_ne_bytes());
        buf[22..24].copy_from_slice(&remote_port.to_ne_bytes());

        // Local addr (offset 24..40)
        if af == AF_INET {
            buf[24..28].copy_from_slice(&[127, 0, 0, 1]);
        } else {
            buf[24] = 0xfe;
            buf[25] = 0x80;
            buf[39] = 1;
        }

        // Remote addr (offset 40..56)
        if af == AF_INET {
            buf[40..44].copy_from_slice(&[10, 0, 0, 1]);
        } else {
            buf[40] = 0x20;
            buf[41] = 0x01;
            buf[55] = 2;
        }

        // State (offset 56..60)
        buf[56..60].copy_from_slice(&state.to_ne_bytes());

        // CC algo (offset 84..100)
        let cc_bytes = cc.as_bytes();
        let cc_len = cc_bytes.len().min(TCP_STATS_CC_MAXLEN);
        buf[84..84 + cc_len].copy_from_slice(&cc_bytes[..cc_len]);

        // RTT = 15000 us at offset 116
        buf[116..120].copy_from_slice(&15000u32.to_ne_bytes());

        // snd_cwnd = 65535 at offset 64
        buf[64..68].copy_from_slice(&65535u32.to_ne_bytes());

        // so_addr at offset 248
        buf[248..256].copy_from_slice(&0xDEAD_BEEF_u64.to_ne_bytes());

        // uid = 1000 at offset 256
        buf[256..260].copy_from_slice(&1000u32.to_ne_bytes());

        buf
    }

    #[test]
    fn test_parse_kld_empty_buffer() {
        let result = parse_kld_records(&[]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_parse_kld_single_ipv4() {
        let buf = make_kld_record(AF_INET, 8080, 443, 4, "cubic");
        let records = parse_kld_records(&buf).unwrap();
        assert_eq!(records.len(), 1);

        let rec = &records[0];
        assert_eq!(rec.ip_version, Some(4));
        assert_eq!(rec.local_port, Some(8080));
        assert_eq!(rec.remote_port, Some(443));
        assert_eq!(rec.state, Some(4)); // ESTABLISHED
        assert_eq!(rec.local_addr, Some(IpAddr::V4([127, 0, 0, 1])));
        assert_eq!(rec.remote_addr, Some(IpAddr::V4([10, 0, 0, 1])));
        assert_eq!(rec.cc_algo, Some("cubic".to_string()));
        assert_eq!(rec.rtt_us, Some(15000));
        assert_eq!(rec.snd_cwnd, Some(65535));
        assert_eq!(rec.uid, Some(1000));
        assert_eq!(rec.socket_id, Some(0xDEAD_BEEF));
        assert_eq!(rec.sources, vec![DATA_SOURCE_FREEBSD_KLD]);
    }

    #[test]
    fn test_parse_kld_single_ipv6() {
        let buf = make_kld_record(AF_INET6, 80, 12345, 4, "newreno");
        let records = parse_kld_records(&buf).unwrap();
        assert_eq!(records.len(), 1);

        let rec = &records[0];
        assert_eq!(rec.ip_version, Some(6));
        assert_eq!(rec.local_port, Some(80));
        assert_eq!(rec.remote_port, Some(12345));
        assert_eq!(rec.cc_algo, Some("newreno".to_string()));

        // Verify IPv6 addresses
        if let Some(IpAddr::V6(addr)) = &rec.local_addr {
            assert_eq!(addr[0], 0xfe);
            assert_eq!(addr[1], 0x80);
            assert_eq!(addr[15], 1);
        } else {
            panic!("expected IPv6 local address");
        }
    }

    #[test]
    fn test_parse_kld_multiple_records() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&make_kld_record(AF_INET, 80, 1000, 4, "cubic"));
        buf.extend_from_slice(&make_kld_record(AF_INET, 443, 2000, 1, "newreno"));
        buf.extend_from_slice(&make_kld_record(AF_INET6, 8080, 3000, 10, "cubic"));

        let records = parse_kld_records(&buf).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].local_port, Some(80));
        assert_eq!(records[1].local_port, Some(443));
        assert_eq!(records[2].local_port, Some(8080));
        assert_eq!(records[2].ip_version, Some(6));
    }

    #[test]
    fn test_parse_kld_version_mismatch() {
        let mut buf = make_kld_record(AF_INET, 80, 443, 4, "cubic");
        // Set version to 99
        buf[0..4].copy_from_slice(&99u32.to_ne_bytes());

        let result = parse_kld_records(&buf);
        assert!(result.is_err());
        match result.unwrap_err() {
            CollectError::VersionMismatch { expected, got } => {
                assert_eq!(expected, 1);
                assert_eq!(got, 99);
            }
            e => panic!("expected VersionMismatch, got {:?}", e),
        }
    }

    #[test]
    fn test_parse_kld_bad_alignment() {
        // 319 bytes is not a multiple of 320
        let buf = vec![0u8; 319];
        let result = parse_kld_records(&buf);
        assert!(result.is_err());
        match result.unwrap_err() {
            CollectError::Parse { .. } => {}
            e => panic!("expected Parse error, got {:?}", e),
        }
    }

    #[test]
    fn test_timer_normalization() {
        let mut buf = make_kld_record(AF_INET, 80, 443, 4, "cubic");
        // Set tsr_tt_rexmt (offset for timers) to a negative value.
        // Timer offsets: tsr_tt_rexmt starts at offset 208:
        //   16 (header) + 48 (identity) + 8 (state) + 52 (cc) + 16 (rtt) + 4 (wscale) +
        //   20 (seqnums) + 20 (counters) + 12 (ecn) + 8 (dsack) + 12 (tlp) = 216
        //   But packed: total_tlp(4) + total_tlp_bytes(8) = 12 total, so 196+12=208
        let timer_offset = 208;
        buf[timer_offset..timer_offset + 4].copy_from_slice(&(-1i32).to_ne_bytes());

        let records = parse_kld_records(&buf).unwrap();
        assert_eq!(records[0].timer_rexmt_ms, Some(0)); // negative normalized to 0
    }

    #[test]
    fn test_cc_algo_string_extraction() {
        let buf = make_kld_record(AF_INET, 80, 443, 4, "cubic");
        let records = parse_kld_records(&buf).unwrap();
        assert_eq!(records[0].cc_algo, Some("cubic".to_string()));

        // Empty CC algo
        let buf2 = make_kld_record(AF_INET, 80, 443, 4, "");
        let records2 = parse_kld_records(&buf2).unwrap();
        assert_eq!(records2[0].cc_algo, None);
    }

    #[test]
    fn test_parse_kern_file_socket_entry() {
        // Build a synthetic kern.file buffer with one xfile entry for a socket.
        let stride: u64 = 128; // typical stride
        let mut entry = vec![0u8; stride as usize];

        // xf_size at offset 0
        entry[0..8].copy_from_slice(&stride.to_ne_bytes());
        // xf_pid at offset 8
        entry[8..12].copy_from_slice(&42i32.to_ne_bytes());
        // xf_fd at offset 16
        entry[16..20].copy_from_slice(&7i32.to_ne_bytes());
        // xf_type at offset 24 = DTYPE_SOCKET
        entry[24..26].copy_from_slice(&DTYPE_SOCKET.to_ne_bytes());
        // xf_data at offset 40 = socket address
        entry[40..48].copy_from_slice(&0xCAFE_BABEu64.to_ne_bytes());

        let map = parse_kern_file(&entry).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&0xCAFE_BABE), Some(&(42, 7)));
    }

    #[test]
    fn test_parse_kern_file_non_socket_skipped() {
        let stride: u64 = 128;
        let mut entry = vec![0u8; stride as usize];
        entry[0..8].copy_from_slice(&stride.to_ne_bytes());
        entry[8..12].copy_from_slice(&42i32.to_ne_bytes());
        entry[16..20].copy_from_slice(&7i32.to_ne_bytes());
        // xf_type = 1 (DTYPE_VNODE, not socket)
        entry[24..26].copy_from_slice(&1u16.to_ne_bytes());
        entry[40..48].copy_from_slice(&0xCAFE_BABEu64.to_ne_bytes());

        let map = parse_kern_file(&entry).unwrap();
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn test_parse_kern_file_first_pid_wins() {
        let stride: u64 = 128;
        let mut buf = Vec::new();

        // First entry: pid=42, fd=3
        let mut e1 = vec![0u8; stride as usize];
        e1[0..8].copy_from_slice(&stride.to_ne_bytes());
        e1[8..12].copy_from_slice(&42i32.to_ne_bytes());
        e1[16..20].copy_from_slice(&3i32.to_ne_bytes());
        e1[24..26].copy_from_slice(&DTYPE_SOCKET.to_ne_bytes());
        e1[40..48].copy_from_slice(&0x1234u64.to_ne_bytes());
        buf.extend_from_slice(&e1);

        // Second entry: same socket, pid=99, fd=5
        let mut e2 = vec![0u8; stride as usize];
        e2[0..8].copy_from_slice(&stride.to_ne_bytes());
        e2[8..12].copy_from_slice(&99i32.to_ne_bytes());
        e2[16..20].copy_from_slice(&5i32.to_ne_bytes());
        e2[24..26].copy_from_slice(&DTYPE_SOCKET.to_ne_bytes());
        e2[40..48].copy_from_slice(&0x1234u64.to_ne_bytes());
        buf.extend_from_slice(&e2);

        let map = parse_kern_file(&buf).unwrap();
        assert_eq!(map.get(&0x1234), Some(&(42, 3))); // first wins
    }

    #[test]
    fn test_kld_field_mapping() {
        let mut buf = make_kld_record(AF_INET, 8080, 443, 4, "cubic");

        // Set specific field values in the raw buffer to verify field mapping.
        // snd_ssthresh at offset 68
        buf[68..72].copy_from_slice(&1000u32.to_ne_bytes());
        // snd_wnd at offset 72
        buf[72..76].copy_from_slice(&2000u32.to_ne_bytes());
        // rcv_wnd at offset 76
        buf[76..80].copy_from_slice(&3000u32.to_ne_bytes());
        // maxseg at offset 80
        buf[80..84].copy_from_slice(&1460u32.to_ne_bytes());

        // rttvar at offset 120
        buf[120..124].copy_from_slice(&500u32.to_ne_bytes());
        // rto at offset 124
        buf[124..128].copy_from_slice(&200000u32.to_ne_bytes());
        // rttmin at offset 128
        buf[128..132].copy_from_slice(&1000u32.to_ne_bytes());

        let records = parse_kld_records(&buf).unwrap();
        let rec = &records[0];

        assert_eq!(rec.snd_ssthresh, Some(1000));
        assert_eq!(rec.snd_wnd, Some(2000));
        assert_eq!(rec.rcv_wnd, Some(3000));
        assert_eq!(rec.maxseg, Some(1460));
        assert_eq!(rec.rttvar_us, Some(500));
        assert_eq!(rec.rto_us, Some(200000));
        assert_eq!(rec.rtt_min_us, Some(1000));
    }
}
