# Integration Test Status

## Overview

The `tests/kmod-integration/` Rust crate is the single test harness for all FreeBSD kernel module testing. It replaces the previous shell scripts (`run-tests-freebsd.sh`, `freebsd-pkg-setup.sh`, `integration_tests.sh`) with a compiled binary that handles environment setup, compile-only targets, and live kernel module integration tests.

The Nix plumbing (`nix/freebsd-integration.nix`) orchestrates deployment to FreeBSD VMs via SSH, building the workspace on the VM, and dispatching to the `kmod-integration` binary with the selected target.

## Test pass status

**Not yet verified on VMs after the Nix refactor.** The Rust `kmod-integration` binary compiles cleanly (`cargo clippy --workspace` passes with zero warnings), and the Nix flake evaluates without errors, but the full suite has not been run on FreeBSD VMs since the shell scripts were removed and the Nix integration scripts were updated.

Previous test runs (before the refactor) passed all targets on both FreeBSD 14.3 and 15.0.

## Test inventory

### Compile-only targets (9)

Run on-VM without loading the kernel module. Validate that C test programs and the kmod itself compile correctly.

| Target | Description |
|--------|-------------|
| `unit` | 78 filter parser unit tests |
| `memcheck` | Valgrind memcheck (leak/error detection) |
| `asan` | AddressSanitizer + UBSan |
| `ubsan` | UndefinedBehaviorSanitizer |
| `bench` | Filter parser benchmark (1M iterations, 11 workloads) |
| `callgrind` | Callgrind CPU profiling |
| `kmod` | Build kernel module (`tcp_stats_kld.ko`) with `-Werror` |
| `bench_read` | Compile read-path microbenchmark |
| `gen_conn` | Compile loopback connection generator |

### Live targets (7)

Require root and a loaded kernel module. Test the full kmod read path end-to-end.

| Target | Description |
|--------|-------------|
| `live_smoke` | Kmod lifecycle: load, read `/dev/tcpstats`, verify sysctl tree, unload |
| `live_bench` | Read-path benchmark at 1K/10K/100K connections; exporter pre/post snapshots per scale |
| `live_stats` | Sysctl counter invariant: `visited == emitted + sum(skipped)`; exporter cross-validation |
| `live_dtrace` | DTrace SDT probe registration + firing (7 probes) |
| `live_dos` | DoS protections: EMFILE limit, read timeout partial results, EINTR signal; exporter diffs per sub-test |
| `live_integration` | 178 filter integration tests across 9 categories (see below) |
| `live_all` | All live targets sequentially; starts exporter after kmod reload, passes to bench/stats/dtrace/dos |

### Setup target

| Target | Description |
|--------|-------------|
| `pkg_setup` | Idempotent FreeBSD environment setup: bootstrap pkg, install kernel source, install valgrind + perl5 |

### Filter integration tests (live_integration): 178 tests in 9 categories

| Category | Module | Tests | What it validates |
|----------|--------|-------|-------------------|
| A | `port_filter.rs` | 12 | Local/remote port filtering, multi-port, edge cases |
| B | `state_filter.rs` | 8 | TCP state exclude/include (`exclude=listen`, `include_state=established`, etc.) |
| C | `ipversion_filter.rs` | 8 | `ipv4_only`, `ipv6_only`, dual-stack fixtures |
| D | `address_filter.rs` | 18 | IPv4/IPv6 CIDR address filtering (local/remote, /8, /16 ranges) |
| E | `combined_filter.rs` | 8 | Multi-filter combinations (port + state + IP version) |
| F | `format_fields.rs` | 3 | Field selection (`fields=identity,state`) and output format (`format=compact`) |
| G | `named_profiles.rs` | 2 | Named profile CRUD + ioctl cross-validation (conditional, skips if unsupported) |
| H | `concurrent_readers.rs` | 3 | 4-8 concurrent readers with same/different filters |
| I | `combinatorial_coverage.rs` | 69 | Exhaustive 2-way through 6-way filter dimension combinations |

Each test creates real TCP connections on loopback, applies a filter via ioctl, reads `/dev/tcpstats`, and asserts the expected record count.

## How to run

### Prerequisites

- FreeBSD VMs running and reachable via SSH (passwordless root access)
- Default hosts: `root@192.168.122.41` (FreeBSD 15.0), `root@192.168.122.27` (FreeBSD 14.3)
- Nix with flakes enabled on the Linux host

### From the Linux host (via Nix)

```sh
# Default: run live_integration (178 filter tests) on both VMs
nix run .#integration-test-freebsd

# Full live suite (smoke + bench + stats + dtrace + dos + integration) on both VMs
INTEGRATION_TARGET=live_all nix run .#integration-test-freebsd

# Compile-only targets on both VMs
INTEGRATION_TARGET=all nix run .#integration-test-freebsd

# Just environment setup on both VMs
INTEGRATION_TARGET=pkg_setup nix run .#integration-test-freebsd

# Target a single VM
INTEGRATION_TARGET=live_all nix run .#integration-test-freebsd150
INTEGRATION_TARGET=live_all nix run .#integration-test-freebsd143

# Filter integration tests by category
nix run .#integration-test-freebsd -- A         # category A only
nix run .#integration-test-freebsd -- A,B,D     # categories A, B, D

# Override SSH host
FREEBSD_HOST=root@10.0.0.5 nix run .#integration-test-freebsd150
```

### Directly on a FreeBSD VM

```sh
# Build the binary
cd /root/bsd-xtcp && cargo build --release --workspace

# Environment setup (idempotent)
./target/release/kmod-integration pkg_setup

# Compile-only targets
./target/release/kmod-integration all
./target/release/kmod-integration unit

# Load kmod, then run live targets
cd kmod/tcp_stats_kld && make clean all
kldload ./tcp_stats_kld.ko
sysctl dev.tcpstats.max_open_fds=64

# Live targets
./target/release/kmod-integration live_smoke --kmod-src kmod/tcp_stats_kld
./target/release/kmod-integration live_integration --category all \
    --tcp-echo ./target/release/tcp-echo --kmod-src kmod/tcp_stats_kld
./target/release/kmod-integration live_all \
    --tcp-echo ./target/release/tcp-echo --kmod-src kmod/tcp_stats_kld \
    --exporter ./target/release/tcp-stats-kld-exporter

# Unload when done
kldunload tcp_stats_kld
```

### Local C parser tests (Linux host, no VM needed)

```sh
nix run .#kmod-test-unit        # gcc, 78 unit tests
nix run .#kmod-test-memcheck    # valgrind memcheck
nix run .#kmod-test-asan        # AddressSanitizer + UBSan
nix run .#kmod-test-ubsan       # UBSan standalone
nix run .#kmod-test-bench       # benchmark
nix run .#kmod-test-callgrind   # callgrind profiling
nix run .#kmod-test-all         # all of the above
```

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `INTEGRATION_TARGET` | `live_integration` | Target to run (see targets table above) |
| `FREEBSD_HOST` | per-VM from `nix/constants.nix` | SSH target override |
| `FREEBSD_DIR` | `/root/bsd-xtcp` | Remote project directory |

## CLI flags (`kmod-integration`)

| Flag | Default | Description |
|------|---------|-------------|
| `--tcp-echo PATH` | `tcp-echo` | Path to tcp-echo binary |
| `--read-tcpstats PATH` | `read_tcpstats` | Path to read_tcpstats binary |
| `--kmod-src PATH` | `kmod/tcp_stats_kld` | Path to kmod source directory |
| `--cc PATH` | `cc` | C compiler |
| `--exporter PATH` | `tcp-stats-kld-exporter` | Path to tcp-stats-kld-exporter binary (optional, used by `live_all`) |
| `--category CAT` | `all` | Filter category for `live_integration` (e.g. `A`, `A,B,D`) |

## What the Nix integration script does

1. Ensures `rsync`, `cargo`, `protoc` are installed on the VM
2. Rsyncs the full project source (excluding `target/` and `.git/`)
3. Builds the full Rust workspace on the VM (`cargo build --release --workspace`)
4. Runs `kmod-integration pkg_setup` (idempotent env setup)
5. Builds the kernel module (`make clean all`)
6. For `live_*` targets: loads the kmod, raises fd limits
7. Runs `kmod-integration <target>` with `--tcp-echo`, `--kmod-src`, and `--exporter` flags
8. For `live_*` targets: unloads the kmod

## File structure

```
tests/kmod-integration/
├── Cargo.toml
└── src/
    ├── main.rs                          CLI, target dispatch
    ├── pkg_setup.rs                     Idempotent FreeBSD env setup
    ├── filter/
    │   ├── mod.rs                       collect_tests() orchestration
    │   ├── macros.rs                    simple_tests! and shared_fixture_tests! macros
    │   ├── port_filter.rs              Category A (12 tests)
    │   ├── state_filter.rs             Category B (8 tests)
    │   ├── ipversion_filter.rs         Category C (8 tests)
    │   ├── address_filter.rs           Category D (18 tests)
    │   ├── combined_filter.rs          Category E (8 tests)
    │   ├── format_fields.rs            Category F (3 tests)
    │   ├── named_profiles.rs           Category G (2 tests)
    │   ├── concurrent_readers.rs       Category H (3 tests)
    │   └── combinatorial_coverage.rs   Category I (69 tests)
    ├── framework/
    │   ├── mod.rs
    │   ├── check.rs                    Assertion helpers (open device, set filter, count records)
    │   ├── compile.rs                  C compilation helpers (gcc invocations)
    │   ├── exporter.rs                 Prometheus exporter lifecycle, scraping, metric diffs
    │   ├── loopback.rs                 Loopback alias management (IPv4 + IPv6)
    │   ├── process.rs                  Process execution helpers
    │   └── system.rs                   System setup helpers
    └── targets/
        ├── mod.rs
        ├── compile_tests.rs            Compile-only target implementations
        ├── dos_protection.rs           live_dos implementation
        ├── dtrace_probes.rs            live_dtrace implementation
        ├── kmod_lifecycle.rs           live_smoke implementation
        ├── read_bench.rs               live_bench implementation
        └── sysctl_counters.rs          live_stats implementation

nix/
├── freebsd-integration.nix             VM deployment + test orchestration
└── kmod-tests.nix                      Local C parser tests (Linux host)
```

## Exporter integration

The `tcp-stats-kld-exporter` Prometheus exporter is optionally integrated into `live_all` runs. The harness spawns it after kmod reload with `TCPSTATS_MAX_QUERY_RATE=20` for rapid test scraping, then passes an `ExporterHandle` to live targets.

**Behaviour by target:**

| Target | Exporter usage |
|--------|----------------|
| `live_smoke` | Not used (runs before exporter starts) |
| `live_bench` | Pre/post scrape around each scale (1K/10K/100K), prints socket count deltas and sys counter movement |
| `live_stats` | Cross-validates `tcpstats_sockets_total` against `read_count` (±10% tolerance), prints state breakdown |
| `live_dtrace` | Exporter running but not scraped (passive) |
| `live_dos` | Brackets timeout and EINTR sub-tests with pre/post snapshots, prints sys counter deltas |
| `live_integration` | No exporter param (filter tests are self-contained) |

**Design constraints:**
- No new Cargo dependencies — HTTP client uses raw `std::net::TcpStream`
- Exporter is optional — all live targets accept `Option<&ExporterHandle>`, pass `None` when run standalone
- Scrape failures are logged but never fail a test
- Metric diffs are informational output printed alongside existing benchmark numbers

## Changes in this refactor

- Removed `kmod-test-freebsd`, `kmod-test-freebsd150`, `kmod-test-freebsd143` Nix packages (VM deployment via old shell scripts)
- Removed 3 shell scripts (~1,850 lines): `integration_tests.sh`, `run-tests-freebsd.sh`, `freebsd-pkg-setup.sh`
- Updated `nix/freebsd-integration.nix` to support `INTEGRATION_TARGET` env var and conditional kmod load/unload
- `nix/kmod-tests.nix` now only exports local C compile test packages
- All VM testing goes through `integration-test-freebsd*` packages exclusively
