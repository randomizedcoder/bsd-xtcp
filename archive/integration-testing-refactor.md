# Plan: Rewrite Test Infrastructure in Rust

## Goal
Replace the three shell test scripts (~1,850 lines) with a single Rust workspace member `tests/kmod-integration/` that is more modular, readable, and uses code generation for combinatorial tests.

## Current Files Being Replaced
- `kmod/tcp_stats_kld/test/integration_tests.sh` (1,056 lines) -- 131 filter integration tests
- `kmod/tcp_stats_kld/test/run-tests-freebsd.sh` (737 lines) -- test orchestrator (compile, sanitizer, bench, live tests)
- `kmod/tcp_stats_kld/test/freebsd-pkg-setup.sh` (56 lines) -- env setup

## New Crate: `tests/kmod-integration/`

### File Structure
```
tests/kmod-integration/
  Cargo.toml
  src/
    main.rs                       -- CLI dispatch + libtest-mimic runner
    framework/
      mod.rs                      -- re-exports
      process.rs                  -- ProcessGroup (server/client lifecycle, Drop cleanup)
      loopback.rs                 -- ifconfig alias setup/teardown
      check.rs                    -- read_count(), check_count(), CheckOp enum
      system.rs                   -- sysctl, kmod build/load/unload, tune_system
      compile.rs                  -- C compilation helpers (unit, asan, memcheck, etc.)
    targets/
      mod.rs                      -- re-exports
      compile_tests.rs            -- unit, memcheck, asan, ubsan, bench, callgrind, kmod, bench_read, gen_conn
      kmod_lifecycle.rs           -- load/unload/read smoke test
      read_bench.rs               -- read-path benchmark at scale (1K/10K/100K conns)
      sysctl_counters.rs          -- sysctl counter invariant validation
      dtrace_probes.rs            -- DTrace probe registration + firing validation
      dos_protection.rs           -- DoS limits (EMFILE, timeout, EINTR)
    filter/
      mod.rs                      -- category dispatcher + shared macros
      macros.rs                   -- simple_tests! and shared_fixture_tests! macros
      port_filter.rs              -- local_port, remote_port filtering (12 tests)
      state_filter.rs             -- exclude/include_state filtering (8 tests)
      ipversion_filter.rs         -- ipv4_only, ipv6_only, dual-stack (8 tests)
      address_filter.rs           -- local_addr, remote_addr, CIDR (18 tests)
      combined_filter.rs          -- multi-dimension filter combos (8 tests)
      format_fields.rs            -- format=full/compact, fields= (3 tests)
      named_profiles.rs           -- sysctl profile create/update/delete vs ioctl (2 tests)
      concurrent_readers.rs       -- parallel reader safety + EBUSY (3 tests)
      combinatorial_coverage.rs   -- systematic 2-way through 6-way combos (69 tests)
    pkg_setup.rs                  -- idempotent FreeBSD env setup
```

### CLI Interface (replaces all 3 scripts)
```
kmod-integration [target]

Compile-only targets:
  unit, memcheck, asan, ubsan, bench, callgrind, kmod, bench_read, gen_conn, all

Live targets (require root + kmod):
  live_smoke, live_bench, live_stats, live_dtrace, live_dos
  live_integration [--category A|B|...|I|all]
  live_all

Setup:
  pkg_setup       -- idempotent FreeBSD env setup

Options:
  --tcp-echo PATH         -- path to tcp-echo binary
  --read-tcpstats PATH    -- path to read_tcpstats binary
  --kmod-src PATH         -- path to kmod source dir
  --cc PATH               -- C compiler (default: cc)
```

### Key Macros

**`simple_tests!`** -- for port_filter, state_filter, ipversion_filter, combined_filter, format_fields (tests that each start their own server+client):
```rust
// port_filter.rs
simple_tests! {
    category: "Port Filtering",
    bind: "127.0.0.10",
    // id,     name,                    ports,                conns, filter,                                                        op,  expected
    a01, "local_port match",            "9001",               20,    "local_addr=127.0.0.10 local_port=9001",                       Eq,  21;
    a02, "local_port no match",         "9001",               20,    "local_addr=127.0.0.10 local_port=9999",                       Eq,  0;
    // ...12 rows total, each is pure data
}
```

**`shared_fixture_tests!`** -- for address_filter, combinatorial_coverage (tests sharing a server setup):
```rust
// combinatorial_coverage.rs
shared_fixture_tests! {
    fixture: setup_combinatorial,
    // id,     name,                                        filter,                                                                         op,  expected
    i01, "{RP,IV} match: RP=9081 ipv4_only",               "remote_port=9081 ipv4_only local_addr=127.0.0.19",                             Eq,  10;
    i02, "{RP,IV} match: RP=9083 ipv6_only",               "remote_port=9083 ipv6_only local_addr=fd00::19",                               Eq,  10;
    // ...69 rows total
}
```

Each macro expands to `libtest-mimic` test entries with automatic fixture management.

### Line Count Comparison
| Component | Shell | Rust | Reduction |
|---|---|---|---|
| Framework/helpers | 160 | ~120 (split across framework/) | 25% |
| Compile targets (unit, asan, etc.) | 290 | ~150 (targets/compile_tests.rs) | 48% |
| Live targets (smoke, bench, stats, dtrace, dos) | 330 | ~200 (targets/live_*.rs) | 39% |
| Filter tests: simple (port, state, ipver, combined, format) | 200 | ~70 (pure data tables) | 65% |
| Filter tests: named_profiles | 74 | ~55 | 26% |
| Filter tests: concurrent_readers | 90 | ~45 | 50% |
| Filter tests: combinatorial_coverage | 280 | ~80 (pure data table) | 71% |
| Macros/boilerplate | 0 | ~60 | -- |
| Pkg setup | 56 | ~40 | 29% |
| CLI dispatch | 100 | ~50 | 50% |
| **Total** | **~1,850** | **~870** | **53%** |

## Implementation Phases

### Phase 1: Crate skeleton + framework
1. Add `tests/kmod-integration` to workspace `Cargo.toml`
2. Create `Cargo.toml` with deps: `anyhow`, `libtest-mimic`
3. Implement `framework/` modules: `process.rs`, `loopback.rs`, `check.rs`
4. Implement `main.rs` with CLI parsing and target dispatch
5. Smoke test: single test case runs on FreeBSD VM

### Phase 2: Port compile-only targets + live targets
6. Implement `framework/system.rs` (kmod build/load/unload, sysctl, tune_system)
7. Implement `framework/compile.rs` (C compilation with cc flags)
8. Port `targets/compile_tests.rs` (unit, memcheck, asan, ubsan, bench, callgrind, kmod, bench_read, gen_conn)
9. Port `targets/kmod_lifecycle.rs`, `read_bench.rs`, `sysctl_counters.rs`, `dtrace_probes.rs`, `dos_protection.rs`
10. Port `pkg_setup.rs` (freebsd-pkg-setup.sh equivalent)

### Phase 3: Port filter integration tests
11. Implement `filter/macros.rs` (simple_tests!, shared_fixture_tests!)
12. Port `port_filter.rs`, `state_filter.rs`, `ipversion_filter.rs`, `combined_filter.rs`, `format_fields.rs` using `simple_tests!`
13. Port `address_filter.rs` using `shared_fixture_tests!` (two fixture groups: IPv4 + IPv6)
14. Port `named_profiles.rs`, `concurrent_readers.rs` as hand-written test functions
15. Port `combinatorial_coverage.rs` using `shared_fixture_tests!` (69 data rows)

### Phase 4: Nix + cleanup
16. Update `nix/freebsd-integration.nix` to build and invoke `kmod-integration` binary
17. Validate on both FreeBSD 15.0 and 14.3 VMs
18. Remove old shell scripts once parity is confirmed

## Files Modified
- `Cargo.toml` -- add workspace member
- `nix/freebsd-integration.nix` -- invoke Rust binary instead of shell scripts
- `nix/kmod-tests.nix` -- update if it references old scripts

## Files Created
- `tests/kmod-integration/Cargo.toml`
- `tests/kmod-integration/src/main.rs`
- `tests/kmod-integration/src/framework/mod.rs`
- `tests/kmod-integration/src/framework/process.rs`
- `tests/kmod-integration/src/framework/loopback.rs`
- `tests/kmod-integration/src/framework/check.rs`
- `tests/kmod-integration/src/framework/system.rs`
- `tests/kmod-integration/src/framework/compile.rs`
- `tests/kmod-integration/src/targets/mod.rs`
- `tests/kmod-integration/src/targets/compile_tests.rs`
- `tests/kmod-integration/src/targets/kmod_lifecycle.rs`
- `tests/kmod-integration/src/targets/read_bench.rs`
- `tests/kmod-integration/src/targets/sysctl_counters.rs`
- `tests/kmod-integration/src/targets/dtrace_probes.rs`
- `tests/kmod-integration/src/targets/dos_protection.rs`
- `tests/kmod-integration/src/filter/mod.rs`
- `tests/kmod-integration/src/filter/macros.rs`
- `tests/kmod-integration/src/filter/port_filter.rs`
- `tests/kmod-integration/src/filter/state_filter.rs`
- `tests/kmod-integration/src/filter/ipversion_filter.rs`
- `tests/kmod-integration/src/filter/address_filter.rs`
- `tests/kmod-integration/src/filter/combined_filter.rs`
- `tests/kmod-integration/src/filter/format_fields.rs`
- `tests/kmod-integration/src/filter/named_profiles.rs`
- `tests/kmod-integration/src/filter/concurrent_readers.rs`
- `tests/kmod-integration/src/filter/combinatorial_coverage.rs`
- `tests/kmod-integration/src/pkg_setup.rs`

## Files Removed (after validation)
- `kmod/tcp_stats_kld/test/integration_tests.sh`
- `kmod/tcp_stats_kld/test/run-tests-freebsd.sh`
- `kmod/tcp_stats_kld/test/freebsd-pkg-setup.sh`

## Verification
1. Build: `cargo build --release -p kmod-integration` compiles on Linux (cross) and FreeBSD
2. Unit: `kmod-integration all` runs compile-only targets on FreeBSD VM (matches old `run-tests-freebsd.sh all` output)
3. Integration: `kmod-integration live_integration --category all` runs all 131 filter tests (matches old `integration_tests.sh all` output -- same pass/fail counts)
4. Live: `kmod-integration live_all` runs all live targets (smoke, bench, stats, dtrace, dos, integration)
5. Nix: `nix run .#integration-test-freebsd150` deploys and runs on VM
