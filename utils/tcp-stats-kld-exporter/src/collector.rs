use bsd_xtcp::sysctl::TcpSysStats;
use std::collections::BTreeMap;
use std::time::Instant;

/// Snapshot of TCP socket state and system-wide counters.
pub struct Snapshot {
    pub total_sockets: u64,
    pub state_counts: Vec<(String, u64)>,
    pub sys_stats: TcpSysStats,
    pub duration_secs: f64,
}

/// Map kernel TCP state (0-10) to human-readable name.
fn state_name(state: i32) -> &'static str {
    match state {
        0 => "CLOSED",
        1 => "LISTEN",
        2 => "SYN_SENT",
        3 => "SYN_RECEIVED",
        4 => "ESTABLISHED",
        5 => "CLOSE_WAIT",
        6 => "FIN_WAIT_1",
        7 => "CLOSING",
        8 => "LAST_ACK",
        9 => "FIN_WAIT_2",
        10 => "TIME_WAIT",
        _ => "UNKNOWN",
    }
}

/// Collect TCP socket stats and system-wide counters.
pub fn collect() -> Result<Snapshot, anyhow::Error> {
    let start = Instant::now();

    let result = bsd_xtcp::platform::collect_tcp_sockets()?;

    let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
    for rec in &result.records {
        let name = state_name(rec.state.unwrap_or(-1));
        *counts.entry(name).or_insert(0) += 1;
    }

    let state_counts: Vec<(String, u64)> = counts
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

    let sys_stats = bsd_xtcp::sysctl::read_tcp_stats()?;

    let duration_secs = start.elapsed().as_secs_f64();

    Ok(Snapshot {
        total_sockets: result.records.len() as u64,
        state_counts,
        sys_stats,
        duration_secs,
    })
}
