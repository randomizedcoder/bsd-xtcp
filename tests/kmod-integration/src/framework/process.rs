use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use chrono::Local;

/// A group of background processes (server + clients) that are killed on drop.
pub struct ProcessGroup {
    children: Vec<(&'static str, Child)>,
    /// Ports used by this group (for post-kill socket drain).
    ports: Vec<u16>,
    /// Bind address used by this group.
    bind: Option<String>,
    /// Directory holding stderr log files for each spawned process.
    log_dir: PathBuf,
    /// Paths to stderr log files (label, path) for post-mortem inspection.
    stderr_logs: Vec<(String, PathBuf)>,
}

impl ProcessGroup {
    pub fn new() -> Self {
        Self::new_in(None)
    }

    /// Create a process group with stderr logs placed under `output_dir/processes/`
    /// if provided, otherwise under a temp directory.
    pub fn new_in(output_dir: Option<&Path>) -> Self {
        let log_dir = match output_dir {
            Some(dir) => dir.join("processes"),
            None => std::env::temp_dir().join(format!(
                "kmod-integration-{}",
                std::process::id()
            )),
        };
        if let Err(e) = fs::create_dir_all(&log_dir) {
            eprintln!("  warn: could not create log dir {}: {e}", log_dir.display());
        } else {
            eprintln!("  stderr logs: {}", log_dir.display());
        }
        Self {
            children: Vec::new(),
            ports: Vec::new(),
            bind: None,
            log_dir,
            stderr_logs: Vec::new(),
        }
    }

    /// Spawn a tcp-echo server.
    /// Returns after a brief settle delay.
    pub fn start_server(
        &mut self,
        tcp_echo: &str,
        bind: &str,
        ports: &str,
        report_interval: u32,
    ) -> Result<()> {
        let log_name = format!("server-{bind}-{ports}.stderr");
        let log_path = self.log_dir.join(&log_name);
        let stderr_file = File::create(&log_path)
            .with_context(|| format!("create stderr log {}", log_path.display()))?;

        let child = Command::new(tcp_echo)
            .args([
                "server",
                "--ports",
                ports,
                "--bind",
                bind,
                "--report-interval",
                &report_interval.to_string(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .with_context(|| format!("spawn server on {bind}:{ports}"))?;

        self.stderr_logs.push((log_name, log_path));

        self.children.push(("server", child));
        self.bind = Some(bind.to_string());
        for p in ports.split(',') {
            if let Ok(port) = p.trim().parse::<u16>() {
                self.ports.push(port);
            }
        }
        // Let the server bind
        thread::sleep(Duration::from_secs(1));
        Ok(())
    }

    /// Spawn tcp-echo clients.
    /// Returns after a brief settle delay for connections to establish.
    pub fn start_clients(
        &mut self,
        tcp_echo: &str,
        host: &str,
        ports: &str,
        connections: u32,
        report_interval: u32,
    ) -> Result<()> {
        let log_name = format!("client-{host}-{ports}.stderr");
        let log_path = self.log_dir.join(&log_name);
        let stderr_file = File::create(&log_path)
            .with_context(|| format!("create stderr log {}", log_path.display()))?;

        // Scale ramp duration with connection count so the client doesn't
        // try to open too many connections per second.
        let ramp = 1 + connections / 40;
        let child = Command::new(tcp_echo)
            .args([
                "client",
                "--host",
                host,
                "--ports",
                ports,
                "--connections",
                &connections.to_string(),
                "--ramp-duration",
                &ramp.to_string(),
                "--rate",
                "1024",
                "--duration",
                "0",
                "--report-interval",
                &report_interval.to_string(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .with_context(|| format!("spawn client to {host}:{ports}"))?;

        self.stderr_logs.push((log_name, log_path));

        self.children.push(("client", child));
        // Let connections establish — scale settle time with connection count.
        // FreeBSD 14.x needs more time for TCP handshakes than 15.x.
        let settle = u64::from(ramp) + 2;
        thread::sleep(Duration::from_secs(settle));
        Ok(())
    }

    /// Spawn tcp-echo clients with caller-controlled ramp and settle times.
    /// Use this for benchmarks where large connection counts need longer ramp-up.
    #[allow(clippy::too_many_arguments)]
    pub fn start_clients_with_ramp(
        &mut self,
        tcp_echo: &str,
        host: &str,
        ports: &str,
        connections: u32,
        report_interval: u32,
        ramp_secs: u32,
        settle_secs: u64,
    ) -> Result<()> {
        let log_name = format!("client-{host}-{ports}.stderr");
        let log_path = self.log_dir.join(&log_name);
        let stderr_file = File::create(&log_path)
            .with_context(|| format!("create stderr log {}", log_path.display()))?;

        let child = Command::new(tcp_echo)
            .args([
                "client",
                "--host",
                host,
                "--ports",
                ports,
                "--connections",
                &connections.to_string(),
                "--ramp-duration",
                &ramp_secs.to_string(),
                "--rate",
                "1024",
                "--duration",
                "0",
                "--report-interval",
                &report_interval.to_string(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .with_context(|| format!("spawn client to {host}:{ports}"))?;

        self.stderr_logs.push((log_name, log_path));

        self.children.push(("client", child));
        // Let connections establish
        thread::sleep(Duration::from_secs(settle_secs));
        Ok(())
    }

    /// Print the contents of any non-empty stderr log files.
    pub fn dump_stderr(&self) {
        for (label, path) in &self.stderr_logs {
            match fs::read_to_string(path) {
                Ok(contents) if !contents.is_empty() => {
                    eprintln!("=== stderr: {label} ===");
                    eprint!("{contents}");
                    if !contents.ends_with('\n') {
                        eprintln!();
                    }
                }
                _ => {}
            }
        }
    }

    /// Kill all processes and wait for sockets to drain.
    pub fn kill_all(&mut self) {
        for (label, child) in self.children.iter_mut().rev() {
            let pid = child.id();
            if let Err(e) = child.kill() {
                eprintln!("  warn: kill {label} (pid {pid}): {e}");
            }
            let _ = child.wait();
        }
        self.children.clear();

        self.dump_stderr();

        // Wait for LISTEN sockets on our ports to close (event-driven, not time-based).
        if let Some(ref bind) = self.bind {
            wait_for_port_release(bind, &self.ports);
        }
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.kill_all();
    }
}

/// Poll sockstat to wait until LISTEN sockets on the given ports are gone.
/// Times out after 5 seconds.
fn wait_for_port_release(_bind: &str, ports: &[u16]) {
    if ports.is_empty() {
        return;
    }

    let deadline = Instant::now() + Duration::from_secs(5);

    while Instant::now() < deadline {
        let mut all_clear = true;
        for &port in ports {
            // Use sockstat to check if any socket is LISTENing on this port
            if let Ok(output) = Command::new("sockstat")
                .args(["-l", "-p", &port.to_string()])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // sockstat header is always present; if >1 line, a socket exists
                if stdout.lines().count() > 1 {
                    all_clear = false;
                    break;
                }
            } else {
                // sockstat not available — fall back to brief sleep
                thread::sleep(Duration::from_millis(200));
                return;
            }
        }

        if all_clear {
            return;
        }

        thread::sleep(Duration::from_millis(50));
    }
}

/// Run a command and return its stdout as a string. Fails if exit code != 0.
pub fn run_cmd(cmd: &str, args: &[&str]) -> Result<String> {
    run_cmd_logged(cmd, args, None)
}

/// Run a command and return its stdout as a string. Fails if exit code != 0.
/// If `log_path` is provided, stdout and stderr are also written (with timestamps)
/// to that file.
pub fn run_cmd_logged(cmd: &str, args: &[&str], log_path: Option<&Path>) -> Result<String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("run {cmd}"))?;

    if let Some(path) = log_path {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let ts = Local::now().format("%Y-%m-%dT%H:%M:%S%.3f");
            let stdout_text = String::from_utf8_lossy(&output.stdout);
            let stderr_text = String::from_utf8_lossy(&output.stderr);
            let _ = writeln!(f, "[{ts}] cmd: {cmd} {}", args.join(" "));
            let _ = writeln!(f, "[{ts}] exit: {}", output.status);
            for line in stdout_text.lines() {
                let _ = writeln!(f, "[{ts}] out: {line}");
            }
            for line in stderr_text.lines() {
                let _ = writeln!(f, "[{ts}] err: {line}");
            }
        }
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{cmd} failed ({}): {stderr}", output.status);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run a command with a timeout, killing it after `timeout` duration.
/// Returns stdout as a string. Useful for DTrace scripts that run indefinitely.
pub fn run_cmd_with_timeout(cmd: &str, args: &[&str], timeout: Duration) -> Result<String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn {cmd}"))?;

    thread::sleep(timeout);

    // Kill the child — SIGKILL is fine, DTrace handles it gracefully
    let _ = child.kill();
    let output = child
        .wait_with_output()
        .with_context(|| format!("wait for {cmd}"))?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run a command, returning Ok(true) if it succeeded, Ok(false) if it failed.
pub fn run_cmd_ok(cmd: &str, args: &[&str]) -> Result<bool> {
    let status = Command::new(cmd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("run {cmd}"))?;

    Ok(status.success())
}
