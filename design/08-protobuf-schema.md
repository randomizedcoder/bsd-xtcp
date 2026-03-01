[← Back to main document](../freebsd-tcp-stats-design.md)

# Protobuf Schema and Cross-Platform Rust Architecture

This document defines the protobuf schema for cross-platform TCP socket statistics (macOS + FreeBSD), the Rust module layout maximizing shared code, key trait definitions for platform abstraction, Cargo.toml dependencies, and the configurable collection interval model.

## Table of Contents

- [18. Configurable Collection Intervals](#18-configurable-collection-intervals)
- [19. Protobuf Schema Design](#19-protobuf-schema-design)
  - [19.1 Design Decisions](#191-design-decisions)
  - [19.2 Schema: `proto/tcp_stats.proto`](#192-schema-prototcp_statsproto)
- [20. Rust Module Architecture](#20-rust-module-architecture)
  - [20.1 Module Layout](#201-module-layout)
  - [20.2 Shared vs Platform-Specific Code](#202-shared-vs-platform-specific-code)
  - [20.3 Key Trait: `PlatformCollector`](#203-key-trait-platformcollector)
  - [20.4 Key Trait: `OutputSink`](#204-key-trait-outputsink)
  - [20.5 Scheduler Design](#205-scheduler-design)
- [21. Cargo.toml Dependencies](#21-cargotoml-dependencies)
- [22. macOS-First Implementation Order](#22-macos-first-implementation-order)
- [23. Verification Checklist](#23-verification-checklist)

---

## 18. Configurable Collection Intervals

The fixed 4-tier model (1s/30s/60s/300s) from [Section 3.2](02-architecture.md#32-polling-architecture) is replaced with user-configurable intervals. This provides flexibility for different use cases without baking assumptions about polling frequency into the schema or architecture.

### Configuration Model

- **Minimum interval:** 10ms (for burst-level debugging)
- **Maximum interval:** 24 hours / 86,400,000ms (for long-term trending)
- Users configure one or more named **schedules**, each with its own interval and field set
- The protobuf stores `interval_ms` (`uint32`) rather than a tier enum

### CLI Interface

```
bsd-xtcp \
  --schedule fast=1s \
  --schedule detail=30s \
  --schedule summary=5m \
  --format json \
  --output stdout
```

Each schedule specifies what data to collect:

| Schedule | Typical Interval | Data Collected |
|----------|-----------------|----------------|
| `fast` | 1s | Socket state, congestion control, timers, buffers |
| `detail` | 30s | All of `fast` + process attribution + system stats |
| `summary` | 5m | System-wide aggregates only |

Schedules are independent — each fires on its own timer and produces a `BatchMessage` with its `schedule_name` and `interval_ms` in the `CollectionMetadata`. Consumers distinguish batches by these fields.

### Why Not Fixed Tiers

- Fixed tiers conflate "what to collect" with "how often to collect it" — a user may want process attribution at 1s intervals, or may not want it at all
- Configurable intervals allow the tool to serve both high-frequency debugging (10ms) and low-overhead monitoring (5m+) without code changes
- The protobuf schema remains clean: `interval_ms` is a plain integer, not an enum that needs updating when new tiers are added
- Backward compatibility: the default schedule configuration reproduces the original 4-tier behavior

---

## 19. Protobuf Schema Design

### 19.1 Design Decisions

**Single proto file (`proto/tcp_stats.proto`)** rather than splitting `tcp_record.proto` / `system_record.proto`:
- Messages share enums (`TcpState`, `Platform`, `DataSource`)
- `BatchMessage` wraps both `TcpSocketRecord` and `SystemSummary`
- Total schema is under 300 lines
- Simplifies `prost-build` to a single `.compile_protos()` call

**Proto3 `optional` on all data fields** to distinguish "not available on this platform" (field absent) from "value is genuinely zero" (field present with value 0). This is the cleanest way to handle cross-platform field availability without per-platform message types.

**Unit normalization in the schema:**
- All RTT values normalized to **microseconds** regardless of source:
  - macOS `t_srtt` (kernel ticks): `(t_srtt >> TCP_RTT_SHIFT) * 1_000_000 / hz`
  - macOS `TCP_CONNECTION_INFO` `tcpi_srtt` (milliseconds): `× 1000`
  - FreeBSD `TCP_INFO` (microseconds): use directly
- All timers normalized to **milliseconds**
- All timestamps in **nanoseconds since epoch**

**IP addresses as `bytes`** (not `string`): 4 bytes for IPv4, 16 bytes for IPv6. Avoids formatting overhead at high-frequency collection. `IpVersion` enum disambiguates. JSON output sink formats to human-readable strings.

**Unified `TcpSocketRecord`** (not per-platform messages): one message covering fields from both platforms. Platform-specific fields are simply absent when not applicable. ~80% of fields are shared. This keeps delta tracking, output formatting, and consumers simple.

### 19.2 Schema: `proto/tcp_stats.proto`

```protobuf
syntax = "proto3";

package bsd_xtcp;

// ─── Enums ───────────────────────────────────────────────────────────

// BSD TCP finite state machine states.
// Values match TCPS_* constants from <netinet/tcp_fsm.h>.
enum TcpState {
  TCP_STATE_UNKNOWN     = 0;   // Not yet determined or parse error
  TCP_STATE_CLOSED      = 1;   // TCPS_CLOSED
  TCP_STATE_LISTEN      = 2;   // TCPS_LISTEN
  TCP_STATE_SYN_SENT    = 3;   // TCPS_SYN_SENT
  TCP_STATE_SYN_RECEIVED = 4;  // TCPS_SYN_RECEIVED
  TCP_STATE_ESTABLISHED = 5;   // TCPS_ESTABLISHED
  TCP_STATE_CLOSE_WAIT  = 6;   // TCPS_CLOSE_WAIT
  TCP_STATE_FIN_WAIT_1  = 7;   // TCPS_FIN_WAIT_1
  TCP_STATE_CLOSING     = 8;   // TCPS_CLOSING
  TCP_STATE_LAST_ACK    = 9;   // TCPS_LAST_ACK
  TCP_STATE_FIN_WAIT_2  = 10;  // TCPS_FIN_WAIT_2
  TCP_STATE_TIME_WAIT   = 11;  // TCPS_TIME_WAIT
}

// Platform the collection was performed on.
enum Platform {
  PLATFORM_UNKNOWN = 0;
  PLATFORM_MACOS   = 1;
  PLATFORM_FREEBSD = 2;
}

// IP version for address interpretation.
enum IpVersion {
  IP_VERSION_UNKNOWN = 0;
  IP_VERSION_4       = 1;  // local_addr/remote_addr are 4 bytes
  IP_VERSION_6       = 2;  // local_addr/remote_addr are 16 bytes
}

// Data sources used to populate fields in a record or batch.
enum DataSource {
  DATA_SOURCE_UNKNOWN           = 0;
  DATA_SOURCE_MACOS_PCBLIST_N   = 1;  // sysctl net.inet.tcp.pcblist_n
  DATA_SOURCE_MACOS_TCP_CONN_INFO = 2;  // getsockopt(TCP_CONNECTION_INFO)
  DATA_SOURCE_FREEBSD_PCBLIST   = 3;  // sysctl net.inet.tcp.pcblist
  DATA_SOURCE_FREEBSD_TCP_INFO  = 4;  // getsockopt(TCP_INFO)
  DATA_SOURCE_FREEBSD_KLD       = 5;  // /dev/tcpstats kernel module
  DATA_SOURCE_KERN_FILE         = 6;  // sysctl kern.file (PID mapping)
}

// ─── Collection Metadata ─────────────────────────────────────────────

// Attached to every BatchMessage. Describes the collection context.
message CollectionMetadata {
  // Timestamp when collection started (nanoseconds since Unix epoch).
  uint64 timestamp_ns = 1;

  // Machine hostname.
  string hostname = 2;

  // Platform this collection was performed on.
  Platform platform = 3;

  // OS version string (e.g. "macOS 15.2" or "FreeBSD 14.1-RELEASE").
  string os_version = 4;

  // Configured collection interval for this schedule (milliseconds).
  // Range: 10..86_400_000.
  uint32 interval_ms = 5;

  // User-defined schedule name (e.g. "fast", "detail", "summary").
  string schedule_name = 6;

  // Data sources consulted during this collection pass.
  repeated DataSource data_sources = 7;

  // Wall-clock duration of this collection pass (nanoseconds).
  // Useful for performance monitoring of the tool itself.
  uint64 collection_duration_ns = 8;

  // pcblist generation counter. If header != trailer, snapshot was
  // inconsistent and was retried. This records the final consistent value.
  optional uint64 pcblist_generation = 9;

  // Monotonically increasing sequence number per schedule.
  // Consumers use this to detect gaps (missed collections).
  uint64 batch_sequence = 10;

  // Tool version string (e.g. "bsd-xtcp 0.1.0").
  string tool_version = 11;
}

// ─── Per-Socket Record ───────────────────────────────────────────────

// Unified TCP socket record covering fields from both macOS and FreeBSD.
// Platform-specific fields are simply absent (proto3 optional) when not
// applicable. ~80% of fields are shared across platforms.
message TcpSocketRecord {

  // ── Connection Identity (fields 1–6) ──

  // Local IP address: 4 bytes (IPv4) or 16 bytes (IPv6).
  bytes local_addr = 1;

  // Remote IP address: 4 bytes (IPv4) or 16 bytes (IPv6).
  bytes remote_addr = 2;

  // Local port (host byte order).
  uint32 local_port = 3;

  // Remote port (host byte order).
  uint32 remote_port = 4;

  // IP version, disambiguates addr field lengths.
  IpVersion ip_version = 5;

  // Kernel socket address — stable within a sample, used as join key
  // for PID mapping on FreeBSD (matches xso_so / xf_data).
  optional uint64 socket_id = 6;

  // ── TCP State (fields 7–8) ──

  // TCP finite state machine state.
  TcpState state = 7;

  // Raw TCP flags bitmask (t_flags from xtcpcb / xtcpcb_n).
  optional uint32 tcp_flags = 8;

  // ── Congestion Control (fields 9–15) ──

  // Congestion window (bytes).
  optional uint32 snd_cwnd = 9;

  // Slow start threshold (bytes).
  optional uint32 snd_ssthresh = 10;

  // Send window (bytes).
  optional uint32 snd_wnd = 11;

  // Receive window (bytes).
  optional uint32 rcv_wnd = 12;

  // Maximum segment size (bytes).
  optional uint32 maxseg = 13;

  // Congestion control algorithm name (e.g. "cubic", "newreno").
  // FreeBSD: xt_cc from xtcpcb. macOS: not available via pcblist_n.
  optional string cc_algo = 14;

  // TCP stack name (e.g. "freebsd", "rack").
  // FreeBSD only: xt_stack from xtcpcb.
  optional string tcp_stack = 15;

  // ── RTT (fields 16–21) ──
  // All RTT values normalized to microseconds.
  // macOS t_srtt: (t_srtt >> TCP_RTT_SHIFT) * 1_000_000 / hz
  // macOS TCP_CONNECTION_INFO tcpi_srtt: ms × 1000
  // FreeBSD TCP_INFO: already microseconds

  // Smoothed RTT (microseconds).
  optional uint32 rtt_us = 16;

  // RTT variance (microseconds).
  optional uint32 rttvar_us = 17;

  // Retransmission timeout (microseconds).
  optional uint32 rto_us = 18;

  // Minimum observed RTT (microseconds).
  // macOS pcblist_n: t_rttmin. FreeBSD KLD: tcpi_rttmin.
  optional uint32 rtt_min_us = 19;

  // Most recent (instantaneous) RTT (microseconds).
  // macOS TCP_CONNECTION_INFO only: tcpi_rttcur × 1000.
  optional uint32 rtt_cur_us = 20;

  // Number of RTT updates (macOS pcblist_n: t_rttupdated).
  optional uint32 rtt_update_count = 21;

  // ── Sequence Numbers (fields 22–26) ──

  // Next send sequence number.
  optional uint32 snd_nxt = 22;

  // Oldest unacknowledged send sequence number.
  optional uint32 snd_una = 23;

  // Highest sequence number sent.
  optional uint32 snd_max = 24;

  // Next expected receive sequence number.
  optional uint32 rcv_nxt = 25;

  // Peer's most recently advertised window edge.
  optional uint32 rcv_adv = 26;

  // ── Window Scale (fields 27–28) ──

  // Send window scale factor (0–14).
  optional uint32 snd_wscale = 27;

  // Receive window scale factor (0–14).
  optional uint32 rcv_wscale = 28;

  // ── Counters (fields 29–36) ──

  // Retransmitted packets (cumulative).
  optional uint32 rexmit_packets = 29;

  // Out-of-order packets received (cumulative).
  optional uint32 ooo_packets = 30;

  // Zero-window probes sent (cumulative).
  optional uint32 zerowin_probes = 31;

  // Consecutive duplicate ACKs received.
  // macOS pcblist_n: t_dupacks. FreeBSD KLD: tcpi_dupacks.
  optional uint32 dupacks = 32;

  // Distinct SACK blocks received.
  // FreeBSD KLD/TCP_INFO only: tcpi_rcv_numsacks.
  optional uint32 sack_blocks = 33;

  // DSACK bytes received. FreeBSD pcblist only.
  optional uint32 dsack_bytes = 34;

  // DSACK packets received. FreeBSD pcblist only.
  optional uint32 dsack_packets = 35;

  // Retransmit backoff exponent (t_rxtshift).
  // macOS pcblist_n: t_rxtshift. FreeBSD: via KLD/TCP_INFO.
  optional uint32 rxt_shift = 36;

  // ── Byte/Packet Counters (fields 37–43) ──
  // macOS TCP_CONNECTION_INFO provides per-connection byte/packet counters.
  // FreeBSD does not have these in any interface.

  // Total bytes sent (macOS TCP_CONNECTION_INFO: tcpi_txbytes).
  optional uint64 tx_bytes = 37;

  // Total bytes received (macOS TCP_CONNECTION_INFO: tcpi_rxbytes).
  optional uint64 rx_bytes = 38;

  // Total packets sent (macOS TCP_CONNECTION_INFO: tcpi_txpackets).
  optional uint64 tx_packets = 39;

  // Total packets received (macOS TCP_CONNECTION_INFO: tcpi_rxpackets).
  optional uint64 rx_packets = 40;

  // Retransmitted bytes (macOS TCP_CONNECTION_INFO: tcpi_txretransmitbytes).
  optional uint64 retransmit_bytes = 41;

  // Out-of-order bytes received (macOS TCP_CONNECTION_INFO: tcpi_rxoutoforderbytes).
  optional uint64 ooo_bytes = 42;

  // Retransmitted packets (macOS TCP_CONNECTION_INFO: tcpi_txretransmitpackets).
  // Separate from rexmit_packets (field 29) which comes from pcblist.
  optional uint64 retransmit_packets_total = 43;

  // ── Timers (fields 44–49) ──
  // All timer values in milliseconds. 0 = timer not running.

  // Retransmit timer remaining (ms).
  optional uint32 timer_rexmt_ms = 44;

  // Persist timer remaining (ms).
  optional uint32 timer_persist_ms = 45;

  // Keepalive timer remaining (ms).
  optional uint32 timer_keep_ms = 46;

  // 2MSL (TIME_WAIT) timer remaining (ms).
  optional uint32 timer_2msl_ms = 47;

  // Delayed ACK timer remaining (ms).
  optional uint32 timer_delack_ms = 48;

  // Time since last data received (ms).
  optional uint32 idle_time_ms = 49;

  // ── Buffers (fields 50–53) ──

  // Send buffer bytes currently in use.
  optional uint32 snd_buf_used = 50;

  // Send buffer high watermark (max capacity).
  optional uint32 snd_buf_hiwat = 51;

  // Receive buffer bytes currently in use.
  optional uint32 rcv_buf_used = 52;

  // Receive buffer high watermark (max capacity).
  optional uint32 rcv_buf_hiwat = 53;

  // ── Process Attribution (fields 54–58) ──

  // Process ID owning this socket.
  // macOS: so_last_pid from xsocket_n (built into pcblist_n).
  // FreeBSD: joined from kern.file via socket_id.
  optional int32 pid = 54;

  // Effective PID (macOS only: so_e_pid from xsocket_n).
  // Differs from pid when socket has been transferred between processes.
  optional int32 effective_pid = 55;

  // User ID owning the socket.
  optional uint32 uid = 56;

  // File descriptor number (from kern.file join or known FD).
  optional int32 fd = 57;

  // Process command name (e.g. "firefox", "curl").
  optional string command = 58;

  // ── ECN (fields 59–61) ──

  // ECN flags bitmask.
  // FreeBSD: xt_ecn from xtcpcb.
  optional uint32 ecn_flags = 59;

  // ECN CE (Congestion Experienced) marks delivered to upper layer.
  // FreeBSD KLD/TCP_INFO: tcpi_delivered_ce.
  optional uint32 ecn_ce_delivered = 60;

  // ECN CE marks received.
  // FreeBSD KLD/TCP_INFO: tcpi_received_ce.
  optional uint32 ecn_ce_received = 61;

  // ── Negotiated Options (field 62) ──

  // Bitmask of negotiated TCP options.
  // Bit 0: Timestamps, Bit 1: SACK, Bit 2: Window Scale,
  // Bit 3: ECN, Bit 4: TFO.
  // FreeBSD KLD/TCP_INFO: tcpi_options.
  // macOS TCP_CONNECTION_INFO: tcpi_options (TCPCI_OPT_*).
  optional uint32 negotiated_options = 62;

  // ── TFO — TCP Fast Open (fields 63–64) ──

  // macOS TCP_CONNECTION_INFO TFO state as a 15-bit bitmask.
  // Bits: cookie_req, cookie_rcv, syn_loss, syn_data_sent,
  //       syn_data_acked, syn_data_rcv, cookie_req_rcv, cookie_sent,
  //       cookie_invalid, cookie_wrong, no_cookie_rcv,
  //       heuristics_disable, send_blackhole, recv_blackhole,
  //       onebyte_proxy.
  // macOS only. FreeBSD has a single-bit TFO indicator.
  optional uint32 tfo_state_macos = 63;

  // Whether TFO was negotiated (FreeBSD: tcpi_options & TCPI_OPT_TFO).
  optional bool tfo_negotiated = 64;

  // ── Loss / Reorder Detection (fields 65–66) ──

  // Connection is currently in loss recovery.
  // macOS TCP_CONNECTION_INFO: TCPCI_FLAG_LOSSRECOVERY.
  optional bool in_loss_recovery = 65;

  // Reordering has been detected on this connection.
  // macOS TCP_CONNECTION_INFO: TCPCI_FLAG_REORDERING_DETECTED.
  optional bool reordering_detected = 66;

  // ── TLP — Tail Loss Probe (fields 67–68) ──
  // FreeBSD KLD/TCP_INFO only.

  // Total tail loss probes sent.
  optional uint32 tlp_probes_sent = 67;

  // Total TLP bytes sent.
  optional uint64 tlp_bytes_sent = 68;

  // ── Platform-Specific (fields 69–71) ──

  // UDP encapsulation port (FreeBSD: xt_encaps_port).
  optional uint32 encaps_port = 69;

  // Inpcb generation count — monotonically increasing per-connection,
  // used for change detection (FreeBSD: inp_gencnt).
  optional uint64 inp_gencnt = 70;

  // Connection start time in seconds (macOS: t_starttime from xtcpcb_n).
  optional uint32 start_time_secs = 71;

  // ── Computed Deltas (fields 72–77) ──
  // These are computed by the tool's delta tracker, not read from kernel.
  // Only present when a previous sample exists for this connection.

  // Retransmit packets this interval.
  optional int32 delta_rexmit_packets = 72;

  // OOO packets this interval.
  optional int32 delta_ooo_packets = 73;

  // Zero-window probes this interval.
  optional int32 delta_zerowin = 74;

  // Time since this connection was first observed by the tool (ms).
  optional uint64 connection_age_ms = 75;

  // Bytes sent this interval (macOS only, from TX_CONNECTION_INFO deltas).
  optional int64 delta_tx_bytes = 76;

  // Bytes received this interval (macOS only, from TCP_CONNECTION_INFO deltas).
  optional int64 delta_rx_bytes = 77;

  // ── Source Tracking (field 78) ──

  // Which data sources contributed to this record's fields.
  repeated DataSource sources = 78;
}

// ─── System-Wide Summary ─────────────────────────────────────────────

// Per-state socket count entry for the SystemSummary.
message StateBucket {
  TcpState state = 1;
  uint32 count = 2;
}

// System-wide TCP statistics for a collection interval.
message SystemSummary {
  // Timestamp when this summary was collected (nanoseconds since epoch).
  uint64 timestamp_ns = 1;

  // Collection interval (milliseconds).
  uint32 interval_ms = 2;

  // Total TCP sockets observed in this sample.
  uint32 total_sockets = 3;

  // Socket count per TCP state.
  repeated StateBucket state_counts = 4;

  // ── Delta Counters (from sysctl net.inet.tcp.stats) ──
  // All deltas computed over the interval_ms window.

  optional uint64 delta_conn_attempts = 5;    // tcps_connattempt
  optional uint64 delta_accepts = 6;          // tcps_accepts
  optional uint64 delta_connects = 7;         // tcps_connects
  optional uint64 delta_drops = 8;            // tcps_drops
  optional uint64 delta_snd_total_packets = 9;  // tcps_sndtotal
  optional uint64 delta_snd_bytes = 10;       // tcps_sndbyte
  optional uint64 delta_snd_rexmit_packets = 11;  // tcps_sndrexmitpack
  optional uint64 delta_snd_rexmit_bytes = 12;    // tcps_sndrexmitbyte
  optional uint64 delta_rcv_total_packets = 13;    // tcps_rcvtotal
  optional uint64 delta_rcv_bytes = 14;       // tcps_rcvbyte
  optional uint64 delta_rcv_dup_packets = 15; // tcps_rcvduppack
  optional uint64 delta_rcv_badsum = 16;      // tcps_rcvbadsum

  // ── Computed Rates ──

  // Retransmit rate: delta_snd_rexmit_packets / delta_snd_total_packets.
  // Absent if delta_snd_total_packets is zero.
  optional double retransmit_rate = 17;

  // Duplicate rate: delta_rcv_dup_packets / delta_rcv_total_packets.
  // Absent if delta_rcv_total_packets is zero.
  optional double duplicate_rate = 18;
}

// ─── Top-Level Wrapper ───────────────────────────────────────────────

// Top-level message emitted once per collection pass.
// Each scheduled collection produces one BatchMessage.
message BatchMessage {
  // Collection context (always present).
  CollectionMetadata metadata = 1;

  // Per-socket records collected in this pass.
  repeated TcpSocketRecord records = 2;

  // System-wide summary (present when the schedule collects system stats).
  optional SystemSummary summary = 3;
}
```

### Field Coverage Summary

| Field Range | Category | Count | Shared | macOS Only | FreeBSD Only |
|-------------|----------|-------|--------|------------|--------------|
| 1–6 | Connection identity | 6 | 6 | — | — |
| 7–8 | TCP state | 2 | 2 | — | — |
| 9–15 | Congestion control | 7 | 5 | — | 2 (cc_algo*, tcp_stack) |
| 16–21 | RTT | 6 | 4 | 2 (rtt_cur_us, rtt_update_count) | — |
| 22–26 | Sequence numbers | 5 | 5 | — | — |
| 27–28 | Window scale | 2 | 2 | — | — |
| 29–36 | Counters | 8 | 5 | — | 3 (sack_blocks, dsack_*) |
| 37–43 | Byte/packet counters | 7 | — | 7 | — |
| 44–49 | Timers | 6 | 6 | — | — |
| 50–53 | Buffers | 4 | 4 | — | — |
| 54–58 | Process attribution | 5 | 4 | 1 (effective_pid) | — |
| 59–61 | ECN | 3 | 1 | — | 2 (ce_delivered, ce_received) |
| 62 | Options | 1 | 1 | — | — |
| 63–64 | TFO | 2 | 1 | 1 (tfo_state_macos) | — |
| 65–66 | Loss/reorder | 2 | — | 2 | — |
| 67–68 | TLP | 2 | — | — | 2 |
| 69–71 | Platform-specific | 3 | — | 1 (start_time_secs) | 2 (encaps_port, inp_gencnt) |
| 72–77 | Computed deltas | 6 | 4 | 2 (delta_tx/rx_bytes) | — |
| 78 | Source tracking | 1 | 1 | — | — |
| **Total** | | **78** | **~55** | **~16** | **~11** |

\* `cc_algo` is available on FreeBSD via pcblist and not on macOS via pcblist_n, but is conceptually shared since consumers don't care about the source.

### Cross-Reference to Existing Design Docs

| Proto Field | macOS Source (design/04) | FreeBSD Source (design/01) | FreeBSD KLD (design/05) |
|-------------|--------------------------|---------------------------|------------------------|
| `rtt_us` | `xtcpcb_n.t_srtt` (ticks→μs) | — | `tcp_info.tcpi_rtt` (μs) |
| `rttvar_us` | `xtcpcb_n.t_rttvar` (ticks→μs) | — | `tcp_info.tcpi_rttvar` (μs) |
| `rto_us` | `xtcpcb_n.t_rxtcur` (ticks→μs) | — | `tcp_info.tcpi_rto` (μs) |
| `rtt_min_us` | `xtcpcb_n.t_rttmin` | — | `tcp_info.tcpi_rttmin` (μs) |
| `rtt_cur_us` | `tcp_connection_info.tcpi_rttcur` (ms→μs) | — | — |
| `snd_cwnd` | `xtcpcb_n.snd_cwnd` | `xtcpcb.t_snd_cwnd` | `tcp_info.tcpi_snd_cwnd` |
| `pid` | `xsocket_n.so_last_pid` | `kern.file` join | `kern.file` join |
| `effective_pid` | `xsocket_n.so_e_pid` | — | — |
| `tx_bytes` | `tcp_connection_info.tcpi_txbytes` | — | — |
| `rx_bytes` | `tcp_connection_info.tcpi_rxbytes` | — | — |
| `tfo_state_macos` | `tcp_connection_info` TFO bitfields | — | — |
| `tlp_probes_sent` | — | — | `tcp_info.tcpi_total_tlp` |
| `tcp_stack` | — | `xtcpcb.xt_stack` | `xtcpcb.xt_stack` |

---

## 20. Rust Module Architecture

### 20.1 Module Layout

```
src/
├── main.rs                 # CLI entry, signal handling, run loop
├── lib.rs                  # Library root for testing
├── config.rs               # CLI args (clap), schedule configuration
│                           #   --schedule name=interval[,fields...]
│                           #   --format json|binary
│                           #   --output stdout|file|socket
├── proto_gen.rs            # include! of prost-generated code
├── sysctl.rs               # SHARED: sysctl reader with retry-on-growth
├── record.rs               # SHARED: RawSocketRecord (internal repr)
├── convert.rs              # SHARED: RawSocketRecord → proto TcpSocketRecord
├── delta.rs                # SHARED: DeltaTracker (per-connection deltas)
├── scheduler.rs            # SHARED: multi-schedule timer loop
├── collector.rs            # SHARED: orchestrator (platform → delta → proto → output)
├── platform/
│   ├── mod.rs              # PlatformCollector trait
│   ├── macos.rs            # macOS: pcblist_n parser, TCP_CONNECTION_INFO, PID built-in
│   └── freebsd.rs          # FreeBSD: pcblist parser, /dev/tcpstats, kern.file PID join
└── output/
    ├── mod.rs              # OutputSink trait
    ├── json.rs             # JSON Lines (pbjson serde)
    ├── binary.rs           # Length-delimited binary protobuf
    └── stdout.rs           # Human-readable debug output
```

### 20.2 Shared vs Platform-Specific Code

**Shared (~80% of codebase):**
- Proto schema and generated types (`proto_gen.rs`)
- CLI argument parsing and configuration (`config.rs`)
- Sysctl reader with retry-on-growth (`sysctl.rs`)
- Internal record types (`record.rs`)
- Proto conversion (`convert.rs`)
- Delta tracking (`delta.rs`)
- Multi-schedule timer loop (`scheduler.rs`)
- Collection orchestration (`collector.rs`)
- All output sinks (`output/*.rs`)

**macOS-specific (`platform/macos.rs`):**
- `pcblist_n` tagged variable-length record parser (switch on `xgn_kind`)
- `TCP_CONNECTION_INFO` (0x106) getsockopt for byte counters, current RTT, TFO state
- PID extraction from `xsocket_n.so_last_pid` / `so_e_pid` (no `kern.file` join needed)
- RTT conversion from kernel ticks: `(t_srtt >> TCP_RTT_SHIFT) * 1_000_000 / hz`

**FreeBSD-specific (`platform/freebsd.rs`):**
- `pcblist` flat `xtcpcb` array parser
- `/dev/tcpstats` reader (KLD module, when available)
- `kern.file` reader and socket→PID join via `xso_so` / `xf_data`
- `TCP_INFO` (32) getsockopt for own-process sockets
- `tcp.states` sysctl reader (not available on macOS)

### 20.3 Key Trait: `PlatformCollector`

```rust
use std::collections::HashMap;

/// Result of a single collection pass.
pub struct CollectionResult {
    pub records: Vec<RawSocketRecord>,
    pub system_summary: Option<RawSystemSummary>,
    pub data_sources: Vec<DataSource>,
    pub pcblist_generation: Option<u64>,
}

/// Process information from kern.file or xsocket_n.
pub struct ProcessInfo {
    pub pid: i32,
    pub effective_pid: Option<i32>,  // macOS only
    pub uid: u32,
    pub fd: Option<i32>,
    pub command: Option<String>,
}

/// Abstraction over platform-specific socket collection.
///
/// Each platform implements this trait. The collector/scheduler
/// code is platform-agnostic and calls through this interface.
pub trait PlatformCollector: Send + Sync {
    /// Collect TCP socket records. The `fields` parameter controls
    /// what data to gather (socket state, process info, system stats).
    fn collect(&self, fields: &FieldSet) -> Result<CollectionResult, CollectError>;

    /// Return the platform identifier for metadata.
    fn platform(&self) -> Platform;

    /// Return the OS version string (e.g. "macOS 15.2").
    fn os_version(&self) -> String;

    /// Optional enrichment via getsockopt (owned sockets only).
    /// macOS: TCP_CONNECTION_INFO for byte counters, current RTT, TFO.
    /// FreeBSD: TCP_INFO for RTT, sequences, SACK on own sockets.
    /// Default: no-op.
    fn enrich_with_getsockopt(
        &self,
        records: &mut [RawSocketRecord],
    ) -> Result<(), CollectError> {
        let _ = records;
        Ok(())
    }

    /// Build socket→PID map.
    /// macOS: no-op (PID is embedded in pcblist_n xsocket_n).
    /// FreeBSD: reads kern.file and builds xso_so→ProcessInfo map.
    /// Default: empty map (no process attribution).
    fn build_process_map(&self) -> Result<HashMap<u64, ProcessInfo>, CollectError> {
        Ok(HashMap::new())
    }
}
```

### 20.4 Key Trait: `OutputSink`

```rust
/// Abstraction over output formats.
///
/// Each format (JSON, binary protobuf, human-readable) implements
/// this trait. The collector calls emit() once per collection pass.
pub trait OutputSink: Send + Sync {
    /// Write a batch of records to the output destination.
    fn emit(&mut self, batch: &BatchMessage) -> Result<(), OutputError>;

    /// Flush any buffered output (e.g. for file or socket sinks).
    fn flush(&mut self) -> Result<(), OutputError>;

    /// Human-readable name for logging (e.g. "json", "binary", "stdout").
    fn format_name(&self) -> &'static str;
}
```

**Implementations:**

| Sink | File | Description |
|------|------|-------------|
| `JsonSink` | `output/json.rs` | JSON Lines via `pbjson` serde traits. IP addresses formatted as human-readable strings. One JSON object per line. |
| `BinarySink` | `output/binary.rs` | Length-delimited binary protobuf (`prost::encode_length_delimited`). Suitable for streaming to analytics backends. |
| `StdoutSink` | `output/stdout.rs` | Human-readable tabular format for interactive debugging. Displays a summary line per socket. |

### 20.5 Scheduler Design

The scheduler manages multiple concurrent collection schedules, each firing independently on its own interval:

```rust
use std::time::Duration;
use tokio::time;

/// What data to collect — each schedule selects a subset.
pub struct FieldSet {
    pub socket_state: bool,       // Always true
    pub congestion_control: bool,
    pub rtt: bool,
    pub sequence_numbers: bool,
    pub timers: bool,
    pub buffers: bool,
    pub counters: bool,
    pub process_attribution: bool,
    pub byte_counters: bool,      // Requires getsockopt enrichment
    pub system_stats: bool,       // SystemSummary
}

/// A named collection schedule.
pub struct Schedule {
    /// User-defined name (e.g. "fast", "detail").
    pub name: String,
    /// Collection interval. Range: 10ms..24h.
    pub interval: Duration,
    /// What fields to collect on this schedule.
    pub fields: FieldSet,
}

// The scheduler spawns one tokio::time::interval per Schedule.
// Each schedule fires independently and calls:
//   collector.collect(schedule) → BatchMessage → output_sink.emit()
// Schedules are concurrent but each individual collection is sequential
// (sysctl → enrich → delta → convert → emit).
```

The scheduler uses `tokio::time::interval` per schedule, which provides automatic drift compensation. If a collection pass takes longer than the interval, the next tick fires immediately (burst mode) and then resumes normal cadence.

---

## 21. Cargo.toml Dependencies

```toml
[package]
name = "bsd-xtcp"
version = "0.1.0"
edition = "2021"

[dependencies]
# Protobuf runtime
prost = "0.13"
prost-types = "0.13"

# JSON serialization for proto types
pbjson = "0.7"
pbjson-types = "0.7"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Async runtime
tokio = { version = "1", features = ["full"] }

# CLI
clap = { version = "4", features = ["derive"] }

# System interfaces
libc = "0.2"
nix = { version = "0.29", features = ["net", "socket"] }

# Error handling
thiserror = "2"
anyhow = "1"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Hostname
hostname = "0.4"

[build-dependencies]
# Proto compilation
prost-build = "0.13"
pbjson-build = "0.7"
```

### `build.rs`

```rust
use std::io::Result;

fn main() -> Result<()> {
    let mut prost_config = prost_build::Config::new();
    prost_config.btree_map(["."]);

    let descriptor_path =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap())
            .join("proto_descriptor.bin");

    prost_config.file_descriptor_set_path(&descriptor_path);
    prost_config.compile_protos(&["proto/tcp_stats.proto"], &["proto/"])?;

    let descriptor_set = std::fs::read(&descriptor_path)?;
    pbjson_build::Builder::new()
        .register_descriptors(&descriptor_set)?
        .build(&[".bsd_xtcp"])?;

    Ok(())
}
```

### `proto_gen.rs`

```rust
// Include prost-generated code.
pub mod bsd_xtcp {
    include!(concat!(env!("OUT_DIR"), "/bsd_xtcp.rs"));
    include!(concat!(env!("OUT_DIR"), "/bsd_xtcp.serde.rs"));
}
```

---

## 22. macOS-First Implementation Order

Development starts on macOS because `pcblist_n` provides the richest single-sysctl dataset (RTT + PID without kernel module), and macOS is the primary developer platform.

| Phase | Module(s) | Description |
|-------|-----------|-------------|
| 1 | `proto/tcp_stats.proto`, `build.rs`, `proto_gen.rs` | Define data contract, verify proto compilation |
| 2 | `sysctl.rs` | Shared sysctl reader with retry-on-growth (works on both platforms) |
| 3 | `platform/macos.rs` | `pcblist_n` tagged record parser — highest-risk binary parsing, fuzz-test early |
| 4 | `record.rs`, `convert.rs` | Internal `RawSocketRecord` type and conversion to proto `TcpSocketRecord` |
| 5 | `output/json.rs` | JSON Lines output for immediate debugging and validation |
| 6 | `config.rs`, `scheduler.rs`, `collector.rs`, `main.rs` | Wire together CLI, scheduling, collection, and output |
| 7 | `delta.rs` | Per-connection delta tracking (rexmit, OOO, zerowin, age) |
| 8 | `platform/macos.rs` (enrichment) | `TCP_CONNECTION_INFO` getsockopt for byte counters, current RTT, TFO, loss/reorder |
| 9 | `output/binary.rs` | Length-delimited binary protobuf output |
| 10 | System summary | `tcp.stats` sysctl parsing + `SystemSummary` message population |

**FreeBSD phases** (after macOS is working):

| Phase | Description |
|-------|-------------|
| 11 | `platform/freebsd.rs` — `pcblist` flat array parser |
| 12 | `kern.file` reader and socket→PID join |
| 13 | `/dev/tcpstats` KLD reader (optional, when module is loaded) |
| 14 | `tcp.states` sysctl reader |
| 15 | `TCP_INFO` getsockopt enrichment for own-process sockets |

All shared code (delta tracking, output sinks, scheduling, proto conversion) works unchanged on FreeBSD — only the `PlatformCollector` implementation differs.

---

## 23. Verification Checklist

After this design is approved, verify the following before implementation begins:

- [ ] **Proto field coverage vs. field comparison table** ([design/06](06-field-comparison.md)):
  every row in the Section 10 comparison table has a corresponding proto field or is documented as intentionally omitted (pacing rate, delivery rate, busy time — Linux-only fields)

- [ ] **macOS `xtcpcb_n` field coverage** ([design/04](04-macos-portability.md)):
  all fields from the `xtcpcb_n`, `xsocket_n`, and `xsockbuf_n` struct tables are mapped to proto fields

- [ ] **FreeBSD `xtcpcb` field coverage** ([design/01](01-freebsd-data-sources.md)):
  all fields from the `xtcpcb`, `xinpcb`, and `xsocket` struct tables are mapped to proto fields

- [ ] **FreeBSD KLD field coverage** ([design/05](05-kernel-module.md)):
  `tcp_info` fields exposed via `/dev/tcpstats` (RTT, RTO, sequences, window scale, SACK, TLP) are mapped

- [ ] **Unit normalization**:
  RTT conversion formulas cover all three source formats (macOS ticks, macOS ms, FreeBSD μs)

- [ ] **Configurable intervals replace fixed tiers**:
  no references to the fixed 4-tier model (1s/30s/60s/300s) remain in this document as requirements — the architecture uses `Schedule` with arbitrary `Duration`

- [ ] **Nix build integration** ([design/07](07-nix-build-system.md)):
  `prost-build` and `protoc` are provided by the Nix build environment; `proto/tcp_stats.proto` is compiled via `build.rs`
