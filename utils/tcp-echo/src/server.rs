use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::cli::ServerConfig;
use crate::shutdown;
use crate::stats::{ConnectionStats, StatsRegistry};

/// Global connection ID counter for the server.
static NEXT_CONN_ID: AtomicU32 = AtomicU32::new(0);

/// Run the echo server.
pub fn run(config: ServerConfig) -> Result<()> {
    let registry = Arc::new(StatsRegistry::new());
    let bind_addrs = config.bind_addrs();

    eprintln!("[server] starting on {} port(s)", bind_addrs.len());

    // Spawn one listener thread per port.
    let mut listener_handles = Vec::new();
    for addr in &bind_addrs {
        let listener =
            TcpListener::bind(addr).with_context(|| format!("failed to bind to {addr}"))?;
        listener
            .set_nonblocking(true)
            .context("failed to set non-blocking")?;
        eprintln!("[server] listening on {addr}");

        let reg = Arc::clone(&registry);
        let bound_addr = *addr;
        let handle = thread::Builder::new()
            .name(format!("listener-{}", bound_addr.port()))
            .spawn(move || {
                accept_loop(listener, bound_addr, &reg);
            })
            .with_context(|| format!("failed to spawn listener thread for {addr}"))?;
        listener_handles.push(handle);
    }

    // Spawn reporting thread.
    let report_reg = Arc::clone(&registry);
    let report_interval = config.report_interval;
    let report_handle = thread::Builder::new()
        .name("server-report".into())
        .spawn(move || {
            report_loop(&report_reg, report_interval);
        })
        .context("failed to spawn report thread")?;

    // Wait for shutdown.
    for handle in listener_handles {
        let _ = handle.join();
    }
    let _ = report_handle.join();

    registry.print_final_summary("server");
    eprintln!("[server] shutdown complete");

    Ok(())
}

/// Accept loop for a single port. Uses non-blocking accept with 100ms sleep
/// to check the shutdown flag.
fn accept_loop(listener: TcpListener, addr: SocketAddr, registry: &Arc<StatsRegistry>) {
    let mut echo_handles: Vec<thread::JoinHandle<()>> = Vec::new();

    while !shutdown::is_shutdown() {
        match listener.accept() {
            Ok((stream, peer_addr)) => {
                let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
                let local_addr = stream.local_addr().unwrap_or(addr);
                eprintln!(
                    "[server] accepted connection {conn_id} from {peer_addr} on port {}",
                    addr.port()
                );

                let stats = Arc::new(ConnectionStats::new(conn_id, local_addr, peer_addr));
                registry.register(Arc::clone(&stats));

                let handle = thread::Builder::new()
                    .name(format!("echo-{conn_id}"))
                    .spawn(move || {
                        echo_loop(stream, &stats);
                    })
                    .expect("failed to spawn echo thread");
                echo_handles.push(handle);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                if !shutdown::is_shutdown() {
                    eprintln!("[server] accept error on port {}: {e}", addr.port());
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    // Wait for echo threads to finish.
    for handle in echo_handles {
        let _ = handle.join();
    }
}

/// Echo loop for a single connection: read data, write it back.
fn echo_loop(mut stream: TcpStream, stats: &ConnectionStats) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut buf = [0u8; 65536];

    while !shutdown::is_shutdown() {
        match stream.read(&mut buf) {
            Ok(0) => {
                // Connection closed by peer.
                eprintln!("[server] connection {} closed by peer", stats.id);
                break;
            }
            Ok(n) => {
                stats.add_read(n as u64);
                if let Err(e) = stream.write_all(&buf[..n]) {
                    if !shutdown::is_shutdown() {
                        eprintln!("[server] write error on connection {}: {e}", stats.id);
                    }
                    break;
                }
                stats.add_written(n as u64);
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(e) => {
                if !shutdown::is_shutdown() {
                    eprintln!("[server] read error on connection {}: {e}", stats.id);
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
            registry.print_report("server");
        }
    }
}
