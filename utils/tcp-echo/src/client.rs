use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::cli::ClientConfig;
use crate::rate::RateLimiter;
use crate::shutdown;
use crate::stats::{ConnectionStats, StatsRegistry};

/// Global connection ID counter for the client.
static NEXT_CONN_ID: AtomicU32 = AtomicU32::new(0);

/// Generate a rotating byte pattern buffer of the given size.
fn make_payload(size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    for (i, byte) in buf.iter_mut().enumerate() {
        *byte = (i % 256) as u8;
    }
    buf
}

/// Run the client.
pub fn run(config: ClientConfig) -> Result<()> {
    let registry = Arc::new(StatsRegistry::new());
    let rate_limiter = Arc::new(RateLimiter::new(config.rate));
    let payload = Arc::new(make_payload(config.payload_size));

    eprintln!(
        "[client] target: {}:{{{}}}, connections: {}, rate: {} B/s, ramp: {}s",
        config.host,
        config
            .ports
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(","),
        config.connections,
        config.rate,
        config.ramp_duration.as_secs(),
    );

    // Start the rate limiter refill thread.
    let _refill_handle = rate_limiter.start_refill_thread();

    // Spawn reporting thread.
    let report_reg = Arc::clone(&registry);
    let report_interval = config.report_interval;
    let _report_handle = thread::Builder::new()
        .name("client-report".into())
        .spawn(move || {
            report_loop(&report_reg, report_interval);
        })
        .context("failed to spawn report thread")?;

    // Spawn duration watchdog if --duration > 0.
    if !config.duration.is_zero() {
        let dur = config.duration;
        thread::Builder::new()
            .name("duration-watchdog".into())
            .spawn(move || {
                let start = Instant::now();
                while !shutdown::is_shutdown() {
                    if start.elapsed() >= dur {
                        eprintln!(
                            "[client] duration {}s reached, shutting down",
                            dur.as_secs()
                        );
                        shutdown::request_shutdown();
                        break;
                    }
                    thread::sleep(Duration::from_millis(250));
                }
            })
            .context("failed to spawn duration watchdog")?;
    }

    // Ramp up connections.
    let ramp_interval = if config.connections > 1 && !config.ramp_duration.is_zero() {
        config.ramp_duration / config.connections
    } else {
        Duration::ZERO
    };

    let mut conn_handles: Vec<thread::JoinHandle<()>> = Vec::new();

    for i in 0..config.connections {
        if shutdown::is_shutdown() {
            break;
        }

        let port = config.ports[i as usize % config.ports.len()];
        let addr_str = if config.host.contains(':') {
            format!("[{}]:{}", config.host, port)
        } else {
            format!("{}:{}", config.host, port)
        };
        let addr: SocketAddr = addr_str
            .parse()
            .with_context(|| format!("invalid target address {}", addr_str))?;

        let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
        let limiter = Arc::clone(&rate_limiter);
        let reg = Arc::clone(&registry);
        let data = Arc::clone(&payload);

        match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            Ok(stream) => {
                let local_addr = stream.local_addr().unwrap_or(addr);
                let remote_addr = stream.peer_addr().unwrap_or(addr);
                eprintln!("[client] connection {conn_id} established to {remote_addr}");

                let stats = Arc::new(ConnectionStats::new(conn_id, local_addr, remote_addr));
                reg.register(Arc::clone(&stats));

                // Spawn writer thread.
                let write_stream = stream
                    .try_clone()
                    .context("failed to clone stream for writer")?;
                let write_stats = Arc::clone(&stats);
                let write_limiter = Arc::clone(&limiter);
                let write_data = Arc::clone(&data);
                let writer = thread::Builder::new()
                    .name(format!("writer-{conn_id}"))
                    .stack_size(65536)
                    .spawn(move || {
                        writer_loop(write_stream, &write_stats, &write_limiter, &write_data);
                    })
                    .context("failed to spawn writer thread")?;

                // Spawn reader thread.
                let read_stats = Arc::clone(&stats);
                let reader = thread::Builder::new()
                    .name(format!("reader-{conn_id}"))
                    .stack_size(65536)
                    .spawn(move || {
                        reader_loop(stream, &read_stats);
                    })
                    .context("failed to spawn reader thread")?;

                conn_handles.push(writer);
                conn_handles.push(reader);
            }
            Err(e) => {
                eprintln!("[client] failed to connect {conn_id} to {addr}: {e}");
            }
        }

        // Ramp delay between connections.
        if !ramp_interval.is_zero() && i + 1 < config.connections {
            // Sleep in small increments to check shutdown.
            let deadline = Instant::now() + ramp_interval;
            while Instant::now() < deadline && !shutdown::is_shutdown() {
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    if !shutdown::is_shutdown() {
        eprintln!("[client] all connections established, running...");
    }

    // Wait for shutdown.
    while !shutdown::is_shutdown() {
        thread::sleep(Duration::from_millis(250));
    }

    // Wait for connection threads to finish.
    for handle in conn_handles {
        let _ = handle.join();
    }

    registry.print_final_summary("client");
    eprintln!("[client] shutdown complete");

    Ok(())
}

/// Writer thread: acquire tokens from rate limiter, write payload data.
fn writer_loop(
    mut stream: TcpStream,
    stats: &ConnectionStats,
    limiter: &RateLimiter,
    payload: &[u8],
) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    while !shutdown::is_shutdown() {
        // Wait for tokens.
        if !limiter.try_acquire(payload.len() as u64) {
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        match stream.write_all(payload) {
            Ok(()) => {
                stats.add_written(payload.len() as u64);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                continue;
            }
            Err(e) => {
                if !shutdown::is_shutdown() {
                    eprintln!("[client] write error on connection {}: {e}", stats.id);
                }
                break;
            }
        }
    }

    // Shut down the write half so the server sees EOF.
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

/// Reader thread: drain echoed data from the server.
fn reader_loop(mut stream: TcpStream, stats: &ConnectionStats) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut buf = [0u8; 8192];

    while !shutdown::is_shutdown() {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                stats.add_read(n as u64);
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(e) => {
                if !shutdown::is_shutdown() {
                    eprintln!("[client] read error on connection {}: {e}", stats.id);
                }
                break;
            }
        }
    }
}

/// Periodic reporting loop.
fn report_loop(registry: &StatsRegistry, interval: Duration) {
    while !shutdown::is_shutdown() {
        thread::sleep(interval);
        if !shutdown::is_shutdown() {
            registry.print_report("client");
        }
    }
}
