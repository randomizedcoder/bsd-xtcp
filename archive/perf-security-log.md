# tcp_stats_kld Performance & Security Implementation Log

Tracking progress against `kernel-module-perfomrance-and-security-plan.md`.

---

## Implementation Progress

### Critical (before production use)

| # | Item | Status | Notes |
|---|------|--------|-------|
| 1 | Concurrent reader limit (4.2) | DONE | `tcp_stats_kld.c:930-934` -- atomic counter + EBUSY |
| 2 | Fix destroy_dev() under sx xlock (4.4) | DONE | `tcp_stats_kld.c:324-335` (`tcpstats_profile_destroy_unlocked`) + `342-350` (`tcpstats_profile_detach`) |
| 3 | Read iteration timeout (4.3) | DONE | `tcp_stats_kld.c:952-957` (deadline) + `996-1001` (timeout check) |
| 4 | Signal check in iteration loop (4.5) | DONE | `tcp_stats_kld.c:988-993` (SIGPENDING every 256 sockets) |

### High Priority (performance)

| # | Item | Status | Notes |
|---|------|--------|-------|
| 5 | Cache getsbinuptime() per read() (2.3) | DONE | `tcp_stats_kld.c:937` (cached `now`) + `1141` (passed to `tcpstats_fill_record`) + `770-772` (function signature) |
| 6 | Implement field_mask gating (2.2) | DONE | `tcp_stats_kld.c:786-897` (9 field groups gated by `if (field_mask & TSR_FIELDS_*)`) |
| 7 | Add IPv6 address filtering (2.4) | DONE | `tcp_stats_kld.c:686-711` (`tsf_match_v6_prefix`) + `1096-1109` (local) + `1122-1135` (remote) |

### Medium Priority (observability)

| # | Item | Status | Notes |
|---|------|--------|-------|
| 8 | DTrace SDT probes (compile-time) (5.1) | DONE | `tcp_stats_kld.c:34-76` (`#ifdef TCPSTATS_DTRACE`) |
| 9 | Sysctl stats counters (two-tier) (5.2) | DONE | `tcp_stats_kld.c:78-132` (Tier 2 `#ifdef TCPSTATS_STATS`) + `94-97` (Tier 1 always-on) |
| 10 | Read-path microbenchmark (5.3) | DONE | `test/bench_read_tcpstats.c` (7 workloads incl. concurrent readers) |
| 11 | Loopback connection generator (5.4) | DONE | `test/gen_connections.c` (up to 500K connections, round-robin) |
| 12 | Max open fds cap (4.2 Cap 1) | DONE | `tcp_stats_kld.c:145,658-662` (default 16, sysctl tunable) |
| 13 | Per-fd rate limiting (4.1) | DONE | `tcp_stats_kld.c:938-945` (min_read_interval_ms, default 0) |

### Beyond plan (additional hardening)

| Item | Status | Notes |
|------|--------|-------|
| Voluntary preemption | DONE | `tcp_stats_kld.c:1004` (`kern_yield(PRI_USER)` every 256 sockets) |
| Makefile compile-flag support | DONE | `Makefile:7-13` (TCPSTATS_DTRACE, TCPSTATS_STATS toggles) |
| All sysctl tunables in MOD_LOAD | DONE | `tcp_stats_kld.c:1269-1378` (DoS limits + Tier 1/2 stats) |

---

## Detailed Log

### 2026-03-02 -- Session Start

- Completed full analysis of kernel module hot paths
- Identified 10 adversarial scenarios (3 HIGH severity)
- Identified critical bug: destroy_dev() called under sx xlock (line 189)
- Identified missing feature: IPv6 address filtering in read path
- Identified top optimization: cache getsbinuptime() (saves N-1 calls per read)
- Decided: DTrace + detailed stats as compile-time features for zero prod overhead
- Decided: max_open_fds default = 16 (conservative), stress tests raise to 64
- Plan written to kernel-module-perfomrance-and-security-plan.md
- Beginning implementation...

### 2026-03-02 -- Implementation Complete (all 13 items)

#### Critical fixes (items 1-4)

**Item 1: Concurrent reader limit** (`tcp_stats_kld.c:930-934`)
- Added `tcpstats_active_readers` volatile counter (line 149)
- `tcpstats_read()` atomically increments on entry, checks against `tcpstats_max_concurrent_readers` (default 32)
- Returns EBUSY if limit exceeded
- Decremented on all exit paths (line 1169)
- Sysctl tunable: `dev.tcpstats.max_concurrent_readers` (line 1275-1279)

**Item 2: Fix destroy_dev() under sx xlock** (`tcp_stats_kld.c:324-350`)
- Restructured into two functions:
  - `tcpstats_profile_detach()` (lines 342-350): removes profile from SLIST under sx xlock, returns detached profile
  - `tcpstats_profile_destroy_unlocked()` (lines 324-335): calls `destroy_dev()` and `free()` outside any lock
- Callers: detach under lock, release lock, then call destroy_unlocked
- Prevents sx xlock from being held during blocking `destroy_dev()`

**Item 3: Read iteration timeout** (`tcp_stats_kld.c:952-957, 996-1001`)
- Deadline computed at read start: `read_start + max_read_duration_ms * SBT_1MS` (lines 952-957)
- Checked every 256 sockets via `getsbinuptime() > deadline` (lines 996-1001)
- On timeout: unlocks current inpcb, breaks loop, returns partial results
- Default: 5000ms. Sysctl: `dev.tcpstats.max_read_duration_ms` (line 1280-1285)

**Item 4: Signal check in iteration loop** (`tcp_stats_kld.c:988-993`)
- `SIGPENDING(curthread)` checked every 256 sockets (TSF_CHECK_INTERVAL)
- On signal: unlocks inpcb, returns EINTR
- Allows SIGINT/SIGTERM to interrupt long reads

#### High priority performance (items 5-7)

**Item 5: Cache getsbinuptime() per read()** (`tcp_stats_kld.c:937, 770-772, 1141`)
- `now = getsbinuptime()` called once at read start (line 937, also used for rate limiting)
- Passed as parameter to `tcpstats_fill_record(rec, inp, field_mask, now)` (line 1141)
- Function signature updated (lines 770-772) to accept `sbintime_t now`
- Used in timer calculation (line 882): `tp->t_timers[i] - now`
- Eliminates N-1 redundant timecounter reads per read() call

**Item 6: field_mask gating** (`tcp_stats_kld.c:786-897`)
- 9 field groups gated by `if (field_mask & TSR_FIELDS_*)`:
  - `TSR_FIELDS_RTT` (line 786): RTT, rttvar, RTO, rttmin
  - `TSR_FIELDS_STATE` (line 796): window scale, options flags
  - `TSR_FIELDS_SEQUENCES` (line 811): snd_nxt, snd_una, snd_max, rcv_nxt, rcv_adv
  - `TSR_FIELDS_CONGESTION` (line 820): cwnd, ssthresh, windows, maxseg
  - `TSR_FIELDS_NAMES` (line 829): CC algo name, TCP stack name (eliminates pointer chases)
  - `TSR_FIELDS_COUNTERS` (line 839): retransmits, OOO, zerowin, dupacks, SACKs
  - `TSR_FIELDS_ECN` (line 848): ECN, DSACK, TLP fields
  - `TSR_FIELDS_TIMERS` (line 867): timer remaining times, rcvtime (eliminates getsbinuptime)
  - `TSR_FIELDS_BUFFERS` (line 889): socket buffer utilization (eliminates so pointer chase)
- field_mask cached per read at line 960; defaults to TSR_FIELDS_DEFAULT if 0 (lines 961-962)

**Item 7: IPv6 address filtering** (`tcp_stats_kld.c:686-711, 1096-1109, 1122-1135`)
- New helper `tsf_match_v6_prefix()` (lines 686-711): byte-by-byte comparison with partial-byte mask
- Local IPv6 filtering (lines 1096-1109): checks `inp->inp_inc.inc6_laddr` against filter via `tsf_match_v6_prefix()`
- Remote IPv6 filtering (lines 1122-1135): checks `inp->inp_inc.inc6_faddr` against filter via `tsf_match_v6_prefix()`
- Both integrated into existing `TSF_LOCAL_ADDR_MATCH` / `TSF_REMOTE_ADDR_MATCH` flag checks
- Skips comparison if filter address is `IN6_IS_ADDR_UNSPECIFIED` (unset)

#### Medium priority observability (items 8-13)

**Item 8: DTrace SDT probes** (`tcp_stats_kld.c:34-76`)
- Compile-time gated: `#ifdef TCPSTATS_DTRACE` (default: not defined)
- 7 probe points: read entry/done, filter skip/match, fill done, profile create/destroy
- TSF_DTRACE_* macros expand to `((void)0)` when disabled -- zero overhead in production
- Makefile: `CFLAGS+= -DTCPSTATS_DTRACE` to enable (line 7)

**Item 9: Sysctl stats counters** (`tcp_stats_kld.c:78-132, 94-97`)
- Tier 1 (always-on, lines 94-97): `tcpstats_active_fds`, `tcpstats_opens_total`, `tcpstats_reads_total`
- Tier 2 (compile-time `#ifdef TCPSTATS_STATS`, lines 100-132): 13 hot-loop counters
  - `records_emitted`, `sockets_visited`, 6x `sockets_skipped_*`, timing, errors
- TSF_STAT_INC/ADD/MAX macros expand to `((void)0)` when disabled
- All registered as sysctl nodes in MOD_LOAD (lines 1293-1378)

**Item 10: Read-path microbenchmark** (`test/bench_read_tcpstats.c`)
- 7 workloads: baseline, port filter, state filter, concurrent readers, buffer sweep, field_mask, no-filter
- Concurrent reader test uses multiple threads
- Outputs CSV: workload, connections, records, time_us, ns_per_record, records_per_sec

**Item 11: Loopback connection generator** (`test/gen_connections.c`)
- Creates up to 500K persistent loopback TCP connections
- Round-robin port allocation across configurable port range
- Used to populate the connection table for benchmarking

**Item 12: Max open fds cap** (`tcp_stats_kld.c:145, 658-662`)
- `tcpstats_max_open_fds` default 16 (line 145)
- `tcpstats_open()` atomically increments `tcpstats_active_fds`, returns EMFILE if over limit (lines 658-662)
- Decremented in `tcpstats_dtor()` on fd close
- Sysctl: `dev.tcpstats.max_open_fds` (line 1270-1274)

**Item 13: Per-fd rate limiting** (`tcp_stats_kld.c:938-945`)
- `tcpstats_min_read_interval_ms` default 0 (unlimited) (line 155)
- Checked after caching `now = getsbinuptime()` (lines 938-945)
- Returns EBUSY if elapsed since last read < minimum interval
- `sc->sc_last_read` updated on each successful read start (line 946)
- Sysctl: `dev.tcpstats.min_read_interval_ms` (line 1286-1291)

#### Additional hardening (beyond plan)

**Voluntary preemption** (`tcp_stats_kld.c:1004`)
- `kern_yield(PRI_USER)` called every 256 sockets (TSF_CHECK_INTERVAL)
- Prevents CPU monopolization during long iterations
- Co-located with signal and timeout checks in the periodic check block (lines 986-1005)

**Makefile compile-flag support** (`Makefile:7-13`)
- Three commented-out lines for enabling profiling features:
  - `-DTCPSTATS_DTRACE` for DTrace SDT probes
  - `-DTCPSTATS_STATS` for detailed per-socket statistics
  - Combined line for full profiling

**Sysctl tunables in MOD_LOAD** (`tcp_stats_kld.c:1269-1378`)
- 4 DoS protection tunables: max_open_fds, max_concurrent_readers, max_read_duration_ms, min_read_interval_ms
- 3 Tier 1 stats: reads_total, active_fds, opens_total
- 13 Tier 2 stats (conditional): records_emitted, sockets_visited, 6x skipped, timing, errors, timeouts, interrupts

### 2026-03-02 -- Live Integration Testing

Added 6 new test targets to `run-tests-freebsd.sh` for live kernel module integration testing. These require root and load/unload the kmod against real connections.

#### New targets

| Target | What it tests |
|--------|---------------|
| `live_smoke` | Kmod lifecycle: build, kldload, verify /dev/tcpstats, read, verify sysctl tree, kldunload |
| `live_bench` | Read-path benchmarks at 1K/10K/100K connections with gen_connections + bench_read_tcpstats |
| `live_stats` | Sysctl counter invariants: visited == emitted + sum(skipped_*), reads_total > 0, opens_total > 0 |
| `live_dtrace` | DTrace SDT probes fire (read-entry, read-done, filter-match); skips gracefully if dtrace unavailable |
| `live_dos` | DoS protection: EMFILE limit, read timeout partial results, EINTR signal interruption |
| `live_all` | Runs all live_* targets sequentially |

#### New files

- `test/test_dos_limits.c` -- Dedicated C program for DoS protection validation (EMFILE, timeout, EINTR sub-tests). Shell-based fd tests are inherently racy; this program holds fds open to reliably test EMFILE, and uses fork()+signal for EINTR.

#### Shared helpers added

- `require_root()` -- checks uid 0
- `tune_system()` -- raises kern.maxfiles, port range for high connection counts
- `kmod_build()` -- builds kmod with optional extra CFLAGS (e.g. `-DTCPSTATS_STATS`)
- `kmod_load()` / `kmod_unload()` -- manages kldload/kldunload with /dev/tcpstats verification
- `gen_start()` / `gen_stop()` -- manages gen_connections lifecycle in background
- `build_live_tools()` -- compiles gen_connections, read_tcpstats, bench_read_tcpstats into $WORK
- `live_cleanup()` -- kills gen_connections and unloads kmod (integrated into EXIT trap)

#### Design decisions

- `all` target does NOT include `live_*` -- they require root, modify kernel state, and are opt-in
- DTrace test skips (PASS, not FAIL) when dtrace is unavailable -- not all VMs have it
- EINTR test accepts both EINTR and successful read completion -- on fast hardware with few connections, the read may finish before the signal arrives
- Timeout test compares record count against expected_connections, not a fixed threshold -- the 50ms timeout should always yield partial results with 50K connections
- gen_connections sleep time scales with connection count: `count / 5000 + 2` seconds
