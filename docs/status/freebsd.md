# FreeBSD Implementation Status

## Kernel Module: tcpstats

The `tcpstats` kernel module provides per-socket TCP statistics via a character device (`/dev/tcp_stats`). It includes a filter parser that accepts filter strings like `local_port=443 exclude=listen,timewait` to control which sockets are reported.

### Kernel module build

The kmod builds cleanly on FreeBSD 14.3-RELEASE, 14.4-RELEASE, and 15.0-RELEASE using the system `cc` (clang) and `/usr/src/sys` kernel headers.

```sh
cd kmod/tcpstats
make clean all
# produces tcpstats.ko
```

### Filter parser (tcp_statsdev_filter.c)

Dual-compile parser: compiles as both `_KERNEL` code inside the kmod and as userspace code for testing. Supports directives:

- `local_port=443,8443` / `remote_port=80` -- port filtering (up to 8 per direction)
- `exclude=listen,timewait` / `include_state=established` -- TCP state filtering
- `local_addr=10.0.0.0/24` / `remote_addr=fe80::/10` -- IPv4/IPv6 CIDR filtering
- `ipv4_only` / `ipv6_only` -- address family flags
- `format=compact|full` -- output format selection
- `fields=state,rtt,buffers` -- field group selection

All string operations use bounded variants (`strlcpy`, `strnlen`, `snprintf`). No `strcpy`, `strcat`, or `sprintf`. Input is validated for length and printable ASCII before parsing.

## Test Suite

Fifteen test targets run on the FreeBSD VM, covering correctness, memory safety, performance, kernel module compilation, tooling, and live kernel module integration testing.

### Test results

Verified on all VMs from a single `nix run .#kmod-test-freebsd -- live_all` invocation:

| Target | FreeBSD 14.3 | FreeBSD 14.4 | FreeBSD 15.0 | What it tests |
|--------|-------------|-------------|-------------|---------------|
| unit | PASS (78/78) | PASS (78/78) | PASS (78/78) | Functional correctness: positive parsing, error rejection, value verification |
| memcheck | PASS (0 errors) | PASS (0 errors) | PASS (0 errors) | Valgrind memcheck: leak detection, use-after-free, uninitialized reads |
| asan | PASS | PASS | PASS | AddressSanitizer + UBSan: buffer overflows, use-after-free, undefined behavior |
| ubsan | PASS | PASS | PASS | UndefinedBehaviorSanitizer standalone: signed overflow, shift errors, null derefs |
| bench | PASS | PASS | PASS | Performance benchmark: 1M iterations across 11 workloads |
| callgrind | PASS | PASS | PASS | Callgrind CPU profiling: instruction-level hotspot analysis |
| kmod | PASS | PASS | PASS | Kernel module compilation: `tcpstats.ko` produced with `-Werror` |
| bench_read | PASS | PASS | PASS | Read-path microbenchmark compilation (7 workloads incl. concurrent readers) |
| gen_conn | PASS | PASS | PASS | Loopback connection generator compilation (up to 500K connections) |
| live_smoke | PASS | PASS | PASS | Kmod lifecycle: load, read /dev/tcpstats, verify sysctl tree, unload |
| live_bench | PASS | PASS | PASS | Read-path benchmark at 1K/10K/100K connections (7 workloads + concurrent readers) |
| live_stats | PASS | PASS | PASS | Sysctl counter invariant: visited == emitted + sum(skipped) |
| live_dtrace | PASS | PASS | PASS | DTrace SDT probes register and fire (7 probes via KDTRACE_HOOKS) |
| live_dos | PASS | PASS | PASS | DoS protections: EMFILE limit, read timeout partial results, EINTR signal |

### Benchmark results (post-optimization)

Measured on FreeBSD 14.3-RELEASE, 1M iterations per workload, compiled with `-O2`:

| Workload | NS/CALL | CALLS/SEC |
|----------|---------|-----------|
| empty | 3.2 | 313M |
| single_port | 54.5 | 18.3M |
| multi_port (8 ports) | 193.9 | 5.2M |
| exclude_states (4 states) | 137.1 | 7.3M |
| ipv4_cidr | 141.2 | 7.1M |
| ipv6_compressed | 146.4 | 6.8M |
| ipv6_full (8 groups) | 135.8 | 7.4M |
| complex_combo | 323.3 | 3.1M |
| worst_case (all features) | 820.5 | 1.2M |
| uppercase_stress | 317.3 | 3.2M |

All workloads complete well under 1 microsecond per call.

### Performance optimizations applied

Callgrind profiling identified five hotspots that were optimized, yielding a **1.6x-2.5x speedup** across all workloads:

| Optimization | Ir saved | Technique |
|-------------|----------|-----------|
| Fix `strlcpy` shim | ~513M (10.7%) | BSD guard on `#ifndef strlcpy` so FreeBSD uses native `strlcpy` instead of `snprintf` shim |
| Merge printable check + copy | ~300M (6.3%) | Single-pass validation and buffer copy instead of separate loop + `strlcpy` |
| Inline port/prefix conversion | ~521M (10.9%) | Manual `val = val * 10 + (c - '0')` replaces `strtoul` (avoids locale lookups) |
| First-char directive dispatch | ~300M (6.3%) | `switch (key[0])` skips irrelevant `strcmp` calls |
| ASCII-only tolower | ~178M (3.7%) | `TSF_TOLOWER` macro via `c \| 0x20` bypasses locale-aware `__sbtolower` |

Before/after comparison (FreeBSD 15.0-RELEASE):

| Workload | Before (ns) | After (ns) | Speedup |
|----------|-------------|------------|---------|
| single_port | 162.8 | 64.7 | 2.5x |
| multi_port | 429.1 | 227.1 | 1.9x |
| ipv4_cidr | 351.7 | 159.4 | 2.2x |
| worst_case | 1436.9 | 851.6 | 1.7x |

## FreeBSD VM Test Deployment

A nix orchestrator (`kmod-test-freebsd`) handles the full workflow: rsync source to the VM, install dependencies, compile, and run tests. Works on fresh FreeBSD installs with no manual setup.

### How to run

```sh
# Full suite on default VM (192.168.122.41)
nix run .#kmod-test-freebsd

# Individual target
nix run .#kmod-test-freebsd -- unit
nix run .#kmod-test-freebsd -- bench
nix run .#kmod-test-freebsd -- kmod

# Live integration tests (require root, load/unload kmod)
nix run .#kmod-test-freebsd -- live_all
nix run .#kmod-test-freebsd -- live_smoke
nix run .#kmod-test-freebsd -- live_bench
nix run .#kmod-test-freebsd -- live_dos

# Custom host
FREEBSD_HOST=root@192.168.122.27 nix run .#kmod-test-freebsd

# Direct on VM (after rsync)
ssh root@192.168.122.41 'sh /root/tcpstats-reader/kmod/tcpstats/test/run-tests-freebsd.sh all'
```

### Available targets

| Target | Description |
|--------|-------------|
| `unit` | Compile + run 78 unit tests |
| `memcheck` | Valgrind memcheck (leak/error detection) |
| `asan` | AddressSanitizer + UBSan |
| `ubsan` | UndefinedBehaviorSanitizer standalone |
| `bench` | Performance benchmark (1M iterations, 11 workloads) |
| `callgrind` | Callgrind CPU profiling + annotation |
| `kmod` | Build kernel module (`tcpstats.ko`) |
| `bench_read` | Compile read-path microbenchmark |
| `gen_conn` | Compile loopback connection generator |
| `all` | All of the above sequentially |
| `live_smoke` | Kmod lifecycle: load, read, verify sysctl, unload |
| `live_bench` | Read-path benchmark at 1K/10K/100K connections |
| `live_stats` | Sysctl counter invariant validation (`-DTCPSTATS_STATS`) |
| `live_dtrace` | DTrace SDT probe registration + firing (`-DTCPSTATS_DTRACE`), skips if dtrace unavailable |
| `live_dos` | DoS protection: EMFILE, read timeout, EINTR tests |
| `live_all` | All live_* targets sequentially (requires root) |

### What the setup script does (freebsd-pkg-setup.sh)

Takes a fresh FreeBSD install to a fully working build + test environment:

1. Bootstraps `pkg` if not installed
2. Fetches and installs kernel source tree (`src.txz`) if `/usr/src/sys` is missing
3. Installs packages: `valgrind`, `perl5` (for `callgrind_annotate`)

FreeBSD base system already provides: `cc` (clang), `make`, sanitizers, `strlcpy`, `strsep`, `bsd.kmod.mk`.

### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `FREEBSD_HOST` | `root@192.168.122.41` | SSH target for the FreeBSD VM (15.0: .41, 14.4: .85, 14.3: .27) |
| `FREEBSD_KMOD_DIR` | `/root/tcpstats-reader/kmod` | Remote directory for kmod source |
| `CC` | `cc` | Compiler (in `run-tests-freebsd.sh`) |
| `BENCH_ITERS` | `1000000` | Benchmark iteration count |

### Linux-side tests

The parser tests also run on the Linux nix host (useful for CI):

```sh
nix run .#kmod-test-unit       # gcc, unit tests
nix run .#kmod-test-asan       # gcc, ASan + UBSan
nix run .#kmod-test-ubsan      # gcc, UBSan standalone
nix run .#kmod-test-memcheck   # gcc + valgrind
nix run .#kmod-test-bench      # gcc, benchmark
nix run .#kmod-test-callgrind  # gcc + valgrind callgrind
nix run .#kmod-test-all        # all of the above
```

## File inventory

```
kmod/tcpstats/
  Makefile                          Kernel module build (bsd.kmod.mk)
  tcp_statsdev.c                   Kernel module: char device, ioctl, profiles
  tcp_statsdev.h                   Shared header: ioctl commands, filter struct
  tcp_statsdev_filter.c          Dual-compile filter string parser
  tcp_statsdev_filter.h          Parser API: tsf_parse_filter_string()
  test/
    test_filter_parse.c             78 unit tests (positive + negative + value verification)
    bench_filter_parse.c            Benchmark harness (11 workloads, configurable iterations)
    fuzz_filter_parse.c             AFL++ / libFuzzer dual harness
    read_tcpstats.c                 Manual test program for /dev/tcp_stats ioctl
    freebsd-pkg-setup.sh            Idempotent FreeBSD setup (pkg, kernel src, valgrind)
    bench_read_tcpstats.c           Read-path microbenchmark (7 workloads, concurrent readers)
    gen_connections.c               Loopback connection generator (up to 500K connections)
    test_dos_limits.c               DoS protection validation (EMFILE, timeout, EINTR)
    run-tests-freebsd.sh            POSIX sh test runner (15 targets incl. 6 live)
    Makefile                        Build read_tcpstats test program
  tools/
    decode_tcpstats.py              Python decoder for /dev/tcp_stats binary output

nix/
  kmod-tests.nix                    Nix targets: kmod-test-{unit,memcheck,asan,ubsan,bench,callgrind,all,freebsd}
```

## Verified FreeBSD versions

All 15 test targets pass on all three versions, including the 6 live kernel module integration tests (`live_all`).

| Version | Arch | Kernel source | Offline tests | Live tests | Kmod build |
|---------|------|---------------|---------------|------------|------------|
| 14.3-RELEASE | amd64 | Fetched via `src.txz` | 9/9 PASS | 5/5 PASS | PASS |
| 14.4-RELEASE | amd64 | Fetched via `src.txz` | 9/9 PASS | 5/5 PASS | PASS |
| 15.0-RELEASE | amd64 | Pre-installed | 9/9 PASS | 5/5 PASS | PASS |

### Live test results (live_all)

| Target | FreeBSD 14.3 | FreeBSD 14.4 | FreeBSD 15.0 | Notes |
|--------|-------------|-------------|-------------|-------|
| `live_smoke` | PASS | PASS | PASS | kmod load/read/sysctl/unload |
| `live_bench` | PASS | PASS | PASS | 1K/10K/100K connections, ~3M rec/s at 10K |
| `live_stats` | PASS | PASS | PASS | Invariant holds: visited == emitted + skipped |
| `live_dtrace` | PASS | PASS | PASS | SDT probes register and fire at runtime (7 probes) |
| `live_dos` | PASS | PASS | PASS | EMFILE, timeout (partial results), EINTR all verified |

### DTrace SDT probe fix

DTrace SDT probes initially compiled but did not register at runtime. The root cause was that `KDTRACE_HOOKS` -- which gates the real SDT macro implementation in `sys/sys/sdt.h` -- is defined in the `GENERIC` kernel config but not in `DEFAULTS`. Out-of-tree `bsd.kmod.mk` builds generate `opt_global.h` only from `DEFAULTS`, so all `SDT_PROVIDER_DEFINE`/`SDT_PROBE_DEFINE*` calls silently expanded to no-ops.

The fix adds `-DKDTRACE_HOOKS` automatically in the Makefile when `-DTCPSTATS_DTRACE` is detected in CFLAGS, plus a `#error` guard in `tcp_statsdev.c` to catch future misconfigurations at compile time. This is safe because GENERIC kernels (the standard FreeBSD kernel) already have `KDTRACE_HOOKS` enabled; the define only controls whether `sdt.h` macros emit linker set entries.

**Changes made:**

| File | Change |
|------|--------|
| `kmod/tcpstats/Makefile` | `.if !empty(CFLAGS:M*TCPSTATS_DTRACE*)` conditional auto-adds `-DKDTRACE_HOOKS` |
| `kmod/tcpstats/tcp_statsdev.c` | `#error` guard if `TCPSTATS_DTRACE` set without `KDTRACE_HOOKS` |
| `kmod/tcpstats/test/run-tests-freebsd.sh` | `live_dtrace` now FAILs (not skips) if probes don't register |

**Verification (both FreeBSD 14.3 and 15.0):**

1. Build with DTrace -- ELF sections present:
   ```
   readelf -S tcpstats.ko | grep sdt
   [ 4] set_sdt_tracepoint_set    PROGBITS  ...  000240
   [ 8] set_sdt_providers_set     PROGBITS  ...  000008
   [10] set_sdt_probes_set        PROGBITS  ...  000038
   [12] set_sdt_argtypes_set      PROGBITS  ...  000060
   ```

2. Runtime -- all 7 probes registered under `tcpstats` provider:
   ```
   dtrace -l -P tcpstats
      ID   PROVIDER            MODULE                          FUNCTION NAME
   73670   tcpstats     tcpstats                              read entry
   73671   tcpstats     tcpstats                              read done
   73672   tcpstats     tcpstats                            filter skip
   73673   tcpstats     tcpstats                            filter match
   73674   tcpstats     tcpstats                              fill done
   73675   tcpstats     tcpstats                           profile create
   73676   tcpstats     tcpstats                           profile destroy
   ```

3. Probes fire under load (`live_dtrace` test, 100 connections, 2 reads):
   ```
   read:done                                                         2
   read:entry                                                        2
   filter:match                                                    858
   ```

4. Production build (no `-DTCPSTATS_DTRACE`) -- zero SDT sections:
   ```
   readelf -S tcpstats.ko | grep sdt    # (empty output)
   ```

## Performance & Security Hardening

All 13 items from the performance/security plan are implemented and verified via `live_all` on both FreeBSD 14.3 and 15.0. Details in [../design/10-performance-security.md](../design/10-performance-security.md) (analysis) and [../../archive/perf-security-log.md](../../archive/perf-security-log.md) (implementation log).

### Implementation summary

| # | Category | Item | Status | Verified by |
|---|----------|------|--------|-------------|
| 1 | Critical | Concurrent reader limit (max 32, EBUSY) | DONE | `live_bench` concurrent reader workload |
| 2 | Critical | Fix `destroy_dev()` under sx xlock | DONE | `live_smoke` load/unload cycles |
| 3 | Critical | Read iteration timeout (default 5s) | DONE | `live_dos` timeout sub-test |
| 4 | Critical | Signal-interruptible reads (EINTR) | DONE | `live_dos` EINTR sub-test |
| 5 | High | Cache `getsbinuptime()` per read() | DONE | `live_bench` throughput numbers |
| 6 | High | field_mask gating (9 field groups) | DONE | `live_bench` field_mask workloads |
| 7 | High | IPv6 address filtering | DONE | `live_bench` ipv4_only/ipv6_only workloads |
| 8 | Medium | DTrace SDT probes (compile-time) | DONE | `live_dtrace` (probes register and fire at runtime) |
| 9 | Medium | Sysctl stats counters (two-tier) | DONE | `live_stats` invariant validation |
| 10 | Medium | Read-path microbenchmark | DONE | `live_bench` (7 workloads) |
| 11 | Medium | Loopback connection generator | DONE | `live_bench` (1K/10K/100K scales) |
| 12 | Medium | Max open fds cap (default 16, EMFILE) | DONE | `live_dos` EMFILE sub-test |
| 13 | Medium | Per-fd rate limiting (default 0/off) | DONE | sysctl tunable available |

### DoS protections

| Protection | Default | Sysctl | Behavior |
|-----------|---------|--------|----------|
| Max open fds | 16 | `dev.tcpstats.max_open_fds` | Returns EMFILE when exceeded |
| Max concurrent readers | 32 | `dev.tcpstats.max_concurrent_readers` | Returns EBUSY when exceeded |
| Read iteration timeout | 5000ms | `dev.tcpstats.max_read_duration_ms` | Returns partial results on timeout |
| Per-fd rate limiting | 0 (off) | `dev.tcpstats.min_read_interval_ms` | Returns EBUSY if too frequent |
| Signal-interruptible reads | Always | -- | SIGPENDING checked every 256 sockets, returns EINTR |
| Voluntary preemption | Always | -- | `kern_yield(PRI_USER)` every 256 sockets |

### Lock ordering fix

`destroy_dev()` is no longer called under sx xlock. Profile deletion is split into `tcpstats_profile_detach()` (under lock) and `tcpstats_profile_destroy_unlocked()` (outside lock), preventing deadlock when readers have open fds.

### Live read-path benchmark results

Measured via `live_bench` with `bench_read_tcpstats`, no filter, 4MB buffer:

| Scale | FreeBSD 14.3 | FreeBSD 15.0 | Notes |
|-------|-------------|-------------|-------|
| 1K connections | 275.9 us avg, 7.3M rec/s | ~280 us avg, ~7.1M rec/s | L2/L3 hot |
| 10K connections | 4.4 ms avg, 3.0M rec/s | 4.7 ms avg, 2.8M rec/s | L3 hot |
| 100K connections | ~4.2 ms (13K rec buffer limit) | ~4.2 ms (13K rec buffer limit) | 4MB buffer caps at ~13K records |

Concurrent reader scaling (10K connections, 5 reads each):

| Threads | FreeBSD 14.3 wall time | FreeBSD 15.0 wall time |
|---------|----------------------|----------------------|
| 1 | 2.1 ms | 2.3 ms |
| 4 | 2.8 ms | 2.5 ms |
| 16 | 9.0 ms | 4.2 ms |

### Read-path optimizations

- **Cached `getsbinuptime()`**: called once per `read()`, passed to `tcpstats_fill_record()` -- eliminates N-1 timecounter reads
- **field_mask gating**: 9 field groups individually gated by `if (field_mask & TSR_FIELDS_*)` -- skips pointer chases for unneeded fields
- **IPv6 address filtering**: `tsf_match_v6_prefix()` for both local and remote addresses, completing the previously half-built feature

### Compile-time observability (zero production overhead)

- **DTrace SDT probes** (`-DTCPSTATS_DTRACE`): 7 probe points (read entry/done, filter skip/match, fill done, profile create/destroy)
- **Sysctl statistics** (`-DTCPSTATS_STATS`): 13 per-socket hot-loop counters (visited, emitted, 6x skip reasons, timing, errors)
- **Tier 1 always-on counters**: `reads_total`, `active_fds`, `opens_total` (outside hot loop, negligible cost)

### Test tooling

- `test/bench_read_tcpstats.c` -- read-path microbenchmark with 7 workloads including concurrent readers
- `test/gen_connections.c` -- loopback connection generator (up to 500K connections) for populating connection tables

## Next steps

### Short-term

- **Fuzz testing on FreeBSD** -- run `fuzz_filter_parse.c` with AFL++ or libFuzzer on the VM to find edge cases the unit tests miss
- ~~**Kmod load/unload testing**~~ -- DONE: `live_smoke` target
- ~~**Ioctl integration tests**~~ -- DONE: `live_bench` and `live_stats` targets cover full pipeline
- ~~**Performance & security hardening**~~ -- DONE: all 13 plan items implemented and verified on both VMs
- ~~**DTrace SDT probe registration**~~ -- DONE: fixed by auto-adding `-DKDTRACE_HOOKS` in Makefile when `-DTCPSTATS_DTRACE` is set
- **Filter parser: `strlcpy` shim for Linux** -- replace the `snprintf`-based shim with a proper `strlcpy` implementation for non-BSD platforms (currently only affects Linux CI performance, not correctness)

### Medium-term

- **FreeBSD platform parser in Rust** -- implement `src/platform/freebsd.rs` to parse `net.inet.tcp.pcblist` sysctl output, matching the macOS parser architecture
- **kern.file PID join** -- FreeBSD pcblist doesn't include PID; join with `kern.file` sysctl to attribute sockets to processes
- **tcpstats enrichment** -- use the kmod's per-socket data to supplement sysctl fields (congestion window, RTT, retransmits)
- **CI pipeline** -- automate `nix run .#kmod-test-freebsd` in CI (requires FreeBSD VM runner or bhyve-in-CI setup)

### Long-term

- **ARM64 FreeBSD** -- test on aarch64 FreeBSD (Raspberry Pi, AWS Graviton)
- **Kernel module packaging** -- FreeBSD port/package for `tcpstats` so it can be installed via `pkg install`
- ~~**DTrace + stats profiling**~~ -- DONE: `live_dtrace` and `live_stats` targets
- ~~**Live socket filtering benchmarks**~~ -- DONE: `live_bench` target (1K/10K/100K connections)
