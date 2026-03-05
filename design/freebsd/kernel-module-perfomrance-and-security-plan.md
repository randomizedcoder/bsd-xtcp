# tcp_stats_kld Performance Analysis & Adversarial Resilience Plan

## Table of Contents

1. [Hot Loop Analysis: `tcpstats_read()` Line-by-Line Cost Model](#1-hot-loop-analysis)
   - 1.1 [Iterator Acquisition (lines 669-673)](#11-iterator-acquisition)
   - 1.2 [inp_next() Main Loop (line 675)](#12-inp_next-main-loop)
   - 1.3 [Buffer Residual Check (lines 676-679)](#13-buffer-residual-check)
   - 1.4 [Generation Count Check (lines 682-683)](#14-generation-count-check)
   - 1.5 [Credential Visibility Check (lines 686-687)](#15-credential-visibility-check)
   - 1.6 [IP Version Filter (lines 690-695)](#16-ip-version-filter)
   - 1.7 [State Filtering with intotcpcb() (lines 698-706)](#17-state-filtering)
   - 1.8 [Port Filtering - Linear Scan (lines 709-734)](#18-port-filtering)
   - 1.9 [IPv4 CIDR Address Filtering (lines 737-754)](#19-ipv4-cidr-filtering)
   - 1.10 [bzero of 320-byte Record (line 756)](#110-bzero)
   - 1.11 [tcpstats_fill_record() (line 757)](#111-fill-record)
   - 1.12 [uiomove() to Userspace (line 759)](#112-uiomove)
2. [Alternative Approaches to Profile](#2-alternative-approaches)
   - 2.1 [Port Filter: Linear Scan vs Bitmap vs Hash](#21-port-filter)
   - 2.2 [Record Fill: Partial Fill via field_mask](#22-field-mask)
   - 2.3 [Timer Extraction: Cache getsbinuptime()](#23-timer-optimization)
   - 2.4 [IPv6 Address Filtering (Missing Feature)](#24-ipv6-filtering)
   - 2.5 [Batch uiomove: Multi-Record Buffering](#25-batch-uiomove)
   - 2.6 [intotcpcb() Deduplication](#26-intotcpcb-dedup)
3. [Adversarial and Pathological Scenarios](#3-adversarial-scenarios)
   - 3.1 [Many Concurrent Readers (100+ fds)](#31-concurrent-readers)
   - 3.2 [Reader That Never Finishes (Lock Starvation)](#32-lock-starvation)
   - 3.3 [Rapid Open/Close Cycles](#33-rapid-open-close)
   - 3.4 [Profile Mutation Under Active Readers](#34-profile-mutation)
   - 3.5 [Extremely Large Connection Tables (1M+)](#35-large-tables)
   - 3.6 [Minimal Buffer Reads (Single Record per Syscall)](#36-minimal-buffer)
   - 3.7 [Tight Poll Loop (read/reset/read/reset)](#37-tight-poll)
   - 3.8 [Sysctl Write Storms](#38-sysctl-storms)
   - 3.9 [Module Unload While Readers Active](#39-module-unload)
   - 3.10 [Jail/VNET Interaction](#310-jail-vnet)
4. [Denial of Service Protections](#4-dos-protections)
   - 4.1 [Rate Limiting](#41-rate-limiting)
   - 4.2 [Resource Caps](#42-resource-caps)
   - 4.3 [Timeout Mechanisms](#43-timeouts)
   - 4.4 [Lock Ordering Fix](#44-lock-ordering)
   - 4.5 [Backpressure Mechanisms](#45-backpressure)
5. [Profiling Strategy](#5-profiling-strategy)
   - 5.1 [DTrace SDT Probes](#51-dtrace-probes)
   - 5.2 [Sysctl Statistics Counters](#52-sysctl-counters)
   - 5.3 [Read-Path Microbenchmark](#53-microbenchmark)
   - 5.4 [Load Generation (100K+ Connections)](#54-load-generation)
   - 5.5 [Metrics Collection Matrix](#55-metrics-matrix)
6. [Prioritized Implementation Order](#6-priorities)

---

## 1. Hot Loop Analysis

All references are to `tcp_stats_kld.c` unless noted.

### 1.1 Iterator Acquisition (lines 669-673)

```c
CURVNET_SET(TD_TO_VNET(curthread));                          // line 669
struct inpcb_iterator inpi = INP_ALL_ITERATOR(&V_tcbinfo,    // line 671
    INPLOOKUP_RLOCKPCB);
gencnt = V_tcbinfo.ipi_gencnt;                              // line 673
```

**Cost:** Fixed, once per `read()`. ~5ns total.
- `CURVNET_SET`: thread-local store (no-op on non-VNET kernels)
- `INP_ALL_ITERATOR`: struct initializer macro, no locks, no function calls
- `ipi_gencnt`: single 64-bit read from frequently-accessed `V_tcbinfo` (L1/L2 hot)

**Lock contention:** None. No locks acquired at this point.

### 1.2 inp_next() Main Loop (line 675)

```c
while ((inp = inp_next(&inpi)) != NULL) {
```

**Cost:** 30-80ns per call uncontended; 200ns+ under contention. Called N times where N = total TCP connections in this VNET.

**This is the single largest cost driver.** `inp_next()`:
1. Releases the read lock on the previous inpcb (`rw_runlock`)
2. Advances to the next inpcb in the hash/list
3. Acquires read lock on the next inpcb (`rw_rlock`)

Each lock operation is an atomic CAS: ~10-30ns uncontended.

**Cache pattern:** Walking the full inpcb list is cache-unfriendly at scale. Each `inpcb` (~500 bytes) is separately allocated via UMA. At 100K+ connections the working set exceeds L3 cache and iteration becomes DRAM-latency dominated (~60-100ns per miss).

| Connections | Working Set | Cache Behavior | Est. Iteration Time |
|------------|-------------|----------------|-------------------|
| 1,000 | ~0.5MB | L2/L3 hot | <0.3ms |
| 10,000 | ~5MB | L3 hot | ~3ms |
| 100,000 | ~50MB | L3 misses | ~30ms |
| 1,000,000 | ~500MB | DRAM dominated | ~300ms |

**Lock contention risk:** Every reader takes read locks on each inpcb. Read locks are shared (multiple readers OK), but any TCP stack operation needing a write lock (packet processing, timer, state change) must wait for ALL readers to release. With N concurrent readers iterating at different speeds, write lock starvation is possible.

### 1.3 Buffer Residual Check (lines 676-679)

```c
if (uio->uio_resid < (ssize_t)sizeof(rec)) {
    INP_RUNLOCK(inp);
    break;
}
```

**Cost:** ~1ns. Register comparison of `uio->uio_resid` (on kernel stack, L1 hot).

**Behavior:** When buffer exhausted, explicitly unlocks current inpcb and breaks. Sets `sc_done = 1` at line 769. No resume mechanism exists -- next read returns EOF.

### 1.4 Generation Count Check (lines 682-683)

```c
if (inp->inp_gencnt > gencnt)
    continue;
```

**Cost:** ~1ns. Single 64-bit comparison. Both values cache-resident.

**Purpose:** Snapshot consistency -- skips connections created after read() started.

### 1.5 Credential Visibility Check (lines 686-687)

```c
if (cr_canseeinpcb(sc->sc_cred, inp) != 0)
    continue;
```

**Cost:** 10-30ns (no jails, no MAC); 50-200ns (with MAC policies).

`cr_canseeinpcb()` (defined in `kern/kern_prot.c`) checks:
1. Jail containment (`prison_check_ip*`)
2. MAC framework (`mac_inpcb_check_visible()`)
3. UID visibility (`security.bsd.see_other_uids`)
4. GID visibility (`security.bsd.see_other_gids`)

For root without jails, this early-returns after `priv_check_cred()` succeeds.

**Cache note:** `sc->sc_cred` stays L1 hot (accessed every socket). `inp->inp_socket->so_cred` follows a pointer chain that may miss.

### 1.6 IP Version Filter (lines 690-695)

```c
if ((sc->sc_filter.flags & TSF_IPV4_ONLY) && !(inp->inp_vflag & INP_IPV4))
    continue;
if ((sc->sc_filter.flags & TSF_IPV6_ONLY) && !(inp->inp_vflag & INP_IPV6))
    continue;
```

**Cost:** ~2ns. Two bitwise ANDs, short-circuit evaluated. When no filter set (flags=0), first AND is false and second check is skipped.

### 1.7 State Filtering with intotcpcb() (lines 698-706)

```c
struct tcpcb *tp = intotcpcb(inp);                           // line 699
if (tp != NULL) {
    if (sc->sc_filter.state_mask != 0xFFFF &&                // line 701
        !(sc->sc_filter.state_mask & (1 << tp->t_state)))    // line 703
        continue;
}
```

**Cost:** ~2ns when `state_mask == 0xFFFF` (common unfiltered case). ~60-100ns on first access to `tcpcb` (cache miss -- separate UMA zone from `inpcb`).

**`intotcpcb(inp)`** is a macro: `((struct tcpcb *)(inp)->inp_ppcb)`. The `tcpcb` is a separate allocation. At scale, this pointer dereference is a cache miss.

**Redundancy:** `intotcpcb(inp)` is called 4 times total per matching socket (lines 699, 521, 552, and indirectly via fill functions). First call pays the miss; subsequent are L1 hot.

### 1.8 Port Filtering - Linear Scan (lines 709-734)

```c
// Local port filter (lines 712-718):
for (int i = 0; i < TSF_MAX_PORTS &&
    sc->sc_filter.local_ports[i] != 0; i++) {
    if (lport == sc->sc_filter.local_ports[i]) {
        found = 1; break;
    }
}
// Remote port filter (lines 725-731): identical structure
```

**Cost:** O(k) per direction, k = configured ports, max 8. The array is 16 bytes (fits in one cache line within the softc).
- 1 port configured: ~2-4ns
- 8 ports (worst case no match): ~8-16ns
- No filter set (flag not in `flags`): ~1ns (short-circuit)

### 1.9 IPv4 CIDR Address Filtering (lines 737-754)

```c
if ((inp->inp_inc.inc_laddr.s_addr & sc->sc_filter.local_mask_v4.s_addr) !=
    (sc->sc_filter.local_addr_v4.s_addr & sc->sc_filter.local_mask_v4.s_addr))
    continue;
```

**Cost:** O(1), ~2-3ns. Two 32-bit ANDs and one comparison. Textbook CIDR match.

**GAP: IPv6 address filtering is NOT implemented in the read path.** The filter parser populates `local_addr_v6`/`remote_addr_v6` and prefix lengths, but `tcpstats_read()` has no code to check them. IPv6 sockets with address filters pass through unfiltered.

### 1.10 bzero of 320-byte Record (line 756)

```c
bzero(&rec, sizeof(rec));
```

**Cost:** ~5-10ns. Stack-allocated, 5 cache lines. Compiles to SSE zero stores with -O2.

The spare fields (52 bytes) and padding bytes are only zeroed by this call -- without it they'd leak stack contents (information leak).

### 1.11 tcpstats_fill_record() (line 757, defined lines 546-651)

**Total estimated cost per matching socket: 200-400ns typical, 400-800ns worst case (all cold).**

Breakdown by sub-operation:

| Operation | Lines | Cost | Notes |
|-----------|-------|------|-------|
| `tcpstats_fill_identity()` | 497-538 | 20-80ns | Address copy, port ntohs, state read |
| RTT calculations | 557-560 | 15ns | 3 multiply-shift ops |
| Window/options flags | 563-571 | 3-5ns | Bit flag checks |
| Sequence numbers | 574-578 | 5ns | 5x 32-bit copy from tcpcb |
| Congestion fields | 581-585 | 5ns | 5x 32-bit copy |
| **CC algo name strlcpy** | 588-590 | 10-30ns | `CC_ALGO(tp)->name` -- 2 pointer chases through CC vtable |
| **Stack name strlcpy** | 591-593 | 10-30ns | `tp->t_fb->tfb_tcp_block_name` -- 1 pointer chase |
| Counter fields | 596-600 | 5ns | 5x 32-bit copy |
| ECN fields | 603-609 | 3-5ns | Conditional + copies |
| DSACK + TLP | 612-617 | 3-5ns | Simple copies |
| **getsbinuptime()** | 621 | **20-50ns** | Timecounter hardware read |
| **Timer loop (TT_N=5)** | 630-637 | 25-50ns | 5 iterations, sbintime comparison + division |
| `tsr_rcvtime` | 639 | 5ns | Global `ticks` read + arithmetic |
| **Socket buffer reads** | 644-648 | 10-70ns | `inp->inp_socket` pointer chase (may cache miss) |

**Key cache miss points (per matching socket):**
1. `inp->inp_ppcb` -> `tcpcb` (separate UMA zone): ~60ns if cold
2. `tp->t_cc->cc_algo` -> CC algorithm struct: L2 hot (shared singleton)
3. `inp->inp_socket` -> socket struct (separate UMA zone): ~60ns if cold
4. `so->so_cred->cr_uid` -> ucred struct (separate alloc): ~60ns if cold

### 1.12 uiomove() to Userspace (line 759)

```c
error = uiomove(&rec, sizeof(rec), uio);
```

**Cost:** ~20-40ns. 320 bytes kernel->user copy. Involves SMAP transitions (STAC/CLAC) on x86.

**Page fault risk:** If user buffer page is not resident, `uiomove()` triggers a page fault while holding the inpcb read lock. The fault handler sleeps, causing the lock to be held for milliseconds. This is a latency spike vector (see Section 3.2).

---

## 2. Alternative Approaches to Profile

### 2.1 Port Filter: Linear Scan vs Bitmap vs Hash

**Current:** Linear scan over `local_ports[8]` at lines 712-718.

| Approach | Cost per check | Memory | Complexity | When better |
|----------|---------------|--------|-----------|-------------|
| **Linear scan (current)** | O(k), k<=8 | 16 bytes | Low | k <= 8 (current) |
| Bitmap (65536 bits) | O(1) | 8KB per direction | Medium | k > 64 |
| Sorted + binary search | O(log k) | 16 bytes | Medium | Never at k=8 |
| Hash table | O(1) amortized | Variable | High | k > 32 |

**Recommendation:** Keep linear scan. At max 8 ports, it is optimal -- the 16-byte array fits in one cache line. Bitmap only wins at k>64 and would blow the 256-byte filter struct limit.

### 2.2 Record Fill: Partial Fill via field_mask

**Current:** `tcpstats_fill_record()` always fills ALL 320 bytes regardless of what the reader needs.

The `field_mask` field exists in `tcpstats_filter` (tcp_stats_kld.h:210) and the parser populates it (`tsf_parse_field_list()` in tcp_stats_filter_parse.c:969-1009), but it is **unused in the read path**.

**Proposed:** Gate each field group on `sc->sc_filter.field_mask`:

| Group Skip | Bytes Saved | Time Saved | Key Savings |
|-----------|-------------|-----------|-------------|
| Timers | 24 | **50-80ns** | Eliminates `getsbinuptime()` call |
| Names (CC+stack) | 32 | 20-60ns | Eliminates pointer chases through CC vtable |
| Buffers | 16 | 10-70ns | Eliminates socket pointer chase |
| RTT | 16 | 15ns | |
| Sequences | 20 | 5ns | |
| Counters | 20 | 5ns | |
| ECN/DSACK/TLP | 32 | 5-10ns | |

**Maximum per-socket savings:** 140-380ns (if reader only needs identity + state).
**At 100K matching sockets:** 14-38ms saved per read().

**Implementation:** Add `if (field_mask & TSR_FIELDS_TIMERS) { ... }` around each group in `tcpstats_fill_record()`. ~7 branches added, all predicted correctly (constant field_mask per read).

### 2.3 Timer Extraction: Cache getsbinuptime()

**Current:** `getsbinuptime()` is called inside `tcpstats_fill_record()` at line 621, once per matching socket.

**Proposed:** Call it once in `tcpstats_read()` before the loop and pass `now` as a parameter to `tcpstats_fill_record()`.

**Savings:** For N matching sockets: eliminates (N-1) calls to `getsbinuptime()` (each ~20-50ns).
- 1000 matches: saves ~30us
- 10000 matches: saves ~300us

This is the single highest-value micro-optimization for the read path.

### 2.4 IPv6 Address Filtering (Missing Feature)

The parser populates `local_addr_v6`, `local_prefix_v6`, `remote_addr_v6`, `remote_prefix_v6` (tcp_stats_filter_parse.c:712-863), but `tcpstats_read()` has no IPv6 address match code.

**Implementation:** Add after line 754:
```c
/* IPv6 address filtering */
if (sc->sc_filter.flags & TSF_LOCAL_ADDR_MATCH) {
    if (inp->inp_vflag & INP_IPV6) {
        if (!tsf_match_v6_prefix(&inp->inp_inc.inc6_laddr,
            &sc->sc_filter.local_addr_v6,
            sc->sc_filter.local_prefix_v6))
            continue;
    }
}
```

Cost per check: ~2-3ns (byte comparison up to prefix length). For /64: one 64-bit comparison.

### 2.5 Batch uiomove: Multi-Record Buffering

**Current:** One `uiomove()` of 320 bytes per matching socket.

**Proposed:** Buffer 16 records in kernel memory, then one `uiomove()` of 5120 bytes.

**Savings:** Eliminates ~15 SMAP transitions per batch (~75ns). At 1000 matches: saves ~14us.

**Tradeoff:** 5KB on kernel stack (15-30% of 16-32KB stack). Would need `malloc` in `open()` instead.

**Recommendation:** Low priority. Savings are modest vs complexity.

### 2.6 intotcpcb() Deduplication

`intotcpcb(inp)` called at lines 699, 521, 552 (4 times total). Each is `(struct tcpcb *)(inp)->inp_ppcb` -- a single load.

**Proposed:** Pass `tp` as parameter to `tcpstats_fill_record()` and `tcpstats_fill_identity()`.

**Savings:** ~2-4ns. The compiler may already CSE this. Low priority but improves clarity.

---

## 3. Adversarial and Pathological Scenarios

### 3.1 Many Concurrent Readers (100+ fds) -- SEVERITY: HIGH

**Attack:** Open 100+ fds, call `read()` on all simultaneously.

**Impact:** Each reader holds inpcb read locks. TCP stack write operations (packet processing, timers, state changes) must wait for ALL readers to release. With 100 readers at different iteration speeds, write lock starvation causes packet processing stalls, retransmission delays, connection timeouts.

**Current protection:** NONE. Device permissions (`0440`, root:network) control access but no rate limiting.

### 3.2 Reader That Never Finishes (Lock Starvation) -- SEVERITY: HIGH

**Attack:** Reader opens device, reads with large buffer. During `uiomove()` (line 759), the inpcb read lock is held. If user buffer page faults (not resident, `MADV_DONTNEED`, `mprotect` tricks), `uiomove()` sleeps while holding the lock.

**Impact:** One connection's inpcb is locked for milliseconds. A malicious reader can rotate through connections, stalling each. TCP processing for each connection pauses while its inpcb is read-locked.

**Mitigation:** The lock is only on ONE inpcb at a time (inp_next releases previous before locking next), limiting blast radius.

### 3.3 Rapid Open/Close Cycles -- SEVERITY: LOW

**Attack:** Tight loop of `open() -> close()` without reading.

**Impact:** ~500ns-1us per cycle. Memory allocator churn (~280 bytes per cycle). Does not hold any inpcb locks. Limited to consuming CPU and allocator resources.

### 3.4 Profile Mutation Under Active Readers -- SEVERITY: HIGH

**Attack/Bug:** Delete profile via sysctl while readers have open fds from that profile device.

**Impact:** `tcpstats_profile_destroy()` (line 180-193) calls `destroy_dev(prof->dev)` at line 189 **while holding `tcpstats_profile_lock` (sx xlock)**. `destroy_dev()` blocks until all fds close. If a reader is stuck, the sx xlock is held indefinitely, preventing ALL profile operations (create, delete, list).

**Root cause:** `destroy_dev()` called under lock. See Section 4.4 for fix.

### 3.5 Extremely Large Connection Tables (1M+) -- SEVERITY: MEDIUM

**Scenario:** CDN/load balancer with 1M TCP connections.

**Impact:** Every `read()` iterates ALL inpcbs regardless of filter selectivity. Even with a filter matching 1 connection, all 1M are visited.

No index/hash for filtered reads exists -- `INP_ALL_ITERATOR` is a full scan. This is architectural (FreeBSD kernel limitation).

### 3.6 Minimal Buffer Reads -- SEVERITY: MEDIUM (perf antipattern)

**Scenario:** Reader provides buffer of exactly 320 bytes.

**Impact:** Gets 1 record, then EOF (`sc_done = 1`). Must `ioctl(TCPSTATS_RESET)` and re-read, which re-scans ALL connections from scratch. With 1M connections and 1000 matches: 1000 * full scan = catastrophic performance.

### 3.7 Tight Poll Loop -- SEVERITY: MEDIUM

**Scenario:** `read() -> TCPSTATS_RESET -> read() -> TCPSTATS_RESET` in tight loop.

**Impact:** Continuous full iteration of all inpcbs. At 10K connections: ~3ms per cycle = ~333 full scans/sec. Continuous read lock pressure on all inpcbs.

### 3.8 Sysctl Write Storms -- SEVERITY: LOW

**Scenario:** Rapid profile create/delete via sysctl.

**Impact:** Serialized by sx xlock. Rate limited by `make_dev_credf()` / `destroy_dev()` overhead (~100-1000us each). 16-profile limit caps resource usage.

### 3.9 Module Unload While Readers Active -- SEVERITY: LOW

`tcp_stats_kld_modevent(MOD_UNLOAD)` (line 884-893) calls `destroy_dev()` which blocks until all fds close. Module unload blocks but does not crash. Risk: stuck reader prevents module unload (and potentially system shutdown).

### 3.10 Jail/VNET Interaction -- SEVERITY: LOW

`CURVNET_SET(TD_TO_VNET(curthread))` at line 669 correctly scopes iteration to calling thread's VNET. `cr_canseeinpcb()` enforces jail visibility. Isolation is correct by construction.

**Edge case to verify:** Jail destruction while reader has open fd -- VNET may be destroyed before fd is closed.

---

## 4. Denial of Service Protections

### 4.1 Rate Limiting

**Protection 1: Per-fd read interval.**
Add `sbintime_t sc_last_read` to `tcpstats_softc`. In `tcpstats_read()`, reject if elapsed < configurable minimum (default 100ms = 10 reads/sec).
Sysctl: `dev.tcpstats.min_read_interval_ms`.

**Protection 2: Global concurrent reader limit.**
Atomic counter `tcpstats_active_readers`. Increment in `tcpstats_read()`, decrement on return. Reject with `EBUSY` if above threshold (default 32).
Sysctl: `dev.tcpstats.max_concurrent_readers`.

### 4.2 Resource Caps

**Cap 1: Maximum open fds.**
Track count in `tcpstats_open()` / `tcpstats_dtor()`. Reject with `EMFILE` above threshold (default 16).
Sysctl: `dev.tcpstats.max_open_fds`.
Note: The default of 16 is deliberately conservative for production. For the 32-concurrent-reader stress test (Section 5.3, workload 4), the test harness must raise this limit via sysctl before starting: `sysctl dev.tcpstats.max_open_fds=64`.

**Cap 2: Read iteration timeout.**
Record `getsbinuptime()` at start of `tcpstats_read()`. Every 1000 sockets, check elapsed. Break loop if exceeding threshold (default 5 seconds). Return partial results and set `sc_done = 1`.
Sysctl: `dev.tcpstats.max_read_duration_ms`.

### 4.3 Timeout Mechanisms

**Mechanism 1: Read timeout (see Cap 2 above).**

**Mechanism 2: `destroy_dev()` non-blocking.**
Use `destroy_dev_sched()` (schedules destruction) instead of blocking `destroy_dev()` for profile deletion. Or restructure to release sx xlock before calling `destroy_dev()`.

### 4.4 Lock Ordering Fix -- CRITICAL

**Bug:** `tcpstats_profile_destroy()` (line 180-193) calls `destroy_dev()` while holding `tcpstats_profile_lock` (sx xlock). `destroy_dev()` blocks if fds are open, holding the sx xlock indefinitely.

**Fix:** Restructure `tcpstats_profile_destroy()`:
1. Under sx xlock: remove from SLIST, decrement count, save `prof->dev` pointer, NULL out `prof->dev`.
2. Release sx xlock.
3. Call `destroy_dev()` outside any lock (may block).
4. Free profile struct.

This prevents the sx lock from being held during the potentially-blocking `destroy_dev()`.

### 4.5 Backpressure Mechanisms

**Mechanism 1: Signal-interruptible read.**
Every N sockets (e.g., 256), check `SIGPENDING(curthread)`. If signal pending, break loop and return `EINTR`. Currently the loop runs uninterruptibly.

**Mechanism 2: Voluntary preemption.**
Every N sockets, call `kern_yield(PRI_USER)` to allow other threads to run. Prevents CPU monopolization during long iterations. At N=256 and 100K sockets: ~400 yields, ~0.4-2ms overhead.

---

## 5. Profiling Strategy

### 5.1 DTrace SDT Probes -- Compile-Time Feature

**Performance concern:** SDT probes, while designed to be low-overhead when not actively traced, still have non-zero cost: each probe site is a NOP-sled or DTRACE_PROBE macro that the compiler cannot fully optimize away. In a hot loop iterating 100K+ sockets, per-socket probes (filter skip/match, fill timing) add measurable overhead even when DTrace is not attached.

**Design:** Gate ALL DTrace instrumentation behind a compile-time flag `TCPSTATS_DTRACE`.

- **Default build (production):** `TCPSTATS_DTRACE` is NOT defined. All probe macros expand to nothing. Zero runtime cost.
- **Profiling build:** Compile with `-DTCPSTATS_DTRACE` to enable probes. Operators who want DTrace in production can rebuild with this flag.
- **Makefile integration:** Add a `TCPSTATS_CFLAGS` variable:
  ```makefile
  # In Makefile: uncomment to enable DTrace SDT probes
  # TCPSTATS_CFLAGS+= -DTCPSTATS_DTRACE
  CFLAGS+= ${TCPSTATS_CFLAGS}
  ```

**Implementation in tcp_stats_kld.c:**
```c
#ifdef TCPSTATS_DTRACE
#include <sys/sdt.h>
SDT_PROVIDER_DEFINE(tcpstats);
SDT_PROBE_DEFINE2(tcpstats, , read, entry, "uio_resid", "filter_flags");
SDT_PROBE_DEFINE3(tcpstats, , read, done, "error", "records_emitted", "elapsed_ns");
SDT_PROBE_DEFINE2(tcpstats, , filter, skip, "inpcb_ptr", "reason_code");
SDT_PROBE_DEFINE1(tcpstats, , filter, match, "inpcb_ptr");
SDT_PROBE_DEFINE2(tcpstats, , fill, done, "elapsed_ns", "record_size");
SDT_PROBE_DEFINE1(tcpstats, , profile, create, "name");
SDT_PROBE_DEFINE1(tcpstats, , profile, destroy, "name");

#define TSF_DTRACE_READ_ENTRY(resid, flags) \
    SDT_PROBE2(tcpstats, , read, entry, (resid), (flags))
#define TSF_DTRACE_READ_DONE(err, nrec, ns) \
    SDT_PROBE3(tcpstats, , read, done, (err), (nrec), (ns))
#define TSF_DTRACE_FILTER_SKIP(inp, reason) \
    SDT_PROBE2(tcpstats, , filter, skip, (inp), (reason))
#define TSF_DTRACE_FILTER_MATCH(inp) \
    SDT_PROBE1(tcpstats, , filter, match, (inp))
#define TSF_DTRACE_FILL_DONE(ns, sz) \
    SDT_PROBE2(tcpstats, , fill, done, (ns), (sz))
#define TSF_DTRACE_PROFILE_CREATE(name) \
    SDT_PROBE1(tcpstats, , profile, create, (name))
#define TSF_DTRACE_PROFILE_DESTROY(name) \
    SDT_PROBE1(tcpstats, , profile, destroy, (name))
#else
#define TSF_DTRACE_READ_ENTRY(resid, flags)    ((void)0)
#define TSF_DTRACE_READ_DONE(err, nrec, ns)    ((void)0)
#define TSF_DTRACE_FILTER_SKIP(inp, reason)    ((void)0)
#define TSF_DTRACE_FILTER_MATCH(inp)           ((void)0)
#define TSF_DTRACE_FILL_DONE(ns, sz)           ((void)0)
#define TSF_DTRACE_PROFILE_CREATE(name)        ((void)0)
#define TSF_DTRACE_PROFILE_DESTROY(name)       ((void)0)
#endif
```

The code uses `TSF_DTRACE_*` macros at each probe site. Without `-DTCPSTATS_DTRACE`, these compile to `((void)0)` and are completely eliminated by the optimizer.

**Probe categories by overhead:**

| Probe | Frequency | Overhead when enabled | Overhead when disabled |
|-------|-----------|----------------------|----------------------|
| read entry/done | 1x per read() | ~10ns (timestamp) | 0 |
| filter skip | N per read (hot) | ~5ns per socket | 0 |
| filter match | M per read (hot) | ~5ns per socket | 0 |
| fill done | M per read (hot) | ~15ns (timestamp delta) | 0 |
| profile create/destroy | Rare | ~5ns | 0 |

**DTrace one-liners (when compiled with -DTCPSTATS_DTRACE):**
```sh
# Read latency histogram (microseconds)
dtrace -n 'tcpstats:::read-entry { self->ts = timestamp; }
           tcpstats:::read-done /self->ts/ {
               @["read_us"] = quantize((timestamp - self->ts) / 1000); }'

# Filter skip reasons
dtrace -n 'tcpstats:::filter-skip { @reasons[arg1] = count(); }'

# Fill time per record
dtrace -n 'tcpstats:::fill-done { @["fill_ns"] = quantize(arg0); }'
```

### 5.2 Sysctl Statistics Counters -- Also Compile-Time Gated

The sysctl statistics counters (per-skip-reason counts, timing) also have non-trivial cost in the hot loop: each `atomic_add_64` on a shared counter causes cache line bouncing between CPUs when multiple readers are active.

**Design:** Two tiers of counters:

**Tier 1 -- Always enabled (low-frequency, outside hot loop):**
| Counter | Type | Description |
|---------|------|-------------|
| `reads_total` | uint64 | Total read() calls |
| `active_fds` | uint32 | Currently open fds |
| `opens_total` | uint64 | Total open() calls |

These are incremented once per read()/open(), not per socket. Overhead: negligible.

**Tier 2 -- Compile-time gated (`-DTCPSTATS_STATS`, per-socket counters):**
| Counter | Type | Description |
|---------|------|-------------|
| `records_emitted` | uint64 | Total records copied to userspace |
| `sockets_visited` | uint64 | Total inpcbs examined |
| `sockets_skipped_gencnt` | uint64 | Skipped: generation count |
| `sockets_skipped_cred` | uint64 | Skipped: credential visibility |
| `sockets_skipped_ipver` | uint64 | Skipped: IP version filter |
| `sockets_skipped_state` | uint64 | Skipped: state filter |
| `sockets_skipped_port` | uint64 | Skipped: port filter |
| `sockets_skipped_addr` | uint64 | Skipped: address filter |
| `read_duration_ns_total` | uint64 | Cumulative read duration |
| `read_duration_ns_max` | uint64 | Max single read duration |
| `uiomove_errors` | uint64 | uiomove failures |

**Makefile:**
```makefile
# Uncomment for detailed per-socket statistics (adds overhead in hot loop)
# TCPSTATS_CFLAGS+= -DTCPSTATS_STATS
# Uncomment for DTrace SDT probes
# TCPSTATS_CFLAGS+= -DTCPSTATS_DTRACE
# Enable both for full profiling:
# TCPSTATS_CFLAGS+= -DTCPSTATS_DTRACE -DTCPSTATS_STATS
```

Implementation: Same macro pattern as DTrace -- `TSF_STAT_INC(counter)` expands to `atomic_add_64(&tcpstats_stats.counter, 1)` when enabled, `((void)0)` when disabled.

### 5.3 Read-Path Microbenchmark

Build `test/bench_read_tcpstats.c` (new file, modeled on existing `bench_filter_parse.c`):

**Workloads:**
1. **Baseline:** No filter, measure full scan time
2. **Port filter selectivity:** 100% match, 50% match, 10% match, 0% match
3. **State filter:** ESTABLISHED only vs all
4. **Concurrent readers:** 1, 2, 4, 8, 16, 32 threads
5. **Buffer size sweep:** 320B, 4KB, 64KB, 1MB, 4MB
6. **Connection count scaling:** 100, 1K, 10K, 100K

**Output:** CSV with columns: workload, connections, records, time_us, ns_per_record, records_per_sec

### 5.4 Load Generation (100K+ Connections)

**Method 1: tcp-echo (already in repo)**
```sh
tcp-echo server --ports 9001-9008 &
tcp-echo client --ports 9001-9008 --connections 100000
```

**Method 2: FreeBSD tuning for large counts**
```sh
sysctl kern.maxfiles=500000
sysctl kern.maxfilesperproc=250000
sysctl net.inet.tcp.maxtcptw=200000
sysctl net.inet.ip.portrange.first=1024
sysctl net.inet.ip.portrange.last=65535
```

**Method 3: Loopback connection generator (new C tool)**
Creates N persistent loopback TCP connections using `connect()` to `127.0.0.1`. Each connection uses ~1KB kernel memory. 100K = ~100MB.

**Method 4: Multi-address for CIDR testing**
```sh
ifconfig lo0 alias 10.0.1.1/24
ifconfig lo0 alias 10.0.2.1/24
```

### 5.5 Metrics Collection Matrix

| Metric | Tool | When | Target |
|--------|------|------|--------|
| Read latency (wall) | bench_read_tcpstats | Every benchmark | <10ms / 10K conns |
| Records/sec throughput | bench_read_tcpstats | Every benchmark | >100K rec/sec |
| Per-socket filter time | DTrace | Profiling runs | <100ns avg |
| Per-socket fill time | DTrace | Profiling runs | <300ns avg |
| Lock hold time/inpcb | DTrace lockstat | Contention analysis | <1us |
| Cache miss rate | hwpmc (LLC-LOAD-MISSES) | Scaling tests | Characterize |
| TLB misses | hwpmc | 100K+ tests | Characterize |
| TCP stack stall time | DTrace on TCP input | Concurrent readers | <1ms / 10K conns |

**hwpmc commands:**
```sh
# Cache misses during benchmark
pmcstat -p LLC-LOAD-MISSES -d /root/bench_read_tcpstats 100
# Instructions per cycle
pmcstat -p INST_RETIRED -p CPU_CLK_UNHALTED -d /root/bench_read_tcpstats 100
# Per-function profile for kmod
pmcstat -p LLC-LOAD-MISSES -O /tmp/pmc.out /root/bench_read_tcpstats 100
pmcstat -R /tmp/pmc.out -z10 -m /boot/kernel/tcp_stats_kld.ko
```

---

## 6. Prioritized Implementation Order

### Critical (implement before production use)

1. **Concurrent reader limit** (4.2) -- prevent write lock starvation from 100+ readers
   - Files: `tcp_stats_kld.c` (add atomic counter + check in `tcpstats_read()`)
2. **Fix `destroy_dev()` under sx xlock** (4.4) -- `tcpstats_profile_destroy()` line 189
   - Files: `tcp_stats_kld.c` lines 179-193
3. **Read iteration timeout** (4.3) -- prevent multi-second blocking at 1M+ connections
   - Files: `tcp_stats_kld.c` in `tcpstats_read()` loop
4. **Signal check in iteration loop** (4.5) -- allow SIGINT to interrupt long reads
   - Files: `tcp_stats_kld.c` in `tcpstats_read()` loop

### High Priority (significant performance improvement)

5. **Cache `getsbinuptime()` per read()** (2.3) -- call once, pass to fill_record
   - Files: `tcp_stats_kld.c` lines 621, 546-651, 654-771
6. **Implement field_mask gating** (2.2) -- skip timer/name/buffer fill when not needed
   - Files: `tcp_stats_kld.c` `tcpstats_fill_record()` lines 546-651
7. **Add IPv6 address filtering** (2.4) -- complete the half-built feature
   - Files: `tcp_stats_kld.c` after line 754

### Medium Priority (observability)

8. **Add DTrace SDT probes as compile-time feature** (5.1)
   - Files: `tcp_stats_kld.c` (add `#ifdef TCPSTATS_DTRACE` block + TSF_DTRACE_* macros), `Makefile` (add TCPSTATS_CFLAGS)
   - Default: disabled (zero overhead). Enable with `-DTCPSTATS_DTRACE` for benchmarking or operator-opted production use.
9. **Add sysctl statistics counters (two-tier)** (5.2)
   - Tier 1 (always-on): reads_total, active_fds, opens_total -- outside hot loop
   - Tier 2 (compile-time `-DTCPSTATS_STATS`): per-skip-reason counts, timing -- hot loop counters
   - Files: `tcp_stats_kld.c` (counter struct + macros + SYSCTL_ADD_U64 in MOD_LOAD)
10. **Build read-path microbenchmark** (5.3)
    - Files: new `test/bench_read_tcpstats.c`
    - Note: concurrent reader test (32 threads) must first `sysctl dev.tcpstats.max_open_fds=64` (default is 16)
11. **Build loopback connection generator** (5.4)
    - Files: new `test/gen_connections.c`

### Low Priority (marginal gains)

12. Port filter optimization (2.1) -- already optimal at N=8
13. bzero optimization -- ~5ns savings, not worth the complexity
14. intotcpcb dedup (2.6) -- ~2-4ns, compiler likely handles it
15. Batch uiomove (2.5) -- modest savings, adds stack pressure

---

## Verification Plan

1. **Unit tests:** Extend `test/test_filter_parse.c` for IPv6 address match in read path
2. **Stress test:** 32 concurrent readers + 10K connections, verify no hangs or panics
   - Must first: `sysctl dev.tcpstats.max_open_fds=64` (default 16 is too low for this test)
3. **DoS simulation:** Run tight poll loop for 60s, verify system remains responsive
4. **Profile deletion under load:** Delete profile while reader has open fd, verify no deadlock (tests the lock ordering fix from 4.4)
5. **Benchmark regression:** Compare read times before/after each optimization
6. **DTrace validation:** Build with `-DTCPSTATS_DTRACE`, run DTrace scripts, verify probe output
7. **Stats validation:** Build with `-DTCPSTATS_STATS`, run workloads, verify counter consistency (visited = emitted + sum of all skipped)
8. **hwpmc profiling:** Collect cache miss data at 1K, 10K, 100K connection counts
9. **Production build validation:** Build WITHOUT `-DTCPSTATS_DTRACE` and `-DTCPSTATS_STATS`, verify zero overhead (compare benchmark numbers vs instrumented build)
