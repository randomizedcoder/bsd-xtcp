[← Back to README](../../README.md)

# Tool Architecture and Record Schemas

## Table of Contents

- [3. Tool Architecture](#3-tool-architecture)
  - [3.1 High-Level Data Flow](#31-high-level-data-flow)
  - [3.2 Polling Architecture](#32-polling-architecture)
  - [3.3 Consistency Model](#33-consistency-model)
- [4. Per-Socket Export Record](#4-per-socket-export-record)
  - [4.1 Connection Identity](#41-connection-identity)
  - [4.2 TCP State](#42-tcp-state)
  - [4.3 Congestion Control & Window State](#43-congestion-control--window-state)
  - [4.4 Retransmission & Error Counters](#44-retransmission--error-counters)
  - [4.5 Timer State](#45-timer-state)
  - [4.6 Buffer Utilization](#46-buffer-utilization)
  - [4.7 Process Attribution](#47-process-attribution-standard-tier-and-above)
  - [4.8 Extended tcp_info Fields](#48-extended-tcp_info-fields-tier-2--own-process-sockets-only)
  - [4.9 Computed Delta Fields](#49-computed-delta-fields)
- [5. System-Wide Summary Record](#5-system-wide-summary-record)

---

## 3. Tool Architecture

### 3.1 High-Level Data Flow

```
                                     ┌──────────────────────┐
                                     │    Poll Scheduler     │
                                     │  (1s, 30s, 60s, 300s)│
                                     └──────────┬───────────┘
                                                │ tick
                                                ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│  sysctl         │  │  sysctl         │  │  sysctl         │
│  tcp.pcblist    │  │  kern.file      │  │  tcp.stats      │
│  (xtcpcb[])     │  │  (xfile[])      │  │  (tcpstat)      │
└────────┬────────┘  └────────┬────────┘  └────────┬────────┘
         │                    │                     │
         ▼                    ▼                     ▼
┌─────────────────────────────────────────────────────────────┐
│                     Merge & Enrich                           │
│                                                              │
│  1. Parse xtcpcb[] into per-socket records                   │
│  2. Build xfile[] into RB-tree keyed by socket kernel addr   │
│  3. Join socket records with PID/UID/FD via kernel addr      │
│  4. Resolve PID → command name via kern.proc.pid.<pid>       │
│  5. Compute deltas from previous sample (retransmits, etc.)  │
│  6. Compute system-wide rate metrics from tcpstat deltas     │
└─────────────────────────────────┬───────────────────────────┘
                                  │
                                  ▼
                        ┌──────────────────┐
                        │  Output Emitter   │
                        │  (JSON / CSV /    │
                        │   binary / stdout)│
                        └──────────────────┘
```

### 3.2 Polling Architecture

> **Note:** The fixed 4-tier polling model described below was superseded by the configurable `interval_ms` and `schedule_name` approach in [08-protobuf-schema.md](08-protobuf-schema.md), which supports user-defined schedules from 10ms to 24h.

The tool supports four polling tiers, each independently configurable:

| Tier | Default Interval | Data Collected | Purpose |
|---|---|---|---|
| **Fast** | 1s | Per-socket: state, cwnd, ssthresh, MSS, buffers, retransmits, OOO, timers, CC algo | Real-time dashboard, anomaly detection |
| **Standard** | 30s | Same as Fast + process mapping + system-wide tcpstat deltas | Connection inventory with process attribution |
| **Slow** | 60s | Same as Standard + connection state distribution | Trend analysis, capacity planning |
| **Aggregate** | 300s | System-wide tcpstat snapshot, state distribution, connection count summary | Long-term trending, low-overhead baseline |

**Rationale for tiered polling:**

- The `pcblist` sysctl iterates all connections while holding per-connection read locks. At 1s intervals on a machine with hundreds of sockets, this is negligible overhead. But the process mapping (`kern.file`) is more expensive (reads the global file table) and changes infrequently — no need to do it every second.
- System-wide tcpstat counters are cheap to read but only meaningful as deltas over longer windows. A 1-second delta of `tcps_sndrexmitpack` on a developer laptop is noisy; a 30-second or 60-second delta is actionable.

**Implementation:** A single timer loop with a tick counter. Each tick (1s), check which tiers are due:

```
tick % 1   == 0  → Fast tier
tick % 30  == 0  → Standard tier (includes Fast data)
tick % 60  == 0  → Slow tier (includes Standard data)
tick % 300 == 0  → Aggregate tier (includes Slow data)
```

Each higher tier is a superset of the lower tiers. When a Standard tick fires, it does not re-read pcblist — it uses the data already read by the Fast tier on the same tick and adds process mapping on top.

### 3.3 Consistency Model

The pcblist sysctl uses generation counters to detect concurrent modification:

```c
do {
    buf = read_sysctl("net.inet.tcp.pcblist");
    header = (struct xinpgen *)buf;
    trailer = (struct xinpgen *)(buf + len - sizeof(xinpgen));
} while (header->xig_gen != trailer->xig_gen && retries-- > 0);
```

Similarly, `kern.file` should be read in the same window as `pcblist` to minimize join mismatches (sockets that appeared or disappeared between reads). The practical impact on a developer laptop is minimal, but the implementation should handle:

- Sockets in pcblist with no matching file entry (kernel sockets, sockets in TIME_WAIT with no owning process)
- File entries with no matching pcblist entry (race condition; socket closed between reads)
- PID that no longer exists when we try to resolve command name (process exited)

All three cases should be handled gracefully (emit the record with available data, mark missing fields as unknown).

---

## 4. Per-Socket Export Record

Each poll sample produces one record per TCP socket. The unified record merges data from all three sources:

### 4.1 Connection Identity

| Field | Source | Type | Description |
|---|---|---|---|
| `local_addr` | pcblist (xinpcb) | `string` | Local IP address |
| `local_port` | pcblist (xinpcb) | `uint16` | Local port |
| `remote_addr` | pcblist (xinpcb) | `string` | Remote IP address |
| `remote_port` | pcblist (xinpcb) | `uint16` | Remote port |
| `ip_version` | pcblist (xinpcb.inp_vflag) | `uint8` | 4 or 6 |
| `socket_id` | pcblist (xsocket.xso_so) | `uint64` | Kernel socket address (stable identifier within a sample) |

### 4.2 TCP State

| Field | Source | Type | Description |
|---|---|---|---|
| `state` | pcblist (xtcpcb.t_state) | `string` | TCP FSM state name |
| `state_code` | pcblist (xtcpcb.t_state) | `uint8` | TCP FSM state numeric |
| `flags` | pcblist (xtcpcb.t_flags) | `uint32` | TCP flags bitmask |

### 4.3 Congestion Control & Window State

| Field | Source | Type | Description |
|---|---|---|---|
| `snd_cwnd` | pcblist (xtcpcb) | `uint32` | Congestion window (bytes) |
| `snd_ssthresh` | pcblist (xtcpcb) | `uint32` | Slow start threshold (bytes) |
| `snd_wnd` | pcblist (xtcpcb) | `uint32` | Send window (bytes) |
| `rcv_wnd` | pcblist (xtcpcb) | `uint32` | Receive window (bytes) |
| `maxseg` | pcblist (xtcpcb) | `uint32` | Maximum segment size (bytes) |
| `cc_algo` | pcblist (xtcpcb.xt_cc) | `string` | Congestion control algorithm name |
| `tcp_stack` | pcblist (xtcpcb.xt_stack) | `string` | TCP stack name |

### 4.4 Retransmission & Error Counters

| Field | Source | Type | Description |
|---|---|---|---|
| `snd_rexmitpack` | pcblist (xtcpcb) | `uint32` | Retransmitted packets (cumulative) |
| `rcv_ooopack` | pcblist (xtcpcb) | `uint32` | Out-of-order packets received (cumulative) |
| `snd_zerowin` | pcblist (xtcpcb) | `uint32` | Zero-window probes sent (cumulative) |
| `dsack_bytes` | pcblist (xtcpcb) | `uint32` | DSACK bytes received |
| `dsack_pack` | pcblist (xtcpcb) | `uint32` | DSACK packets received |
| `ecn` | pcblist (xtcpcb.xt_ecn) | `uint32` | ECN negotiation flags |

### 4.5 Timer State

| Field | Source | Type | Description |
|---|---|---|---|
| `timer_rexmt_ms` | pcblist (xtcpcb.tt_rexmt) | `int32` | Retransmit timer remaining (ms), 0 = not running |
| `timer_persist_ms` | pcblist (xtcpcb.tt_persist) | `int32` | Persist timer remaining (ms) |
| `timer_keep_ms` | pcblist (xtcpcb.tt_keep) | `int32` | Keepalive timer remaining (ms) |
| `timer_2msl_ms` | pcblist (xtcpcb.tt_2msl) | `int32` | 2MSL timer remaining (ms) |
| `timer_delack_ms` | pcblist (xtcpcb.tt_delack) | `int32` | Delayed ACK timer remaining (ms) |
| `idle_time_ms` | pcblist (xtcpcb.t_rcvtime) | `int32` | Time since last data received (ms) |

### 4.6 Buffer Utilization

| Field | Source | Type | Description |
|---|---|---|---|
| `snd_buf_used` | pcblist (xsocket.so_snd.sb_cc) | `uint32` | Send buffer bytes in use |
| `snd_buf_hiwat` | pcblist (xsocket.so_snd.sb_hiwat) | `uint32` | Send buffer high watermark |
| `rcv_buf_used` | pcblist (xsocket.so_rcv.sb_cc) | `uint32` | Receive buffer bytes in use |
| `rcv_buf_hiwat` | pcblist (xsocket.so_rcv.sb_hiwat) | `uint32` | Receive buffer high watermark |
| `snd_buf_pct` | computed | `float` | Send buffer utilization % |
| `rcv_buf_pct` | computed | `float` | Receive buffer utilization % |

### 4.7 Process Attribution (Standard tier and above)

| Field | Source | Type | Description |
|---|---|---|---|
| `pid` | kern.file join | `int32` | Process ID, -1 if unmapped |
| `uid` | kern.file join | `uint32` | User ID |
| `fd` | kern.file join | `int32` | File descriptor number |
| `command` | kern.proc.pid | `string` | Process command name (20 chars max) |

### 4.8 Extended tcp_info Fields (Tier 2 — own-process sockets only)

These fields are only populated when the tool has an FD for the socket (i.e., when used as a library linked into the target application, or when the tool itself opens the connection):

| Field | Source | Type | Description |
|---|---|---|---|
| `rtt_us` | getsockopt TCP_INFO | `uint32` | Smoothed RTT (microseconds) |
| `rttvar_us` | getsockopt TCP_INFO | `uint32` | RTT variance (microseconds) |
| `rto_us` | getsockopt TCP_INFO | `uint32` | Retransmission timeout (microseconds) |
| `rtt_min_us` | getsockopt TCP_INFO | `uint32` | Minimum observed RTT (microseconds) |
| `snd_nxt` | getsockopt TCP_INFO | `uint32` | Next send sequence number |
| `rcv_nxt` | getsockopt TCP_INFO | `uint32` | Next receive sequence number |
| `snd_una` | getsockopt TCP_INFO | `uint32` | Unacknowledged send sequence |
| `snd_max` | getsockopt TCP_INFO | `uint32` | Highest sequence number sent |
| `dupacks` | getsockopt TCP_INFO | `uint32` | Consecutive duplicate ACKs |
| `rcv_numsacks` | getsockopt TCP_INFO | `uint32` | Distinct SACK blocks received |
| `options` | getsockopt TCP_INFO | `uint8` | Negotiated options bitmask |
| `snd_wscale` | getsockopt TCP_INFO | `uint8` | Send window scale factor |
| `rcv_wscale` | getsockopt TCP_INFO | `uint8` | Receive window scale factor |

### 4.9 Computed Delta Fields

For cumulative counters, the tool computes per-interval deltas by tracking previous values keyed by connection identity (4-tuple):

| Field | Derived From | Type | Description |
|---|---|---|---|
| `delta_rexmitpack` | `snd_rexmitpack` | `int32` | Retransmits this interval |
| `delta_ooopack` | `rcv_ooopack` | `int32` | OOO packets this interval |
| `delta_zerowin` | `snd_zerowin` | `int32` | Zero-window events this interval |
| `connection_age_ms` | first-seen tracking | `uint64` | Time since first observed |

---

## 5. System-Wide Summary Record

Each Aggregate-tier poll produces a summary record computed from `tcpstat` deltas and state counters:

| Field | Source | Type | Description |
|---|---|---|---|
| `timestamp` | system clock | `uint64` | Unix timestamp (nanoseconds) |
| `interval_ms` | configured | `uint32` | Polling interval for this sample |
| `total_sockets` | pcblist count | `uint32` | Total TCP sockets |
| `state_counts[11]` | tcp.states | `uint32[]` | Socket count per TCP state |
| `delta_connattempt` | tcpstat delta | `uint64` | New connection attempts |
| `delta_accepts` | tcpstat delta | `uint64` | New connections accepted |
| `delta_connects` | tcpstat delta | `uint64` | New connections established |
| `delta_drops` | tcpstat delta | `uint64` | Connections dropped |
| `delta_sndtotal` | tcpstat delta | `uint64` | Packets sent |
| `delta_sndbyte` | tcpstat delta | `uint64` | Bytes sent |
| `delta_sndrexmitpack` | tcpstat delta | `uint64` | Retransmitted packets |
| `delta_sndrexmitbyte` | tcpstat delta | `uint64` | Retransmitted bytes |
| `delta_rcvtotal` | tcpstat delta | `uint64` | Packets received |
| `delta_rcvbyte` | tcpstat delta | `uint64` | Bytes received |
| `delta_rcvduppack` | tcpstat delta | `uint64` | Duplicate packets received |
| `delta_rcvbadsum` | tcpstat delta | `uint64` | Checksum errors |
| `retransmit_rate` | computed | `float` | `delta_sndrexmitpack / delta_sndtotal` |
| `dup_rate` | computed | `float` | `delta_rcvduppack / delta_rcvtotal` |
