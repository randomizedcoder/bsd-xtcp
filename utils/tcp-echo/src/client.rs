use std::collections::VecDeque;
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

/// Result of a ramp phase (fixed or adaptive).
struct RampResult {
    _connected: u32,
    _failed: u32,
    handles: Vec<thread::JoinHandle<()>>,
}

/// Run the client.
pub fn run(config: ClientConfig) -> Result<()> {
    let registry = Arc::new(StatsRegistry::new());
    let rate_limiter = Arc::new(RateLimiter::new(config.rate));
    let payload = Arc::new(make_payload(config.payload_size));

    eprintln!(
        "[client] target: {}:{{{}}}, connections: {}, rate: {} B/s, ramp: {}",
        config.host,
        config
            .ports
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(","),
        config.connections,
        config.rate,
        if config.adaptive_ramp {
            format!("adaptive (batch={}..{})", config.ramp_batch, config.ramp_max_batch)
        } else {
            format!("{}s", config.ramp_duration.as_secs())
        },
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
    let ramp_result = if config.adaptive_ramp {
        adaptive_ramp(&config, &registry, &rate_limiter, &payload)?
    } else {
        fixed_ramp(&config, &registry, &rate_limiter, &payload)?
    };

    let mut conn_handles = ramp_result.handles;

    if !shutdown::is_shutdown() {
        eprintln!("[client] all connections established, running...");
    }

    // Wait for shutdown.
    while !shutdown::is_shutdown() {
        thread::sleep(Duration::from_millis(250));
    }

    // Wait for connection threads to finish.
    for handle in conn_handles.drain(..) {
        let _ = handle.join();
    }

    registry.print_final_summary("client");
    eprintln!("[client] shutdown complete");

    Ok(())
}

/// Resolve a target address for a given connection index.
fn resolve_addr(config: &ClientConfig, conn_index: u32) -> Result<SocketAddr> {
    let port = config.ports[conn_index as usize % config.ports.len()];
    let addr_str = if config.host.contains(':') {
        format!("[{}]:{}", config.host, port)
    } else {
        format!("{}:{}", config.host, port)
    };
    addr_str
        .parse()
        .with_context(|| format!("invalid target address {}", addr_str))
}

/// Attempt a single TCP connection and spawn reader/writer threads on success.
/// Returns Ok(true) if connected, Ok(false) if failed.
fn try_connect(
    addr: SocketAddr,
    registry: &Arc<StatsRegistry>,
    rate_limiter: &Arc<RateLimiter>,
    payload: &Arc<Vec<u8>>,
    handles: &mut Vec<thread::JoinHandle<()>>,
) -> Result<bool> {
    let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);

    match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
        Ok(stream) => {
            let local_addr = stream.local_addr().unwrap_or(addr);
            let remote_addr = stream.peer_addr().unwrap_or(addr);

            let stats = Arc::new(ConnectionStats::new(conn_id, local_addr, remote_addr));
            registry.register(Arc::clone(&stats));

            let write_stream = stream
                .try_clone()
                .context("failed to clone stream for writer")?;
            let write_stats = Arc::clone(&stats);
            let write_limiter = Arc::clone(rate_limiter);
            let write_data = Arc::clone(payload);
            let writer = thread::Builder::new()
                .name(format!("writer-{conn_id}"))
                .stack_size(65536)
                .spawn(move || {
                    writer_loop(write_stream, &write_stats, &write_limiter, &write_data);
                })
                .context("failed to spawn writer thread")?;

            let read_stats = Arc::clone(&stats);
            let reader = thread::Builder::new()
                .name(format!("reader-{conn_id}"))
                .stack_size(65536)
                .spawn(move || {
                    reader_loop(stream, &read_stats);
                })
                .context("failed to spawn reader thread")?;

            handles.push(writer);
            handles.push(reader);
            Ok(true)
        }
        Err(e) => {
            eprintln!("[client] failed to connect {conn_id} to {addr}: {e}");
            Ok(false)
        }
    }
}

/// Fixed-interval ramp: sleep(ramp_duration / connections) between each connect.
fn fixed_ramp(
    config: &ClientConfig,
    registry: &Arc<StatsRegistry>,
    rate_limiter: &Arc<RateLimiter>,
    payload: &Arc<Vec<u8>>,
) -> Result<RampResult> {
    let ramp_start = Instant::now();
    let ramp_interval = if config.connections > 1 && !config.ramp_duration.is_zero() {
        config.ramp_duration / config.connections
    } else {
        Duration::ZERO
    };

    let mut handles: Vec<thread::JoinHandle<()>> = Vec::new();
    let mut connected = 0u32;
    let mut failed = 0u32;

    for i in 0..config.connections {
        if shutdown::is_shutdown() {
            break;
        }

        let addr = resolve_addr(config, i)?;

        match try_connect(addr, registry, rate_limiter, payload, &mut handles)? {
            true => connected += 1,
            false => failed += 1,
        }

        // Ramp delay between connections.
        if !ramp_interval.is_zero() && i + 1 < config.connections {
            let deadline = Instant::now() + ramp_interval;
            while Instant::now() < deadline && !shutdown::is_shutdown() {
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    let elapsed = ramp_start.elapsed();
    eprintln!(
        "[client] RAMP_COMPLETE connected={connected} failed={failed} elapsed={:.1}s",
        elapsed.as_secs_f64()
    );

    Ok(RampResult {
        _connected: connected,
        _failed: failed,
        handles,
    })
}

/// Entry in the retry queue for adaptive ramp.
struct RetryEntry {
    addr: SocketAddr,
    attempts: u32,
    conn_index: u32,
}

/// Adaptive batch-based ramp with MISD (Multiplicative Increase, Slow Decrease) rate control.
fn adaptive_ramp(
    config: &ClientConfig,
    registry: &Arc<StatsRegistry>,
    rate_limiter: &Arc<RateLimiter>,
    payload: &Arc<Vec<u8>>,
) -> Result<RampResult> {
    let ramp_start = Instant::now();
    let min_batch: u32 = 10;
    let mut batch_size = config.ramp_batch;
    let mut consecutive_good: u32 = 0;
    let mut handles: Vec<thread::JoinHandle<()>> = Vec::new();
    let mut retry_queue: VecDeque<RetryEntry> = VecDeque::new();

    let mut total_connected = 0u32;
    let mut total_failed = 0u32;
    let mut next_conn_index = 0u32;
    let mut batch_num = 0u32;
    let total = config.connections;

    while next_conn_index < total || !retry_queue.is_empty() {
        if shutdown::is_shutdown() {
            break;
        }

        batch_num += 1;
        let current_batch = batch_size;

        // Mix retries into this batch: up to 25% of batch_size
        let max_retries_this_batch = current_batch / 4;
        let mut batch_ok = 0u32;
        let mut batch_attempted = 0u32;

        // Process retries first (up to limit)
        let retry_count = std::cmp::min(retry_queue.len() as u32, max_retries_this_batch);
        for _ in 0..retry_count {
            if shutdown::is_shutdown() {
                break;
            }
            if let Some(mut entry) = retry_queue.pop_front() {
                entry.attempts += 1;
                batch_attempted += 1;

                match try_connect(entry.addr, registry, rate_limiter, payload, &mut handles)? {
                    true => {
                        batch_ok += 1;
                        total_connected += 1;
                    }
                    false => {
                        if entry.attempts < config.ramp_max_retries {
                            retry_queue.push_back(entry);
                        } else {
                            total_failed += 1;
                            eprintln!(
                                "[client] permanently failed connection {} to {} after {} attempts",
                                entry.conn_index, entry.addr, entry.attempts
                            );
                        }
                    }
                }

                thread::sleep(Duration::from_millis(1));
            }
        }

        // New connections for the rest of the batch
        let new_count = current_batch.saturating_sub(retry_count);
        for _ in 0..new_count {
            if shutdown::is_shutdown() || next_conn_index >= total {
                break;
            }

            let conn_index = next_conn_index;
            next_conn_index += 1;
            batch_attempted += 1;

            let addr = resolve_addr(config, conn_index)?;

            match try_connect(addr, registry, rate_limiter, payload, &mut handles)? {
                true => {
                    batch_ok += 1;
                    total_connected += 1;
                }
                false => {
                    if config.ramp_max_retries > 0 {
                        retry_queue.push_back(RetryEntry {
                            addr,
                            attempts: 1,
                            conn_index,
                        });
                    } else {
                        total_failed += 1;
                    }
                }
            }

            thread::sleep(Duration::from_millis(1));
        }

        // Evaluate batch success rate
        let success_rate = if batch_attempted > 0 {
            batch_ok as f64 / batch_attempted as f64
        } else {
            1.0
        };
        let is_good = success_rate >= 0.95;

        // Adjust batch size
        let action;
        if is_good {
            consecutive_good += 1;
            if consecutive_good >= 3 {
                let new_size = std::cmp::min(batch_size.saturating_mul(2), config.ramp_max_batch);
                if new_size > batch_size {
                    batch_size = new_size;
                    action = "increased";
                } else {
                    action = "at_max";
                }
                consecutive_good = 0;
            } else {
                action = "ok";
            }
        } else {
            consecutive_good = 0;
            let reduced = batch_size * 3 / 4;
            batch_size = std::cmp::max(reduced, min_batch);
            action = "decreased";
        }

        let done = total_connected + total_failed;
        let pct = done as f64 / total as f64 * 100.0;
        let elapsed = ramp_start.elapsed();

        eprintln!(
            "[client] ramp batch {batch_num}: +{batch_ok}/{batch_attempted} ok ({:.1}%), \
             total {done}/{total} ({pct:.1}%), batch_size={batch_size} [{action}], \
             retries={}, elapsed={:.1}s",
            success_rate * 100.0,
            retry_queue.len(),
            elapsed.as_secs_f64(),
        );

        // Inter-batch pause
        if !shutdown::is_shutdown() {
            if is_good {
                thread::sleep(Duration::from_millis(100));
            } else {
                thread::sleep(Duration::from_millis(500));
            }
        }

        // If all new connections dispatched, only retries remain
        if next_conn_index >= total && retry_queue.is_empty() {
            break;
        }
    }

    // Final drain pass: attempt remaining retries at min_batch_size
    if !retry_queue.is_empty() && !shutdown::is_shutdown() {
        eprintln!(
            "[client] ramp drain: {} retries remaining",
            retry_queue.len()
        );
        while let Some(mut entry) = retry_queue.pop_front() {
            if shutdown::is_shutdown() {
                break;
            }
            entry.attempts += 1;
            match try_connect(entry.addr, registry, rate_limiter, payload, &mut handles)? {
                true => total_connected += 1,
                false => {
                    if entry.attempts < config.ramp_max_retries {
                        retry_queue.push_back(entry);
                    } else {
                        total_failed += 1;
                    }
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    let elapsed = ramp_start.elapsed();
    eprintln!(
        "[client] RAMP_COMPLETE connected={total_connected} failed={total_failed} elapsed={:.1}s",
        elapsed.as_secs_f64()
    );

    Ok(RampResult {
        _connected: total_connected,
        _failed: total_failed,
        handles,
    })
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
