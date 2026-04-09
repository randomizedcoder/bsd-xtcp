use std::path::Path;

use anyhow::{bail, Result};

use crate::framework::check::read_count;
use crate::framework::exporter::ExporterHandle;
use crate::framework::process::ProcessGroup;
use crate::framework::system::sysctl_get_u64;

/// Validate sysctl counter invariants:
///   visited == emitted + sum(all skipped_*)
pub fn run_stats_validation(
    tcp_echo: &str,
    read_tcpstats: &str,
    exporter: Option<&ExporterHandle>,
    output_dir: Option<&Path>,
) -> Result<()> {
    // Set up some connections first
    let mut procs = ProcessGroup::new_in(output_dir);
    procs.start_server(tcp_echo, "127.0.0.1", "9090", 600)?;
    procs.start_clients(tcp_echo, "127.0.0.1", "9090", 50, 600)?;

    // Trigger a filtered read to populate counters
    let _ = read_count(read_tcpstats, "local_port=9090 exclude=listen")?;

    // Read all counters
    let visited = sysctl_get_u64("dev.tcpstats.sockets_visited")?;
    let emitted = sysctl_get_u64("dev.tcpstats.records_emitted")?;

    let skipped_names = &[
        "dev.tcpstats.sockets_skipped_gencnt",
        "dev.tcpstats.sockets_skipped_cred",
        "dev.tcpstats.sockets_skipped_ipver",
        "dev.tcpstats.sockets_skipped_state",
        "dev.tcpstats.sockets_skipped_port",
        "dev.tcpstats.sockets_skipped_addr",
    ];

    let mut total_skipped = 0u64;
    for name in skipped_names {
        let val = sysctl_get_u64(name)?;
        println!("  {name} = {val}");
        total_skipped += val;
    }

    println!("  visited={visited} emitted={emitted} skipped={total_skipped}");

    let sum = emitted + total_skipped;
    if visited != sum {
        bail!(
            "counter invariant violated: visited({visited}) != emitted({emitted}) + skipped({total_skipped}) = {sum}"
        );
    }

    // Also check reads_total and opens_total are non-zero
    let reads = sysctl_get_u64("dev.tcpstats.reads_total")?;
    let opens = sysctl_get_u64("dev.tcpstats.opens_total")?;
    println!("  reads_total={reads} opens_total={opens}");

    if reads == 0 {
        bail!("reads_total should be > 0 after a read");
    }
    if opens == 0 {
        bail!("opens_total should be > 0 after a read");
    }

    // Cross-validate exporter vs read_tcpstats
    if let Some(exp) = exporter {
        if let Ok(snap) = exp.scrape() {
            let exporter_total = snap
                .simple
                .get("tcpstats_sockets_total")
                .copied()
                .unwrap_or(0.0) as u64;
            let read_total = read_count(read_tcpstats, "")?;

            // Allow +-10% tolerance (sockets can change between reads)
            let lo = read_total.saturating_sub(read_total / 10);
            let hi = read_total + read_total / 10;
            if exporter_total >= lo && exporter_total <= hi {
                println!(
                    "  exporter cross-check: sockets_total={exporter_total} vs read_count={read_total} (OK)"
                );
            } else {
                println!(
                    "  exporter cross-check: sockets_total={exporter_total} vs read_count={read_total} (drift)"
                );
            }

            // Print state breakdown
            if let Some(states) = snap.labeled.get("tcpstats_sockets_by_state") {
                let mut parts: Vec<String> = Vec::new();
                for (k, v) in states {
                    if *v > 0.0 {
                        parts.push(format!("{k}={}", *v as u64));
                    }
                }
                if !parts.is_empty() {
                    println!("  exporter states: {}", parts.join(", "));
                }
            }
        }
    }

    procs.kill_all();
    Ok(())
}
