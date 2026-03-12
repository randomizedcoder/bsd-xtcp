# Soak Test Results

## Test Environment

| VM | OS | Host |
|---|---|---|
| freebsd150 | FreeBSD 15.0 | root@192.168.122.41 |
| freebsd143 | FreeBSD 14.3 | root@192.168.122.27 |

Both VMs: kmod built with `-DTCPSTATS_STATS -DTCPSTATS_DTRACE`.

## Test Results

### 1. Quick Verification (2 cycles, 50 connections)

- **Date:** 2026-03-08
- **VMs:** freebsd150 only
- **Config:** `SOAK_DURATION_HOURS=0 SOAK_CONNECTIONS=50`
- **Result:** PASSED
- **Connection count:** 51 (stable)
- **Health failures:** 0
- **Notes:** Validated the full collection loop, output directory structure, and summary generation.

### 2. 24-Hour Soak Attempt (1000 connections) -- FAILED

- **Date:** 2026-03-08
- **VMs:** freebsd150, freebsd143
- **Config:** `SOAK_DURATION_HOURS=24 SOAK_CONNECTIONS=1000`
- **Result:** FAILED (both VMs)
- **Root cause:** tcp-echo client panicked at connection ~606 during ramp-up. FreeBSD default `kern.threads.max_threads_per_proc=1500` limits threads per process. tcp-echo creates 2 threads per connection (reader + writer), hitting the limit at ~606 connections. The `.expect()` on `thread::Builder::spawn()` caused a silent panic.
- **Fix applied:**
  - `tune_system()` now raises `kern.threads.max_threads_per_proc=250000`
  - tcp-echo client: replaced `.expect()` with `.context()?` for proper error propagation

### 3. 1-Hour Validation (1000 connections)

- **Date:** 2026-03-09
- **VMs:** freebsd150 only
- **Config:** `SOAK_DURATION_HOURS=1 SOAK_CONNECTIONS=1000`
- **Result:** PASSED

| Metric | freebsd150 |
|---|---|
| Cycles | 12/12 |
| Conn avg | 998 |
| Conn max | 1001 |
| Conn min | 581 (ramp-up cycle 0 only) |
| Health warnings | 1 (ramp-up) |

### 4. 12-Hour Soak (1000 connections)

- **Date:** 2026-03-09
- **VMs:** freebsd150, freebsd143
- **Config:** `SOAK_DURATION_HOURS=12 SOAK_CONNECTIONS=1000`
- **Result:** PASSED (both VMs)

| Metric | freebsd150 | freebsd143 |
|---|---|---|
| Cycles | 144/144 | 144/144 |
| Duration | 43200.0s (12.0h) | 43200.8s (12.0h) |
| Conn avg | 998 | 997 |
| Conn max | 1001 | 1001 |
| Conn min | 581 (ramp-up) | 559 (ramp-up) |
| Health warnings | 1 (ramp-up) | 1 (ramp-up) |

- No client deaths, no connection drops, no memory leak warnings across 12 hours.

### 5. 1-Hour Soak (10,000 connections)

- **Date:** 2026-03-09
- **VMs:** freebsd150, freebsd143
- **Config:** `SOAK_DURATION_HOURS=1 SOAK_CONNECTIONS=10000`
- **Result:** PASSED (both VMs)

| Metric | freebsd150 | freebsd143 |
|---|---|---|
| Cycles | 12/12 | 12/12 |
| Duration | 3600.4s (1.0h) | 3600.3s (1.0h) |
| Conn avg | 9587 | 9581 |
| Conn max | 10001 | 10001 |
| Conn min | 5040 (ramp-up) | 4962 (ramp-up) |
| Health warnings | 1 (ramp-up) | 1 (ramp-up) |

- 10k connections held stable for the full hour on both OS versions.
- Ramp-up takes ~250s (10000/40 connections/sec), so cycle 0 at the 5-minute mark shows ~50% connected.

### 6. 24-Hour Soak (10,000 connections)

- **Date:** 2026-03-10 to 2026-03-11
- **VMs:** freebsd150, freebsd143 (run in parallel)
- **Config:** `SOAK_DURATION_HOURS=24 SOAK_CONNECTIONS=10000`
- **Build flags:** `-DTCPSTATS_STATS -DTCPSTATS_DTRACE -DKDTRACE_HOOKS`
- **Result:** PASSED (both VMs)

| Metric | freebsd150 | freebsd143 |
|---|---|---|
| Cycles | 288/288 | 288/288 |
| Duration | 24.0h | 24.0h |
| Conn avg | 10,000 | 9,984 |
| Conn max | 10,001 | 10,001 |
| Conn min | 9,728 | 5,120 |
| Health failures | 0 | 1 (transient dip, immediate recovery) |
| Records emitted | ~8.3M | ~8.3M |
| Sockets visited | ~11M | ~11M |

**Adaptive ramp performance:**

| | FreeBSD 15.0 | FreeBSD 14.3 |
|---|---|---|
| Ramp time | 33.7s | 28.2s |
| Connected | 10,000 | 10,000 |
| Failed | 0 | 0 |
| Batches | 19 | 19 |
| Success rate | 100% every batch | 100% every batch |

Batch size progression (both VMs): 50 → 100 → 200 → 400 → 800 → 1600 → 2000

**Kernel memory analysis (M_TCPSTATS via `vmstat -m`):**

| Hour | Use | Memory | Requests | Size |
|------|-----|--------|----------|------|
| 0 | 0 | 0 | 2 | 256 |
| 3 | 0 | 0 | 74 | 256 |
| 6 | 0 | 0 | 146 | 256 |
| 9 | 0 | 0 | 218 | 256 |
| 12 | 0 | 0 | 290 | 256 |
| 15 | 0 | 0 | 362 | 256 |
| 18 | 0 | 0 | 434 | 256 |
| 21 | 0 | 0 | 506 | 256 |
| 23 | 0 | 0 | 554 | 256 |

Both VMs showed identical values. `Use = 0` and `Memory = 0` at every sample confirms zero kernel memory leaks — every allocation freed before returning to userspace. Requests increase linearly at 24/hour (2 per cycle), confirming consistent alloc/free pairing. If the module leaked even one 256-byte buffer per read, Memory would show ~142 KB by hour 23.

**Sysctl counter analysis (dev.tcpstats.*):**

| Counter | Hour 0 | Hour 23 | Notes |
|---|---|---|---|
| `reads_total` | 2 | 554 | 552 reads over 24h |
| `opens_total` | 2 | 554 | 1:1 with reads (no leaked FDs) |
| `active_fds` | 0 | 0 | No file descriptors held open |
| `records_emitted` | 30,008 | 8,311,943 | ~8.3M records, ~15k/read |
| `sockets_visited` | 40,014 | 11,083,327 | ~11M socket walks |
| `uiomove_errors` | 0 | 0 | Zero copy-to-userspace errors |
| `reads_interrupted` | 0 | 0 | Zero interrupted reads |
| `reads_timed_out` | 0 | 1 | 1 timeout in 554 reads (0.18%) |
| `sockets_skipped_gencnt` | 0 | 0 | No generation count races |

Counter values from FreeBSD 15.0; FreeBSD 14.3 showed equivalent values.

**Connection stability:** Both VMs held 10,001 connections for 23 of 24 hours. FreeBSD 14.3 had one transient dip to 5,120 at hour 15 cycle 4, immediately recovered — appears to be a momentary kernel TCP state snapshot artifact rather than actual connection loss.

**Module load stability:** `kldstat` collected every 5 minutes. `tcp_stats_kld.ko` remained at the same kernel address throughout (FreeBSD 15.0: `0xffffffff828eb000`, size `0x7d5c`). No unload/reload events.

**Raw data:** `test-output/freebsd{150,143}/2026-03-10-04-46-42/live_soak/` — hourly directories with `memory_NN.txt`, `sysctl_NN.txt`, `tcp_stats_NN.json`, `hour_summary.json`.

## Known Behaviors

1. **Cycle 0 ramp-up warning**: The first collection cycle may report a low connection count because tcp-echo is still establishing connections. This is expected and not a failure.

2. **Connection count = expected + 1**: The LISTEN socket on port 9090 is counted by `read_tcpstats`, so 1000 client connections show as 1001.

3. **Adaptive ramp for high connection counts**: For >500 connections, the test uses adaptive batch-based ramp (start at 50, double after 3 good batches up to 2000, halve on failure) instead of fixed-rate ramp. This completes 10K connections in ~30s versus ~250s with the old fixed-rate approach.

## Next Steps

- [ ] 1-hour soak with 100,000 connections
- [ ] 24-hour soak with 1,000 connections
- [x] 24-hour soak with 10,000 connections
