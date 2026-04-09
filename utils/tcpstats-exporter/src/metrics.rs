use crate::collector::Snapshot;

/// Render a Snapshot and exporter self-metrics as Prometheus text exposition format.
pub fn render(snapshot: &Snapshot, http_requests: u64, latency_secs: f64) -> String {
    let mut out = String::with_capacity(4096);

    // Exporter self-metrics
    out.push_str("# HELP tcpstats_exporter_up Exporter liveness indicator\n");
    out.push_str("# TYPE tcpstats_exporter_up gauge\n");
    out.push_str("tcpstats_exporter_up 1\n");
    out.push('\n');

    out.push_str("# HELP tcpstats_exporter_http_requests_total Total HTTP requests handled\n");
    out.push_str("# TYPE tcpstats_exporter_http_requests_total counter\n");
    push_metric_u64(
        &mut out,
        "tcpstats_exporter_http_requests_total",
        http_requests,
    );
    out.push('\n');

    out.push_str(
        "# HELP tcpstats_exporter_collection_latency_seconds Most recent collection latency\n",
    );
    out.push_str("# TYPE tcpstats_exporter_collection_latency_seconds gauge\n");
    push_metric_f64(
        &mut out,
        "tcpstats_exporter_collection_latency_seconds",
        latency_secs,
    );
    out.push('\n');

    // Socket counts
    out.push_str("# HELP tcpstats_sockets_total Total TCP sockets observed\n");
    out.push_str("# TYPE tcpstats_sockets_total gauge\n");
    push_metric_u64(&mut out, "tcpstats_sockets_total", snapshot.total_sockets);
    out.push('\n');

    out.push_str("# HELP tcpstats_sockets_by_state Count of TCP sockets per state\n");
    out.push_str("# TYPE tcpstats_sockets_by_state gauge\n");
    for (state, count) in &snapshot.state_counts {
        out.push_str(&format!(
            "tcpstats_sockets_by_state{{state=\"{}\"}} {}\n",
            state, count
        ));
    }
    out.push('\n');

    // System-wide sysctl counters
    let sys = &snapshot.sys_stats;
    push_counter_block(
        &mut out,
        "tcpstats_sys_connection_attempts_total",
        "Connection attempts (tcps_connattempt)",
        sys.connattempt,
    );
    push_counter_block(
        &mut out,
        "tcpstats_sys_accepts_total",
        "Connections accepted (tcps_accepts)",
        sys.accepts,
    );
    push_counter_block(
        &mut out,
        "tcpstats_sys_connects_total",
        "Connections established (tcps_connects)",
        sys.connects,
    );
    push_counter_block(
        &mut out,
        "tcpstats_sys_drops_total",
        "Connections dropped (tcps_drops)",
        sys.drops,
    );
    push_counter_block(
        &mut out,
        "tcpstats_sys_sent_packets_total",
        "Total packets sent (tcps_sndtotal)",
        sys.sndtotal,
    );
    push_counter_block(
        &mut out,
        "tcpstats_sys_sent_bytes_total",
        "Total bytes sent (tcps_sndbyte)",
        sys.sndbyte,
    );
    push_counter_block(
        &mut out,
        "tcpstats_sys_retransmit_packets_total",
        "Retransmitted packets (tcps_sndrexmitpack)",
        sys.sndrexmitpack,
    );
    push_counter_block(
        &mut out,
        "tcpstats_sys_retransmit_bytes_total",
        "Retransmitted bytes (tcps_sndrexmitbyte)",
        sys.sndrexmitbyte,
    );
    push_counter_block(
        &mut out,
        "tcpstats_sys_received_packets_total",
        "Total packets received (tcps_rcvtotal)",
        sys.rcvtotal,
    );
    push_counter_block(
        &mut out,
        "tcpstats_sys_received_bytes_total",
        "Total bytes received (tcps_rcvbyte)",
        sys.rcvbyte,
    );
    push_counter_block(
        &mut out,
        "tcpstats_sys_duplicate_packets_total",
        "Duplicate packets received (tcps_rcvduppack)",
        sys.rcvduppack,
    );
    push_counter_block(
        &mut out,
        "tcpstats_sys_bad_checksum_total",
        "Bad checksum packets (tcps_rcvbadsum)",
        sys.rcvbadsum,
    );

    out
}

fn push_metric_u64(out: &mut String, name: &str, val: u64) {
    out.push_str(&format!("{} {}\n", name, val));
}

fn push_metric_f64(out: &mut String, name: &str, val: f64) {
    out.push_str(&format!("{} {:.6}\n", name, val));
}

fn push_counter_block(out: &mut String, name: &str, help: &str, val: u64) {
    out.push_str(&format!("# HELP {} {}\n", name, help));
    out.push_str(&format!("# TYPE {} counter\n", name));
    push_metric_u64(out, name, val);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::Snapshot;
    use tcpstats_reader::sysctl::TcpSysStats;

    #[test]
    fn test_render_contains_expected_metrics() {
        let snapshot = Snapshot {
            total_sockets: 42,
            state_counts: vec![
                ("ESTABLISHED".to_string(), 30),
                ("TIME_WAIT".to_string(), 12),
            ],
            sys_stats: TcpSysStats {
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
            },
            duration_secs: 0.001,
        };

        let output = render(&snapshot, 7, 0.001);

        assert!(output.contains("tcpstats_exporter_up 1"));
        assert!(output.contains("tcpstats_exporter_http_requests_total 7"));
        assert!(output.contains("tcpstats_exporter_collection_latency_seconds"));
        assert!(output.contains("tcpstats_sockets_total 42"));
        assert!(output.contains("tcpstats_sockets_by_state{state=\"ESTABLISHED\"} 30"));
        assert!(output.contains("tcpstats_sockets_by_state{state=\"TIME_WAIT\"} 12"));
        assert!(output.contains("tcpstats_sys_connection_attempts_total 100"));
        assert!(output.contains("tcpstats_sys_accepts_total 50"));
        assert!(output.contains("tcpstats_sys_connects_total 80"));
        assert!(output.contains("tcpstats_sys_drops_total 2"));
        assert!(output.contains("tcpstats_sys_sent_packets_total 1000"));
        assert!(output.contains("tcpstats_sys_sent_bytes_total 500000"));
        assert!(output.contains("tcpstats_sys_retransmit_packets_total 10"));
        assert!(output.contains("tcpstats_sys_retransmit_bytes_total 5000"));
        assert!(output.contains("tcpstats_sys_received_packets_total 900"));
        assert!(output.contains("tcpstats_sys_received_bytes_total 400000"));
        assert!(output.contains("tcpstats_sys_duplicate_packets_total 5"));
        assert!(output.contains("tcpstats_sys_bad_checksum_total 0"));
    }

    #[test]
    fn test_render_has_help_and_type() {
        let snapshot = Snapshot {
            total_sockets: 0,
            state_counts: vec![],
            sys_stats: TcpSysStats::default(),
            duration_secs: 0.0,
        };

        let output = render(&snapshot, 0, 0.0);

        // Every metric should have HELP and TYPE
        assert!(output.contains("# HELP tcpstats_exporter_up"));
        assert!(output.contains("# TYPE tcpstats_exporter_up gauge"));
        assert!(output.contains("# HELP tcpstats_sockets_total"));
        assert!(output.contains("# TYPE tcpstats_sockets_total gauge"));
        assert!(output.contains("# HELP tcpstats_sys_connection_attempts_total"));
        assert!(output.contains("# TYPE tcpstats_sys_connection_attempts_total counter"));
    }
}
