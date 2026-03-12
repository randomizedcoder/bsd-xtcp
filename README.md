# bsd-xtcp

System-wide TCP socket statistics exporter for FreeBSD and macOS.

## Overview

bsd-xtcp polls all TCP connections on the host and exports per-socket metrics — state, congestion window, RTT, retransmits, buffer utilization, process attribution — as structured data. On FreeBSD, a kernel module (`tcp_stats_kld`) exposes `/dev/tcpstats` with full `tcp_info`-equivalent data for every socket in a single kernel pass. On macOS, the `pcblist_n` sysctl provides comparable coverage without a kernel extension.

This is the BSD counterpart to [xtcp](https://github.com/randomizedcoder/xtcp) and [xtcp2](https://github.com/randomizedcoder/xtcp2), which use Linux Netlink.

Key properties:

- **Cross-platform:** unified protobuf schema with 78+ fields covering both macOS and FreeBSD; platform-specific fields are simply absent when not applicable
- **FreeBSD kernel module:** character device `/dev/tcpstats` providing system-wide per-socket TCP stats without requiring file descriptors
- **Low overhead:** targets < 1% CPU and < 10 MB RSS; read-path benchmarked at 275us for 1K connections, 4.4ms for 10K
- **Comprehensive testing:** 16 test targets, 178 filter integration tests, sanitizers, benchmarks, soak tests up to 10K connections
- **Rust implementation:** synchronous collection loop, protobuf via prost, Nix-based build system
- **CI-friendly:** pure parsing functions compile and test on Linux; sysctl/kmod calls are cfg-gated

## FreeBSD Kernel Module: tcp_stats_kld

### What it does

The kernel module creates `/dev/tcpstats`, a read-only character device that streams fixed-size 320-byte records containing per-socket TCP statistics for every connection on the system. Each record includes:

- RTT, RTO, rttvar, rttmin (microsecond precision)
- Congestion control: cwnd, ssthresh, algorithm name, TCP stack name
- Retransmissions, TLP, DSACK counters
- Timer state: rexmt, persist, keepalive, 2MSL, delack (milliseconds)
- ECN flags, delivered/received CE counters
- Window scaling, zero-window events, OOO packets, SACK blocks
- Process attribution via `kern.file` join (PID, FD, command name)

Named filter profiles allow selective socket matching by port, TCP state, IP version, and CIDR address ranges, with 9 individually gatable field groups.

### Security model

Five independent defense layers:

1. **Credential enforcement** — `cr_canseeinpcb()` for UID/GID/jail/MAC isolation
2. **File descriptor limit** — max 16 open fds (tunable via sysctl)
3. **Concurrent reader limit** — max 32 simultaneous readers (EBUSY when exceeded)
4. **Read iteration timeout** — 5s default, returns partial results (tunable via sysctl)
5. **Signal-interruptible reads** — checked every 256 sockets, returns EINTR

### Performance

Filter parser benchmarks (1M iterations per workload, 11 workloads):

- Parse throughput: 3.2–820.5 ns/call (all sub-microsecond)

Live read-path:

| Connections | Throughput | Latency |
|-------------|-----------|---------|
| 1K | 7.3M records/sec | 275us |
| 10K | 3M records/sec | 4.4ms |

Concurrent readers (16 threads): 9.0ms on FreeBSD 14.3, 4.2ms on FreeBSD 15.0.

DTrace SDT probes: 7 probes covering the read/filter/fill lifecycle, all verified at runtime.

## Filters

Using filters is recommended. On a busy host with thousands of sockets, reading every connection wastes kernel cycles and bus bandwidth on records you'll discard in userspace. The kernel module's filter parser runs entirely in-kernel at parse time and applies zero-copy match logic in the read path — benchmarked at 3.2–820 ns per call (sub-microsecond for all workloads). Filtering at the source means fewer records cross the kernel/user boundary and less work for downstream consumers.

Filters are space-separated directives, all AND'd together. Set them via sysctl (named profiles) or ioctl (programmatic). Up to 8 ports per direction, CIDR prefixes for addresses, and TCP state include/exclude lists.

### Examples

```sh
# HTTPS connections only — skip listeners and stale TIME_WAIT
sysctl dev.tcpstats.profiles.https="local_port=443 exclude=listen,timewait"

# Upstream origin fetches from a specific IPv4 subnet
sysctl dev.tcpstats.profiles.origin="remote_port=443,80 local_addr=10.0.1.0/24 exclude=listen,timewait"

# IPv6-only monitoring on a dual-stack edge proxy
sysctl dev.tcpstats.profiles.v6edge="ipv6_only local_port=443,8443 exclude=listen,timewait"

# Database connections — only the fields you need
sysctl dev.tcpstats.profiles.db="remote_port=5432,3306 fields=state,congestion,rtt,buffers"

# IPv6 CIDR filter for a /48 allocation
sysctl dev.tcpstats.profiles.v6net="local_addr=2001:db8:abcd::/48 exclude=listen"

# Read from a named profile
cat /dev/tcpstats/https
```

### Filter reference

| Directive | Syntax | Example |
|-----------|--------|---------|
| Local port | `local_port=PORT[,PORT,...]` | `local_port=443,8443` |
| Remote port | `remote_port=PORT[,PORT,...]` | `remote_port=80,443` |
| Local address (CIDR) | `local_addr=ADDR[/PREFIX]` | `local_addr=10.0.0.0/8` |
| Remote address (CIDR) | `remote_addr=ADDR[/PREFIX]` | `remote_addr=2001:db8::/32` |
| IPv4 only | `ipv4_only` | |
| IPv6 only | `ipv6_only` | |
| Exclude states | `exclude=STATE[,STATE,...]` | `exclude=listen,timewait` |
| Include states | `include_state=STATE[,STATE,...]` | `include_state=established` |
| Field groups | `fields=GROUP[,GROUP,...]` | `fields=identity,rtt,counters` |
| Output format | `format=compact` or `format=full` | `format=compact` |

Valid TCP states: `closed`, `listen`, `syn_sent`, `syn_received`, `established`, `close_wait`, `fin_wait_1`, `fin_wait_2`, `closing`, `last_ack`, `time_wait`.

Field groups: `identity`, `state`, `congestion`, `rtt`, `sequences`, `counters`, `timers`, `buffers`, `ecn`, `names`, `all`, `default` (identity + state + congestion + rtt + buffers).

Constraints: max 8 ports per direction, max 16 directives, max 512-byte filter string, CIDR host bits must be zero.

## Testing

### Test matrix (16 targets)

| Target | What it tests | FreeBSD 14.3 | FreeBSD 15.0 |
|--------|--------------|:------------:|:------------:|
| `unit` | 78 filter parser unit tests | PASS | PASS |
| `memcheck` | Valgrind leak/error detection | PASS | PASS |
| `asan` | AddressSanitizer + UBSan | PASS | PASS |
| `ubsan` | UndefinedBehaviorSanitizer | PASS | PASS |
| `bench` | 1M-iteration benchmarks (11 workloads) | PASS | PASS |
| `callgrind` | CPU profiling | PASS | PASS |
| `kmod` | Kernel module build (`-Werror`) | PASS | PASS |
| `bench_read` | Read-path microbenchmark (7 workloads) | PASS | PASS |
| `gen_conn` | Loopback connection generator | PASS | PASS |
| `live_smoke` | Load kmod, read `/dev/tcpstats`, verify sysctl, unload | PASS | PASS |
| `live_bench` | Benchmark at 1K/10K/100K connections | PASS | PASS |
| `live_stats` | Invariant validation: visited == emitted + skipped | PASS | PASS |
| `live_dtrace` | SDT probe registration + firing (7 probes) | PASS | PASS |
| `live_dos` | DoS protections: EMFILE, timeout, EINTR | PASS | PASS |
| `live_integration` | 178 filter tests across 9 categories | PASS | PASS |
| `live_soak` | Long-running stability with sustained connections | PASS | PASS |

Targets 1–9 are compile-only (run on the build host). Targets 10–16 require root and a loaded kernel module on a FreeBSD VM.

### Integration test harness

The `kmod-integration` crate is a Rust-based test harness that replaces the original shell scripts. It manages kernel module lifecycle, connection generators, and result collection.

178 filter integration tests across 9 categories:

| Category | Tests | Coverage |
|----------|------:|----------|
| A: Port filters | 12 | Source/dest port matching |
| B: State filters | 8 | TCP state include/exclude |
| C: IP version filters | 8 | IPv4/IPv6 selection |
| D: Address filters | 18 | CIDR matching (v4 + v6) |
| E: Combined filters | 8 | Multi-dimension filter combinations |
| F: Format/fields | 3 | Output format and field group gating |
| G: Named profiles | 2 | Filter profile management |
| H: Concurrent readers | 3 | Multi-reader correctness |
| I: Combinatorial | 69 | Exhaustive 2–6 way combinations |

### Soak testing

Long-running stability tests maintain N TCP connections via `tcp-echo` and collect stats from `/dev/tcpstats` every 5 minutes (one cycle = one collection). For high connection counts (>500), ramp-up uses adaptive batching — starting at 50 connections per batch, doubling after 3 consecutive successful batches (up to 2000), and halving on failure. This avoids overwhelming the kernel during connection setup.

| Test | Connections | Duration | Cycles | VMs | Result |
|------|------------|----------|--------|-----|--------|
| Quick verify | 50 | 2 cycles | 2 | 15.0 | PASSED |
| 1h validation | 1,000 | 1 hour | 12 | 15.0 | PASSED |
| 12h soak | 1,000 | 12 hours | 144 | 14.3, 15.0 | PASSED |
| 1h high-conn | 10,000 | 1 hour | 12 | 14.3, 15.0 | PASSED |
| **24h soak** | **10,000** | **24 hours** | **288** | **14.3, 15.0** | **PASSED** |

The 24h/10K test is the primary stability validation: 288 health check cycles per VM, ~8.3M records emitted per VM, zero kernel memory leaks (M_TCPSTATS `Use` and `Memory` held at 0 across all 576 combined samples), zero `uiomove_errors`, zero `reads_interrupted`, and `opens_total == reads_total` confirming no file descriptor leaks. One read timeout (out of 554) on each VM, handled gracefully with partial results.

Health checks each cycle: process liveness, connection count within +/-10%, device availability. Memory leak detection via `vmstat -m` trend analysis.

An early 24h/1K attempt failed due to FreeBSD's default `kern.threads.max_threads_per_proc=1500` limit — `tcp-echo` creates 2 threads per connection, hitting the cap at ~606 connections. Fixed by tuning the limit to 250,000 and replacing `.expect()` with proper error propagation.

Full results and per-cycle metrics: [tests/kmod-integration/SOAK_TEST_RESULTS.md](tests/kmod-integration/SOAK_TEST_RESULTS.md).

### VM-based CI

Nix-driven workflow: rsync source to FreeBSD VM, build on VM, run tests, fetch results. Tested on:

- FreeBSD 14.3-RELEASE (amd64)
- FreeBSD 15.0-RELEASE (amd64)

## Rust Client

The `bsd-xtcp` binary reads kernel data and outputs JSON Lines with 78+ fields per socket record.

**FreeBSD:** reads `/dev/tcpstats` for per-socket TCP stats, joins with `kern.file` sysctl for process attribution (PID, FD, command), collects system-wide counters from `net.inet.tcp.stats`.

**macOS:** reads `net.inet.tcp.pcblist_n` sysctl (tagged variable-length records with built-in RTT and PID data). No kernel extension required.

**Linux:** compiles and tests (parser logic, unit tests), but returns `UnsupportedPlatform` at runtime. Cross-compiled macOS binaries can be built on Linux.

## Quick Start

### Native build (macOS or Linux)

```sh
nix build .
./result/bin/bsd-xtcp --count 1 --pretty
```

### Cross-compile for macOS from Linux

No Xcode or macOS SDK required. Uses Nix + cargo-zigbuild + zig.

```sh
# Option 1: Makefile (simplest)
make cross-aarch64-darwin          # Apple Silicon (M1/M2/M3/M4)
make cross-x86_64-darwin           # Intel Mac
make cross-all                     # both targets

# Option 2: nix run (auto-names output directories)
nix run .#cross-aarch64-darwin     # -> result-cross-aarch64-darwin/bin/bsd-xtcp
nix run .#cross-x86_64-darwin      # -> result-cross-x86_64-darwin/bin/bsd-xtcp
nix run .#build-cross-all          # builds both with separate output dirs

# Option 3: nix build (manual output link)
nix build .#cross-aarch64-darwin -o result-cross-aarch64-darwin
nix build .#cross-all              # both binaries in result/bin/ named by target
```

### Deploy to a Mac

```sh
scp result-cross-aarch64-darwin/bin/bsd-xtcp user@mac:~/
ssh user@mac '~/bsd-xtcp --count 1 --pretty'
```

### FreeBSD integration tests

```sh
nix run .#integration-test-freebsd150    # FreeBSD 15.0 VM
nix run .#integration-test-freebsd143    # FreeBSD 14.3 VM
```

## Available Nix Targets

| Target | Description |
|--------|-------------|
| `default` / `bsd-xtcp` | Native build for current platform |
| `proto` | Standalone protobuf schema validation |
| `cross-x86_64-darwin` | Cross-compile for Intel Mac (Linux host only) |
| `cross-aarch64-darwin` | Cross-compile for Apple Silicon M1/M2/M3/M4 (Linux host only) |
| `cross-all` | All cross targets in one output, binaries named by target triple |
| `integration-test-freebsd150` | Run integration tests on FreeBSD 15.0 VM |
| `integration-test-freebsd143` | Run integration tests on FreeBSD 14.3 VM |

## Usage

```sh
# Single collection pass, pretty-printed
cargo run -- --count 1 --pretty

# 3 passes at 2-second intervals
cargo run -- --count 3 --interval 2

# Continuous collection (Ctrl-C to stop)
cargo run
```

| Flag | Default | Description |
|------|---------|-------------|
| `--interval SECS` | 1 | Collection interval in seconds |
| `--count N` | 0 (infinite) | Number of collection passes |
| `--pretty` | off | Pretty-print JSON output |
| `--help` | | Show usage |

## Status

| Phase | Status | Description |
|-------|--------|-------------|
| 1 - Build pipeline | Done | Nix flake + proto + prost-build + pbjson serde + cross-compilation via cargo-zigbuild |
| 2 - Sysctl reader | Done | `read_sysctl()`, `read_pcblist_validated()`, `read_clock_hz()` with retry + Linux stubs |
| 3 - macOS pcblist_n parser | Done | Cursor-based tagged record parser with `ConnectionAccumulator` |
| 4 - Record conversion | Done | `RawSocketRecord` intermediate type + proto conversion |
| 5 - JSON output | Done | `JsonSink` with `OutputSink` trait, JSON Lines + pretty-print |
| 6 - CLI + collection loop | Done | Hand-rolled `--interval`/`--count`/`--pretty` args, synchronous loop |
| 7-10 | Not started | Delta tracking, getsockopt enrichment, binary output, system summary enrichment |
| 11 - FreeBSD kernel module | Done | `tcp_stats_kld` with filters, DTrace, DoS protections, 16 test targets |
| 12 - FreeBSD Rust client | Done | KLD reader, `kern.file` join, 22 new fields, 24 tests |
| 13 - Integration test harness | Done | Rust-based `kmod-integration` replacing shell scripts, 178 filter tests |
| 14 - Soak testing | Done | 12h @ 1K connections, 1h @ 10K connections, both FreeBSD versions |
| 15 - Prometheus exporter | Done | `tcp-stats-kld-exporter` with rate/concurrency limiting |

Platform status documents:

- [docs/status/macos.md](docs/status/macos.md) — macOS pcblist_n parser, cross-compilation, phases 1–6
- [docs/status/freebsd.md](docs/status/freebsd.md) — FreeBSD tcp_stats_kld kernel module, filter parser, test suite
- [docs/status/integration-tests.md](docs/status/integration-tests.md) — Rust integration test harness, 178 filter tests, 16 targets
- [docs/status/freebsd-client.md](docs/status/freebsd-client.md) — FreeBSD Rust client, kern.file join, 22 new fields
- [docs/status/tcp-stats-kld-exporter.md](docs/status/tcp-stats-kld-exporter.md) — Prometheus exporter for tcp_stats_kld

## Design Documents

| # | Document | Description |
|---|----------|-------------|
| 1 | [FreeBSD Kernel Data Sources](docs/design/01-freebsd-data-sources.md) | Three kernel interfaces (`tcp.pcblist`, `getsockopt(TCP_INFO)`, `kern.file`) for socket enumeration, per-socket TCP state, and process-to-socket mapping |
| 2 | [Tool Architecture](docs/design/02-architecture.md) | Tiered polling architecture, data flow, record schemas, consistency model via generation counters |
| 3 | [Implementation Plan](docs/design/03-implementation.md) | Output formats (JSON Lines, CSV, protobuf), Rust module structure, six implementation phases |
| 4 | [macOS Portability](docs/design/04-macos-portability.md) | macOS differences: `TCP_CONNECTION_INFO`, `pcblist_n` tagged records, built-in RTT + PID attribution |
| 5 | [Kernel Module Design](docs/design/05-kernel-module.md) | `tcp_stats_kld` character device: `/dev/tcpstats`, 320-byte records, `cr_canseeinpcb()`, `uiomove()` streaming |
| 6 | [Field Comparison](docs/design/06-field-comparison.md) | Performance budget (< 1% CPU, < 10 MB RSS), field coverage matrix across Linux/FreeBSD/macOS, open questions |
| 7 | [Nix Build System](docs/design/07-nix-build-system.md) | Nix flake, `rustPlatform.buildRustPackage`, cross-compilation, security analysis toolkit |
| 8 | [Protobuf Schema](docs/design/08-protobuf-schema.md) | Unified `TcpSocketRecord` (78 fields), `BatchMessage`, configurable `interval_ms` replacing fixed tiers |
| 9 | [Filter Parsing](docs/design/09-filter-parsing.md) | EBNF grammar, in-kernel filter parser, CIDR matching, validation tables, 178 integration tests |
| 10 | [Performance & Security](docs/design/10-performance-security.md) | Hot-loop cost model for `tcpstats_read()`, adversarial resilience analysis, 13 hardening items |

## Project Structure

```
bsd-xtcp/
├── src/                    # Root crate — FreeBSD/macOS TCP stats reader library
├── proto/                  # Protobuf schema (tcp_stats.proto)
├── kmod/tcp_stats_kld/     # FreeBSD kernel module (C)
├── tests/kmod-integration/ # Rust integration test harness
├── utils/tcp-echo/         # TCP echo server/client for testing
├── nix/                    # Nix packaging, cross-compilation, VM deployment
├── docs/
│   ├── design/             # Design specifications (10 sections)
│   └── status/             # Implementation status documents (5 files)
└── archive/                # Historical working notes
```
