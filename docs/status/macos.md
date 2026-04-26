# macOS Implementation Status

## Phase 1: Build Pipeline (Complete)

The Nix + Protobuf + Rust build pipeline is fully operational. A demo binary compiles the protobuf schema, populates a sample `BatchMessage` with two socket records and a system summary, and serializes it to pretty-printed JSON.

### What was built

**Nix build system** (`flake.nix`, `nix/`)
- Flake inputs: nixpkgs-unstable, rust-overlay, flake-utils, advisory-db
- Rust 1.93.0 pinned via rust-overlay with clippy, rustfmt, rust-src extensions
- Custom `rustPlatform` via `makeRustPlatform` for reproducible builds
- `nix/constants.nix` — centralized pname, version, rustVersion, systems, tool lists, crossTargets
- `nix/package.nix` — `buildRustPackage` with protobuf, pkg-config, darwin framework support
- `nix/cross.nix` — cross-compilation derivation using cargo-zigbuild + zig for macOS targets
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
- Generated code included via `include!` macro in `proto_gen::tcpstats_reader` module

### Issues found and resolved

| Issue | Root cause | Fix |
|-------|-----------|-----|
| `IpVersion::V4` not found | prost strips `IP_VERSION_` prefix, generates `IpVersion4` | Changed to `IpVersion::IpVersion4` |
| `cargo clippy` reported rustc 1.77.2 | `buildRustPackage` hooks set `RUSTC_WRAPPER` to cargo-auditable | Replaced `inputsFrom` with explicit dependency list in shell.nix |
| `cargo clippy` still found old rustc | cargo checks `~/.cargo/bin` for subcommands, found rustup proxy | Set `CARGO_HOME=.cargo` in dev shell env |
| `cargoLock.lockFile` eval error | String interpolation `"${src}/..."` not valid for Nix path | Changed to path concatenation `src + "/Cargo.lock"` |
| `.cargo/` registry staged in git | `CARGO_HOME=.cargo` + `git add -A` | Added `/.cargo` to .gitignore |
| `sysctlbyname` newp arg type mismatch | macOS `libc` expects `*mut c_void`, code used `std::ptr::null()` (`*const`) | Changed to `std::ptr::null_mut()` — silent on native builds, caught by cross-compile |
| cargo-zigbuild HOME cache dir | Nix sandbox sets `$HOME=/homeless-shelter` (doesn't exist) | Set `export HOME=$(mktemp -d)` in buildPhase |

### Cross-compilation (Complete)

Cross-compile macOS binaries from Linux using `cargo-zigbuild` + `zig` (which bundles macOS SDK/linker stubs). No Xcode or macOS SDK installation required.

**Nix files added/changed:**

- `nix/constants.nix` — added `crossTargets` mapping (`cross-x86_64-darwin`, `cross-aarch64-darwin`) to Rust target triples
- `nix/cross.nix` (new) — `stdenv.mkDerivation` using `cargo zigbuild --release --target <triple>`:
  - `nativeBuildInputs`: `rustToolchainWithTargets`, `cargo-zigbuild`, `zig`, `protobuf`, `pkg-config`, `cargoSetupHook`
  - `cargoDeps`: `rustPlatform.importCargoLock` for offline builds
  - `doCheck = false` (can't run Mach-O on Linux)
  - `meta.platforms = platforms.linux` (only builds on Linux hosts)
- `flake.nix` — added `rustToolchainWithTargets` (base toolchain + darwin targets), `crossPackages` conditional on `stdenv.isLinux`, merged into `packages` output

**Key insight:** No Apple frameworks are actually linked at build time. The `SystemConfiguration` entry in `package.nix` is precautionary — `Cargo.lock` shows zero crates that depend on it. The only native dependency is `libc` (for `sysctlbyname`), which zig's bundled `libSystem.B.dylib` stubs satisfy.

**Build convenience targets:**

Three ways to build cross targets, all producing per-target output directories:

| Method | Command | Output |
|--------|---------|--------|
| Makefile | `make cross-aarch64-darwin` | `result-cross-aarch64-darwin/bin/tcpstats-reader` |
| Makefile | `make cross-x86_64-darwin` | `result-cross-x86_64-darwin/bin/tcpstats-reader` |
| Makefile | `make cross-all` | Both targets |
| nix run | `nix run .#cross-aarch64-darwin` | `result-cross-aarch64-darwin/bin/tcpstats-reader` |
| nix run | `nix run .#cross-x86_64-darwin` | `result-cross-x86_64-darwin/bin/tcpstats-reader` |
| nix run | `nix run .#build-cross-all` | Both targets with separate output dirs |
| nix build | `nix build .#cross-all` | `result/bin/tcpstats-reader-{x86_64,aarch64}-apple-darwin` |

The `apps` outputs wrap `nix build` with `--out-link result-<target>` so each target gets its own output directory automatically. The `cross-all` package collects all targets into a single output with binaries named by target triple.

**Binary sizes** (release, stripped):

| Target | Size |
|--------|------|
| `x86_64-apple-darwin` | ~613 KB |
| `aarch64-apple-darwin` | ~633 KB |

**Tested end-to-end:** built on Linux via `nix build .#cross-x86_64-darwin`, scp'd to macOS 11.7.10 x86_64 host, confirmed working with live TCP socket data:

```
$ nix build .#cross-x86_64-darwin
$ file ./result/bin/tcpstats-reader
tcpstats-reader: Mach-O 64-bit x86_64 executable, flags:<NOUNDEFS|DYLDLINK|TWOLEVEL|NO_REEXPORTED_DYLIBS|PIE|HAS_TLV_DESCRIPTORS>

$ scp ./result/bin/tcpstats-reader 172.16.50.135:
tcpstats-reader                                          100%  613KB  17.3MB/s   00:00

$ ssh 172.16.50.135 './tcpstats-reader --count 1'
{"metadata":{"timestampNs":"1772408699298216000","hostname":"dass-MBP.localdomain",
"platform":"PLATFORM_MACOS","osVersion":"11.7.10","intervalMs":1000,
"dataSources":["DATA_SOURCE_MACOS_PCBLIST_N"],"collectionDurationNs":"50816",
"pcblistGeneration":"626","batchSequence":"1","toolVersion":"tcpstats-reader 0.1.0"},
"records":[...7 sockets...],"summary":{"timestampNs":"1772408699298229000",
"intervalMs":1000,"totalSockets":7,"stateCounts":[{"state":"TCP_STATE_CLOSED","count":7}]}}
```

## Phase 2: Sysctl Reader (Complete)

**`src/sysctl.rs`** — Shared sysctl reader with platform cfg-gating.

- `read_sysctl(name)` — two-call pattern (get size, allocate +25% headroom, read)
- `read_pcblist_validated(name, max_retries)` — reads sysctl, parses xinpgen header/trailer, retries if `xig_gen` mismatch between header and trailer
- `read_clock_hz()` — reads `kern.clockrate` struct, returns `hz` field for RTT tick conversion
- `read_os_version()` — reads `kern.osproductversion` for metadata
- All real implementations `#[cfg(any(target_os = "macos", target_os = "freebsd"))]`
- Stubs return `Err(SysctlError::UnsupportedPlatform)` on Linux
- `SysctlError` enum with `thiserror` derives: `NameToMib`, `ReadFailed`, `GenerationMismatch`, `TooSmall`, `UnsupportedPlatform`

## Phase 3: macOS pcblist_n Parser (Complete)

**`src/platform/macos_layout.rs`** — Offset constants isolated for easy correction.

- Record kind tags: `XSO_SOCKET=0x001`, `XSO_RCVBUF=0x002`, `XSO_SNDBUF=0x004`, `XSO_STATS=0x008`, `XSO_INPCB=0x010`, `XSO_TCPCB=0x020`
- Named offset constants for each struct (e.g. `XSOCKET_N_SO_LAST_PID_OFFSET`, `XTCPCB_N_T_SRTT_OFFSET`)
- `roundup64()` for XNU 8-byte record alignment
- `TCP_RTT_SHIFT=3`, `TCP_RTTVAR_SHIFT=2`, `INP_IPV4=0x1`, `INP_IPV6=0x2`

**`src/platform/macos.rs`** — Pure parsing functions (testable on all platforms).

- `parse_pcblist_n(buf, hz)` — walks tagged records, returns `Vec<RawSocketRecord>`
- `ConnectionAccumulator` — collects fields from tagged records for one connection:
  - `parse_xsocket_n()` — socket_id (so_pcb), uid, pid, effective_pid
  - `parse_rcvbuf()` / `parse_sndbuf()` — buffer cc + hiwat
  - `parse_xinpcb_n()` — IP addrs (v4/v6 based on inp_vflag), ports (network byte order), inp_gencnt
  - `parse_xtcpcb_n()` — state, flags, cwnd, ssthresh, maxseg, windows, RTT (raw ticks), seq nums, window scale, dupacks, rxtshift, starttime
  - `build()` — tags data source, returns `RawSocketRecord`
- RTT conversion: `((t_srtt >> TCP_RTT_SHIFT) * 1_000_000) / hz` for microseconds
- Byte-reading helpers: `read_u8_at`, `read_u16_be_at`, `read_i32_at`, `read_u32_at`, `read_u64_at`
- New `XSO_SOCKET` record = new connection group; emit previous when complete
- Unknown kinds skipped gracefully (forward compat)

**`src/platform/mod.rs`** — Error types and platform dispatch.

- `CollectError` enum: `Sysctl`, `Parse`, `Truncated`, `UnknownKind`, `UnsupportedPlatform`
- `CollectionResult { records, generation, collection_duration_ns }`
- `collect_tcp_sockets()` — cfg-dispatches to `macos::collect()` or `stub::collect()`

**`src/platform/stub.rs`** — Linux CI stub returning `Err(CollectError::UnsupportedPlatform)`.

### Design decisions

- **Cursor-based parsing, not `#[repr(C)]` structs** — offsets in `macos_layout.rs` are more robust across XNU versions and testable with synthetic byte blobs on Linux CI.
- **`macos.rs` always compiled** — only `collect()` (which calls sysctl) is cfg-gated. The pure `parse_pcblist_n()` function and all tests run on all platforms.

## Phase 4: Record Types + Proto Conversion (Complete)

**`src/record.rs`** — Internal intermediate types.

- `enum IpAddr { V4([u8; 4]), V6([u8; 16]) }` — raw byte representation
- `struct RawSocketRecord` — ~35 fields with `Option<T>`, all in native Rust types with normalized units (RTT in microseconds)
- Bridge between platform parser and proto conversion

**`src/convert.rs`** — Proto conversion functions.

- `kernel_state_to_proto(i32) -> i32` — macOS TCPS_* (0-10) maps to proto enum (1-11), offset by +1
- `ip_version_to_proto(u8) -> i32`, `ip_addr_to_bytes(&IpAddr) -> Vec<u8>`
- `raw_to_proto(&RawSocketRecord) -> TcpSocketRecord` — maps all fields, sets `sources = [MacosPcblistN]`
- `build_metadata(generation, duration, count, seq, interval_ms) -> CollectionMetadata` — timestamp, hostname, platform, os_version
- `build_summary_from_records(&[TcpSocketRecord], interval_ms) -> SystemSummary` — counts states using BTreeMap
- `build_batch()` — assembles a full `BatchMessage` from raw records

## Phase 5: JSON Output (Complete)

**`src/output/mod.rs`** — Output abstraction.

- `OutputError` enum: `Serialization`, `Io`
- `trait OutputSink { emit(&mut self, &BatchMessage), flush(&mut self), format_name() }`

**`src/output/json.rs`** — JSON Lines sink.

- `JsonSink<W: Write>` wrapping `BufWriter<W>`
- `emit()` uses `serde_json::to_writer` (or `to_writer_pretty` with `--pretty`)
- One JSON object per line (JSON Lines format)

## Phase 6: CLI + Collection Loop (Complete)

**`src/config.rs`** — Minimal hand-rolled CLI config.

- `Config { interval: Duration, count: u64, pretty: bool }`
- `Config::from_args()` — parses `--interval SECS`, `--count N`, `--pretty`, `--help`
- No `clap` dependency

**`src/main.rs`** — Synchronous collection loop.

- Parse config, create `JsonSink` on stdout
- Loop: `collect_tcp_sockets()` -> `build_batch()` -> `sink.emit()` -> sleep
- Stop after `--count` passes (0 = infinite)
- Uses `anyhow::Result` for top-level error handling
- No tokio — single loop with `std::thread::sleep`

### Dependencies added

- `libc = "0.2"` — sysctl FFI calls
- `thiserror = "2"` — typed error enums in library code
- `anyhow = "1"` — error handling in main.rs
- `hostname = "0.4"` — hostname detection for metadata

Not added (intentionally): tokio, clap, tracing, byteorder, nix crate.

## Verified operations

| Command | Result |
|---------|--------|
| `nix develop -c cargo build` | Compiles on Linux (stubs) and macOS |
| `nix develop -c cargo clippy --all-targets -- -D warnings` | Zero warnings |
| `nix develop -c cargo fmt --check` | Clean |
| `nix develop -c cargo test` | 8 tests pass (state mapping, conversion, parser with synthetic byte buffers) |
| On macOS: `cargo run -- --count 1` | Prints one JSON BatchMessage with live socket data |
| On macOS: `cargo run -- --count 3 --interval 2` | Prints 3 batches, 2 seconds apart |
| `nix build .#cross-x86_64-darwin` | Cross-compiles Mach-O x86_64 binary (~613 KB) on Linux |
| `nix build .#cross-aarch64-darwin` | Cross-compiles Mach-O arm64 binary (~633 KB) on Linux |
| `nix build .#cross-all` | Both targets in `result/bin/` named by triple |
| `nix run .#cross-aarch64-darwin` | Builds to `result-cross-aarch64-darwin/` automatically |
| `nix run .#build-cross-all` | Builds both targets with separate output dirs |
| `make cross-all` | Builds both targets via Makefile |
| Cross-compiled x86_64 binary on macOS 11.7.10 | Runs successfully, collects live TCP socket data |

## File inventory

```
Makefile                         # Build targets: build, test, clippy, fmt, cross-*
src/
  lib.rs                       # Library root — 7 module declarations
  main.rs                      # Collection loop with anyhow error handling
  proto_gen.rs                 # Include generated prost + pbjson code
  config.rs                    # CLI arg parser (--interval, --count, --pretty)
  record.rs                    # RawSocketRecord intermediate type
  sysctl.rs                    # Sysctl reader with cfg-gated BSD/Linux
  convert.rs                   # RawSocketRecord -> proto TcpSocketRecord
  platform/
    mod.rs                     # CollectError, CollectionResult, dispatch
    macos.rs                   # pcblist_n parser (always compiled, collect() cfg-gated)
    macos_layout.rs            # XNU struct offset constants
    stub.rs                    # Linux stub
  output/
    mod.rs                     # OutputSink trait, OutputError
    json.rs                    # JSON Lines sink
```

## Known risks and next steps

- **Struct offsets**: The `macos_layout.rs` offsets are derived from XNU headers. They must be validated on a real macOS host by comparing parsed output to `netstat -an`. First run may need offset corrections — the isolated layout file makes this a single-file fix.
- **IPv4-mapped IPv6**: Some sockets use `::ffff:a.b.c.d` with vflag=0x2. Handle in convert.rs later if needed.
- **LISTEN sockets**: No remote addr. Parser handles all-zeros gracefully.

### Remaining phases

| Module | Phase | Purpose |
|--------|-------|---------|
| `delta.rs` | 7 | Per-connection delta tracking (retransmit rate, byte deltas) |
| `platform/macos.rs` (enrich) | 8 | TCP_CONNECTION_INFO getsockopt for richer per-socket data |
| `output/binary.rs` | 9 | Length-delimited binary protobuf output |
| System summary enrichment | 10 | tcp.stats sysctl for system-wide counters in SystemSummary |
| FreeBSD platform | 11-15 | pcblist parser, tcpstats kernel module, kern.file join |
