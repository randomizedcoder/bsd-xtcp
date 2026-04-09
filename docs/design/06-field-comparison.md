[← Back to README](../../README.md)

# Performance Budget, Field Comparison, and Open Questions

## Table of Contents

- [9. Performance Budget](#9-performance-budget)
- [10. Fields Available vs. Linux `tcp_info` — Comparison](#10-fields-available-vs-linux-tcp_info--comparison)
- [12. Open Questions and Future Considerations](#12-open-questions-and-future-considerations)

---

## 9. Performance Budget

Target: < 1% CPU, < 10 MB RSS on a developer machine with ~500 TCP sockets.

| Operation | Estimated Cost per Call | Frequency |
|---|---|---|
| `sysctl tcp.pcblist` (500 sockets) | ~0.5ms | Every 1s |
| `sysctl kern.file` (all FDs) | ~1-2ms | Every 30s |
| `sysctl tcp.stats` | ~0.01ms | Every 30s |
| `sysctl tcp.states` | ~0.01ms | Every 60s |
| PID→command resolution (50 unique PIDs) | ~0.5ms | Every 30s |
| JSON serialization (500 records) | ~1ms | Every 1s |

Total per-second overhead: ~1.5ms of CPU time = 0.15% of one core. Well within budget.

Memory: 500 sockets * ~512 bytes/record = 250 KB working set, plus buffers. Under 5 MB total.

---

## 10. Fields Available vs. Linux `tcp_info` — Comparison

This table documents the gap between the FreeBSD Developer Edition and the full Linux product, as referenced in the business plan's positioning strategy.

| Field | Linux `tcp_info` | FreeBSD `xtcpcb` | FreeBSD KLD | macOS `pcblist_n` | macOS `TCP_CONNECTION_INFO` |
|---|---|---|---|---|---|
| TCP state | Yes | Yes | Yes | Yes (`t_state`) | Yes (`tcpi_state`) |
| RTT (smoothed) | Yes (usec) | **No** | Yes (usec) | Yes (`t_srtt`, ticks) | Yes (`tcpi_srtt`, **ms**) |
| RTT (most recent) | No | **No** | No | No | Yes (`tcpi_rttcur`, ms) |
| RTT variance | Yes | **No** | Yes | Yes (`t_rttvar`, ticks) | Yes (`tcpi_rttvar`) |
| RTO | Yes | **No** | Yes | Yes (`t_rxtcur`) | Yes (`tcpi_rto`, ms) |
| Min RTT | Yes | **No** | Yes | Yes (`t_rttmin`) | **No** |
| Send cwnd | Yes | Yes | Yes | Yes | Yes |
| Slow start threshold | Yes | Yes | Yes | Yes | Yes |
| Send/recv window | Yes | Yes | Yes | Yes | Yes |
| MSS | Yes | Yes | Yes | Yes (`t_maxseg`) | Yes |
| Retransmit count | Yes | Yes | Yes | No (in pcblist) | Yes (`tcpi_txretransmitpackets`) |
| OOO packet count | Yes | Yes | Yes | No (in pcblist) | Yes (`tcpi_rxoutoforderbytes`) |
| Zero-window count | Yes | Yes | Yes | No | **No** |
| CC algorithm name | Yes | Yes | Yes | **No** | **No** |
| TCP stack name | N/A | Yes | Yes | **No** | **No** |
| Timer state (5 timers) | Partial | Yes | Yes | Partial (`t_timer[]`) | **No** |
| Buffer utilization | Yes | Yes | Yes | Yes (`xsockbuf_n`) | Partial (`tcpi_snd_sbbytes`) |
| DSACK stats | Yes | Yes | Yes | **No** | **No** |
| ECN state | Yes | Yes | Yes | **No** | Yes (`TCPCI_OPT_ECN`) |
| Window scale factors | Yes | **No** | Yes | Yes (`snd_scale`/`rcv_scale`) | Yes |
| Sequence numbers | Yes | **No** | Yes | Yes (`snd_una`/`snd_nxt`/etc.) | **No** |
| SACK block count | Yes | **No** | Yes | **No** | **No** |
| Duplicate ACKs | Yes | **No** | Yes | Yes (`t_dupacks`) | **No** |
| Pacing rate | Yes | **No** | **No** | **No** | **No** |
| Delivery rate | Yes | **No** | **No** | **No** | **No** |
| Per-conn bytes tx/rx | Yes | **No** | **No** | **No** | Yes (`tcpi_txbytes`/`tcpi_rxbytes`) |
| TFO state | 1 bit | 1 bit | 1 bit | **No** | Yes (15 bitfields) |
| Loss recovery flag | No | No | No | No | Yes (`TCPCI_FLAG_LOSSRECOVERY`) |
| Reorder detection | No | No | No | No | Yes (`TCPCI_FLAG_REORDERING_DETECTED`) |
| Busy time | Yes | **No** | **No** | **No** | **No** |
| Process PID | Yes (Netlink) | No (kern.file join) | No (kern.file join) | Yes (`so_last_pid`) | N/A (requires FD) |

**FreeBSD without kernel module:** `xtcpcb` sysctl provides ~20 useful fields but no RTT. Requires separate `kern.file` join for PID mapping.

**FreeBSD with `tcpstats` ([Section 11](05-kernel-module.md)):** ~35 fields in a single kernel pass — RTT, RTO, rttvar, rttmin, window scale, sequence numbers, SACK state, TLP counters. Still needs `kern.file` for PID mapping.

**macOS `pcblist_n` sysctl (no kernel module):** ~25 fields including RTT (`t_srtt`), RTT variance, sequences, window scale, duplicate ACKs, and PID (`so_last_pid`) — all from a single sysctl. Supplemented by `getsockopt(TCP_CONNECTION_INFO)` for per-connection byte/packet counters, current RTT (ms), TFO state, loss recovery, and reorder detection on owned sockets.

**Remaining gap vs. Linux (~60 fields):** Pacing rate, delivery rate, busy time, and per-connection byte counters (Linux has them in `tcp_info`; macOS has them in `tcp_connection_info` but only via getsockopt; FreeBSD lacks them entirely). These are genuinely absent from or differently exposed in the BSD TCP stacks. The positioning holds: "For the full 60-field tcp_info struct and per-route optimization, deploy on Linux."

---

## 12. Open Questions and Future Considerations

1. **DTrace integration:** FreeBSD has full DTrace support without SIP restrictions. A DTrace probe on `tcp_output` and `tcp_input` could capture per-packet RTT samples (not just the smoothed EWMA from `tcp_info`). This provides higher-fidelity data for jitter analysis. Consider as a future enhancement.

2. **kqueue-based event notification:** Instead of pure polling, use `EVFILT_TIMER` kqueue events for precise interval scheduling, and potentially `EVFILT_PROC` for process lifecycle tracking (detect when a PID exits between polls).

3. **Container/jail awareness:** FreeBSD jails are the equivalent of Linux containers. Socket enumeration within a jail should be scoped to that jail's connections only. The `cr_canseeinpcb()` check in the kernel module already handles this via `prison_check()`, but the userspace tool should detect jail context and label connections accordingly.

4. **Local dashboard protocol:** The tool needs a way to serve data to a local dashboard UI. Options: embedded HTTP server (simple but adds dependency), Unix domain socket with JSON streaming (lightweight), shared memory ring buffer (fastest but complex). Recommend Unix domain socket for v1.

5. **Upstreaming the kernel module:** The module uses only public KPI (`INP_ALL_ITERATOR`, `tcp_fill_info`, `cr_canseeinpcb`, `uiomove`). It could be submitted as a FreeBSD port or as a kernel patch that extends `tcp_pcblist` with an optional `tcp_info` inclusion flag. The latter is cleaner but has a longer review cycle.

6. **macOS kernel extension (kext) equivalent:** macOS uses a different kernel extension framework (IOKit / kext / DriverKit). A macOS System Extension using the Network Extension framework could potentially achieve similar results, but Apple's restrictions on kernel extensions (deprecated since macOS 11) make this less viable. The macOS path should remain userspace-only (libproc + getsockopt) for the foreseeable future, accepting the per-process enumeration cost.

7. **Sequence number exposure considerations:** The `tcp_stats_record` includes `snd_nxt`, `snd_una`, `snd_max`, and `rcv_nxt`. On a system with untrusted local users, these could theoretically aid TCP injection attacks against other users' connections. However: (a) `cr_canseeinpcb()` prevents non-root users from seeing other users' sockets by default, (b) the same data is available to root via `/dev/kmem` or `getsockopt` on jail-accessible sockets, and (c) local attackers with sufficient access to exploit sequence numbers typically have simpler attack paths available. The risk is equivalent to the existing attack surface. If stronger isolation is needed, a compile-time or ioctl flag can omit sequence number fields.
