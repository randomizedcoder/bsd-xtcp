# CLAUDE.md

## Build environment

**Always use `nix develop` to run commands.** This project uses a Nix flake to provide the exact toolchain (Rust, protobuf, clang, valgrind, etc.). Running commands outside the Nix shell will fail or use wrong tool versions.

```sh
nix develop --command cargo build
nix develop --command cargo check
nix develop --command cargo clippy
nix develop --command cargo build --release -p kmod-integration
```

## Project structure

- Root crate (`bsd-xtcp`) — FreeBSD TCP stats reader library
- `utils/tcp-echo/` — TCP echo server/client for integration testing
- `tests/kmod-integration/` — Rust-based integration test harness (replaces shell scripts)
- `kmod/tcp_stats_kld/` — FreeBSD kernel module source (C)
- `nix/` — Nix packaging, cross-compilation, VM deployment

## Key commands

```sh
# Check a single crate
nix develop --command cargo check -p kmod-integration

# Lint
nix develop --command cargo clippy --workspace

# Release build
nix develop --command cargo build --release

# Run integration tests on FreeBSD VM
nix run .#integration-test-freebsd150
```
