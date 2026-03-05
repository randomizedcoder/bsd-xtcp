#[cfg(any(target_os = "macos", target_os = "freebsd"))]
use crate::proto_gen::bsd_xtcp::DataSource;
use crate::proto_gen::bsd_xtcp::{
    BatchMessage, CollectionMetadata, IpVersion, Platform, StateBucket, SystemSummary,
    TcpSocketRecord, TcpState,
};
use crate::record::{IpAddr, RawSocketRecord};
use crate::sysctl::TcpSysStats;
use std::collections::BTreeMap;

/// Map macOS/FreeBSD kernel TCPS_* state (0-10) to proto TcpState enum.
///
/// macOS/FreeBSD values: CLOSED=0, LISTEN=1, SYN_SENT=2, SYN_RECEIVED=3, ESTABLISHED=4,
///   CLOSE_WAIT=5, FIN_WAIT_1=6, CLOSING=7, LAST_ACK=8, FIN_WAIT_2=9, TIME_WAIT=10
///
/// Proto values: TCP_STATE_CLOSED=1, ..., TCP_STATE_TIME_WAIT=11
/// Mapping: proto = kernel + 1
pub fn kernel_state_to_proto(kernel_state: i32) -> i32 {
    if (0..=10).contains(&kernel_state) {
        kernel_state + 1
    } else {
        TcpState::Unknown.into()
    }
}

/// Map ip_version byte (4 or 6) to proto IpVersion enum value.
pub fn ip_version_to_proto(version: u8) -> i32 {
    match version {
        4 => IpVersion::IpVersion4.into(),
        6 => IpVersion::IpVersion6.into(),
        _ => IpVersion::Unknown.into(),
    }
}

/// Convert internal IpAddr to raw bytes for proto.
pub fn ip_addr_to_bytes(addr: &IpAddr) -> Vec<u8> {
    match addr {
        IpAddr::V4(a) => a.to_vec(),
        IpAddr::V6(a) => a.to_vec(),
    }
}

/// Convert a `RawSocketRecord` to a proto `TcpSocketRecord`.
pub fn raw_to_proto(raw: &RawSocketRecord) -> TcpSocketRecord {
    TcpSocketRecord {
        local_addr: raw
            .local_addr
            .as_ref()
            .map(ip_addr_to_bytes)
            .unwrap_or_default(),
        remote_addr: raw
            .remote_addr
            .as_ref()
            .map(ip_addr_to_bytes)
            .unwrap_or_default(),
        local_port: raw.local_port.unwrap_or(0) as u32,
        remote_port: raw.remote_port.unwrap_or(0) as u32,
        ip_version: raw
            .ip_version
            .map(ip_version_to_proto)
            .unwrap_or(IpVersion::Unknown.into()),
        socket_id: raw.socket_id,
        state: raw
            .state
            .map(kernel_state_to_proto)
            .unwrap_or(TcpState::Unknown.into()),
        tcp_flags: raw.tcp_flags,
        snd_cwnd: raw.snd_cwnd,
        snd_ssthresh: raw.snd_ssthresh,
        snd_wnd: raw.snd_wnd,
        rcv_wnd: raw.rcv_wnd,
        maxseg: raw.maxseg,
        cc_algo: raw.cc_algo.clone(),
        tcp_stack: raw.tcp_stack.clone(),
        rtt_us: raw.rtt_us,
        rttvar_us: raw.rttvar_us,
        rto_us: raw.rto_us,
        rtt_min_us: raw.rtt_min_us,
        snd_nxt: raw.snd_nxt,
        snd_una: raw.snd_una,
        snd_max: raw.snd_max,
        rcv_nxt: raw.rcv_nxt,
        rcv_adv: raw.rcv_adv,
        snd_wscale: raw.snd_wscale,
        rcv_wscale: raw.rcv_wscale,
        rexmit_packets: raw.snd_rexmitpack,
        ooo_packets: raw.rcv_ooopack,
        zerowin_probes: raw.snd_zerowin,
        dupacks: raw.dupacks,
        sack_blocks: raw.rcv_numsacks,
        dsack_bytes: raw.dsack_bytes,
        dsack_packets: raw.dsack_pack,
        rxt_shift: raw.rxt_shift,
        timer_rexmt_ms: raw.timer_rexmt_ms,
        timer_persist_ms: raw.timer_persist_ms,
        timer_keep_ms: raw.timer_keep_ms,
        timer_2msl_ms: raw.timer_2msl_ms,
        timer_delack_ms: raw.timer_delack_ms,
        idle_time_ms: raw.idle_time_ms,
        snd_buf_used: raw.snd_buf_used,
        snd_buf_hiwat: raw.snd_buf_hiwat,
        rcv_buf_used: raw.rcv_buf_used,
        rcv_buf_hiwat: raw.rcv_buf_hiwat,
        pid: raw.pid,
        effective_pid: raw.effective_pid,
        uid: raw.uid,
        fd: raw.fd,
        ecn_flags: raw.ecn_flags,
        ecn_ce_delivered: raw.delivered_ce,
        ecn_ce_received: raw.received_ce,
        negotiated_options: raw.options.map(|o| o as u32),
        tlp_probes_sent: raw.total_tlp,
        tlp_bytes_sent: raw.total_tlp_bytes,
        inp_gencnt: raw.inp_gencnt,
        start_time_secs: raw.start_time_secs,
        sources: raw.sources.iter().map(|&s| s as i32).collect(),
        ..Default::default()
    }
}

/// Build `CollectionMetadata` for a batch.
pub fn build_metadata(
    generation: u64,
    collection_duration_ns: u64,
    _record_count: u32,
    batch_sequence: u64,
    interval_ms: u32,
) -> CollectionMetadata {
    let timestamp_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".into());

    let os_version = crate::sysctl::read_os_version().unwrap_or_else(|_| "unknown".into());

    #[cfg(target_os = "macos")]
    let (platform, data_sources) = (
        Platform::Macos.into(),
        vec![DataSource::MacosPcblistN.into()],
    );

    #[cfg(target_os = "freebsd")]
    let (platform, data_sources) = (
        Platform::Freebsd.into(),
        vec![DataSource::FreebsdKld.into(), DataSource::KernFile.into()],
    );

    #[cfg(not(any(target_os = "macos", target_os = "freebsd")))]
    let (platform, data_sources): (i32, Vec<i32>) = (Platform::Unknown.into(), vec![]);

    CollectionMetadata {
        timestamp_ns,
        hostname,
        platform,
        os_version,
        interval_ms,
        schedule_name: String::new(),
        data_sources,
        collection_duration_ns,
        pcblist_generation: Some(generation),
        batch_sequence,
        tool_version: format!("bsd-xtcp {}", env!("CARGO_PKG_VERSION")),
    }
}

/// Build a `SystemSummary` from the converted proto records.
pub fn build_summary_from_records(records: &[TcpSocketRecord], interval_ms: u32) -> SystemSummary {
    let timestamp_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    let mut state_map: BTreeMap<i32, u32> = BTreeMap::new();
    for rec in records {
        *state_map.entry(rec.state).or_insert(0) += 1;
    }

    let state_counts: Vec<StateBucket> = state_map
        .into_iter()
        .map(|(state, count)| StateBucket { state, count })
        .collect();

    SystemSummary {
        timestamp_ns,
        interval_ms,
        total_sockets: records.len() as u32,
        state_counts,
        ..Default::default()
    }
}

/// Build a `SystemSummary` enriched with system-wide TCP stats deltas.
pub fn build_summary_with_sys_stats(
    records: &[TcpSocketRecord],
    interval_ms: u32,
    sys_stats: &TcpSysStats,
) -> SystemSummary {
    let mut summary = build_summary_from_records(records, interval_ms);

    summary.delta_conn_attempts = Some(sys_stats.connattempt);
    summary.delta_accepts = Some(sys_stats.accepts);
    summary.delta_connects = Some(sys_stats.connects);
    summary.delta_drops = Some(sys_stats.drops);
    summary.delta_snd_total_packets = Some(sys_stats.sndtotal);
    summary.delta_snd_bytes = Some(sys_stats.sndbyte);
    summary.delta_snd_rexmit_packets = Some(sys_stats.sndrexmitpack);
    summary.delta_snd_rexmit_bytes = Some(sys_stats.sndrexmitbyte);
    summary.delta_rcv_total_packets = Some(sys_stats.rcvtotal);
    summary.delta_rcv_bytes = Some(sys_stats.rcvbyte);
    summary.delta_rcv_dup_packets = Some(sys_stats.rcvduppack);
    summary.delta_rcv_badsum = Some(sys_stats.rcvbadsum);

    // Compute rates
    if sys_stats.sndtotal > 0 {
        summary.retransmit_rate = Some(sys_stats.sndrexmitpack as f64 / sys_stats.sndtotal as f64);
    }
    if sys_stats.rcvtotal > 0 {
        summary.duplicate_rate = Some(sys_stats.rcvduppack as f64 / sys_stats.rcvtotal as f64);
    }

    summary
}

/// Assemble a full `BatchMessage` from collection results.
pub fn build_batch(
    raw_records: &[RawSocketRecord],
    generation: u64,
    collection_duration_ns: u64,
    batch_sequence: u64,
    interval_ms: u32,
) -> BatchMessage {
    let proto_records: Vec<TcpSocketRecord> = raw_records.iter().map(raw_to_proto).collect();

    let metadata = build_metadata(
        generation,
        collection_duration_ns,
        proto_records.len() as u32,
        batch_sequence,
        interval_ms,
    );

    let summary = build_summary_from_records(&proto_records, interval_ms);

    BatchMessage {
        metadata: Some(metadata),
        records: proto_records,
        summary: Some(summary),
    }
}

/// Assemble a full `BatchMessage` with system-wide TCP stats.
pub fn build_batch_with_sys_stats(
    raw_records: &[RawSocketRecord],
    generation: u64,
    collection_duration_ns: u64,
    batch_sequence: u64,
    interval_ms: u32,
    sys_stats: &TcpSysStats,
) -> BatchMessage {
    let proto_records: Vec<TcpSocketRecord> = raw_records.iter().map(raw_to_proto).collect();

    let metadata = build_metadata(
        generation,
        collection_duration_ns,
        proto_records.len() as u32,
        batch_sequence,
        interval_ms,
    );

    let summary = build_summary_with_sys_stats(&proto_records, interval_ms, sys_stats);

    BatchMessage {
        metadata: Some(metadata),
        records: proto_records,
        summary: Some(summary),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kernel_state_to_proto() {
        // CLOSED=0 -> proto 1
        assert_eq!(kernel_state_to_proto(0), 1);
        // ESTABLISHED=4 -> proto 5
        assert_eq!(kernel_state_to_proto(4), 5);
        // TIME_WAIT=10 -> proto 11
        assert_eq!(kernel_state_to_proto(10), 11);
        // Invalid -> UNKNOWN=0
        assert_eq!(kernel_state_to_proto(-1), 0);
        assert_eq!(kernel_state_to_proto(11), 0);
    }

    #[test]
    fn test_ip_version_to_proto() {
        assert_eq!(ip_version_to_proto(4), IpVersion::IpVersion4 as i32);
        assert_eq!(ip_version_to_proto(6), IpVersion::IpVersion6 as i32);
        assert_eq!(ip_version_to_proto(0), IpVersion::Unknown as i32);
    }

    #[test]
    fn test_ip_addr_to_bytes() {
        assert_eq!(
            ip_addr_to_bytes(&IpAddr::V4([10, 0, 0, 1])),
            vec![10, 0, 0, 1]
        );
        let v6 = [0u8; 16];
        assert_eq!(ip_addr_to_bytes(&IpAddr::V6(v6)), vec![0u8; 16]);
    }

    #[test]
    fn test_raw_to_proto_basic() {
        let raw = RawSocketRecord {
            local_addr: Some(IpAddr::V4([127, 0, 0, 1])),
            remote_addr: Some(IpAddr::V4([10, 0, 0, 1])),
            local_port: Some(8080),
            remote_port: Some(443),
            ip_version: Some(4),
            state: Some(4), // ESTABLISHED
            pid: Some(1234),
            uid: Some(501),
            snd_cwnd: Some(65535),
            rtt_us: Some(15000),
            sources: vec![1],
            ..Default::default()
        };

        let proto = raw_to_proto(&raw);
        assert_eq!(proto.local_addr, vec![127, 0, 0, 1]);
        assert_eq!(proto.remote_addr, vec![10, 0, 0, 1]);
        assert_eq!(proto.local_port, 8080);
        assert_eq!(proto.remote_port, 443);
        assert_eq!(proto.state, TcpState::Established as i32);
        assert_eq!(proto.pid, Some(1234));
        assert_eq!(proto.snd_cwnd, Some(65535));
        assert_eq!(proto.rtt_us, Some(15000));
    }

    #[test]
    fn test_raw_to_proto_freebsd_fields() {
        let raw = RawSocketRecord {
            local_addr: Some(IpAddr::V4([127, 0, 0, 1])),
            remote_addr: Some(IpAddr::V4([10, 0, 0, 1])),
            local_port: Some(80),
            remote_port: Some(12345),
            ip_version: Some(4),
            state: Some(4),
            cc_algo: Some("cubic".to_string()),
            tcp_stack: Some("freebsd".to_string()),
            rtt_min_us: Some(1000),
            snd_rexmitpack: Some(5),
            rcv_ooopack: Some(2),
            snd_zerowin: Some(1),
            rcv_numsacks: Some(3),
            ecn_flags: Some(0x03),
            delivered_ce: Some(10),
            received_ce: Some(20),
            dsack_bytes: Some(100),
            dsack_pack: Some(2),
            total_tlp: Some(4),
            total_tlp_bytes: Some(5000),
            timer_rexmt_ms: Some(200),
            timer_persist_ms: Some(0),
            timer_keep_ms: Some(7200000),
            timer_2msl_ms: Some(0),
            timer_delack_ms: Some(40),
            idle_time_ms: Some(5000),
            options: Some(0x07),
            fd: Some(3),
            sources: vec![5, 6], // FREEBSD_KLD, KERN_FILE
            ..Default::default()
        };

        let proto = raw_to_proto(&raw);
        assert_eq!(proto.cc_algo, Some("cubic".to_string()));
        assert_eq!(proto.tcp_stack, Some("freebsd".to_string()));
        assert_eq!(proto.rtt_min_us, Some(1000));
        assert_eq!(proto.rexmit_packets, Some(5));
        assert_eq!(proto.ooo_packets, Some(2));
        assert_eq!(proto.zerowin_probes, Some(1));
        assert_eq!(proto.sack_blocks, Some(3));
        assert_eq!(proto.ecn_flags, Some(0x03));
        assert_eq!(proto.ecn_ce_delivered, Some(10));
        assert_eq!(proto.ecn_ce_received, Some(20));
        assert_eq!(proto.dsack_bytes, Some(100));
        assert_eq!(proto.dsack_packets, Some(2));
        assert_eq!(proto.tlp_probes_sent, Some(4));
        assert_eq!(proto.tlp_bytes_sent, Some(5000));
        assert_eq!(proto.timer_rexmt_ms, Some(200));
        assert_eq!(proto.timer_persist_ms, Some(0));
        assert_eq!(proto.timer_keep_ms, Some(7200000));
        assert_eq!(proto.timer_delack_ms, Some(40));
        assert_eq!(proto.idle_time_ms, Some(5000));
        assert_eq!(proto.negotiated_options, Some(0x07));
        assert_eq!(proto.fd, Some(3));
        assert_eq!(proto.sources, vec![5, 6]);
    }

    #[test]
    fn test_build_summary() {
        let records = vec![
            TcpSocketRecord {
                state: TcpState::Established.into(),
                ..Default::default()
            },
            TcpSocketRecord {
                state: TcpState::Established.into(),
                ..Default::default()
            },
            TcpSocketRecord {
                state: TcpState::TimeWait.into(),
                ..Default::default()
            },
        ];

        let summary = build_summary_from_records(&records, 1000);
        assert_eq!(summary.total_sockets, 3);
        assert_eq!(summary.interval_ms, 1000);

        let established_count = summary
            .state_counts
            .iter()
            .find(|b| b.state == TcpState::Established as i32)
            .map(|b| b.count);
        assert_eq!(established_count, Some(2));

        let tw_count = summary
            .state_counts
            .iter()
            .find(|b| b.state == TcpState::TimeWait as i32)
            .map(|b| b.count);
        assert_eq!(tw_count, Some(1));
    }

    #[test]
    fn test_build_summary_with_sys_stats() {
        let records = vec![TcpSocketRecord {
            state: TcpState::Established.into(),
            ..Default::default()
        }];

        let sys_stats = TcpSysStats {
            connattempt: 100,
            accepts: 50,
            connects: 80,
            drops: 2,
            sndtotal: 1000,
            sndbyte: 500000,
            sndrexmitpack: 10,
            sndrexmitbyte: 5000,
            rcvtotal: 900,
            rcvbyte: 400000,
            rcvduppack: 5,
            rcvbadsum: 0,
        };

        let summary = build_summary_with_sys_stats(&records, 1000, &sys_stats);
        assert_eq!(summary.delta_conn_attempts, Some(100));
        assert_eq!(summary.delta_snd_rexmit_packets, Some(10));
        assert!(summary.retransmit_rate.is_some());
        let rate = summary.retransmit_rate.unwrap();
        assert!((rate - 0.01).abs() < 0.001); // 10/1000 = 0.01
    }
}
