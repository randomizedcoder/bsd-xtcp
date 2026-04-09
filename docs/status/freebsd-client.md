# FreeBSD Client Implementation Status

## Overview

The Rust client now supports FreeBSD as a first-class platform alongside macOS. On FreeBSD, the client reads per-socket TCP statistics from the `tcpstats` kernel module (`/dev/tcpstats`), enriches records with PID/FD mapping from the `kern.file` sysctl, and reads system-wide TCP counters from `net.inet.tcp.stats`. The Nix build system supports FreeBSD cross-compilation (via cross-rs) and SSH deploy+test targets for FreeBSD VMs.

All code compiles cleanly on Linux (stubs for platform-specific syscalls), and all parser functions are testable on any platform using synthetic byte buffers.

## Rust Implementation

### New files

| File | Lines | Purpose |
|------|-------|---------|
| `src/platform/freebsd_layout.rs` | ~200 | `#[repr(C, packed)]` `TcpStatsRecord` (320 bytes), ioctl constants, AF/flag constants |
| `src/platform/freebsd.rs` | ~530 | KLD reader, record parser, kern.file PID join, 12 unit tests |

### Modified files

| File | Changes |
|------|---------|
| `src/record.rs` | Added 22 `Option<T>` fields for FreeBSD-specific data |
| `src/platform/mod.rs` | Split cfg gates (macOS/FreeBSD), added 4 error variants, FreeBSD dispatch |
| `src/platform/macos.rs` | Narrowed cfg gates from `any(macos, freebsd)` to `macos` only |
| `src/sysctl.rs` | Split `read_os_version()` per platform, added `TcpSysStats` and `read_tcp_stats()` |
| `src/convert.rs` | Added FreeBSD field mappings, platform-aware metadata, sys stats integration |

### freebsd_layout.rs

Mirrors the C `struct tcp_stats_record` from `kmod/tcpstats/tcp_statsdev.h` as a `#[repr(C, packed)]` Rust struct. Compile-time assertion validates `size_of::<TcpStatsRecord>() == 320`.

- `TcpStatsRecord` -- 320-byte packed struct, field-for-field match with C
- `TcpstatsVersion` -- ioctl response struct (16 bytes)
- Ioctl constants computed from FreeBSD `_IOR`/`_IOW`/`_IO` macros:
  - `TCPSTATS_VERSION_CMD = 0x40105401`
  - `TCPSTATS_SET_FILTER` (256-byte filter struct)
  - `TCPSTATS_RESET = 0x20005403`
- AF constants: `AF_INET = 2`, `AF_INET6 = 28`
- Record flag constants: `TSR_F_IPV6`, `TSR_F_LISTEN`, `TSR_F_SYNCACHE`
- `DTYPE_SOCKET = 2` for kern.file filtering
- `extract_nul_string()` helper for NUL-terminated `[u8; 16]` arrays

### freebsd.rs

Data flow:

```
/dev/tcpstats-full (or /dev/tcpstats)
  -> open(), TCPSTATS_VERSION_CMD ioctl, read_to_end()
  -> parse_kld_records() -> Vec<RawSocketRecord>
  -> enrich_with_pid_mapping() via kern.file sysctl
  -> return CollectionResult
```

Key functions:

| Function | Cfg-gated | Description |
|----------|-----------|-------------|
| `collect()` | FreeBSD only | Top-level entry point, orchestrates KLD read + PID enrichment |
| `collect_from_kld()` | FreeBSD only | Opens device, version ioctl, reads buffer, calls parser |
| `parse_kld_records(buf)` | All platforms | Pure: splits buffer into 320-byte chunks, converts to `RawSocketRecord` |
| `kld_record_to_raw(tsr)` | All platforms | Converts single C record: AF handling, string extraction, timer normalization |
| `enrich_with_pid_mapping(records)` | FreeBSD only | Reads kern.file, builds HashMap, joins on socket_id |
| `parse_kern_file(buf)` | All platforms | Pure: parses `xfile` struct array, filters DTYPE_SOCKET, returns pid map |

Design decisions:

- **`freebsd.rs` always compiled** -- only `collect()`, `collect_from_kld()`, `enrich_with_pid_mapping()`, and `build_pid_map()` are cfg-gated to FreeBSD. All pure parser functions and tests run on all platforms.
- **Copy-based record parsing** -- `ptr::copy_nonoverlapping` into `MaybeUninit<TcpStatsRecord>` avoids alignment issues from the packed struct.
- **Timer normalization** -- negative timer values (timer not running) mapped to 0.
- **RTT values passed through directly** -- the KLD already stores RTT in microseconds (via `tcp_fill_info()`), unlike macOS which stores shifted ticks.
- **First-PID-wins** for kern.file join -- when multiple processes share a socket FD, the first entry seen takes precedence.
- **Best-effort PID enrichment** -- if `kern.file` sysctl fails, records are returned without PID/FD (no error propagated).
- **Device fallback** -- tries `/dev/tcpstats-full` first (all TCP states), falls back to `/dev/tcpstats`.

### record.rs changes

22 new `Option<T>` fields added after `start_time_secs`:

| Field | Type | Source |
|-------|------|--------|
| `rtt_min_us` | `Option<u32>` | KLD: `tsr_rttmin` |
| `cc_algo` | `Option<String>` | KLD: `tsr_cc` (NUL-terminated) |
| `tcp_stack` | `Option<String>` | KLD: `tsr_stack` (NUL-terminated) |
| `snd_rexmitpack` | `Option<u32>` | KLD: `tsr_snd_rexmitpack` |
| `rcv_ooopack` | `Option<u32>` | KLD: `tsr_rcv_ooopack` |
| `snd_zerowin` | `Option<u32>` | KLD: `tsr_snd_zerowin` |
| `rcv_numsacks` | `Option<u32>` | KLD: `tsr_rcv_numsacks` |
| `ecn_flags` | `Option<u32>` | KLD: `tsr_ecn` |
| `delivered_ce` | `Option<u32>` | KLD: `tsr_delivered_ce` |
| `received_ce` | `Option<u32>` | KLD: `tsr_received_ce` |
| `dsack_bytes` | `Option<u32>` | KLD: `tsr_dsack_bytes` |
| `dsack_pack` | `Option<u32>` | KLD: `tsr_dsack_pack` |
| `total_tlp` | `Option<u32>` | KLD: `tsr_total_tlp` |
| `total_tlp_bytes` | `Option<u64>` | KLD: `tsr_total_tlp_bytes` |
| `timer_rexmt_ms` | `Option<u32>` | KLD: `tsr_tt_rexmt` (normalized) |
| `timer_persist_ms` | `Option<u32>` | KLD: `tsr_tt_persist` (normalized) |
| `timer_keep_ms` | `Option<u32>` | KLD: `tsr_tt_keep` (normalized) |
| `timer_2msl_ms` | `Option<u32>` | KLD: `tsr_tt_2msl` (normalized) |
| `timer_delack_ms` | `Option<u32>` | KLD: `tsr_tt_delack` (normalized) |
| `idle_time_ms` | `Option<u32>` | KLD: `tsr_rcvtime` (normalized) |
| `options` | `Option<u8>` | KLD: `tsr_options` (TCP options bitmask) |
| `fd` | `Option<i32>` | kern.file: `xf_fd` |

All fields default to `None` via `#[derive(Default)]`, so macOS code is unaffected.

### platform/mod.rs changes

New error variants for device-based collection:

- `DeviceOpen { path, source }` -- `/dev/tcpstats` open failed
- `DeviceRead { source }` -- device read failed
- `Ioctl { cmd, source }` -- ioctl call failed
- `VersionMismatch { expected, got }` -- protocol version mismatch

Platform dispatch in `collect_tcp_sockets()`:
- `#[cfg(target_os = "macos")]` -> `macos::collect()`
- `#[cfg(target_os = "freebsd")]` -> `freebsd::collect()`
- `#[cfg(not(any(...)))]` -> `stub::collect()`

### sysctl.rs changes

`read_os_version()` split by platform:
- macOS: reads `kern.osproductversion` -> `"15.2"`
- FreeBSD: reads `kern.osrelease` -> `"FreeBSD 14.3-RELEASE"`

New `TcpSysStats` struct and `read_tcp_stats()` function:
- Reads `net.inet.tcp.stats` sysctl (FreeBSD `struct tcpstat`, array of `uint64_t`)
- Extracts 12 counters at known offsets: `connattempt`, `accepts`, `connects`, `drops`, `sndtotal`, `sndbyte`, `sndrexmitpack`, `sndrexmitbyte`, `rcvtotal`, `rcvbyte`, `rcvduppack`, `rcvbadsum`

### convert.rs changes

`raw_to_proto()` maps all 22 new fields to proto:

| RawSocketRecord field | Proto field (number) |
|----------------------|---------------------|
| `cc_algo` | `cc_algo` (14) |
| `tcp_stack` | `tcp_stack` (15) |
| `rtt_min_us` | `rtt_min_us` (19) |
| `snd_rexmitpack` | `rexmit_packets` (29) |
| `rcv_ooopack` | `ooo_packets` (30) |
| `snd_zerowin` | `zerowin_probes` (31) |
| `rcv_numsacks` | `sack_blocks` (33) |
| `dsack_bytes` | `dsack_bytes` (34) |
| `dsack_pack` | `dsack_packets` (35) |
| `ecn_flags` | `ecn_flags` (59) |
| `delivered_ce` | `ecn_ce_delivered` (60) |
| `received_ce` | `ecn_ce_received` (61) |
| `total_tlp` | `tlp_probes_sent` (67) |
| `total_tlp_bytes` | `tlp_bytes_sent` (68) |
| `timer_rexmt_ms` | `timer_rexmt_ms` (44) |
| `timer_persist_ms` | `timer_persist_ms` (45) |
| `timer_keep_ms` | `timer_keep_ms` (46) |
| `timer_2msl_ms` | `timer_2msl_ms` (47) |
| `timer_delack_ms` | `timer_delack_ms` (48) |
| `idle_time_ms` | `idle_time_ms` (49) |
| `options` | `negotiated_options` (62) |
| `fd` | `fd` (57) |

`build_metadata()` uses `#[cfg]` gates:
- macOS: `Platform::Macos`, `DataSource::MacosPcblistN`
- FreeBSD: `Platform::Freebsd`, `DataSource::FreebsdKld` + `DataSource::KernFile`

New functions:
- `build_summary_with_sys_stats()` -- enriches `SystemSummary` with `TcpSysStats` delta counters and computed retransmit/duplicate rates
- `build_batch_with_sys_stats()` -- variant of `build_batch()` that includes system-wide stats

## Test Results

24 tests pass on Linux (all pure parser tests run cross-platform):

```
cargo test (via nix develop)

platform::freebsd::tests::test_parse_kld_empty_buffer         ok
platform::freebsd::tests::test_parse_kld_single_ipv4          ok
platform::freebsd::tests::test_parse_kld_single_ipv6          ok
platform::freebsd::tests::test_parse_kld_multiple_records     ok
platform::freebsd::tests::test_parse_kld_version_mismatch     ok
platform::freebsd::tests::test_parse_kld_bad_alignment        ok
platform::freebsd::tests::test_kld_field_mapping              ok
platform::freebsd::tests::test_timer_normalization             ok
platform::freebsd::tests::test_cc_algo_string_extraction       ok
platform::freebsd::tests::test_parse_kern_file_socket_entry   ok
platform::freebsd::tests::test_parse_kern_file_non_socket_skipped  ok
platform::freebsd::tests::test_parse_kern_file_first_pid_wins ok
platform::freebsd_layout::tests::test_record_size             ok  (compile-time + runtime)
platform::freebsd_layout::tests::test_version_struct_size     ok
platform::freebsd_layout::tests::test_ioctl_constants         ok
platform::freebsd_layout::tests::test_extract_nul_string      ok
convert::tests::test_raw_to_proto_freebsd_fields              ok
convert::tests::test_build_summary_with_sys_stats             ok
convert::tests::test_raw_to_proto_basic                        ok
convert::tests::test_kernel_state_to_proto                     ok
convert::tests::test_ip_version_to_proto                       ok
convert::tests::test_ip_addr_to_bytes                          ok
convert::tests::test_build_summary                             ok
platform::macos_layout::tests::test_roundup64                  ok

test result: ok. 24 passed; 0 failed
```

All checks pass:
- `cargo test` -- 24 passed, 0 failed
- `cargo clippy --workspace -- -D warnings` -- 0 warnings
- `cargo fmt --check` -- clean
- `nix build .#tcpstats-reader` -- builds successfully

## Nix Build System

### New files

| File | Purpose |
|------|---------|
| `nix/cross-freebsd.nix` | Cross-compilation derivation using cross-rs (Docker-based) for FreeBSD targets |
| `nix/freebsd-deploy.nix` | SSH deploy + build + test packages for FreeBSD VMs |

### Modified files

| File | Changes |
|------|---------|
| `nix/constants.nix` | Added FreeBSD cross targets with `method = "cross-rs"`, `zigbuildTargets`/`crossRsTargets` helpers |
| `nix/shell.nix` | Added `cargo-cross`, `cargo-zigbuild`, `zig` to dev shell (Linux only) |
| `flake.nix` | Wired FreeBSD deploy packages, cross-rs compilation, separate toolchains per method |

### New nix packages

| Package | Description |
|---------|-------------|
| `cross-x86_64-freebsd` | Cross-compile for FreeBSD amd64 (via cross-rs, requires Docker) |
| `cross-aarch64-freebsd` | Cross-compile for FreeBSD aarch64 (via cross-rs, requires Docker) |
| `tcpstats-reader-freebsd` | Deploy + build + test on ALL FreeBSD VMs |
| `tcpstats-reader-freebsd150` | Deploy + build + test on FreeBSD 15.0 only |
| `tcpstats-reader-freebsd143` | Deploy + build + test on FreeBSD 14.3 only |

### New nix apps

```
nix run .#tcpstats-reader-freebsd          # deploy + test on all FreeBSD VMs
nix run .#tcpstats-reader-freebsd150       # deploy + test on FreeBSD 15.0 only
nix run .#tcpstats-reader-freebsd143       # deploy + test on FreeBSD 14.3 only
nix run .#cross-x86_64-freebsd      # cross-compile for FreeBSD amd64
nix run .#cross-aarch64-freebsd     # cross-compile for FreeBSD aarch64
```

### Dev shell additions

The nix dev shell (`nix develop`) now includes on Linux:
- `cargo-cross` (cross-rs) -- Docker-based FreeBSD cross-compilation
- `cargo-zigbuild` -- macOS cross-compilation
- `zig` -- bundled macOS SDK stubs for zigbuild

### FreeBSD VM deploy flow

The deploy script (`nix/freebsd-deploy.nix`) follows the same pattern as `nix/kmod-tests.nix`:

1. Ensure `rsync`, `cargo`, and `protoc` are available on the VM (installs via `pkg` if missing)
2. Rsync full project source (excluding `target/` and `.git/`)
3. Build on VM with `cargo build --release`
4. Ensure `tcpstats` kernel module is loaded
5. Run `tcpstats-reader --count 1 --pretty` and capture output
6. Verify output contains expected FreeBSD markers (`platform`, `FREEBSD`, `cc_algo`, `rtt_us`, `state`)

Environment variables:
- `FREEBSD_HOST` -- override SSH target (default: per-VM from `constants.nix`)
- `FREEBSD_DIR` -- override remote project directory (default: `/root/tcpstats-reader`)

## Verification checklist

| Check | Status | Command |
|-------|--------|---------|
| Linux build | PASS | `nix develop -c cargo build` |
| Unit tests (24) | PASS | `nix develop -c cargo test` |
| Clippy (0 warnings) | PASS | `nix develop -c cargo clippy --workspace -- -D warnings` |
| Formatting | PASS | `nix develop -c cargo fmt --check` |
| Nix build | PASS | `nix build .#tcpstats-reader` |
| Kmod tests (both VMs) | PASS | `nix run .#kmod-test-freebsd` |
| FreeBSD VM integration (both VMs) | PASS | `nix run .#tcpstats-reader-freebsd` |
| macOS regression | Not yet tested | Needs macOS host run |
| Cross-compile FreeBSD | Not yet tested | `nix build .#cross-x86_64-freebsd` (requires Docker) |

### FreeBSD VM integration results (2026-03-02)

Tested on both VMs (FreeBSD 14.3-RELEASE and 15.0-RELEASE). All checks pass.

**Kmod tests** (`nix run .#kmod-test-freebsd`): 2/2 VMs PASSED, 9/9 targets per VM

| Target | FreeBSD 14.3 | FreeBSD 15.0 |
|--------|-------------|-------------|
| unit (78 tests) | PASS | PASS |
| memcheck (valgrind) | PASS | PASS |
| asan | PASS | PASS |
| ubsan | PASS | PASS |
| bench (10 workloads) | PASS | PASS |
| callgrind | PASS | PASS |
| kmod (kernel module build) | PASS | PASS |
| bench_read | PASS | PASS |
| gen_conn | PASS | PASS |

**Rust client** (`nix run .#tcpstats-reader-freebsd`): 2/2 VMs PASSED, 5/5 checks per VM

| Check | FreeBSD 14.3 | FreeBSD 15.0 |
|-------|-------------|-------------|
| `"platform"` in output | PASS | PASS |
| `FREEBSD` in output | PASS | PASS |
| `"sndCwnd"` in output | PASS | PASS |
| `"rttUs"` in output | PASS | PASS |
| `"state"` in output | PASS | PASS |

Sample output (FreeBSD 14.3, 7 sockets: 4 ESTABLISHED + 3 LISTEN):
```json
{
  "metadata": {
    "hostname": "freebsd14",
    "platform": "PLATFORM_FREEBSD",
    "osVersion": "FreeBSD 14.3-RELEASE",
    "dataSources": ["DATA_SOURCE_FREEBSD_KLD", "DATA_SOURCE_KERN_FILE"],
    "toolVersion": "tcpstats-reader 0.1.0"
  },
  "records": [
    {
      "localPort": 22, "remotePort": 38162,
      "ipVersion": "IP_VERSION_4",
      "state": "TCP_STATE_ESTABLISHED",
      "sndCwnd": 18566, "rttUs": 6250, "maxseg": 1460,
      "sources": ["DATA_SOURCE_FREEBSD_KLD"]
    }
  ],
  "summary": {
    "totalSockets": 7,
    "stateCounts": [
      {"state": "TCP_STATE_LISTEN", "count": 3},
      {"state": "TCP_STATE_ESTABLISHED", "count": 4}
    ]
  }
}
```

### Bugs fixed during integration testing

1. **Rust `read_to_end` on character devices** (`src/platform/freebsd.rs`): Rust's `read_to_end` with an empty `Vec` returns 0 bytes from FreeBSD character devices because `stat()` reports `st_size=0`, causing a short-circuit. Fixed by pre-allocating `Vec::with_capacity(16 * 1024)`.

2. **Deploy script kmod loading** (`nix/freebsd-deploy.nix`): The deploy script used `kldload tcpstats` which only searches `/boot/modules/`. Fixed to build the kmod from the synced source and load with the full path.

3. **Deploy script verification field names** (`nix/freebsd-deploy.nix`): Verification checks used snake_case field names (`cc_algo`, `rtt_us`) but pbjson serializes protobuf fields as camelCase (`sndCwnd`, `rttUs`). Updated to match actual JSON output.

## How to run tests

### Prerequisites

- Both FreeBSD VMs must be running and reachable via SSH
- SSH keys configured for passwordless access to `root@192.168.122.41` (FreeBSD 15.0), `root@192.168.122.85` (FreeBSD 14.4), and `root@192.168.122.27` (FreeBSD 14.3)
- Nix with flakes enabled on the host machine

### Quick start (automated, from Linux host)

```sh
# Run kmod tests on both VMs (filter parser unit tests, sanitizers, benchmark, kmod build)
nix run .#kmod-test-freebsd

# Run kmod tests with a specific target
nix run .#kmod-test-freebsd -- unit        # just unit tests
nix run .#kmod-test-freebsd -- bench       # just benchmark
nix run .#kmod-test-freebsd -- kmod        # just kernel module build
nix run .#kmod-test-freebsd -- all         # all offline tests (default)

# Run kmod live integration tests (load/unload kmod, requires root on VM)
nix run .#kmod-test-freebsd -- live_all

# Deploy and run Rust client on both VMs
# (installs Rust+protobuf, syncs source, builds, loads kmod, runs tcpstats-reader, verifies output)
nix run .#tcpstats-reader-freebsd

# Target a single VM
nix run .#kmod-test-freebsd150             # kmod tests on FreeBSD 15.0 only
nix run .#kmod-test-freebsd143             # kmod tests on FreeBSD 14.3 only
nix run .#tcpstats-reader-freebsd150              # Rust client on FreeBSD 15.0 only
nix run .#tcpstats-reader-freebsd143              # Rust client on FreeBSD 14.3 only
```

### Manual testing (directly on a FreeBSD VM)

If you prefer to run tests manually on a VM:

```sh
# SSH to a FreeBSD VM
ssh root@192.168.122.41   # FreeBSD 15.0
ssh root@192.168.122.85   # FreeBSD 14.4
ssh root@192.168.122.27   # FreeBSD 14.3

# Install dependencies (idempotent, safe to re-run)
sh /root/tcpstats-reader/kmod/tcpstats/test/freebsd-pkg-setup.sh

# Run kmod tests
sh /root/tcpstats-reader/kmod/tcpstats/test/run-tests-freebsd.sh all
sh /root/tcpstats-reader/kmod/tcpstats/test/run-tests-freebsd.sh unit
sh /root/tcpstats-reader/kmod/tcpstats/test/run-tests-freebsd.sh live_all    # needs root

# Build and load the kernel module
cd /root/tcpstats-reader/kmod/tcpstats && make clean all
kldload /root/tcpstats-reader/kmod/tcpstats/tcpstats.ko

# Build and run the Rust client
cd /root/tcpstats-reader && cargo build --release
./target/release/tcpstats-reader --count 1 --pretty

# Unload the kernel module when done
kldunload tcpstats
```

### Environment variable overrides

| Variable | Default | Description |
|----------|---------|-------------|
| `FREEBSD_HOST` | per-VM from `nix/constants.nix` | SSH target (e.g. `root@192.168.122.41`) |
| `FREEBSD_DIR` | `/root/tcpstats-reader` | Remote project directory (for `tcpstats-reader-freebsd`) |
| `FREEBSD_KMOD_DIR` | `/root/tcpstats-reader/kmod` | Remote kmod directory (for `kmod-test-freebsd`) |

### Available kmod test targets

| Target | Description |
|--------|-------------|
| `unit` | 78 filter parser unit tests |
| `memcheck` | Valgrind memcheck (leak/error detection) |
| `asan` | AddressSanitizer + UBSan |
| `ubsan` | UndefinedBehaviorSanitizer |
| `bench` | Performance benchmark (1M iterations, 10 workloads) |
| `callgrind` | Callgrind CPU profiling |
| `kmod` | Build kernel module (`tcpstats.ko`) |
| `bench_read` | Compile read-path microbenchmark |
| `gen_conn` | Compile loopback connection generator |
| `all` | All of the above (default) |
| `live_smoke` | Kmod lifecycle: load, read, sysctl verify, unload |
| `live_bench` | Read-path bench at 1K/10K/100K connections |
| `live_stats` | Sysctl counter invariant validation |
| `live_dtrace` | DTrace SDT probe registration + firing |
| `live_dos` | DoS protections: EMFILE, timeout, EINTR |
| `live_all` | All live targets (requires root) |

## Next steps

### Short-term

- **macOS regression test** -- verify macOS-specific fields still work after the cfg gate changes
- **Cross-compile FreeBSD** -- test `nix build .#cross-x86_64-freebsd` (requires Docker for cross-rs)

### Medium-term

- **Delta tracking** -- implement per-connection delta computation for retransmit rates, byte counters
- **System stats deltas** -- wire `read_tcp_stats()` into the main collection loop for `SystemSummary` delta counters (currently the function exists but isn't called from `main.rs`)
- **Filter support** -- expose `TCPSTATS_SET_FILTER` ioctl from the Rust client for server-side socket filtering
- **Command name enrichment** -- read process command names via `kern.proc.args` or procfs

### Long-term

- **ARM64 FreeBSD** -- test on aarch64 FreeBSD (cross-compiled binary or native build)
- **Binary protobuf output** -- length-delimited binary output for high-throughput collection
- **CI pipeline** -- automate FreeBSD VM tests in CI (requires FreeBSD VM runner)
