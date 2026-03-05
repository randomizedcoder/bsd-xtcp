use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::Result;

use crate::framework::exporter::ExporterHandle;
use crate::framework::process::{ProcessGroup, run_cmd};
use crate::framework::system::{sysctl_set, tune_system};

/// Run DoS protection tests using test_dos_limits binary.
pub fn run_dos_tests(
    tcp_echo: &str,
    _kmod_src: &str,
    dos_bin: &str,
    exporter: Option<&ExporterHandle>,
    output_dir: Option<&Path>,
) -> Result<()> {
    // Sub-test 1: EMFILE (max_open_fds exhaustion)
    println!("  dos: EMFILE test");
    sysctl_set("dev.tcpstats.max_open_fds", "4")?;
    run_cmd(dos_bin, &["emfile"])?;
    sysctl_set("dev.tcpstats.max_open_fds", "64")?;

    // Sub-test 2: timeout (read duration limit)
    println!("  dos: timeout test");
    tune_system()?;

    let mut procs = ProcessGroup::new_in(output_dir);
    procs.start_server(tcp_echo, "127.0.0.1", "9090", 600)?;
    procs.start_clients(tcp_echo, "127.0.0.1", "9090", 100_000, 600)?;
    thread::sleep(Duration::from_secs(5));

    let before_timeout = if let Some(e) = exporter {
        e.scrape().ok()
    } else {
        None
    };

    sysctl_set("dev.tcpstats.max_read_duration_ms", "10")?;
    run_cmd(dos_bin, &["timeout", "100000"])?;
    sysctl_set("dev.tcpstats.max_read_duration_ms", "5000")?;

    if let (Some(before), Some(e)) = (before_timeout, exporter) {
        if let Ok(after) = e.scrape() {
            let diff = before.diff(after);
            ExporterHandle::print_diff("dos timeout 100K", &diff);
        }
    }

    procs.kill_all();

    // Sub-test 3: EINTR (signal interruption)
    println!("  dos: EINTR test");
    let mut procs = ProcessGroup::new_in(output_dir);
    procs.start_server(tcp_echo, "127.0.0.1", "9090", 600)?;
    procs.start_clients(tcp_echo, "127.0.0.1", "9090", 100_000, 600)?;
    thread::sleep(Duration::from_secs(5));

    let before_eintr = if let Some(e) = exporter {
        e.scrape().ok()
    } else {
        None
    };

    run_cmd(dos_bin, &["eintr", "100000"])?;

    if let (Some(before), Some(e)) = (before_eintr, exporter) {
        if let Ok(after) = e.scrape() {
            let diff = before.diff(after);
            ExporterHandle::print_diff("dos eintr 100K", &diff);
        }
    }

    procs.kill_all();

    Ok(())
}
