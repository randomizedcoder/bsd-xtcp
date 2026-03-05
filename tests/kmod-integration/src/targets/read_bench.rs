use std::path::Path;
use std::time::Instant;

use anyhow::Result;

use crate::framework::check::read_count;
use crate::framework::exporter::ExporterHandle;
use crate::framework::process::ProcessGroup;
use crate::framework::system::tune_system;

/// Run read-path benchmark at various connection scales.
pub fn run_bench(
    tcp_echo: &str,
    read_tcpstats: &str,
    exporter: Option<&ExporterHandle>,
    output_dir: Option<&Path>,
) -> Result<()> {
    tune_system()?;

    for &scale in &[1_000, 10_000, 100_000] {
        println!("  bench: {scale} connections");

        let ramp_secs = (scale / 100).max(5);
        let settle_secs = match scale {
            s if s >= 100_000 => ramp_secs as u64 + 15,
            s if s >= 10_000 => ramp_secs as u64 + 8,
            _ => ramp_secs as u64 + 3,
        };

        let mut procs = ProcessGroup::new_in(output_dir);
        procs.start_server(tcp_echo, "127.0.0.1", "9090", 600)?;
        procs.start_clients_with_ramp(
            tcp_echo,
            "127.0.0.1",
            "9090",
            scale,
            600,
            ramp_secs,
            settle_secs,
        )?;

        let before = if let Some(e) = exporter {
            e.scrape().ok()
        } else {
            None
        };

        let start = Instant::now();
        let count = read_count(read_tcpstats, "local_port=9090")?;
        let elapsed = start.elapsed();

        println!(
            "    {scale} conns: read {count} records in {:.3}s",
            elapsed.as_secs_f64()
        );

        if let (Some(before), Some(e)) = (before, exporter) {
            if let Ok(after) = e.scrape() {
                let diff = before.diff(after);
                ExporterHandle::print_diff(&format!("bench {scale} conns"), &diff);
            }
        }

        procs.kill_all();
    }

    Ok(())
}
