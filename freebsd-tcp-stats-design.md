# FreeBSD TCP Socket Statistics Export Tool — Design Document

## 1. Purpose and Scope

This document specifies the design of a FreeBSD (and macOS) userspace tool that polls all TCP sockets on the system at configurable intervals and exports per-socket `tcp_info`-equivalent statistics along with process attribution metadata. This tool is the core sensor component of the NetPulse Mac/FreeBSD Developer Edition described in the business plan.

### Goals

- Enumerate all TCP sockets system-wide via `sysctl net.inet.tcp.pcblist`
- Retrieve per-socket TCP statistics equivalent to `struct tcp_info`
- Map each socket to its owning process (PID, command name, UID)
- Poll at configurable intervals: 1s, 30s, 60s, 300s
- Export structured data suitable for local dashboard display or upstream streaming
- Operate with minimal overhead on developer-scale workloads (hundreds of sockets, not tens of thousands)

### Non-Goals (Developer Edition)

- No remediation or TCP tuning (observe-only)
- No fleet-wide aggregation or SaaS streaming
- No eBPF/DTrace instrumentation (sysctl and getsockopt only)
- No per-route congestion control manipulation (FreeBSD/macOS limitation)

---

## Table of Contents

| # | Section | Document |
|---|---------|----------|
| 2 | [FreeBSD Kernel Data Sources](#2-freebsd-kernel-data-sources) | [design/01-freebsd-data-sources.md](design/01-freebsd-data-sources.md) |
| 3–5 | [Tool Architecture and Record Schemas](#3-5-tool-architecture-and-record-schemas) | [design/02-architecture.md](design/02-architecture.md) |
| 6–7 | [Output Formats and Implementation Plan](#6-7-output-formats-and-implementation-plan) | [design/03-implementation.md](design/03-implementation.md) |
| 8 | [macOS Portability Considerations](#8-macos-portability-considerations) | [design/04-macos-portability.md](design/04-macos-portability.md) |
| 11 | [Kernel Module: `tcp_stats_kld`](#11-kernel-module-tcp_stats_kld) | [design/05-kernel-module.md](design/05-kernel-module.md) |
| 9–10, 12 | [Performance Budget, Field Comparison, and Open Questions](#9-10-12-performance-budget-field-comparison-and-open-questions) | [design/06-field-comparison.md](design/06-field-comparison.md) |
| 13–17 | [Nix Build System](#13-17-nix-build-system) | [design/07-nix-build-system.md](design/07-nix-build-system.md) |
| 18–23 | [Protobuf Schema and Cross-Platform Rust Architecture](#18-23-protobuf-schema-and-cross-platform-rust-architecture) | [design/08-protobuf-schema.md](design/08-protobuf-schema.md) |

---

## 2. FreeBSD Kernel Data Sources

Three kernel interfaces provide the data needed, all accessible from unprivileged userspace (some fields require root): the `tcp.pcblist` sysctl for bulk socket enumeration and TCP state, `getsockopt(TCP_INFO)` for per-socket detailed state (RTT, RTO, sequence numbers), and `tcp.stats`/`tcp.states`/`kern.file` for system-wide counters and process-to-socket mapping. The `pcblist` sysctl provides ~20 fields per socket in a single kernel round-trip, but critically omits RTT — the most important diagnostic metric. A two-tier architecture addresses this: Tier 1 (all sockets via sysctl) and Tier 2 (own-process sockets via `getsockopt`).

**[Read full section →](design/01-freebsd-data-sources.md)**

---

## 3–5. Tool Architecture and Record Schemas

The tool uses a tiered polling architecture with four intervals (1s, 30s, 60s, 300s), each collecting progressively more data. The data flow reads `tcp.pcblist`, `kern.file`, and `tcp.stats` sysctls, merges and enriches the results, then emits structured records. Each poll sample produces one record per TCP socket containing connection identity, TCP state, congestion control parameters, retransmission counters, timer state, buffer utilization, and process attribution. A system-wide summary record provides aggregate counters and connection state distribution. Generation counters ensure snapshot consistency.

**[Read full section →](design/02-architecture.md)**

---

## 6–7. Output Formats and Implementation Plan

The tool outputs JSON Lines (default), CSV, or binary (Protocol Buffers) formats. Implementation is in Rust, consistent with the Linux agent codebase, using `libc`, `serde`, and `tokio`. The module structure separates sysctl readers (`pcblist.rs`, `tcpstat.rs`, `procmap.rs`), record definitions, delta tracking, and output emitters. Six implementation phases progress from core sysctl-based enumeration through the kernel module, process attribution, delta tracking, macOS portability, and output format integration.

**[Read full section →](design/03-implementation.md)**

---

## 8. macOS Portability Considerations

macOS diverges from FreeBSD despite shared BSD lineage: it uses `TCP_CONNECTION_INFO` (not `TCP_INFO`), provides `pcblist_n` (tagged variable-length records instead of flat `xtcpcb` arrays), and includes PID attribution directly in the socket export struct. The `pcblist_n` sysctl also includes RTT data (`t_srtt`) — meaning macOS gets system-wide RTT without a kernel module. macOS cannot use a kernel extension (kext deprecated since macOS 11), so it remains userspace-only. The platform split is clean: FreeBSD uses the KLD module (or sysctl fallback) + `kern.file` join; macOS uses `pcblist_n` (with built-in RTT + PID) + optional `getsockopt(TCP_CONNECTION_INFO)` for byte counters.

**[Read full section →](design/04-macos-portability.md)**

---

## 11. Kernel Module: `tcp_stats_kld`

A FreeBSD kernel loadable module that exposes `/dev/tcpstats`, providing system-wide `tcp_info` data (RTT, RTO, rttvar, rttmin, sequence numbers, window scale, SACK state) for every TCP socket in a single kernel pass — without requiring file descriptors. The module is read-only, uses `cr_canseeinpcb()` for credential enforcement (UID/GID/jail/MAC isolation), and emits fixed-size 320-byte records via `uiomove()` streaming. The security architecture has five independent layers. With this module, the sysctl-only "~20 fields" limitation becomes "~35 fields," closing the most critical gaps for developer-facing TCP profiling.

**[Read full section →](design/05-kernel-module.md)**

---

## 9, 10, 12. Performance Budget, Field Comparison, and Open Questions

The performance budget targets < 1% CPU and < 10 MB RSS on a developer machine with ~500 sockets (~1.5ms/s overhead). A comprehensive field comparison matrix shows coverage across Linux `tcp_info` (~60 fields), FreeBSD `xtcpcb` (~20), FreeBSD KLD (~35), macOS `pcblist_n` (~25), and macOS `TCP_CONNECTION_INFO`. The remaining gap vs. Linux (pacing rate, delivery rate, busy time, per-connection byte counters) reflects genuine differences in the BSD TCP stacks. Open questions cover DTrace integration, kqueue-based scheduling, jail awareness, local dashboard protocol, kernel module upstreaming, and sequence number exposure considerations.

**[Read full section →](design/06-field-comparison.md)**

---

## 13–17. Nix Build System

The build system uses a Nix flake with modular `.nix` files under a `nix/` directory. The Rust binary is built using `rustPlatform.buildRustPackage` (standard cargo) with the toolchain pinned to Rust 1.93.1 via `rust-overlay`. Protobuf compilation is handled by `prost-build` in `build.rs` with `protoc` provided by the Nix build environment. The flake provides four targets: `nix build` (binary), `nix build .#proto` (proto validation), `nix flake check` (clippy, fmt, tests, cargo-audit, cargo-deny, doc), and `nix develop` (dev shell with the full security analysis toolkit — cargo-audit, cargo-deny, cargo-fuzz, cargo-geiger, cargo-vet, cargo-nextest, cargo-tarpaulin, cargo-machete, cargo-udeps). Fuzz targets cover the highest-risk code: sysctl binary parsers for `pcblist`, `kern.file`, and macOS `pcblist_n`.

**[Read full section →](design/07-nix-build-system.md)**

---

## 18–23. Protobuf Schema and Cross-Platform Rust Architecture

The protobuf schema (`proto/tcp_stats.proto`) defines a unified `TcpSocketRecord` message with 78 fields covering both macOS and FreeBSD, using proto3 `optional` to distinguish "not available on this platform" from "genuinely zero." All RTT values are normalized to microseconds, all timers to milliseconds, and IP addresses are stored as `bytes` (4 or 16) for efficiency. A `BatchMessage` wraps per-socket records and an optional `SystemSummary` with `CollectionMetadata` that includes a configurable `interval_ms` and `schedule_name` — replacing the fixed 4-tier polling model with user-defined schedules (10ms–24h). The Rust architecture uses a `PlatformCollector` trait for platform abstraction (~80% shared code), an `OutputSink` trait for format abstraction (JSON Lines, binary protobuf, human-readable), and a `tokio`-based multi-schedule timer loop. Development follows a macOS-first implementation order since `pcblist_n` provides the richest single-sysctl dataset.

**[Read full section →](design/08-protobuf-schema.md)**
