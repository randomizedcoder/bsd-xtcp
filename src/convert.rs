use crate::proto_gen::bsd_xtcp::{
    BatchMessage, CollectionMetadata, DataSource, IpVersion, Platform, StateBucket, SystemSummary,
    TcpSocketRecord, TcpState,
};
use crate::record::{IpAddr, RawSocketRecord};
use std::collections::BTreeMap;

/// Map macOS kernel TCPS_* state (0-10) to proto TcpState enum.
///
/// macOS values: CLOSED=0, LISTEN=1, SYN_SENT=2, SYN_RECEIVED=3, ESTABLISHED=4,
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
        rtt_us: raw.rtt_us,
        rttvar_us: raw.rttvar_us,
        rto_us: raw.rto_us,
        snd_nxt: raw.snd_nxt,
        snd_una: raw.snd_una,
        snd_max: raw.snd_max,
        rcv_nxt: raw.rcv_nxt,
        rcv_adv: raw.rcv_adv,
        snd_wscale: raw.snd_wscale,
        rcv_wscale: raw.rcv_wscale,
        dupacks: raw.dupacks,
        rxt_shift: raw.rxt_shift,
        snd_buf_used: raw.snd_buf_used,
        snd_buf_hiwat: raw.snd_buf_hiwat,
        rcv_buf_used: raw.rcv_buf_used,
        rcv_buf_hiwat: raw.rcv_buf_hiwat,
        pid: raw.pid,
        effective_pid: raw.effective_pid,
        uid: raw.uid,
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

    CollectionMetadata {
        timestamp_ns,
        hostname,
        platform: Platform::Macos.into(),
        os_version,
        interval_ms,
        schedule_name: String::new(),
        data_sources: vec![DataSource::MacosPcblistN.into()],
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
}
