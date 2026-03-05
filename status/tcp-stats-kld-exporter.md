# tcp-stats-kld-exporter Status

## Overview

`tcp-stats-kld-exporter` is a Prometheus exporter for the `tcp_stats_kld` FreeBSD kernel module. It runs as a standalone HTTP daemon on the FreeBSD host, serving Prometheus text exposition format metrics at `/metrics`.

The exporter reads per-socket TCP state via `bsd_xtcp::platform::collect_tcp_sockets()` (which opens `/dev/tcpstats` on FreeBSD) and system-wide TCP counters via `bsd_xtcp::sysctl::read_tcp_stats()` (which reads `net.inet.tcp.stats` via sysctl). Each scrape performs a fresh collection.

## Build status

Compiles and passes clippy with zero warnings across the full workspace. Unit tests for the metrics renderer pass on Linux (they don't require a running kernel module).

Not yet verified running on a FreeBSD VM end-to-end. The Nix deployment scripts (`nix/exporter-deploy.nix`) are wired up but have not been exercised.

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bsd-xtcp` | workspace (path) | Socket collection and sysctl reading |
| `tiny_http` | 0.12 | HTTP server |
| `anyhow` | 1 | Error handling |

No async runtime. Single-threaded request loop.

## Architecture

```
main.rs         CLI parsing, HTTP request loop, rate/concurrency limiting
cli.rs          Config struct, --listen flag, TCPSTATS_MAX_QUERY_RATE env
collector.rs    collect() -> Snapshot (socket enumeration + sysctl read)
metrics.rs      render() -> Prometheus text format string
```

### Request flow

1. `tiny_http::Server` accepts a connection
2. Rate limiter checks elapsed time since last request against `1/max_query_rate`; returns 429 if too soon
3. Concurrency limiter checks `ACTIVE_REQUESTS` atomic counter against `max_concurrent`; returns 429 if at capacity
4. `GET /metrics` -> `collector::collect()` -> `metrics::render()` -> 200 response
5. `GET /` -> liveness page (200)
6. Everything else -> 404

### Rate limiting

Uses a `Mutex<Option<Instant>>` to track the last request timestamp. Requests arriving before `1/max_query_rate` seconds have elapsed get a 429 response. Default rate: 2 requests/second. Configurable via `TCPSTATS_MAX_QUERY_RATE` env var (the integration test harness sets this to 20).

### Concurrency limiting

An `AtomicU32` tracks active in-flight requests. If `active >= max_concurrent` (default 2), excess requests get a 429. This prevents multiple simultaneous `/dev/tcpstats` reads from overloading the kernel module.

## Exposed metrics

### Exporter self-metrics

| Metric | Type | Description |
|--------|------|-------------|
| `tcpstats_exporter_up` | gauge | Always 1 (liveness indicator) |
| `tcpstats_exporter_http_requests_total` | counter | Total HTTP requests handled |
| `tcpstats_exporter_collection_latency_seconds` | gauge | Most recent collection duration |

### Socket metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `tcpstats_sockets_total` | gauge | — | Total TCP sockets observed in latest collection |
| `tcpstats_sockets_by_state` | gauge | `state` | Socket count per TCP state (ESTABLISHED, LISTEN, TIME_WAIT, etc.) |

### System-wide TCP counters (from sysctl)

| Metric | Type | sysctl source |
|--------|------|---------------|
| `tcpstats_sys_connection_attempts_total` | counter | `tcps_connattempt` |
| `tcpstats_sys_accepts_total` | counter | `tcps_accepts` |
| `tcpstats_sys_connects_total` | counter | `tcps_connects` |
| `tcpstats_sys_drops_total` | counter | `tcps_drops` |
| `tcpstats_sys_sent_packets_total` | counter | `tcps_sndtotal` |
| `tcpstats_sys_sent_bytes_total` | counter | `tcps_sndbyte` |
| `tcpstats_sys_retransmit_packets_total` | counter | `tcps_sndrexmitpack` |
| `tcpstats_sys_retransmit_bytes_total` | counter | `tcps_sndrexmitbyte` |
| `tcpstats_sys_received_packets_total` | counter | `tcps_rcvtotal` |
| `tcpstats_sys_received_bytes_total` | counter | `tcps_rcvbyte` |
| `tcpstats_sys_duplicate_packets_total` | counter | `tcps_rcvduppack` |
| `tcpstats_sys_bad_checksum_total` | counter | `tcps_rcvbadsum` |

## CLI and configuration

```
Usage: tcp-stats-kld-exporter [OPTIONS]

Options:
  --listen ADDR:PORT   Listen address (default: 127.0.0.1:9814)
  --help, -h           Show this help message

Environment:
  TCPSTATS_MAX_QUERY_RATE   Max requests per second (default: 2.0)
```

Hardcoded defaults: `max_concurrent=2`, `max_query_rate=2.0`, `listen_addr=127.0.0.1:9814`.

## How to run

```sh
# On a FreeBSD host with tcp_stats_kld loaded
cargo build --release -p tcp-stats-kld-exporter
./target/release/tcp-stats-kld-exporter

# Override listen address
./target/release/tcp-stats-kld-exporter --listen 0.0.0.0:9814

# Raise rate limit for testing
TCPSTATS_MAX_QUERY_RATE=20 ./target/release/tcp-stats-kld-exporter

# Scrape
fetch -q -o - http://127.0.0.1:9814/metrics
```

## Nix packaging

### Local build (Linux/macOS — compiles but cannot collect on non-FreeBSD)

Defined in `nix/tcp-stats-kld-exporter-package.nix`. Uses `rustPlatform.buildRustPackage` with workspace-scoped `cargoBuildFlags = ["-p" "tcp-stats-kld-exporter"]`. Runs the unit tests during build.

### FreeBSD VM deployment

Defined in `nix/exporter-deploy.nix`. Provides per-VM scripts:

| Script | Description |
|--------|-------------|
| `exporter-build-<vm>` | Rsync + `cargo build --release -p tcp-stats-kld-exporter` |
| `exporter-lint-<vm>` | Rsync + `cargo clippy -p tcp-stats-kld-exporter -- -D warnings` |
| `exporter-test-<vm>` | Build, load kmod, start tcp-echo + exporter, scrape /metrics, verify output + rate limiting |
| `exporter-all-<vm>` | Build + lint + test sequentially |

## Integration with kmod-integration test harness

The `kmod-integration` binary accepts `--exporter PATH` and uses the exporter during `live_all` runs:

- Spawns the exporter after kmod reload with `TCPSTATS_MAX_QUERY_RATE=20`
- `live_bench`: pre/post scrape around each connection scale, prints socket count deltas and sys counter movement
- `live_stats`: cross-validates `tcpstats_sockets_total` against `read_count` with +-10% tolerance, prints state breakdown
- `live_dos`: brackets timeout and EINTR sub-tests with pre/post snapshots, prints sys counter deltas
- `live_dtrace`: exporter running passively (not scraped)

Scrape failures are logged but never fail a test. The integration uses raw `std::net::TcpStream` HTTP (no additional dependencies).

## File structure

```
utils/tcp-stats-kld-exporter/
├── Cargo.toml
└── src/
    ├── main.rs          HTTP server, request routing, rate/concurrency limiting
    ├── cli.rs           Config struct, arg parsing, env var override
    ├── collector.rs     Snapshot collection (socket enumeration + sysctl)
    └── metrics.rs       Prometheus text format renderer + unit tests

nix/
├── tcp-stats-kld-exporter-package.nix   Local Nix build
└── exporter-deploy.nix                  FreeBSD VM deploy/lint/test scripts
```

## Tests

| Location | What | Runs on |
|----------|------|---------|
| `metrics.rs` unit tests | Verify rendered output contains all expected metrics, HELP/TYPE annotations | Linux (cargo test) |
| `nix/exporter-deploy.nix` test script | End-to-end: start exporter, scrape, verify metrics present, verify rate limiting | FreeBSD VM |
| `kmod-integration` harness | Cross-validation of exporter vs read_tcpstats counts, metric diffs during benchmarks | FreeBSD VM |

## Known limitations

- Single-threaded: one request at a time (concurrency limit exists but `tiny_http` processes sequentially)
- No TLS or authentication — intended for localhost scraping or trusted networks
- No graceful shutdown signal handling — relies on SIGKILL from parent process or manual `pkill`
- `max_concurrent` and `max_query_rate` are not exposed as CLI flags (only env var for rate, hardcoded for concurrency)
- Collection reads all sockets unfiltered on every scrape — performance at very high socket counts (100K+) has not been characterized
