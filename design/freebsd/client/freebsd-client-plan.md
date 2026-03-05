# FreeBSD bsd-xtcp Client Implementation Plan

## Goal

Add FreeBSD platform support to the bsd-xtcp Rust client, reading TCP socket statistics from the `tcp_stats_kld` kernel module (`/dev/tcpstats`), PID mapping from `kern.file` sysctl, and system-wide stats from `sysctl net.inet.tcp.stats`. Include Nix build targets for cross-compilation (amd64 + aarch64) and FreeBSD VM deploy+test.

## Files to Modify

| File | Action |
|------|--------|
| `src/platform/mod.rs` | Modify: separate macOS/FreeBSD cfg gates, add error variants |
| `src/platform/macos.rs` | Modify: narrow cfg gates to `target_os = "macos"` only |
| `src/platform/freebsd.rs` | **Create**: KLD reader, record parser, kern.file PID join |
| `src/platform/freebsd_layout.rs` | **Create**: `#[repr(C, packed)]` TcpStatsRecord, ioctl/AF constants |
| `src/record.rs` | Modify: add ~20 FreeBSD-specific fields |
| `src/convert.rs` | Modify: map new fields to proto, platform-aware metadata |
| `src/sysctl.rs` | Modify: FreeBSD-specific `read_os_version`, add `read_tcp_stats` |
| `nix/constants.nix` | Modify: add FreeBSD cross targets (amd64 + aarch64) |
| `nix/cross.nix` | Modify: support FreeBSD targets via cross-rs (Docker-based) |
| `nix/freebsd-deploy.nix` | **Create**: SSH deploy + build + test targets for FreeBSD VMs |
| `flake.nix` | Modify: wire in FreeBSD deploy/test packages and cross targets |

---

## Part A: Rust Client Implementation

### Step 1: Extend `RawSocketRecord` (`src/record.rs`)

Add new fields after `start_time_secs` (before `sources`):

```
rtt_min_us: Option<u32>           # RTT minimum (microseconds)
cc_algo: Option<String>           # CC algorithm name ("cubic", "newreno")
tcp_stack: Option<String>         # TCP stack name ("freebsd", "rack")
snd_rexmitpack: Option<u32>      # Retransmit packet counter
rcv_ooopack: Option<u32>         # OOO packet counter
snd_zerowin: Option<u32>         # Zero-window probe counter
rcv_numsacks: Option<u32>        # SACK block counter
ecn_flags: Option<u32>           # ECN flags bitmask
delivered_ce: Option<u32>        # CE marks delivered
received_ce: Option<u32>         # CE marks received
dsack_bytes: Option<u32>         # DSACK bytes
dsack_pack: Option<u32>          # DSACK packets
total_tlp: Option<u32>           # TLP probes sent
total_tlp_bytes: Option<u64>     # TLP bytes sent
timer_rexmt_ms: Option<u32>      # Retransmit timer (ms)
timer_persist_ms: Option<u32>    # Persist timer (ms)
timer_keep_ms: Option<u32>       # Keepalive timer (ms)
timer_2msl_ms: Option<u32>       # TIME_WAIT timer (ms)
timer_delack_ms: Option<u32>     # Delayed ACK timer (ms)
idle_time_ms: Option<u32>        # Time since last recv (ms)
options: Option<u8>              # Negotiated TCP options bitmask
fd: Option<i32>                  # File descriptor (from kern.file join)
```

All `Option<T>` fields default to `None` via `#[derive(Default)]`, so macOS code is unaffected.

### Step 2: Create FreeBSD layout module (`src/platform/freebsd_layout.rs`)

Define a `#[repr(C, packed)]` Rust struct mirroring the C `tcp_stats_record` from `kmod/tcp_stats_kld/tcp_stats_kld.h`:

```rust
#[repr(C, packed)]
pub struct TcpStatsRecord {
    // 320 bytes total, exact field-for-field match with C struct
    // See tcp_stats_kld.h lines 39-133
}
const _: () = assert!(std::mem::size_of::<TcpStatsRecord>() == 320);
```

Also define:
- `TcpstatsVersion` struct for `TCPSTATS_VERSION_CMD` ioctl response
- Ioctl command constants computed from FreeBSD `_IOR`/`_IOW`/`_IO` macros
- AF constants (`AF_INET = 2`, `AF_INET6 = 28`)
- Record flag constants (`TSR_F_IPV6`, etc.)

### Step 3: Refactor platform dispatch (`src/platform/mod.rs`)

Current: both macOS and FreeBSD dispatch to `macos::collect()`.

Change to:
- `#[cfg(target_os = "macos")] pub mod macos;` / `pub mod macos_layout;`
- `#[cfg(target_os = "freebsd")] pub mod freebsd;` / `pub mod freebsd_layout;`
- `collect_tcp_sockets()` dispatches to `macos::collect()` on macOS, `freebsd::collect()` on FreeBSD

Add error variants:
- `DeviceOpen { path, source }` -- `/dev/tcpstats` open failed
- `DeviceRead { source }` -- read failed
- `Ioctl { cmd, source }` -- ioctl failed
- `VersionMismatch { expected, got }` -- protocol version mismatch

### Step 4: Narrow macOS cfg gates (`src/platform/macos.rs`)

Change `#[cfg(any(target_os = "macos", target_os = "freebsd"))]` to `#[cfg(target_os = "macos")]` on the three cfg-gated items (the `use CollectionResult`, the constants, and the `collect()` function). The parser and helpers remain uncfg-gated for cross-platform testing.

### Step 5: Create FreeBSD collector (`src/platform/freebsd.rs`)

Core module with this data flow:

```
/dev/tcpstats-full (or /dev/tcpstats)
  -> open(), TCPSTATS_VERSION_CMD ioctl, read()
  -> parse_kld_records() -> Vec<RawSocketRecord>
  -> enrich_with_pid_mapping() via kern.file sysctl
  -> return CollectionResult
```

Key functions:

1. **`collect()`** -- Top-level entry point, orchestrates KLD read + PID enrichment + system stats
2. **`collect_from_kld()`** -- Opens device, calls version ioctl, reads buffer, calls parser
3. **`parse_kld_records(buf)`** -- Pure function: splits buffer into 320-byte chunks, casts to `TcpStatsRecord`, converts to `RawSocketRecord`. Testable on any platform with synthetic data.
4. **`kld_record_to_raw(tsr)`** -- Converts single C record to `RawSocketRecord`. Handles:
   - AF_INET vs AF_INET6 address extraction
   - NUL-terminated string extraction for cc_algo/tcp_stack
   - Timer normalization (negative -> 0)
   - RTT values passed through directly (already microseconds from KLD)
   - `tsr_so_addr` -> `socket_id` for kern.file join
5. **`enrich_with_pid_mapping(records)`** -- Reads `kern.file` sysctl, builds HashMap<u64, (pid, fd)>, joins on socket_id
6. **`parse_kern_file(buf)`** -- Parses FreeBSD `xfile` struct array. Uses self-describing `xf_size` for stride. Filters to `DTYPE_SOCKET = 2`. Returns HashMap of socket_addr -> (pid, fd).

### Step 6: Add system-wide stats support (`src/sysctl.rs`)

Add a new function `read_tcp_stats()` to read `sysctl net.inet.tcp.stats` and return a struct with the delta-relevant counters from FreeBSD's `struct tcpstat`:

```rust
pub struct TcpSysStats {
    pub connattempt: u64,
    pub accepts: u64,
    pub connects: u64,
    pub drops: u64,
    pub sndtotal: u64,
    pub sndbyte: u64,
    pub sndrexmitpack: u64,
    pub sndrexmitbyte: u64,
    pub rcvtotal: u64,
    pub rcvbyte: u64,
    pub rcvduppack: u64,
    pub rcvbadsum: u64,
}
```

The FreeBSD `struct tcpstat` is an array of `uint64_t` counters. Parse by reading the binary sysctl and extracting at known offsets.

The FreeBSD collector will read this once per collection pass and pass it into the batch builder.

### Step 7: Update conversion layer (`src/convert.rs`)

1. **`raw_to_proto()`** -- Add field mappings for all new `RawSocketRecord` fields:
   - `cc_algo` -> proto field 14
   - `tcp_stack` -> proto field 15
   - `rtt_min_us` -> proto field 19
   - `snd_rexmitpack` -> proto field 29 (rexmit_packets)
   - `rcv_ooopack` -> proto field 30 (ooo_packets)
   - `snd_zerowin` -> proto field 31 (zerowin_probes)
   - `rcv_numsacks` -> proto field 33 (sack_blocks)
   - `dsack_bytes/pack` -> proto fields 34-35
   - `ecn_flags`, `delivered_ce`, `received_ce` -> proto fields 59-61
   - `total_tlp`, `total_tlp_bytes` -> proto fields 67-68
   - `timer_*_ms`, `idle_time_ms` -> proto fields 44-49
   - `options` -> proto field 62 (negotiated_options)
   - `fd` -> proto field 57

2. **`build_metadata()`** -- Use `#[cfg]` gates to set:
   - `Platform::Freebsd` on FreeBSD
   - `DataSource::FreebsdKld` + `DataSource::KernFile` in data_sources

3. **`build_batch()`** -- Accept optional `TcpSysStats` for the SystemSummary delta counters (or make this a new `build_batch_with_sys_stats()` variant).

### Step 8: Fix `read_os_version()` for FreeBSD (`src/sysctl.rs`)

FreeBSD doesn't have `kern.osproductversion`. Split into:
- macOS: reads `kern.osproductversion`
- FreeBSD: reads `kern.osrelease` -> `"FreeBSD 14.3-RELEASE"`

### Step 9: Unit Tests

All parser functions are pure and testable on any platform:

**In `freebsd.rs`:**
- `test_parse_kld_empty_buffer` -- 0 bytes -> 0 records
- `test_parse_kld_single_ipv4` -- synthetic 320-byte IPv4 record
- `test_parse_kld_single_ipv6` -- synthetic 320-byte IPv6 record
- `test_parse_kld_multiple_records` -- 3x320 bytes -> 3 records
- `test_parse_kld_version_mismatch` -- wrong version -> error
- `test_parse_kld_bad_alignment` -- non-320-multiple buffer -> error
- `test_kld_field_mapping` -- verify every field transfers correctly
- `test_timer_normalization` -- negative timers become 0
- `test_cc_algo_string_extraction` -- NUL-terminated string from `[u8; 16]`
- `test_parse_kern_file_socket_entry` -- single socket -> pid mapping
- `test_parse_kern_file_non_socket_skipped`
- `test_parse_kern_file_first_pid_wins`

**In `convert.rs`:**
- `test_raw_to_proto_freebsd_fields` -- new fields map to proto correctly

**In `freebsd_layout.rs`:**
- Compile-time assertion: `size_of::<TcpStatsRecord>() == 320`

---

## Part B: Nix Build System

### Step 10: Add FreeBSD cross targets to `nix/constants.nix`

Add FreeBSD targets alongside existing Darwin targets:

```nix
crossTargets = {
  # Existing macOS targets (cargo-zigbuild)
  "cross-x86_64-darwin" = {
    rustTarget = "x86_64-apple-darwin";
    method = "zigbuild";
  };
  "cross-aarch64-darwin" = {
    rustTarget = "aarch64-apple-darwin";
    method = "zigbuild";
  };
  # New FreeBSD targets (cross-rs / Docker)
  "cross-x86_64-freebsd" = {
    rustTarget = "x86_64-unknown-freebsd";
    method = "cross-rs";
  };
  "cross-aarch64-freebsd" = {
    rustTarget = "aarch64-unknown-freebsd";
    method = "cross-rs";
  };
};
```

Note: `cross-rs` uses Docker containers with FreeBSD cross-compilation sysroot. We need to handle the two methods (`zigbuild` vs `cross-rs`) differently in the cross.nix derivation.

### Step 11: Update cross-compilation (`nix/cross.nix`)

Current `cross.nix` uses `cargo-zigbuild`. Split into two approaches based on target method:

**For zigbuild targets (macOS):** Keep existing `cargo zigbuild` approach.

**For cross-rs targets (FreeBSD):** Use `cross build --target <target>`. This requires:
- `cross` binary (from nixpkgs `cross` or built from crate)
- Docker daemon running (cross-rs spawns Docker containers)
- Cross.toml configuration pointing to the FreeBSD cross image

Alternative approach: since cross-rs requires Docker, create a separate `nix/cross-freebsd.nix` that uses `cross` instead of `cargo-zigbuild`:

```nix
# nix/cross-freebsd.nix
{ pkgs, rustToolchainWithTargets, src, constants, rustTarget }:

pkgs.stdenv.mkDerivation {
  pname = "${constants.pname}-${rustTarget}";
  version = constants.version;
  inherit src;

  nativeBuildInputs = [
    rustToolchainWithTargets
    pkgs.cross     # cross-rs
    pkgs.docker    # required by cross-rs
    pkgs.protobuf
    pkgs.pkg-config
  ];

  env.PROTOC = "${pkgs.protobuf}/bin/protoc";

  buildPhase = ''
    cross build --release --target ${rustTarget}
  '';

  installPhase = ''
    mkdir -p $out/bin
    cp target/${rustTarget}/release/${constants.pname} $out/bin/
  '';
}
```

In `flake.nix`, route to zigbuild or cross-rs based on `targetCfg.method`.

### Step 12: Create FreeBSD VM deploy + test targets (`nix/freebsd-deploy.nix`)

Following the existing `kmod-tests.nix` pattern for SSH deploy + test. Create per-VM and combined packages.

**New Nix packages:**

| Package | Description |
|---------|-------------|
| `bsd-xtcp-freebsd150` | Deploy source to FreeBSD 15.0 VM, build + run `--count 1 --pretty`, verify output |
| `bsd-xtcp-freebsd143` | Deploy source to FreeBSD 14.3 VM, build + run `--count 1 --pretty`, verify output |
| `bsd-xtcp-freebsd` | Run on all VMs sequentially |

**Per-VM script logic:**

```bash
# 1. Ensure Rust is available on FreeBSD VM (install via pkg if needed)
ssh "$VM_HOST" 'command -v cargo || env ASSUME_ALWAYS_YES=yes pkg install -y rust'

# 2. Rsync full project source
rsync -av --delete "${src}/" "$VM_HOST:$VM_DIR/"

# 3. Ensure protobuf compiler is available
ssh "$VM_HOST" 'command -v protoc || env ASSUME_ALWAYS_YES=yes pkg install -y protobuf'

# 4. Build on VM
ssh "$VM_HOST" "cd $VM_DIR && cargo build --release"

# 5. Ensure KLD is loaded
ssh "$VM_HOST" 'kldstat -q -n tcp_stats_kld || kldload tcp_stats_kld'

# 6. Run and capture output
OUTPUT=$(ssh "$VM_HOST" "$VM_DIR/target/release/bsd-xtcp --count 1 --pretty")

# 7. Verify output contains expected FreeBSD markers
echo "$OUTPUT" | grep -q '"platform".*FREEBSD'
echo "$OUTPUT" | grep -q '"data_sources".*FREEBSD_KLD'
echo "$OUTPUT" | grep -q '"cc_algo"'
echo "$OUTPUT" | grep -q '"rtt_us"'
echo "$OUTPUT" | grep -q '"pid"'
echo "PASSED: All FreeBSD fields present"
```

### Step 13: Wire FreeBSD targets into `flake.nix`

Update `flake.nix` to:

1. Import `nix/freebsd-deploy.nix` (similar to how `nix/kmod-tests.nix` is imported)
2. Add FreeBSD deploy packages to the packages output
3. Route cross-compilation to zigbuild or cross-rs based on method
4. Update `cross-all` to include FreeBSD targets
5. Add FreeBSD-specific apps for `nix run` convenience

**New packages in flake output:**
```
bsd-xtcp-freebsd150        Deploy + build + test on FreeBSD 15.0 VM
bsd-xtcp-freebsd143        Deploy + build + test on FreeBSD 14.3 VM
bsd-xtcp-freebsd           All VMs sequentially
cross-x86_64-freebsd       Cross-compile for FreeBSD amd64 (via cross-rs)
cross-aarch64-freebsd      Cross-compile for FreeBSD aarch64 (via cross-rs)
```

**New apps (nix run):**
```
nix run .#bsd-xtcp-freebsd          # deploy + test on all FreeBSD VMs
nix run .#bsd-xtcp-freebsd150       # deploy + test on FreeBSD 15.0 only
nix run .#cross-x86_64-freebsd      # cross-compile for FreeBSD amd64
nix run .#cross-aarch64-freebsd     # cross-compile for FreeBSD aarch64
```

### Step 14: Update flake.nix header comment

Add new packages to the comment block at the top of `flake.nix` (lines 1-37).

---

## Verification

1. **Linux build**: `cargo build` -- verifies compilation, pure parser tests run
2. **Unit tests**: `cargo test` -- all parser tests pass with synthetic data
3. **Nix build (local)**: `nix build .#bsd-xtcp` -- native Linux build works
4. **Nix checks**: `nix flake check` -- clippy, fmt, test all pass
5. **FreeBSD VM deploy + test**: `nix run .#bsd-xtcp-freebsd` -- rsyncs, builds on VM, runs, verifies output
6. **Manual integration on FreeBSD**:
   - `kldload tcp_stats_kld`
   - `./bsd-xtcp --count 1 --pretty`
   - Verify JSON: `platform: "PLATFORM_FREEBSD"`, `data_sources: ["DATA_SOURCE_FREEBSD_KLD", "DATA_SOURCE_KERN_FILE"]`
   - Verify records have `cc_algo`, `tcp_stack`, `rtt_us`, timer fields, ECN, PID
   - Compare record count with `read_tcpstats -c`
7. **Cross-compile (if Docker available)**: `nix build .#cross-x86_64-freebsd`
8. **macOS regression**: Run on macOS, verify macOS-specific fields still work
