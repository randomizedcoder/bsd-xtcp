use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Per-connection statistics tracked atomically.
pub struct ConnectionStats {
    pub id: u32,
    pub local_addr: SocketAddr,
    pub remote_addr: SocketAddr,
    pub bytes_written: AtomicU64,
    pub bytes_read: AtomicU64,
    pub created_at: Instant,
}

impl ConnectionStats {
    pub fn new(id: u32, local_addr: SocketAddr, remote_addr: SocketAddr) -> Self {
        Self {
            id,
            local_addr,
            remote_addr,
            bytes_written: AtomicU64::new(0),
            bytes_read: AtomicU64::new(0),
            created_at: Instant::now(),
        }
    }

    pub fn add_written(&self, n: u64) {
        self.bytes_written.fetch_add(n, Ordering::Relaxed);
    }

    pub fn add_read(&self, n: u64) {
        self.bytes_read.fetch_add(n, Ordering::Relaxed);
    }

    pub fn written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }

    pub fn read(&self) -> u64 {
        self.bytes_read.load(Ordering::Relaxed)
    }

    pub fn age_secs(&self) -> f64 {
        self.created_at.elapsed().as_secs_f64()
    }

    /// The port from the remote address (server-side: client's port; client-side: server's port).
    pub fn port(&self) -> u16 {
        self.remote_addr.port()
    }
}

/// Registry of all active connections for reporting.
pub struct StatsRegistry {
    pub connections: Mutex<Vec<Arc<ConnectionStats>>>,
    pub started_at: Instant,
}

impl StatsRegistry {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(Vec::new()),
            started_at: Instant::now(),
        }
    }

    pub fn register(&self, stats: Arc<ConnectionStats>) {
        self.connections.lock().unwrap().push(stats);
    }

    /// Print a periodic report to stderr.
    pub fn print_report(&self, role: &str) {
        let conns = self.connections.lock().unwrap();
        let elapsed = self.started_at.elapsed().as_secs_f64();

        if conns.is_empty() {
            eprintln!("[{role}] no active connections (elapsed: {elapsed:.1}s)");
            return;
        }

        eprintln!(
            "[{role}] --- report ({} connections, elapsed: {elapsed:.1}s) ---",
            conns.len()
        );

        // Per-connection stats
        eprintln!(
            "  {:>4}  {:>21}  {:>21}  {:>12}  {:>12}  {:>8}",
            "ID", "LOCAL", "REMOTE", "WRITTEN", "READ", "AGE(s)"
        );
        for c in conns.iter() {
            eprintln!(
                "  {:>4}  {:>21}  {:>21}  {:>12}  {:>12}  {:>8.1}",
                c.id,
                c.local_addr,
                c.remote_addr,
                c.written(),
                c.read(),
                c.age_secs(),
            );
        }

        // Per-port summary
        let mut port_stats: std::collections::BTreeMap<u16, (u32, u64, u64)> =
            std::collections::BTreeMap::new();
        for c in conns.iter() {
            let entry = port_stats.entry(c.port()).or_insert((0, 0, 0));
            entry.0 += 1;
            entry.1 += c.written();
            entry.2 += c.read();
        }
        eprintln!(
            "\n  {:>6}  {:>6}  {:>12}  {:>12}",
            "PORT", "CONNS", "WRITTEN", "READ"
        );
        for (port, (count, written, read)) in &port_stats {
            eprintln!("  {:>6}  {:>6}  {:>12}  {:>12}", port, count, written, read);
        }

        // Grand totals
        let total_written: u64 = conns.iter().map(|c| c.written()).sum();
        let total_read: u64 = conns.iter().map(|c| c.read()).sum();
        let rate_written = if elapsed > 0.0 {
            total_written as f64 / elapsed
        } else {
            0.0
        };
        let rate_read = if elapsed > 0.0 {
            total_read as f64 / elapsed
        } else {
            0.0
        };

        eprintln!(
            "\n  totals: written={total_written} read={total_read} \
             write_rate={rate_written:.0} B/s read_rate={rate_read:.0} B/s"
        );
        eprintln!();
    }

    /// Print final summary on shutdown.
    pub fn print_final_summary(&self, role: &str) {
        eprintln!("[{role}] === final summary ===");
        self.print_report(role);
    }
}
