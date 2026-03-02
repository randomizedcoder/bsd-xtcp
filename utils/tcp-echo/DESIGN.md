# TCP Echo Utility -- Design Document

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [CLI Interface](#cli-interface)
   - [Server Subcommand](#server-subcommand)
   - [Client Subcommand](#client-subcommand)
4. [File Layout](#file-layout)
5. [Design Decisions](#design-decisions)
   - [Connection Management](#connection-management)
   - [Rate Limiting](#rate-limiting)
   - [Server Echo Loop](#server-echo-loop)
   - [Reporting](#reporting)
   - [Graceful Shutdown](#graceful-shutdown)
   - [Data Pattern](#data-pattern)
6. [Implementation Phases](#implementation-phases)
7. [Verification](#verification)
8. [Platform Notes](#platform-notes)

---

## Overview

`tcp-echo` is a standalone Rust binary with `server` and `client` subcommands for
generating known TCP connections with controlled traffic rates. It lets us verify
bsd-xtcp kernel socket stats against client-measured stats.

Same pattern as iperf3: single binary, two modes, shared infrastructure.

## Architecture

**Single binary with subcommands** (`tcp-echo server ...` / `tcp-echo client ...`).

**Synchronous, thread-per-connection** (no tokio). Up to ~100 connections fits
comfortably with OS threads. Matches the main bsd-xtcp crate's synchronous style.

**Dependencies:** `anyhow`, `thiserror`, `libc` only. Hand-rolled CLI parsing
matching `src/config.rs` style from the main crate.

## CLI Interface

### Server Subcommand

```
tcp-echo server [OPTIONS]
  --ports PORTS          Comma-separated ports to listen on (required, max 10)
  --bind ADDR            Bind address (default: 0.0.0.0)
  --report-interval SECS Reporting interval (default: 10)
  --help, -h             Show help
```

### Client Subcommand

```
tcp-echo client [OPTIONS]
  --host HOST              Target host (default: 127.0.0.1)
  --ports PORTS            Comma-separated server ports (required, max 10)
  --connections N          Total TCP connections to open (default: 10, max: 1000)
  --rate BYTES             Total bytes/sec across all connections (default: 1024)
  --ramp-duration SECS     Duration to ramp up connections (default: 10)
  --report-interval SECS   Stats reporting interval (default: 10)
  --duration SECS          Total runtime, 0 = infinite (default: 0)
  --payload-size BYTES     Size of each write call (default: 1024)
  --help, -h               Show help
```

Rate is total across all connections (not per-connection). The rate limiter divides
budget across active connections. This is easier to reason about when verifying
against kernel stats.

## File Layout

```
utils/tcp-echo/
  Cargo.toml
  DESIGN.md
  src/
    main.rs        -- Entry point: parse subcommand, dispatch
    cli.rs         -- ServerConfig, ClientConfig, parse_args()
    server.rs      -- Listen on ports, accept, spawn echo threads
    client.rs      -- Connect, ramp up, write data, collect stats
    stats.rs       -- ConnectionStats, StatsRegistry, periodic reporting
    rate.rs        -- Token-bucket rate limiter (AtomicU64-based)
    shutdown.rs    -- Ctrl+C / SIGTERM via AtomicBool + libc signals
```

## Design Decisions

### Connection Management

- **Round-robin port distribution**: connection `i` goes to `ports[i % ports.len()]`
- **Linear ramp-up**: one connection every `ramp_duration / connections` seconds
- **Per-connection**: two threads (writer with rate limiting + reader draining echo)
  via `TcpStream::try_clone()`

### Rate Limiting

- Shared token-bucket across all connections
- Refill thread adds tokens every 50ms
- Connection write threads acquire `payload_size` tokens atomically before each write
- `AtomicU64` for lock-free token acquisition

### Server Echo Loop

- One listener thread per port (non-blocking accept with 100ms poll for shutdown check)
- One echo thread per accepted connection: `read()` then `write_all()` in a loop

### Reporting

Both client and server share the same reporting pattern:

- Dedicated report thread, wakes every `--report-interval`
- Per-connection: ID, local addr, remote addr, port, bytes written, bytes read, age
- Per-port summary: connection count, total bytes
- Grand totals + actual measured rate vs target rate
- Output to stderr (so stdout stays clean for piping)

### Graceful Shutdown

- `AtomicBool` set by SIGINT/SIGTERM handler (via `libc::sigaction`)
- All threads check flag each loop iteration
- Listeners use non-blocking accept with poll loop
- Final summary report on exit

### Data Pattern

- Rotating byte pattern (0x00..0xFF), pre-generated into a buffer
- Deterministic, non-zero (catches corruption), no allocation per write

## Implementation Phases

### Phase 1: Scaffold + CLI + Shutdown
- Create directory structure and Cargo.toml
- Implement cli.rs (parse subcommands and all flags)
- Implement shutdown.rs (AtomicBool + signal handler)
- Implement main.rs (dispatch to server/client, print usage)

### Phase 2: Server Mode
- Implement server.rs (listener threads, echo threads)
- Basic stats.rs (server-side connection tracking)

### Phase 3: Client Connection Management
- Implement client.rs (connect with ramp-up, round-robin port distribution)
- Extend stats.rs (client-side ConnectionStats, StatsRegistry)

### Phase 4: Rate Limiter + Data Flow
- Implement rate.rs (token bucket)
- Add write/read thread pairs per connection in client.rs

### Phase 5: Reporting
- Add periodic reporting to both client and server
- Tabular output with per-connection, per-port, and total stats
- Final summary on shutdown

### Phase 6: Polish
- `--duration` auto-stop support
- Edge cases: all connections fail, port in use, zero rate

## Verification

1. `cd utils/tcp-echo && cargo build` -- compiles clean
2. `cargo clippy -- -D warnings` -- no warnings
3. Run server: `./target/debug/tcp-echo server --ports 9001,9002,9003`
4. Run client: `./target/debug/tcp-echo client --ports 9001,9002,9003 --connections 30 --rate 5000 --report-interval 5`
5. Verify client report shows ~5000 B/s actual rate
6. Verify connections distributed ~10 per port
7. Ctrl+C both sides, verify clean shutdown with final summary
8. Run bsd-xtcp alongside and compare socket 4-tuples and byte counts

## Platform Notes

- All APIs used are portable: `std::net`, `std::thread`, `std::sync`, `libc` signals
- No Linux-specific, macOS-specific, or FreeBSD-specific code
- Compiles and runs on Linux too (useful for development)
- `TcpStream::try_clone()` for read/write split is portable
