# macOS Implementation Status

## Phase 1: Build Pipeline (Complete)

The Nix + Protobuf + Rust build pipeline is fully operational. A demo binary compiles the protobuf schema, populates a sample `BatchMessage` with two socket records and a system summary, and serializes it to pretty-printed JSON.

### What was built

**Nix build system** (`flake.nix`, `nix/`)
- Flake inputs: nixpkgs-unstable, rust-overlay, flake-utils, advisory-db
- Rust 1.93.0 pinned via rust-overlay with clippy, rustfmt, rust-src extensions
- Custom `rustPlatform` via `makeRustPlatform` for reproducible builds
- `nix/constants.nix` — centralized pname, version, rustVersion, systems, tool lists
- `nix/package.nix` — `buildRustPackage` with protobuf, pkg-config, darwin framework support
- `nix/proto.nix` — standalone proto validation (`nix build .#proto`)
- `nix/checks.nix` — clippy, fmt, test as separate derivations for parallel CI
- `nix/shell.nix` — dev shell with explicit deps, `CARGO_HOME=.cargo` to isolate from rustup

**Protobuf schema** (`proto/tcp_stats.proto`)
- Full schema from design doc Section 19.2
- 4 enums: `TcpState` (12 values), `Platform` (3), `IpVersion` (3), `DataSource` (7)
- 6 messages: `CollectionMetadata` (11 fields), `TcpSocketRecord` (78 fields), `StateBucket`, `SystemSummary` (18 fields), `BatchMessage`
- Proto3 `optional` on all data fields to distinguish absent vs zero

**Rust code generation** (`build.rs`, `src/proto_gen.rs`)
- prost-build compiles proto and writes descriptor set
- pbjson-build generates serde Serialize/Deserialize impls from the descriptor
- `btree_map(["."])` for deterministic serialization order
- Generated code included via `include!` macro in `proto_gen::bsd_xtcp` module

**Demo binary** (`src/main.rs`)
- Creates a `BatchMessage` with:
  - `CollectionMetadata`: timestamp, hostname, platform=MACOS, schedule="fast", interval=1s
  - ESTABLISHED socket: 127.0.0.1:52301 -> 93.184.216.34:443, cwnd/RTT/buffers/pid
  - TIME_WAIT socket: 127.0.0.1:48920 -> 10.0.0.5:80, 2MSL timer
  - `SystemSummary` with state bucket counts
- Serializes with `serde_json::to_string_pretty` and prints to stdout

**Cargo dependencies** (minimal for phase 1)
- Runtime: prost 0.13, prost-types 0.13, pbjson 0.7, pbjson-types 0.7, serde 1, serde_json 1
- Build: prost-build 0.13, pbjson-build 0.7
- Not yet added: tokio, clap, libc, nix, thiserror, tracing, hostname

### Verified operations

| Command | Result |
|---------|--------|
| `nix develop -c cargo build` | Compiles proto + Rust successfully |
| `nix develop -c cargo run` | Prints JSON BatchMessage to stdout |
| `nix develop -c cargo clippy --all-targets -- -D warnings` | Zero warnings |
| `nix develop -c cargo fmt --check` | Clean |
| `nix build` | Full reproducible release build |
| `./result/bin/bsd-xtcp` | Nix-built binary runs correctly |
| `nix build .#proto` | Proto validates independently |

### Issues found and resolved

| Issue | Root cause | Fix |
|-------|-----------|-----|
| `IpVersion::V4` not found | prost strips `IP_VERSION_` prefix, generates `IpVersion4` | Changed to `IpVersion::IpVersion4` |
| `cargo clippy` reported rustc 1.77.2 | `buildRustPackage` hooks set `RUSTC_WRAPPER` to cargo-auditable | Replaced `inputsFrom` with explicit dependency list in shell.nix |
| `cargo clippy` still found old rustc | cargo checks `~/.cargo/bin` for subcommands, found rustup proxy | Set `CARGO_HOME=.cargo` in dev shell env |
| `cargoLock.lockFile` eval error | String interpolation `"${src}/..."` not valid for Nix path | Changed to path concatenation `src + "/Cargo.lock"` |
| `.cargo/` registry staged in git | `CARGO_HOME=.cargo` + `git add -A` | Added `/.cargo` to .gitignore |

### File inventory

```
flake.nix                 # Top-level Nix orchestration
flake.lock                # Pinned flake inputs
nix/
  constants.nix           # Centralized config
  package.nix             # Rust binary build
  proto.nix               # Proto validation target
  checks.nix              # Clippy, fmt, test checks
  shell.nix               # Dev shell
proto/
  tcp_stats.proto          # Full protobuf schema (78 fields)
Cargo.toml                # Minimal phase-1 dependencies
Cargo.lock                # Committed lockfile
build.rs                  # Proto code generation
src/
  lib.rs                  # Library root
  proto_gen.rs            # Include generated prost + pbjson code
  main.rs                 # Demo binary
```

## Next: Phase 2 — Sysctl Reader

The next step is `src/sysctl.rs`: a shared sysctl reader with retry-on-growth that works on both macOS and FreeBSD. This is the foundation for the `pcblist_n` parser in phase 3.

Modules not yet implemented:

| Module | Phase | Purpose |
|--------|-------|---------|
| `sysctl.rs` | 2 | Shared sysctl reader with retry-on-growth |
| `platform/macos.rs` | 3 | pcblist_n tagged record parser |
| `record.rs` | 4 | Internal `RawSocketRecord` type |
| `convert.rs` | 4 | RawSocketRecord to proto TcpSocketRecord |
| `output/json.rs` | 5 | JSON Lines output sink |
| `config.rs` | 6 | CLI args (clap), schedule configuration |
| `scheduler.rs` | 6 | Multi-schedule timer loop |
| `collector.rs` | 6 | Collection orchestrator |
| `delta.rs` | 7 | Per-connection delta tracking |
| `platform/macos.rs` (enrich) | 8 | TCP_CONNECTION_INFO getsockopt |
| `output/binary.rs` | 9 | Length-delimited binary protobuf |
| System summary | 10 | tcp.stats sysctl + SystemSummary |
