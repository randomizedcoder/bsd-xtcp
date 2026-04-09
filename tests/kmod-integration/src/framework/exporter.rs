use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Parsed Prometheus metrics snapshot.
pub struct MetricsSnapshot {
    /// Simple metrics: name -> value.
    pub simple: BTreeMap<String, f64>,
    /// Labeled metrics: name -> (label_value -> value).
    pub labeled: BTreeMap<String, BTreeMap<String, f64>>,
    /// When this snapshot was taken.
    pub timestamp: Instant,
}

/// Difference between two metric snapshots.
pub struct MetricsDiff {
    pub before: MetricsSnapshot,
    pub after: MetricsSnapshot,
    pub elapsed: Duration,
}

impl MetricsSnapshot {
    /// Compute the diff between self (before) and another (after) snapshot.
    pub fn diff(self, after: MetricsSnapshot) -> MetricsDiff {
        let elapsed = after.timestamp.duration_since(self.timestamp);
        MetricsDiff {
            before: self,
            after,
            elapsed,
        }
    }
}

/// Handle for a running tcpstats-exporter process.
pub struct ExporterHandle {
    child: Child,
    addr: String,
}

impl ExporterHandle {
    /// Spawn the exporter, wait for it to become ready (up to 5s).
    pub fn start(bin: &str, listen: &str) -> Result<Self, String> {
        let child = Command::new(bin)
            .args(["--listen", listen])
            .env("TCPSTATS_MAX_QUERY_RATE", "20")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn exporter: {e}"))?;

        let mut handle = Self {
            child,
            addr: listen.to_string(),
        };

        handle.wait_ready()?;
        Ok(handle)
    }

    /// Poll GET / until we get a 200 response (up to 5s, 200ms retries).
    fn wait_ready(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(5);

        while Instant::now() < deadline {
            if let Ok((200, _)) = http_get(&self.addr, "/") {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(200));
        }

        Err("exporter did not become ready within 5s".to_string())
    }

    /// Scrape /metrics, parse the Prometheus text body into a snapshot.
    /// Retries once on 429 (rate limited).
    pub fn scrape(&self) -> Result<MetricsSnapshot, String> {
        let do_scrape = || -> Result<MetricsSnapshot, String> {
            let (status, body) = http_get(&self.addr, "/metrics")?;
            if status == 429 {
                return Err("rate_limited".to_string());
            }
            if status != 200 {
                return Err(format!("scrape returned HTTP {status}"));
            }
            let timestamp = Instant::now();
            let (simple, labeled) = parse_prometheus(&body);
            Ok(MetricsSnapshot {
                simple,
                labeled,
                timestamp,
            })
        };

        match do_scrape() {
            Err(ref e) if e == "rate_limited" => {
                // Wait briefly and retry once
                thread::sleep(Duration::from_millis(100));
                do_scrape()
            }
            other => other,
        }
    }

    /// Print a human-readable diff between two metric snapshots.
    pub fn print_diff(label: &str, diff: &MetricsDiff) {
        println!(
            "    --- exporter: {} ({:.2}s) ---",
            label,
            diff.elapsed.as_secs_f64()
        );

        // sockets_total before/after
        let before_total = diff
            .before
            .simple
            .get("tcpstats_sockets_total")
            .copied()
            .unwrap_or(0.0);
        let after_total = diff
            .after
            .simple
            .get("tcpstats_sockets_total")
            .copied()
            .unwrap_or(0.0);
        let delta_total = after_total - before_total;
        println!(
            "    sockets_total: {} -> {} (delta {:+})",
            before_total as u64, after_total as u64, delta_total as i64
        );

        // Non-zero sys counter deltas (short names)
        let counter_names = [
            (
                "tcpstats_sys_connection_attempts_total",
                "connection_attempts_total",
            ),
            ("tcpstats_sys_accepts_total", "accepts_total"),
            ("tcpstats_sys_connects_total", "connects_total"),
            ("tcpstats_sys_drops_total", "drops_total"),
            ("tcpstats_sys_sent_packets_total", "sent_packets_total"),
            ("tcpstats_sys_sent_bytes_total", "sent_bytes_total"),
            (
                "tcpstats_sys_retransmit_packets_total",
                "retransmit_packets_total",
            ),
            (
                "tcpstats_sys_retransmit_bytes_total",
                "retransmit_bytes_total",
            ),
            (
                "tcpstats_sys_received_packets_total",
                "received_packets_total",
            ),
            ("tcpstats_sys_received_bytes_total", "received_bytes_total"),
            (
                "tcpstats_sys_duplicate_packets_total",
                "duplicate_packets_total",
            ),
            ("tcpstats_sys_bad_checksum_total", "bad_checksum_total"),
        ];

        for (full, short) in &counter_names {
            let before = diff.before.simple.get(*full).copied().unwrap_or(0.0);
            let after = diff.after.simple.get(*full).copied().unwrap_or(0.0);
            let delta = (after - before) as i64;
            if delta != 0 {
                println!("    {short}: {:+}", delta);
            }
        }

        // State breakdown from the after snapshot
        if let Some(states) = diff.after.labeled.get("tcpstats_sockets_by_state") {
            let parts: Vec<String> = states
                .iter()
                .filter(|(_, v)| **v > 0.0)
                .map(|(k, v)| format!("{k}={}", *v as u64))
                .collect();
            if !parts.is_empty() {
                println!("    states: {}", parts.join(", "));
            }
        }
    }
}

impl Drop for ExporterHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Minimal HTTP/1.1 GET request over a raw TCP connection.
/// Returns (status_code, body).
fn http_get(addr: &str, path: &str) -> Result<(u16, String), String> {
    let mut stream = TcpStream::connect(addr).map_err(|e| format!("connect {addr}: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("set_read_timeout: {e}"))?;

    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write: {e}"))?;

    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);

    let response = String::from_utf8_lossy(&buf);

    // Parse status code from first line: "HTTP/1.1 200 OK"
    let status_line = response
        .lines()
        .next()
        .ok_or_else(|| "empty response".to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("bad status line: {status_line}"))?;

    // Split headers from body at \r\n\r\n
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();

    Ok((status, body))
}

/// Parse Prometheus text exposition format into simple and labeled metric maps.
fn parse_prometheus(
    body: &str,
) -> (
    BTreeMap<String, f64>,
    BTreeMap<String, BTreeMap<String, f64>>,
) {
    let mut simple = BTreeMap::new();
    let mut labeled: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(brace_pos) = line.find('{') {
            // Labeled metric: name{label="value"} number
            let name = &line[..brace_pos];
            if let Some(close_pos) = line.find('}') {
                let label_part = &line[brace_pos + 1..close_pos];
                let value_str = line[close_pos + 1..].trim();

                // Extract label value from key="value"
                let label_value = label_part
                    .split_once('=')
                    .map(|(_, v)| v.trim_matches('"').to_string())
                    .unwrap_or_default();

                if let Ok(val) = value_str.parse::<f64>() {
                    labeled
                        .entry(name.to_string())
                        .or_default()
                        .insert(label_value, val);
                }
            }
        } else {
            // Simple metric: name value
            let mut parts = line.split_whitespace();
            if let (Some(name), Some(value_str)) = (parts.next(), parts.next()) {
                if let Ok(val) = value_str.parse::<f64>() {
                    simple.insert(name.to_string(), val);
                }
            }
        }
    }

    (simple, labeled)
}
