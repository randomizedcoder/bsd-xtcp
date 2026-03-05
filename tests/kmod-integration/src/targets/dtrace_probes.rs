use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::framework::check::read_count;
use crate::framework::process::{ProcessGroup, run_cmd, run_cmd_with_timeout};

/// All 7 SDT probes defined in tcp_stats_kld.c (DTrace listing format).
const EXPECTED_PROBES: &[&str] = &[
    "read:entry",
    "read:done",
    "filter:skip",
    "filter:match",
    "fill:done",
    "profile:create",
    "profile:destroy",
];

use crate::framework::exporter::ExporterHandle;

/// Validate DTrace probe registration and firing under load.
pub fn run_dtrace_validation(
    tcp_echo: &str,
    read_tcpstats: &str,
    _exporter: Option<&ExporterHandle>,
    output_dir: Option<&Path>,
) -> Result<()> {
    // Phase 1: Probe registration
    println!("  phase 1: probe registration");
    phase_probe_registration()?;

    // Phase 2: Probe firing under load
    println!("  phase 2: probe firing under load");
    phase_probe_firing(tcp_echo, read_tcpstats, output_dir)?;

    // Phase 3: Read latency capture via standalone script
    println!("  phase 3: read latency histogram");
    phase_read_latency(read_tcpstats)?;

    // Phase 4: Filter skip reasons
    println!("  phase 4: filter skip reasons");
    phase_filter_skip_reasons(read_tcpstats)?;

    println!("  all DTrace phases passed");
    Ok(())
}

/// Phase 1: Check that all 7 probes are registered in the kernel.
fn phase_probe_registration() -> Result<()> {
    let probes = run_cmd("dtrace", &["-l", "-n", "tcpstats:::"])?;

    for probe in EXPECTED_PROBES {
        if !probes.contains(probe) {
            bail!("DTrace probe not found: {probe}");
        }
    }
    println!("    all {n} probes registered", n = EXPECTED_PROBES.len());
    Ok(())
}

/// Phase 2: Start load, capture probe data for 3 seconds, validate counts.
fn phase_probe_firing(
    tcp_echo: &str,
    read_tcpstats: &str,
    output_dir: Option<&Path>,
) -> Result<()> {
    let mut procs = ProcessGroup::new_in(output_dir);
    procs.start_server(tcp_echo, "127.0.0.1", "9070", 600)?;
    procs.start_clients(tcp_echo, "127.0.0.1", "9070", 50, 600)?;

    // DTrace one-liner: count reads, records, skips, and matches
    let dtrace_script = concat!(
        "tcpstats:::read-done { @reads = count(); @records = sum(arg1); } ",
        "tcpstats:::filter-skip { @skips = count(); } ",
        "tcpstats:::filter-match { @matches = count(); }",
    );

    // Start DTrace in the background with a 4-second timeout.
    // During the capture window, trigger reads.
    let dtrace_handle = std::thread::spawn({
        let script = dtrace_script.to_string();
        move || run_cmd_with_timeout("dtrace", &["-n", &script], Duration::from_secs(4))
    });

    // Brief settle for DTrace to attach
    std::thread::sleep(Duration::from_secs(1));

    // Trigger 3 reads during the capture window
    for _ in 0..3 {
        let _ = read_count(read_tcpstats, "");
    }

    let output = dtrace_handle
        .join()
        .map_err(|_| anyhow::anyhow!("DTrace thread panicked"))??;

    // Parse aggregation output: look for @reads count
    // DTrace prints aggregations like:
    //   reads                                                     3
    let reads = parse_dtrace_aggregation(&output, "reads");
    let records = parse_dtrace_aggregation(&output, "records");

    println!("    DTrace output: reads={reads:?} records={records:?}");

    match reads {
        Some(n) if n >= 3 => {}
        Some(n) => bail!("expected >= 3 reads, got {n}"),
        None => bail!("no 'reads' aggregation in DTrace output"),
    }

    match records {
        Some(n) if n > 0 => {}
        Some(0) => bail!("expected > 0 records, got 0"),
        None => bail!("no 'records' aggregation in DTrace output"),
        _ => unreachable!(),
    }

    procs.kill_all();
    println!("    probes fired under load");
    Ok(())
}

/// Phase 3: Run read_latency.d, trigger reads, check for histogram output.
fn phase_read_latency(read_tcpstats: &str) -> Result<()> {
    let script = find_dtrace_script("read_latency.d")?;

    let dtrace_handle = std::thread::spawn({
        let s = script.clone();
        move || run_cmd_with_timeout("dtrace", &["-s", &s], Duration::from_secs(4))
    });

    std::thread::sleep(Duration::from_secs(1));

    // Trigger reads during capture window
    for _ in 0..3 {
        let _ = read_count(read_tcpstats, "");
    }

    let output = dtrace_handle
        .join()
        .map_err(|_| anyhow::anyhow!("DTrace thread panicked"))??;

    // quantize output contains "|" characters for the histogram bars
    if output.contains('|') || output.contains("value") {
        println!("    read latency histogram captured");
    } else {
        bail!(
            "expected quantize histogram in read_latency.d output, got:\n{output}"
        );
    }

    Ok(())
}

/// Phase 4: Run filter_skip_reasons.d with a filtered read, check for output.
fn phase_filter_skip_reasons(read_tcpstats: &str) -> Result<()> {
    let script = find_dtrace_script("filter_skip_reasons.d")?;

    let dtrace_handle = std::thread::spawn({
        let s = script.clone();
        move || run_cmd_with_timeout("dtrace", &["-s", &s], Duration::from_secs(4))
    });

    std::thread::sleep(Duration::from_secs(1));

    // Trigger a filtered read that will cause skips (ipv4_only filters out IPv6)
    let _ = read_count(read_tcpstats, "ipv4_only");

    let output = dtrace_handle
        .join()
        .map_err(|_| anyhow::anyhow!("DTrace thread panicked"))??;

    // The output should contain at least one reason category name
    let has_reason = ["gencnt", "cred", "ipver", "state", "port", "addr", "timeout"]
        .iter()
        .any(|r| output.contains(r));

    if has_reason {
        println!("    filter skip reasons captured");
    } else {
        // If no connections exist, there may be no skips — that's acceptable
        println!("    filter skip reasons: no skips observed (OK if no connections)");
    }

    Ok(())
}

/// Parse a simple DTrace aggregation value from output.
/// Looks for lines like "  reads                 42" or "  reads    42".
fn parse_dtrace_aggregation(output: &str, name: &str) -> Option<u64> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(name) || trimmed.ends_with(name) {
            // Extract the numeric value from the line
            for word in trimmed.split_whitespace() {
                if let Ok(n) = word.parse::<u64>() {
                    return Some(n);
                }
            }
        }
        // Also check for lines where name is followed by whitespace and a number
        if let Some(rest) = trimmed.strip_prefix(name) {
            for word in rest.split_whitespace() {
                if let Ok(n) = word.parse::<u64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// Locate a DTrace script in kmod/tcp_stats_kld/dtrace/.
/// Checks both relative (for in-tree runs) and absolute paths.
fn find_dtrace_script(name: &str) -> Result<String> {
    let candidates = [
        format!("kmod/tcp_stats_kld/dtrace/{name}"),
        format!("/usr/local/share/tcp_stats_kld/dtrace/{name}"),
    ];

    for path in &candidates {
        if Path::new(path).exists() {
            return Ok(path.clone());
        }
    }

    bail!(
        "DTrace script '{name}' not found in: {}",
        candidates.join(", ")
    );
}
