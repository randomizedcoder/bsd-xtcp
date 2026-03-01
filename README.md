# bsd-xtcp

A system-wide TCP socket statistics exporter for FreeBSD and macOS. Polls all TCP connections on the host via kernel sysctl interfaces and exports per-socket metrics (state, congestion window, RTT, retransmits, buffer utilization, process attribution) as structured data — JSON Lines or binary protobuf.

This is the BSD counterpart to [xtcp](https://github.com/randomizedcoder/xtcp) and [xtcp2](https://github.com/randomizedcoder/xtcp2), which use Linux Netlink.

## Overview

The tool reads `sysctl net.inet.tcp.pcblist` (FreeBSD) or `net.inet.tcp.pcblist_n` (macOS) to enumerate every TCP socket on the system in a single kernel round-trip. On macOS, this sysctl includes RTT and PID data directly; on FreeBSD, a kernel module (`tcp_stats_kld`) and `kern.file` join provide the equivalent coverage.

Key properties:

- **Cross-platform:** unified protobuf schema with 78 fields covering both macOS and FreeBSD; platform-specific fields are simply absent when not applicable
- **Configurable intervals:** user-defined named schedules from 10ms to 24h (e.g. `--schedule fast=1s --schedule detail=30s`)
- **Multiple output formats:** JSON Lines, length-delimited binary protobuf, human-readable stdout
- **Low overhead:** targets < 1% CPU and < 10 MB RSS on a developer machine with ~500 sockets
- **Rust implementation:** async runtime (tokio), protobuf via prost, Nix-based build system

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
