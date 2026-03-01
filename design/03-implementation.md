[← Back to main document](../freebsd-tcp-stats-design.md)

# Output Formats and Implementation Plan

## Table of Contents

- [6. Output Formats](#6-output-formats)
  - [6.1 JSON Lines (default)](#61-json-lines-default)
  - [6.2 CSV](#62-csv)
  - [6.3 Binary (for streaming to analytics backend)](#63-binary-for-streaming-to-analytics-backend)
- [7. Implementation Plan](#7-implementation-plan)
  - [7.1 Language: Rust](#71-language-rust)
  - [7.2 Module Structure](#72-module-structure)
  - [7.3 Core Sysctl Reader Pattern](#73-core-sysctl-reader-pattern)
  - [7.4 Implementation Phases](#74-implementation-phases)

---

## 6. Output Formats

### 6.1 JSON Lines (default)

One JSON object per line. Each poll cycle produces a header line followed by per-socket lines:

```json
{"type":"system","ts":1709312400000000000,"interval_ms":1000,"total_sockets":47,"state_counts":[0,3,0,0,38,2,1,0,0,1,2]}
{"type":"socket","ts":1709312400000000000,"local":"192.168.1.5:52341","remote":"151.101.1.69:443","state":"ESTABLISHED","cwnd":65536,"ssthresh":2147483647,"mss":1460,"rcv_wnd":131328,"snd_wnd":65535,"cc":"cubic","rexmit":0,"ooo":0,"snd_buf_pct":0.0,"rcv_buf_pct":12.3,"pid":1234,"command":"firefox"}
```

### 6.2 CSV

Header row on first output, data rows thereafter. One file per record type (system, socket).

### 6.3 Binary (for streaming to analytics backend)

Protocol Buffers or MessagePack encoding of the same record structure. Defined in a `.proto` file for cross-language compatibility. This format is used when the tool streams to the NetPulse ingestion tier (future).

---

## 7. Implementation Plan

### 7.1 Language: Rust

Consistent with the Linux agent codebase. Shared traits and serialization code between Linux and FreeBSD agents. The build system is Nix-based with a pinned Rust 1.93.1 toolchain and integrated security analysis tools — see [Section 13–17: Nix Build System](07-nix-build-system.md) for the full build and tooling design. Key crates:

- `libc` — raw sysctl bindings, struct definitions
- `nix` — higher-level POSIX wrappers
- `serde` / `serde_json` — JSON serialization
- `tokio` — async timer loop (or `std::thread::sleep` for the simple case)
- `prost` — Protocol Buffers (if binary output needed)

### 7.2 Module Structure

```
netpulse-agent-freebsd/
├── src/
│   ├── main.rs              # CLI entry point, argument parsing, poll loop
│   ├── pcblist.rs           # sysctl net.inet.tcp.pcblist reader & parser
│   ├── tcpstat.rs           # sysctl net.inet.tcp.stats reader & delta computation
│   ├── tcpstates.rs         # sysctl net.inet.tcp.states reader
│   ├── procmap.rs           # sysctl kern.file reader, PID-to-socket mapping
│   ├── procinfo.rs          # sysctl kern.proc.pid.<pid> for command names
│   ├── record.rs            # Per-socket and system-wide record structs
│   ├── delta.rs             # Delta tracking for cumulative counters
│   ├── output/
│   │   ├── mod.rs           # Output trait
│   │   ├── json.rs          # JSON Lines emitter
│   │   ├── csv.rs           # CSV emitter
│   │   └── binary.rs        # Binary/protobuf emitter (future)
│   └── platform/
│       ├── mod.rs           # Platform abstraction trait
│       ├── freebsd.rs       # FreeBSD sysctl implementations
│       └── macos.rs         # macOS proc_pidinfo / libproc implementations (future)
├── proto/
│   └── tcpstats.proto       # Protocol buffer definitions (future)
├── Cargo.toml
└── README.md
```

### 7.3 Core Sysctl Reader Pattern

All sysctl readers follow the same retry-with-growth pattern:

```rust
fn read_sysctl(name: &str) -> Result<Vec<u8>> {
    let mut buf_size: usize = 0;
    // First call: get required size
    sysctlbyname(name, null_mut(), &mut buf_size, null(), 0)?;

    // Allocate with headroom (connections can appear between calls)
    buf_size = buf_size * 5 / 4;
    let mut buf = vec![0u8; buf_size];

    // Second call: read data
    sysctlbyname(name, buf.as_mut_ptr(), &mut buf_size, null(), 0)?;
    buf.truncate(buf_size);
    Ok(buf)
}
```

For `pcblist`, wrap this in a generation-count validation loop (compare header and trailer `xinpgen.xig_gen`).

### 7.4 Implementation Phases

**Phase 1: Core socket enumeration and display (sysctl-only baseline)**

- Implement `pcblist.rs`: parse `xtcpcb` array from sysctl binary output
- Implement `record.rs`: define the per-socket record struct
- Implement JSON output
- Implement 1s polling loop with stdout output
- Result: a working tool that dumps all TCP sockets with state/cwnd/MSS/buffers every second (no RTT yet)

**Phase 2: `tcp_stats_kld` kernel module**

- Implement the kernel module as specified in [Section 11](05-kernel-module.md)
- Build, test, and load on FreeBSD development host
- Implement `/dev/tcpstats` reader in Rust as an alternative backend to pcblist
- Result: full per-socket `tcp_info` (including RTT, RTO, sequences) for all system sockets
- The userspace tool auto-detects whether `/dev/tcpstats` exists and falls back to sysctl if not

**Phase 3: Process attribution**

- Implement `procmap.rs`: read `kern.file`, build socket→PID lookup
- Implement `procinfo.rs`: resolve PID→command name
- Join with pcblist/devtcpstats data via `tsr_so_addr` / `xso_so` key
- Add tiered polling (1s without process info, 30s with process info)

**Phase 4: Delta tracking and system-wide stats**

- Implement `delta.rs`: track previous sample per-connection, compute deltas
- Implement `tcpstat.rs`: read and delta system-wide TCP counters
- Implement `tcpstates.rs`: read per-state connection counts
- Add Slow (60s) and Aggregate (300s) tiers

**Phase 5: macOS portability**

- Implement `platform/macos.rs` using `proc_pidinfo` / `libproc` for socket enumeration
- macOS uses the same `tcp_info` struct via `getsockopt` but socket enumeration differs
- macOS does not have `net.inet.tcp.pcblist` — use `proc_listpids` + `proc_pidfdinfo` instead
- Note: macOS does NOT get the kernel module (kext deprecated); remains sysctl/libproc only

**Phase 6: Output formats and integration**

- Add CSV output
- Add binary/protobuf output
- Add Unix domain socket or HTTP endpoint for local dashboard consumption
- Add signal handling (SIGHUP to reload config, SIGTERM for graceful shutdown)
