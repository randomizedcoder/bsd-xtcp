[← Back to main document](../freebsd-tcp-stats-design.md)

# FreeBSD Kernel Data Sources

Three kernel interfaces provide the data needed. All are accessible from unprivileged userspace (some fields require root).

## Table of Contents

- [2.1 `sysctl net.inet.tcp.pcblist` — Socket Enumeration + Bulk TCP State](#21-sysctl-netinettcppcblist--socket-enumeration--bulk-tcp-state)
- [2.2 `getsockopt(TCP_INFO)` — Per-Socket Detailed State](#22-getsockopttcp_info--per-socket-detailed-state)
- [2.3 `sysctl net.inet.tcp.stats` — System-Wide Aggregate Statistics](#23-sysctl-netinettcpstats--system-wide-aggregate-statistics)
- [2.4 `sysctl net.inet.tcp.states` — Connection State Distribution](#24-sysctl-netinettcpstates--connection-state-distribution)
- [2.5 `sysctl kern.file` + `sysctl kern.proc.filedesc.<pid>` — Process-to-Socket Mapping](#25-sysctl-kernfile--sysctl-kernprocfiledescpid--process-to-socket-mapping)

---

## 2.1 `sysctl net.inet.tcp.pcblist` — Socket Enumeration + Bulk TCP State

**Source:** `sys/netinet/tcp_subr.c` — `tcp_pcblist()` handler (line 2616)

This sysctl returns a binary stream of all TCP control blocks on the system. The wire format is:

```
┌──────────────┐
│  xinpgen     │  Header: count, generation number
├──────────────┤
│  xtcpcb[0]   │  First TCP connection
├──────────────┤
│  xtcpcb[1]   │  Second TCP connection
├──────────────┤
│  ...          │
├──────────────┤
│  xtcpcb[N-1] │  Last TCP connection
├──────────────┤
│  xinpgen     │  Trailer: updated count, generation number
└──────────────┘
```

The caller must verify that the header and trailer generation numbers match. If they differ, the snapshot is inconsistent and must be retried (the kernel modified state mid-read). This is the same retry loop `sockstat` uses (`usr.bin/sockstat/main.c`, line 665).

**Key structure: `struct xtcpcb`** (`sys/netinet/tcp_var.h`, line 1204)

Each `xtcpcb` contains:

| Field | Type | Description |
|---|---|---|
| `t_state` | `int32_t` | TCP FSM state (CLOSED through TIME_WAIT) |
| `t_flags` | `uint32_t` | TCP flags bitmask |
| `t_snd_cwnd` | `uint32_t` | Send congestion window (bytes) |
| `t_snd_ssthresh` | `uint32_t` | Slow start threshold (bytes) |
| `t_maxseg` | `uint32_t` | Maximum segment size |
| `t_rcv_wnd` | `uint32_t` | Receive window (bytes) |
| `t_snd_wnd` | `uint32_t` | Send window (bytes) |
| `t_sndrexmitpack` | `int32_t` | Retransmitted packets |
| `t_rcvoopack` | `int32_t` | Out-of-order packets received |
| `t_sndzerowin` | `int32_t` | Zero-window probes sent |
| `t_rcvtime` | `int32_t` | Time since last data received (ms) |
| `tt_rexmt` | `int32_t` | Retransmit timer remaining (ms) |
| `tt_persist` | `int32_t` | Persist timer remaining (ms) |
| `tt_keep` | `int32_t` | Keepalive timer remaining (ms) |
| `tt_2msl` | `int32_t` | 2MSL timer remaining (ms) |
| `tt_delack` | `int32_t` | Delayed ACK timer remaining (ms) |
| `xt_stack` | `char[32]` | TCP stack name (e.g., "freebsd") |
| `xt_cc` | `char[TCP_CA_NAME_MAX]` | Congestion control algorithm name |
| `xt_ecn` | `uint32_t` | ECN flags |
| `t_dsack_bytes` | `uint32_t` | DSACK bytes received |
| `t_dsack_pack` | `uint32_t` | DSACK packets received |
| `xt_encaps_port` | `uint16_t` | UDP encapsulation port |

Each `xtcpcb` embeds a `struct xinpcb` (`sys/netinet/in_pcb.h`, line 265) which contains:

| Field | Type | Description |
|---|---|---|
| `inp_inc.inc_ie.ie_lport` | `uint16_t` | Local port (network byte order) |
| `inp_inc.inc_ie.ie_fport` | `uint16_t` | Foreign port (network byte order) |
| `inp_inc.inc_ie.ie_dependladdr` | `union` | Local IP address (v4 or v6) |
| `inp_inc.inc_ie.ie_dependfaddr` | `union` | Foreign IP address (v4 or v6) |
| `inp_vflag` | `uint8_t` | `INP_IPV4` (0x1) or `INP_IPV6` (0x2) |
| `inp_flags` | `int32_t` | Protocol flags |
| `inp_gencnt` | `uint64_t` | Generation count for change detection |

And a `struct xsocket` (`sys/sys/socketvar.h`, line 608) which contains:

| Field | Type | Description |
|---|---|---|
| `xso_so` | `kvaddr_t` | Kernel socket address (used for PID mapping) |
| `so_uid` | `uid_t` | User ID owning the socket |
| `so_type` | `int16_t` | Socket type (SOCK_STREAM) |
| `so_options` | `int16_t` | Socket options |
| `so_rcv.sb_cc` | `uint32_t` | Receive buffer current bytes |
| `so_rcv.sb_hiwat` | `uint32_t` | Receive buffer high watermark |
| `so_snd.sb_cc` | `uint32_t` | Send buffer current bytes |
| `so_snd.sb_hiwat` | `uint32_t` | Send buffer high watermark |

**This single sysctl call gives us the bulk of what we need** — connection tuples, TCP state, congestion parameters, buffer utilization, timers, and CC algorithm. This is more efficient than calling `getsockopt(TCP_INFO)` per-socket because it requires no open file descriptors and retrieves all connections in one kernel round-trip.

## 2.2 `getsockopt(TCP_INFO)` — Per-Socket Detailed State

**Source:** `sys/netinet/tcp_usrreq.c` — TCP_INFO case handler (line 2504), `tcp_fill_info()` (line 1569)

The `struct tcp_info` on FreeBSD contains fields **not present** in the `xtcpcb` bulk export:

| Field | Type | Description | In xtcpcb? |
|---|---|---|---|
| `tcpi_rtt` | `uint32_t` | Smoothed RTT (usec) | **No** |
| `tcpi_rttvar` | `uint32_t` | RTT variance (usec) | **No** |
| `tcpi_rto` | `uint32_t` | Retransmission timeout (usec) | **No** |
| `tcpi_snd_nxt` | `uint32_t` | Next send sequence number | **No** |
| `tcpi_rcv_nxt` | `uint32_t` | Next receive sequence number | **No** |
| `tcpi_snd_una` | `uint32_t` | Unacknowledged send sequence | **No** |
| `tcpi_snd_max` | `uint32_t` | Highest sequence number sent | **No** |
| `tcpi_rttmin` | `uint32_t` | Minimum observed RTT | **No** |
| `tcpi_dupacks` | `uint32_t` | Consecutive duplicate ACKs | **No** |
| `tcpi_rcv_numsacks` | `uint32_t` | Distinct SACK blocks received | **No** |
| `tcpi_rcv_adv` | `uint32_t` | Peer's advertised window | **No** |
| `tcpi_options` | `uint8_t` | Negotiated options (SACK, timestamps, wscale, ECN, TFO) | **No** |
| `tcpi_snd_wscale` | `uint8_t:4` | Send window scale factor | **No** |
| `tcpi_rcv_wscale` | `uint8_t:4` | Receive window scale factor | **No** |
| `tcpi_delivered_ce` | `uint32_t` | ECN CE marks delivered | **No** |
| `tcpi_received_ce` | `uint32_t` | ECN CE marks received | **No** |
| `tcpi_total_tlp` | `uint32_t` | Tail loss probes sent | **No** |
| `tcpi_total_tlp_bytes` | `uint64_t` | TLP bytes sent | **No** |
| `tcpi_snd_rexmitpack` | `uint32_t` | Retransmit packet count | Yes |
| `tcpi_rcv_ooopack` | `uint32_t` | Out-of-order packet count | Yes |
| `tcpi_snd_zerowin` | `uint32_t` | Zero-window probes sent | Yes |

**Critical gap:** The `xtcpcb` bulk export does **not** include RTT, RTT variance, RTO, sequence numbers, window scale, negotiated options, or SACK state. These are the most important fields for developer-facing TCP profiling. RTT alone is the single most useful metric for diagnosing connection health.

**Implication:** We need both data sources. The `pcblist` sysctl gives us enumeration and the fields it has, but `getsockopt(TCP_INFO)` is required for RTT, RTO, and sequence-level detail. However, `getsockopt` requires an open file descriptor to the socket — which means we can only call it on sockets owned by our process, unless we use a different approach.

**Resolution — two-tier architecture:**

1. **Tier 1 (all sockets):** `sysctl net.inet.tcp.pcblist` provides connection tuples, state, cwnd, ssthresh, MSS, buffer sizes, timer state, CC algorithm, retransmit/OOO counts. Available for every socket on the system without privileges.

2. **Tier 2 (own-process sockets only, or with helper):** `getsockopt(TCP_INFO)` provides RTT, RTO, rttvar, rttmin, sequence numbers, window scale, SACK state. Only available for sockets the calling process owns (has an FD for). For system-wide TCP_INFO, a privileged helper or kernel module would be needed — out of scope for v1.

For the developer use case, Tier 1 alone is sufficient: developers primarily need to see connection state, buffer utilization, retransmit rates, CC algorithm, and timer state across all sockets. RTT data is available for their own application's sockets via Tier 2 (by injecting a library or using the tool from within the application's process context).

**Alternative for system-wide RTT (future consideration):** A `SOCK_DIAG`-equivalent kernel extension that fills `tcp_info` for each socket during pcblist iteration. This would require a FreeBSD kernel patch adding a new sysctl (e.g., `net.inet.tcp.pcblist_info`) that runs `tcp_fill_info()` per-connection during iteration. This is a bounded kernel change (~50 lines) and could be upstreamed.

## 2.3 `sysctl net.inet.tcp.stats` — System-Wide Aggregate Statistics

**Source:** `sys/netinet/tcp_var.h` — `struct tcpstat` (line 966)

Provides system-wide counters (not per-socket):

| Counter | Description |
|---|---|
| `tcps_connattempt` | Connections initiated |
| `tcps_accepts` | Connections accepted |
| `tcps_connects` | Connections established |
| `tcps_drops` | Connections dropped |
| `tcps_sndtotal` | Total packets sent |
| `tcps_sndpack` / `tcps_sndbyte` | Data packets/bytes sent |
| `tcps_sndrexmitpack` / `tcps_sndrexmitbyte` | Retransmitted packets/bytes |
| `tcps_rcvtotal` | Total packets received |
| `tcps_rcvpack` / `tcps_rcvbyte` | Data packets/bytes received in sequence |
| `tcps_rcvduppack` / `tcps_rcvdupbyte` | Duplicate packets/bytes received |
| `tcps_rcvbadsum` | Packets received with checksum errors |
| `tcps_sndacks` | ACK-only packets sent |
| `tcps_sndprobe` | Window probes sent |
| `tcps_sc_added` / `tcps_sc_completed` | Syncache entries added/completed |

These counters are monotonically increasing. The tool computes deltas between poll intervals to derive rates (e.g., retransmit rate = `delta(tcps_sndrexmitpack) / delta(tcps_sndtotal)`).

**Access:** `sysctlbyname("net.inet.tcp.stats", &tcpstat, &len, NULL, 0)`

## 2.4 `sysctl net.inet.tcp.states` — Connection State Distribution

**Source:** `sys/netinet/tcp_var.h` — per-state counter array

Returns an array of `counter_u64_t` values indexed by TCP state constant:

| Index | State | Constant |
|---|---|---|
| 0 | CLOSED | `TCPS_CLOSED` |
| 1 | LISTEN | `TCPS_LISTEN` |
| 2 | SYN_SENT | `TCPS_SYN_SENT` |
| 3 | SYN_RECEIVED | `TCPS_SYN_RECEIVED` |
| 4 | ESTABLISHED | `TCPS_ESTABLISHED` |
| 5 | CLOSE_WAIT | `TCPS_CLOSE_WAIT` |
| 6 | FIN_WAIT_1 | `TCPS_FIN_WAIT_1` |
| 7 | CLOSING | `TCPS_CLOSING` |
| 8 | LAST_ACK | `TCPS_LAST_ACK` |
| 9 | FIN_WAIT_2 | `TCPS_FIN_WAIT_2` |
| 10 | TIME_WAIT | `TCPS_TIME_WAIT` |

Useful for the dashboard summary view (connection state distribution pie chart).

## 2.5 `sysctl kern.file` + `sysctl kern.proc.filedesc.<pid>` — Process-to-Socket Mapping

**Source:** `sys/kern/kern_proc.c`, `lib/libutil/kinfo_getfile.c`, `lib/libprocstat/libprocstat.c`

Two approaches for mapping sockets to processes:

**Approach A: Global file table (what sockstat uses)**

1. Read `sysctl kern.file` — returns all open file descriptors in the system
2. Each `struct xfile` contains `xf_data` (kernel socket address), `xf_pid`, `xf_uid`, `xf_fd`
3. Build a lookup table: `socket_kernel_addr → (pid, uid, fd)`
4. Match against `xso_so` field from the `xsocket` embedded in each `xtcpcb`

**Approach B: Per-PID file descriptor query (what procstat uses)**

1. For each PID of interest, call `kinfo_getfile(pid, &count)`
2. Uses `sysctl(CTL_KERN, KERN_PROC, KERN_PROC_FILEDESC, pid)`
3. Returns array of `struct kinfo_file` with socket details
4. Each entry with `kf_type == KF_TYPE_SOCKET` contains local/peer addresses and `kf_sock_pcb`

**Decision:** Use Approach A (global file table). It is a single sysctl call that covers all processes, which is more efficient when we're already enumerating all sockets. We need to join two datasets:

- `pcblist` gives us: `{socket_kernel_addr, local_addr, local_port, foreign_addr, foreign_port, tcp_state, ...}`
- `kern.file` gives us: `{socket_kernel_addr, pid, uid, fd}`

The join key is `socket_kernel_addr` (`xso_so` from the xsocket in xtcpcb vs. `xf_data` from xfile).

To get the command name for each PID, call `sysctl kern.proc.pid.<pid>` which returns `struct kinfo_proc` containing `ki_comm` (command name, 20 chars) and `ki_args` (full command line).
