[← Back to main document](../freebsd-tcp-stats-design.md)

# macOS Portability Considerations

macOS diverges from FreeBSD in important ways despite shared Darwin/BSD lineage. The key differences are:

1. **macOS uses `TCP_CONNECTION_INFO` (0x106), not `TCP_INFO` (0x20).** The struct is `struct tcp_connection_info`, not `struct tcp_info`. Different fields, different layout, different option/flag constants.
2. **macOS has `sysctl net.inet.tcp.pcblist_n`** — a bulk socket enumeration sysctl that returns a stream of tagged structs (`_n` variants). This is the same mechanism that Apple's `netstat` uses. It is **not** the per-process `proc_pidinfo` path.
3. **There is no `ss` on macOS or FreeBSD.** The closest tool is `netstat`. Apple's open-source `netstat` ([network_cmds](https://github.com/apple-oss-distributions/network_cmds/tree/97bfa5b71464f1286b51104ba3e60db78cd832c9/netstat.tproj)) is buildable externally, but it does not currently dump `tcp_connection_info` data — it only shows basic connection state from the pcblist_n sysctl.

## Table of Contents

- [8.1 macOS Socket Enumeration: `net.inet.tcp.pcblist_n`](#81-macos-socket-enumeration-netinettcppcblist_n)
- [8.2 macOS `TCP_CONNECTION_INFO` via `getsockopt`](#82-macos-tcp_connection_info-via-getsockopt)
- [8.3 macOS Platform Capability Summary](#83-macos-platform-capability-summary)
- [8.4 macOS Implementation Strategy](#84-macos-implementation-strategy)
- [8.5 Updated Platform Architecture](#85-updated-platform-architecture)

---

## 8.1 macOS Socket Enumeration: `net.inet.tcp.pcblist_n`

macOS provides a bulk TCP socket enumeration sysctl similar to FreeBSD's `net.inet.tcp.pcblist`, but using a different wire format. Instead of a flat array of `xtcpcb` structs, macOS returns a stream of tagged, variable-length records identified by `xso_kind` / `xi_kind` / `xt_kind` fields:

```
┌──────────────┐
│  xinpgen     │  Header
├──────────────┤
│  xsocket_n   │  XSO_SOCKET (0x001)  ─┐
│  xsockbuf_n  │  XSO_RCVBUF (0x002)   │
│  xsockbuf_n  │  XSO_SNDBUF (0x004)   ├─ Connection 0
│  xsockstat_n │  XSO_STATS  (0x008)   │
│  xinpcb_n    │  XSO_INPCB  (0x010)   │
│  xtcpcb_n    │  XSO_TCPCB  (0x020)  ─┘
├──────────────┤
│  xsocket_n   │                       ─┐
│  ...         │                        ├─ Connection 1
│  xtcpcb_n    │                       ─┘
├──────────────┤
│  ...         │
├──────────────┤
│  xinpgen     │  Trailer
└──────────────┘
```

Each record is prefixed with `{u_int32_t len, u_int32_t kind}`. The parser iterates using `next += ROUNDUP64(xgn->xgn_len)` and switches on the kind field.

**Key macOS structures:**

**`struct xtcpcb_n`** — TCP control block (from `xnu/bsd/netinet/tcp_var.h`):

| Field | Type | Description |
|---|---|---|
| `t_state` | `int` | TCP FSM state |
| `t_flags` | `u_int` | TCP flags |
| `snd_una` | `tcp_seq` | Unacknowledged send sequence |
| `snd_max` | `tcp_seq` | Highest sequence sent |
| `snd_nxt` | `tcp_seq` | Next send sequence |
| `snd_wnd` | `u_int32_t` | Send window |
| `snd_cwnd` | `u_int32_t` | Congestion window |
| `snd_ssthresh` | `u_int32_t` | Slow start threshold |
| `rcv_nxt` | `tcp_seq` | Next receive sequence |
| `rcv_wnd` | `u_int32_t` | Receive window |
| `rcv_adv` | `tcp_seq` | Advertised window |
| `t_srtt` | `int` | Smoothed RTT (ticks, needs conversion) |
| `t_rttvar` | `int` | RTT variance (ticks) |
| `t_rxtcur` | `int` | Current retransmit timeout |
| `t_rttmin` | `u_int` | Minimum RTT |
| `t_maxseg` | `u_int` | Maximum segment size |
| `t_dupacks` | `int` | Duplicate ACKs received |
| `t_rxtshift` | `int` | Retransmit backoff exponent |
| `snd_scale` / `rcv_scale` | `u_char` | Window scale factors |
| `t_starttime` | `u_int32_t` | Connection start time |
| `t_rttupdated` | `u_int32_t` | RTT update count |

**`struct xsocket_n`** — Socket metadata:

| Field | Type | Description |
|---|---|---|
| `xso_so` | `u_int64_t` | Kernel socket address (join key) |
| `so_type` | `short` | Socket type (SOCK_STREAM) |
| `so_uid` | `uid_t` | Owner UID |
| `so_last_pid` | `pid_t` | **Last PID that operated on socket** |
| `so_e_pid` | `pid_t` | **Effective PID** |
| `so_pcb` | `u_int64_t` | PCB kernel address |

**Critical difference from FreeBSD:** macOS's `xsocket_n` includes `so_last_pid` and `so_e_pid` directly in the socket export struct. This means **process attribution is built into the pcblist_n sysctl on macOS** — no separate `kern.file` join is needed.

**`struct xsockbuf_n`** — Buffer state (one for send, one for receive):

| Field | Type | Description |
|---|---|---|
| `sb_cc` | `u_int32_t` | Current bytes in buffer |
| `sb_hiwat` | `u_int32_t` | High watermark |
| `sb_mbcnt` | `u_int32_t` | Mbuf cluster count |
| `sb_lowat` | `int32_t` | Low watermark |

## 8.2 macOS `TCP_CONNECTION_INFO` via `getsockopt`

macOS uses `getsockopt(fd, IPPROTO_TCP, TCP_CONNECTION_INFO, ...)` instead of FreeBSD's `TCP_INFO`. The constant is `0x106` (vs. FreeBSD's `32`). The struct is `struct tcp_connection_info`:

```c
struct tcp_connection_info {
    u_int8_t    tcpi_state;             /* TCP FSM state */
    u_int8_t    tcpi_snd_wscale;        /* Send window scale */
    u_int8_t    tcpi_rcv_wscale;        /* Receive window scale */
    u_int8_t    __pad1;
    u_int32_t   tcpi_options;           /* TCPCI_OPT_* flags */
    u_int32_t   tcpi_flags;            /* TCPCI_FLAG_* flags */
    u_int32_t   tcpi_rto;               /* Retransmit timeout (ms) */
    u_int32_t   tcpi_maxseg;            /* Maximum segment size */
    u_int32_t   tcpi_snd_ssthresh;      /* Slow start threshold */
    u_int32_t   tcpi_snd_cwnd;          /* Congestion window */
    u_int32_t   tcpi_snd_wnd;           /* Send window */
    u_int32_t   tcpi_snd_sbbytes;       /* Send buffer bytes (incl. in-flight) */
    u_int32_t   tcpi_rcv_wnd;           /* Receive window */
    u_int32_t   tcpi_rttcur;            /* Most recent RTT (ms) */
    u_int32_t   tcpi_srtt;              /* Smoothed RTT (ms) */
    u_int32_t   tcpi_rttvar;            /* RTT variance */
    u_int32_t                           /* TFO bitfield: */
        tcpi_tfo_cookie_req:1,          /*   Cookie requested */
        tcpi_tfo_cookie_rcv:1,          /*   Cookie received */
        tcpi_tfo_syn_loss:1,            /*   SYN+data lost */
        tcpi_tfo_syn_data_sent:1,       /*   SYN+data sent */
        tcpi_tfo_syn_data_acked:1,      /*   SYN+data acked */
        tcpi_tfo_syn_data_rcv:1,        /*   SYN+data received */
        tcpi_tfo_cookie_req_rcv:1,      /*   Cookie request received */
        tcpi_tfo_cookie_sent:1,         /*   Cookie sent */
        tcpi_tfo_cookie_invalid:1,      /*   Cookie invalid */
        tcpi_tfo_cookie_wrong:1,        /*   Cookie wrong */
        tcpi_tfo_no_cookie_rcv:1,       /*   No cookie received */
        tcpi_tfo_heuristics_disable:1,  /*   TFO heuristically disabled */
        tcpi_tfo_send_blackhole:1,      /*   Send blackhole detected */
        tcpi_tfo_recv_blackhole:1,      /*   Recv blackhole detected */
        tcpi_tfo_onebyte_proxy:1,       /*   One-byte proxy detected */
        __pad2:17;
    u_int64_t   tcpi_txpackets;         /* Total packets sent */
    u_int64_t   tcpi_txbytes;           /* Total bytes sent */
    u_int64_t   tcpi_txretransmitbytes; /* Retransmitted bytes */
    u_int64_t   tcpi_rxpackets;         /* Total packets received */
    u_int64_t   tcpi_rxbytes;           /* Total bytes received */
    u_int64_t   tcpi_rxoutoforderbytes; /* Out-of-order bytes received */
    u_int64_t   tcpi_txretransmitpackets;/* Retransmitted packets */
};

#define TCPCI_OPT_TIMESTAMPS    0x00000001
#define TCPCI_OPT_SACK          0x00000002
#define TCPCI_OPT_WSCALE        0x00000004
#define TCPCI_OPT_ECN           0x00000008

#define TCPCI_FLAG_LOSSRECOVERY         0x00000001
#define TCPCI_FLAG_REORDERING_DETECTED  0x00000002
```

**macOS `tcp_connection_info` notable differences from FreeBSD `tcp_info`:**

| Aspect | FreeBSD `tcp_info` | macOS `tcp_connection_info` |
|---|---|---|
| RTT unit | Microseconds | **Milliseconds** |
| RTT fields | `tcpi_rtt` (smoothed) | `tcpi_srtt` (smoothed) + `tcpi_rttcur` (most recent) |
| Byte counters | Not present | `tcpi_txbytes`, `tcpi_rxbytes`, `tcpi_txretransmitbytes`, `tcpi_rxoutoforderbytes` |
| Packet counters | `tcpi_snd_rexmitpack`, `tcpi_rcv_ooopack` | `tcpi_txpackets`, `tcpi_rxpackets`, `tcpi_txretransmitpackets` |
| TFO state | `tcpi_options & TCPI_OPT_TFO` (1 bit) | 15 individual TFO state bitfields |
| Sequence numbers | Present (`snd_nxt`, `snd_una`, etc.) | **Not present** |
| SACK block count | `tcpi_rcv_numsacks` | **Not present** |
| Duplicate ACKs | `tcpi_dupacks` | **Not present** |
| Min RTT | `tcpi_rttmin` | **Not present** |
| Send buffer | Not in tcp_info (in xsocket) | `tcpi_snd_sbbytes` (includes in-flight) |
| Option constant | `TCP_INFO` = 32 | `TCP_CONNECTION_INFO` = 0x106 |
| Loss recovery flag | Not present | `TCPCI_FLAG_LOSSRECOVERY` |
| Reordering detected | Not present | `TCPCI_FLAG_REORDERING_DETECTED` |

**Key insight:** macOS's `tcp_connection_info` has per-connection byte/packet counters that FreeBSD's `tcp_info` lacks, but is missing sequence numbers, SACK state, and min RTT. The `pcblist_n` sysctl's `xtcpcb_n` struct fills some of these gaps (it includes `snd_una`, `snd_nxt`, `snd_max`, `rcv_nxt`, `t_srtt`, `t_rttvar`, `t_dupacks`, `snd_scale`, `rcv_scale`).

## 8.3 macOS Platform Capability Summary

| Capability | FreeBSD | macOS | Notes |
|---|---|---|---|
| Bulk socket enumeration | `sysctl net.inet.tcp.pcblist` | `sysctl net.inet.tcp.pcblist_n` | Different wire format; both are single-sysctl system-wide |
| Process attribution in pcblist | Not included (needs kern.file join) | **Included** (`so_last_pid`, `so_e_pid` in xsocket_n) | macOS is better here |
| Per-socket getsockopt | `TCP_INFO` (32) — `struct tcp_info` | `TCP_CONNECTION_INFO` (0x106) — `struct tcp_connection_info` | Different struct, different fields |
| RTT in bulk export | `xtcpcb_n.t_srtt` (raw ticks) | `xtcpcb_n.t_srtt` (raw ticks) | Both have it in the pcblist struct |
| RTT in getsockopt | Microseconds | **Milliseconds** | Unit conversion needed |
| Per-connection byte counters | **Not available** | `tcpi_txbytes`, `tcpi_rxbytes` in tcp_connection_info | macOS advantage |
| Sequence numbers | In `tcp_info` and kernel module | In `xtcpcb_n` only (not in tcp_connection_info) | Both have it via pcblist |
| System-wide TCP stats | `sysctl net.inet.tcp.stats` | `sysctl net.inet.tcp.stats` | Similar |
| Connection state counts | `sysctl net.inet.tcp.states` | **Not available** (count from enumeration) | |
| Kernel module option | Yes (KLD) | **No** (kext deprecated since macOS 11) | FreeBSD-only |
| Tool equivalent to `ss` | **None** (`sockstat`, `netstat`) | **None** (`netstat`, `lsof`) | Neither platform has `ss` |

## 8.4 macOS Implementation Strategy

macOS does **not** need the per-process `proc_pidinfo` enumeration path previously assumed. The `pcblist_n` sysctl provides system-wide enumeration including PID attribution. The implementation:

1. Read `sysctl net.inet.tcp.pcblist_n` — returns tagged stream of `xsocket_n` + `xsockbuf_n` + `xinpcb_n` + `xtcpcb_n` per connection
2. Parse the tagged record stream (switch on `kind` field: `XSO_SOCKET`, `XSO_RCVBUF`, `XSO_SNDBUF`, `XSO_STATS`, `XSO_INPCB`, `XSO_TCPCB`)
3. Extract RTT from `xtcpcb_n.t_srtt` (convert from ticks: `srtt * 1000 / hz / 8` for milliseconds, or use the TCP_RTT_SHIFT constant)
4. Extract PID from `xsocket_n.so_last_pid` — no separate kern.file join needed
5. For connections owned by the tool's user, optionally call `getsockopt(TCP_CONNECTION_INFO)` for byte counters and current RTT

**RTT availability on macOS without a kernel module:** The `xtcpcb_n` struct in `pcblist_n` includes `t_srtt` and `t_rttvar` directly. This means **macOS gets system-wide RTT data from the existing sysctl without needing a kernel module or getsockopt**. The RTT is stored as raw kernel ticks and needs conversion (`t_srtt >> TCP_RTT_SHIFT` then multiply by tick duration), but it's present. This is a meaningful advantage of the macOS `pcblist_n` export over FreeBSD's `xtcpcb` export (which omits RTT entirely).

**SIP (System Integrity Protection) impact:** The `pcblist_n` sysctl is readable without root on macOS (same as FreeBSD's `pcblist`). However, visibility may be limited by privacy protections in newer macOS versions. The tool should handle reduced visibility gracefully and document any sudo requirements.

## 8.5 Updated Platform Architecture

Given that macOS `pcblist_n` includes both RTT and PID data, the platform split is cleaner than originally assumed:

```
                    ┌──────────────────────────────────────┐
                    │         Userspace Tool (Rust)         │
                    │                                      │
                    │   ┌──────────────┐ ┌──────────────┐  │
                    │   │ FreeBSD      │ │ macOS        │  │
                    │   │ Backend      │ │ Backend      │  │
                    │   │              │ │              │  │
                    │   │ Preferred:   │ │ pcblist_n    │  │
                    │   │ /dev/tcpstats│ │ sysctl       │  │
                    │   │ (KLD module) │ │ (has RTT+PID)│  │
                    │   │              │ │              │  │
                    │   │ Fallback:    │ │ + optional   │  │
                    │   │ pcblist      │ │ getsockopt   │  │
                    │   │ sysctl       │ │ TCP_CONN_INFO│  │
                    │   │ (no RTT)     │ │ (byte ctrs)  │  │
                    │   │              │ │              │  │
                    │   │ + kern.file  │ │ (PID already │  │
                    │   │   PID join   │ │  in pcblist) │  │
                    │   └──────────────┘ └──────────────┘  │
                    │                                      │
                    │   ┌──────────────────────────────┐   │
                    │   │  Common: record.rs, delta.rs,│   │
                    │   │  output/json.rs, etc.        │   │
                    │   └──────────────────────────────┘   │
                    └──────────────────────────────────────┘
```
