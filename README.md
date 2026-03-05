# bsd-xtcp

A system-wide TCP socket statistics exporter for FreeBSD and macOS. Polls all TCP connections on the host via kernel sysctl interfaces and exports per-socket metrics (state, congestion window, RTT, retransmits, buffer utilization, process attribution) as structured data — JSON Lines or binary protobuf.

This is the BSD counterpart to [xtcp](https://github.com/randomizedcoder/xtcp) and [xtcp2](https://github.com/randomizedcoder/xtcp2), which use Linux Netlink.

## Quick Start

### Native build (macOS or Linux)

```sh
nix build .
./result/bin/bsd-xtcp --count 1 --pretty
```

### Cross-compile for macOS from Linux

No Xcode or macOS SDK required. Uses Nix + cargo-zigbuild + zig.

```sh
# Option 1: Makefile (simplest)
make cross-aarch64-darwin          # Apple Silicon (M1/M2/M3/M4)
make cross-x86_64-darwin           # Intel Mac
make cross-all                     # both targets

# Option 2: nix run (auto-names output directories)
nix run .#cross-aarch64-darwin     # -> result-cross-aarch64-darwin/bin/bsd-xtcp
nix run .#cross-x86_64-darwin      # -> result-cross-x86_64-darwin/bin/bsd-xtcp
nix run .#build-cross-all          # builds both with separate output dirs

# Option 3: nix build (manual output link)
nix build .#cross-aarch64-darwin -o result-cross-aarch64-darwin
nix build .#cross-all              # both binaries in result/bin/ named by target
```

### Deploy to a Mac

```sh
scp result-cross-aarch64-darwin/bin/bsd-xtcp user@mac:~/
ssh user@mac '~/bsd-xtcp --count 1 --pretty'
```

### Available Nix targets

| Target | Description |
|--------|-------------|
| `default` / `bsd-xtcp` | Native build for current platform |
| `proto` | Standalone protobuf schema validation |
| `cross-x86_64-darwin` | Cross-compile for Intel Mac (Linux host only) |
| `cross-aarch64-darwin` | Cross-compile for Apple Silicon M1/M2/M3/M4 (Linux host only) |
| `cross-all` | All cross targets in one output, binaries named by target triple |

## Overview

The tool reads `sysctl net.inet.tcp.pcblist` (FreeBSD) or `net.inet.tcp.pcblist_n` (macOS) to enumerate every TCP socket on the system in a single kernel round-trip. On macOS, this sysctl includes RTT and PID data directly; on FreeBSD, a kernel module (`tcp_stats_kld`) and `kern.file` join provide the equivalent coverage.

Key properties:

- **Cross-platform:** unified protobuf schema with 78 fields covering both macOS and FreeBSD; platform-specific fields are simply absent when not applicable
- **Configurable intervals:** `--interval SECS` and `--count N` for collection control
- **Multiple output formats:** JSON Lines (current), length-delimited binary protobuf (planned)
- **Low overhead:** targets < 1% CPU and < 10 MB RSS on a developer machine with ~500 sockets
- **Rust implementation:** synchronous collection loop, protobuf via prost, Nix-based build system
- **CI-friendly:** pure parsing functions compile and test on Linux; sysctl calls are cfg-gated

The full design is documented in [freebsd-tcp-stats-design.md](freebsd-tcp-stats-design.md).

## Design Documents

| Document | Description |
|----------|-------------|
| [freebsd-tcp-stats-design.md](freebsd-tcp-stats-design.md) | Master design document with summaries of all sections |
| [design/01-freebsd-data-sources.md](design/01-freebsd-data-sources.md) | FreeBSD kernel data sources (sysctl, getsockopt, kern.file) |
| [design/02-architecture.md](design/02-architecture.md) | Tool architecture, polling, record schemas |
| [design/03-implementation.md](design/03-implementation.md) | Output formats, Rust module structure, implementation phases |
| [design/04-macos-portability.md](design/04-macos-portability.md) | macOS platform differences (pcblist_n, TCP_CONNECTION_INFO) |
| [design/05-kernel-module.md](design/05-kernel-module.md) | FreeBSD tcp_stats_kld kernel module design |
| [design/06-field-comparison.md](design/06-field-comparison.md) | Performance budget, field comparison matrix, open questions |
| [design/07-nix-build-system.md](design/07-nix-build-system.md) | Nix flake build system, security tooling, dev shell |
| [design/08-protobuf-schema.md](design/08-protobuf-schema.md) | Protobuf schema, Rust architecture, traits, dependencies |

## Usage

On macOS:

```sh
# Single collection pass, pretty-printed
cargo run -- --count 1 --pretty

# 3 passes at 2-second intervals
cargo run -- --count 3 --interval 2

# Continuous collection (Ctrl-C to stop)
cargo run
```

Options:

| Flag | Default | Description |
|------|---------|-------------|
| `--interval SECS` | 1 | Collection interval in seconds |
| `--count N` | 0 (infinite) | Number of collection passes |
| `--pretty` | off | Pretty-print JSON output |
| `--help` | | Show usage |

On Linux the binary compiles but returns an `UnsupportedPlatform` error at runtime (sysctl calls are macOS/FreeBSD only). The parser and conversion logic are fully testable on Linux via unit tests with synthetic byte buffers.

On Linux the cross-compiled binaries can be built for macOS — see [Quick Start](#quick-start) above.

## Status

Phases 1-6 are complete. The tool reads live TCP socket data from the macOS kernel via `net.inet.tcp.pcblist_n`, parses the tagged binary stream, converts to the protobuf schema, and outputs JSON Lines to stdout.

| Phase | Status | Description |
|-------|--------|-------------|
| 1 - Build pipeline | Done | Nix flake + proto + prost-build + pbjson serde + cross-compilation via cargo-zigbuild |
| 2 - Sysctl reader | Done | `read_sysctl()`, `read_pcblist_validated()`, `read_clock_hz()` with retry + Linux stubs |
| 3 - macOS pcblist_n parser | Done | Cursor-based tagged record parser with `ConnectionAccumulator` |
| 4 - Record conversion | Done | `RawSocketRecord` intermediate type + proto conversion |
| 5 - JSON output | Done | `JsonSink` with `OutputSink` trait, JSON Lines + pretty-print |
| 6 - CLI + collection loop | Done | Hand-rolled `--interval`/`--count`/`--pretty` args, synchronous loop |
| 7-10 | Not started | Delta tracking, getsockopt enrichment, binary output, system summary enrichment |
| 11-15 | In progress | FreeBSD platform support |

See platform status documents for detailed implementation status:

- [status/macos.md](status/macos.md) -- macOS pcblist_n parser, cross-compilation, phases 1-6
- [status/freebsd.md](status/freebsd.md) -- FreeBSD tcp_stats_kld kernel module, filter parser, test suite (unit/asan/ubsan/memcheck/bench/callgrind/kmod), VM test deployment
