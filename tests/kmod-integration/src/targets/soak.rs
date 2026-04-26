use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::Local;

use crate::framework::check::read_count;
use crate::framework::process::{run_cmd, ProcessGroup};
use crate::framework::system;

/// Configuration for a soak test run.
pub struct SoakConfig {
    pub tcp_echo: String,
    pub read_tcpstats: String,
    pub tcpstats_reader: String,
    pub kmod_src: String,
    pub duration_hours: u64,
    pub connections: u32,
}

/// Per-cycle health check result.
struct HealthCheck {
    all_alive: bool,
    dead_processes: Vec<String>,
    connection_count: u64,
    device_exists: bool,
}

/// Accumulated stats for an hour.
struct HourStats {
    hour: u64,
    cycles: u32,
    health_failures: Vec<String>,
    connection_counts: Vec<u64>,
    memory_values: Vec<Option<String>>,
}

/// Run the soak test with the given configuration.
pub fn run_soak(config: &SoakConfig, output_dir: Option<&Path>) -> Result<()> {
    let duration_hours = config.duration_hours;
    let connections = config.connections;

    // Determine cycle count: duration_hours=0 means quick-verify with 2 cycles
    let (total_hours, cycles_per_hour) = if duration_hours == 0 {
        (1u64, 2u32) // 1 "hour" with 2 cycles for quick test
    } else {
        (duration_hours, 12u32) // 12 cycles per hour (every 5 minutes)
    };

    let cycle_interval = if duration_hours == 0 {
        Duration::from_secs(10) // short interval for quick verify
    } else {
        Duration::from_secs(300) // 5 minutes
    };

    println!("=== live_soak ===");
    println!(
        "  duration: {}h ({} cycles)",
        duration_hours,
        total_hours * u64::from(cycles_per_hour)
    );
    println!("  connections: {connections}");
    println!("  cycle interval: {}s", cycle_interval.as_secs());

    // Create output directory
    let soak_dir = match output_dir {
        Some(d) => {
            let dir = d.join("live_soak");
            fs::create_dir_all(&dir)?;
            dir
        }
        None => {
            let dir = PathBuf::from("/tmp/kmod-integration-soak");
            fs::create_dir_all(&dir)?;
            dir
        }
    };

    println!("  output: {}", soak_dir.display());

    // Write config
    write_soak_config(&soak_dir, config, total_hours, cycles_per_hour)?;

    // Tune system for high connection counts
    system::tune_system()?;
    system::tune_tcp_timers()?;

    // Start tcp-echo server + client
    let mut procs = ProcessGroup::new_in(Some(&soak_dir));

    let port = "9090";
    let bind = "127.0.0.1";

    procs.start_server(&config.tcp_echo, bind, port, 600)?;

    // Use longer ramp for high connection counts
    let ramp_secs = std::cmp::max(5, connections / 40);
    let settle_secs = u64::from(ramp_secs) + 5;
    procs.start_clients_with_ramp(
        &config.tcp_echo,
        bind,
        port,
        connections,
        600,
        ramp_secs,
        settle_secs,
    )?;

    println!("  server + client started, connections ramping...");

    // Verify initial connection count
    let initial_count = read_count(&config.read_tcpstats, "local_port=9090")?;
    println!("  initial connection count: {initial_count}");

    // Main collection loop
    let mut all_health_failures: Vec<String> = Vec::new();
    let mut all_connection_counts: Vec<u64> = Vec::new();

    let soak_start = Instant::now();

    for hour in 0..total_hours {
        let hour_dir = soak_dir.join(format!("hour_{hour:03}"));
        fs::create_dir_all(&hour_dir)?;

        println!("  --- hour {hour}/{total_hours} ---");

        let mut hour_stats = HourStats {
            hour,
            cycles: 0,
            health_failures: Vec::new(),
            connection_counts: Vec::new(),
            memory_values: Vec::new(),
        };

        for cycle in 0..cycles_per_hour {
            let cycle_start = Instant::now();

            println!("    cycle {cycle:02}: collecting...");

            // Collect samples
            if let Err(e) = collect_tcp_stats_sample(&config.tcpstats_reader, &hour_dir, cycle) {
                eprintln!("    warn: tcp_stats collection failed: {e}");
            }

            if let Err(e) = collect_sysctl_counters(&hour_dir, cycle) {
                eprintln!("    warn: sysctl collection failed: {e}");
            }

            let mem_val = match collect_memory_stats(&hour_dir, cycle) {
                Ok(v) => Some(v),
                Err(e) => {
                    eprintln!("    warn: memory collection failed: {e}");
                    None
                }
            };

            // Health check
            let health = check_health(&mut procs, &config.read_tcpstats)?;

            if !health.all_alive {
                let msg = format!(
                    "hour={hour} cycle={cycle}: dead processes: {:?}",
                    health.dead_processes
                );
                eprintln!("    WARN: {msg}");
                hour_stats.health_failures.push(msg.clone());
                all_health_failures.push(msg);
            }

            // Connection count tolerance: 90-110% of expected
            let low = u64::from(connections) * 90 / 100;
            let high = u64::from(connections) * 110 / 100;
            if health.connection_count < low || health.connection_count > high {
                let msg = format!(
                    "hour={hour} cycle={cycle}: connection count {} outside [{low}, {high}]",
                    health.connection_count
                );
                eprintln!("    WARN: {msg}");
                hour_stats.health_failures.push(msg.clone());
                all_health_failures.push(msg);
            }

            // Device node check (first cycle of each hour)
            if cycle == 0 && !health.device_exists {
                let msg = format!("hour={hour}: /dev/tcpstats missing");
                eprintln!("    WARN: {msg}");
                hour_stats.health_failures.push(msg.clone());
                all_health_failures.push(msg);
            }

            hour_stats.cycles += 1;
            hour_stats.connection_counts.push(health.connection_count);
            hour_stats.memory_values.push(mem_val);
            all_connection_counts.push(health.connection_count);

            // Sleep until next cycle boundary (calculated, not fixed)
            let elapsed = cycle_start.elapsed();
            if elapsed < cycle_interval {
                let remaining = cycle_interval - elapsed;
                println!(
                    "    cycle {cycle:02}: done in {:.1}s, sleeping {:.1}s",
                    elapsed.as_secs_f64(),
                    remaining.as_secs_f64()
                );
                thread::sleep(remaining);
            } else {
                println!(
                    "    cycle {cycle:02}: done in {:.1}s (overran interval)",
                    elapsed.as_secs_f64()
                );
            }
        }

        // Write hour summary
        write_hour_summary(&hour_dir, &hour_stats)?;

        // Check memory trend (flag monotonically increasing)
        if hour_stats.memory_values.len() >= 3 {
            let vals: Vec<&str> = hour_stats
                .memory_values
                .iter()
                .filter_map(|v| v.as_deref())
                .collect();
            if is_monotonically_increasing(&vals) {
                let msg = format!(
                    "hour={hour}: M_TCPSTATS memory monotonically increasing — potential leak"
                );
                eprintln!("    WARN: {msg}");
                all_health_failures.push(msg);
            }
        }
    }

    let total_duration = soak_start.elapsed();
    println!(
        "  soak complete: {:.1}h elapsed",
        total_duration.as_secs_f64() / 3600.0
    );

    // Write final summary
    write_soak_summary(
        &soak_dir,
        total_hours,
        cycles_per_hour,
        &all_connection_counts,
        &all_health_failures,
        total_duration,
    )?;

    // Kill processes
    procs.kill_all();

    if all_health_failures.is_empty() {
        println!("  soak PASSED: no health failures");
    } else {
        println!(
            "  soak completed with {} health warnings (non-fatal)",
            all_health_failures.len()
        );
    }

    Ok(())
}

/// Collect a TCP stats sample using tcpstats-reader.
fn collect_tcp_stats_sample(tcpstats_reader: &str, hour_dir: &Path, cycle: u32) -> Result<()> {
    let output_file = hour_dir.join(format!("tcp_stats_{cycle:02}.json"));

    let output = run_cmd(tcpstats_reader, &["--count", "1"])?;

    let mut f = fs::File::create(&output_file)?;
    f.write_all(output.as_bytes())?;

    Ok(())
}

/// Collect sysctl dev.tcpstats.* counters.
fn collect_sysctl_counters(hour_dir: &Path, cycle: u32) -> Result<()> {
    let output_file = hour_dir.join(format!("sysctl_{cycle:02}.txt"));

    let pairs = system::sysctl_get_all_tcpstats()?;
    let ts = Local::now().format("%Y-%m-%dT%H:%M:%S%.3f");

    let mut f = fs::File::create(&output_file)?;
    writeln!(f, "# timestamp: {ts}")?;
    for (key, val) in &pairs {
        writeln!(f, "{key}: {val}")?;
    }

    Ok(())
}

/// Collect memory stats (kldstat + vmstat -m), filtering for M_TCPSTATS.
/// Returns the M_TCPSTATS memory value string if found.
fn collect_memory_stats(hour_dir: &Path, cycle: u32) -> Result<String> {
    let output_file = hour_dir.join(format!("memory_{cycle:02}.txt"));
    let ts = Local::now().format("%Y-%m-%dT%H:%M:%S%.3f");

    let mut f = fs::File::create(&output_file)?;
    writeln!(f, "# timestamp: {ts}")?;

    // kldstat
    writeln!(f, "\n# kldstat")?;
    match run_cmd("kldstat", &[]) {
        Ok(out) => writeln!(f, "{out}")?,
        Err(e) => writeln!(f, "# error: {e}")?,
    }

    // vmstat -m (all kernel malloc zones)
    writeln!(f, "\n# vmstat -m (M_TCPSTATS)")?;
    let vmstat_output = run_cmd("vmstat", &["-m"])?;

    let mut tcpstats_line = String::new();
    for line in vmstat_output.lines() {
        if line.contains("TCPSTATS") || line.contains("tcpstats") {
            writeln!(f, "{line}")?;
            tcpstats_line = line.to_string();
        }
    }

    // Also write header for context
    if let Some(header) = vmstat_output.lines().next() {
        if header.contains("Type") || header.contains("size") {
            writeln!(f, "# header: {header}")?;
        }
    }

    if tcpstats_line.is_empty() {
        writeln!(f, "# M_TCPSTATS zone not found in vmstat -m output")?;
    }

    Ok(tcpstats_line)
}

/// Check health: process liveness, connection count, device node.
fn check_health(procs: &mut ProcessGroup, read_tcpstats: &str) -> Result<HealthCheck> {
    // Process liveness
    let alive_status = procs.check_alive();
    let dead: Vec<String> = alive_status
        .iter()
        .filter(|(_, alive)| !alive)
        .map(|(label, _)| (*label).to_string())
        .collect();
    let all_alive = dead.is_empty();

    // Connection count
    let connection_count = read_count(read_tcpstats, "local_port=9090").unwrap_or(0);

    // Device node
    let device_exists = Path::new("/dev/tcpstats").exists();

    Ok(HealthCheck {
        all_alive,
        dead_processes: dead,
        connection_count,
        device_exists,
    })
}

/// Write config as JSON.
fn write_soak_config(
    soak_dir: &Path,
    config: &SoakConfig,
    total_hours: u64,
    cycles_per_hour: u32,
) -> Result<()> {
    let path = soak_dir.join("soak_config.json");
    let mut f = fs::File::create(&path)?;
    writeln!(f, "{{")?;
    writeln!(f, "  \"duration_hours\": {},", config.duration_hours)?;
    writeln!(f, "  \"effective_hours\": {total_hours},")?;
    writeln!(f, "  \"cycles_per_hour\": {cycles_per_hour},")?;
    writeln!(f, "  \"connections\": {},", config.connections)?;
    writeln!(f, "  \"tcp_echo\": {:?},", config.tcp_echo)?;
    writeln!(f, "  \"read_tcpstats\": {:?},", config.read_tcpstats)?;
    writeln!(f, "  \"tcpstats_reader\": {:?},", config.tcpstats_reader)?;
    writeln!(f, "  \"kmod_src\": {:?},", config.kmod_src)?;
    writeln!(f, "  \"started_at\": {:?}", Local::now().to_rfc3339())?;
    writeln!(f, "}}")?;
    Ok(())
}

/// Write per-hour summary JSON.
fn write_hour_summary(hour_dir: &Path, stats: &HourStats) -> Result<()> {
    let path = hour_dir.join("hour_summary.json");
    let mut f = fs::File::create(&path)?;

    let min_conn = stats.connection_counts.iter().copied().min().unwrap_or(0);
    let max_conn = stats.connection_counts.iter().copied().max().unwrap_or(0);
    let avg_conn = if stats.connection_counts.is_empty() {
        0
    } else {
        stats.connection_counts.iter().sum::<u64>() / stats.connection_counts.len() as u64
    };

    writeln!(f, "{{")?;
    writeln!(f, "  \"hour\": {},", stats.hour)?;
    writeln!(f, "  \"cycles\": {},", stats.cycles)?;
    writeln!(f, "  \"connection_count_min\": {min_conn},")?;
    writeln!(f, "  \"connection_count_max\": {max_conn},")?;
    writeln!(f, "  \"connection_count_avg\": {avg_conn},")?;
    writeln!(f, "  \"health_failures\": {},", stats.health_failures.len())?;
    writeln!(f, "  \"health_failure_details\": [")?;
    for (i, msg) in stats.health_failures.iter().enumerate() {
        let comma = if i + 1 < stats.health_failures.len() {
            ","
        } else {
            ""
        };
        writeln!(f, "    {msg:?}{comma}")?;
    }
    writeln!(f, "  ]")?;
    writeln!(f, "}}")?;

    Ok(())
}

/// Write final soak summary JSON.
fn write_soak_summary(
    soak_dir: &Path,
    total_hours: u64,
    cycles_per_hour: u32,
    all_connection_counts: &[u64],
    all_health_failures: &[String],
    duration: Duration,
) -> Result<()> {
    let path = soak_dir.join("soak_summary.json");
    let mut f = fs::File::create(&path)?;

    let min_conn = all_connection_counts.iter().copied().min().unwrap_or(0);
    let max_conn = all_connection_counts.iter().copied().max().unwrap_or(0);
    let avg_conn = if all_connection_counts.is_empty() {
        0
    } else {
        all_connection_counts.iter().sum::<u64>() / all_connection_counts.len() as u64
    };
    let total_cycles = all_connection_counts.len();

    writeln!(f, "{{")?;
    writeln!(f, "  \"total_hours\": {total_hours},")?;
    writeln!(f, "  \"cycles_per_hour\": {cycles_per_hour},")?;
    writeln!(f, "  \"total_cycles\": {total_cycles},")?;
    writeln!(f, "  \"duration_secs\": {:.1},", duration.as_secs_f64())?;
    writeln!(f, "  \"connection_count_min\": {min_conn},")?;
    writeln!(f, "  \"connection_count_max\": {max_conn},")?;
    writeln!(f, "  \"connection_count_avg\": {avg_conn},")?;
    writeln!(f, "  \"health_failures\": {},", all_health_failures.len())?;
    writeln!(f, "  \"health_failure_details\": [")?;
    for (i, msg) in all_health_failures.iter().enumerate() {
        let comma = if i + 1 < all_health_failures.len() {
            ","
        } else {
            ""
        };
        writeln!(f, "    {msg:?}{comma}")?;
    }
    writeln!(f, "  ],")?;
    writeln!(f, "  \"completed_at\": {:?}", Local::now().to_rfc3339())?;
    writeln!(f, "}}")?;

    Ok(())
}

/// Check if a series of memory values (as strings) is monotonically increasing.
/// Tries to extract numeric "InUse" or "MemUse" values from vmstat -m lines.
fn is_monotonically_increasing(vals: &[&str]) -> bool {
    let nums: Vec<u64> = vals
        .iter()
        .filter_map(|line| {
            // vmstat -m lines typically look like:
            //   Type           InUse  MemUse ...
            //   M_TCPSTATS       42   12345  ...
            // Try to parse the third whitespace-separated field as bytes
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() >= 3 {
                fields[2].parse::<u64>().ok()
            } else {
                None
            }
        })
        .collect();

    if nums.len() < 3 {
        return false;
    }

    nums.windows(2).all(|w| w[1] > w[0])
}
