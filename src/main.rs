use bsd_xtcp::proto_gen::bsd_xtcp::{
    BatchMessage, CollectionMetadata, DataSource, IpVersion, Platform, StateBucket, SystemSummary,
    TcpSocketRecord, TcpState,
};

fn main() {
    let metadata = CollectionMetadata {
        timestamp_ns: 1_700_000_000_000_000_000,
        hostname: "sample-host.local".into(),
        platform: Platform::Macos.into(),
        os_version: "macOS 15.2".into(),
        interval_ms: 1000,
        schedule_name: "fast".into(),
        data_sources: vec![DataSource::MacosPcblistN.into()],
        collection_duration_ns: 2_500_000,
        pcblist_generation: Some(42),
        batch_sequence: 1,
        tool_version: "bsd-xtcp 0.1.0".into(),
    };

    let established = TcpSocketRecord {
        local_addr: vec![127, 0, 0, 1],
        remote_addr: vec![93, 184, 216, 34],
        local_port: 52301,
        remote_port: 443,
        ip_version: IpVersion::IpVersion4.into(),
        state: TcpState::Established.into(),
        snd_cwnd: Some(65535),
        snd_ssthresh: Some(1048576),
        snd_wnd: Some(131072),
        rcv_wnd: Some(131072),
        maxseg: Some(1460),
        rtt_us: Some(15000),
        rttvar_us: Some(3000),
        rto_us: Some(200_000),
        snd_buf_used: Some(0),
        snd_buf_hiwat: Some(131072),
        rcv_buf_used: Some(4096),
        rcv_buf_hiwat: Some(131072),
        pid: Some(1234),
        uid: Some(501),
        command: Some("curl".into()),
        sources: vec![DataSource::MacosPcblistN.into()],
        ..Default::default()
    };

    let time_wait = TcpSocketRecord {
        local_addr: vec![127, 0, 0, 1],
        remote_addr: vec![10, 0, 0, 5],
        local_port: 48920,
        remote_port: 80,
        ip_version: IpVersion::IpVersion4.into(),
        state: TcpState::TimeWait.into(),
        timer_2msl_ms: Some(30000),
        snd_buf_used: Some(0),
        snd_buf_hiwat: Some(131072),
        rcv_buf_used: Some(0),
        rcv_buf_hiwat: Some(131072),
        pid: Some(5678),
        uid: Some(501),
        command: Some("firefox".into()),
        sources: vec![DataSource::MacosPcblistN.into()],
        ..Default::default()
    };

    let summary = SystemSummary {
        timestamp_ns: 1_700_000_000_000_000_000,
        interval_ms: 1000,
        total_sockets: 2,
        state_counts: vec![
            StateBucket {
                state: TcpState::Established.into(),
                count: 1,
            },
            StateBucket {
                state: TcpState::TimeWait.into(),
                count: 1,
            },
        ],
        ..Default::default()
    };

    let batch = BatchMessage {
        metadata: Some(metadata),
        records: vec![established, time_wait],
        summary: Some(summary),
    };

    let json = serde_json::to_string_pretty(&batch).expect("failed to serialize BatchMessage");
    println!("{json}");
}
