# FreeBSD `tcp_stats_kld` Filter Parsing — Implementation Log

[Back to filter parsing plan](filter-parsing-plan.md) | [Back to filter parsing design](filter-parsing.md)

## Progress Tracker

| Phase | Description | Status | Date |
|-------|-------------|--------|------|
| 1 | Expand `tcpstats_filter` struct + update ioctl | Done | 2026-03-02 |
| 2 | Port filtering in read path | Done | 2026-03-02 |
| 3 | Dual-compile parser scaffold + port parsing | Done | 2026-03-02 |
| 4 | State parsing (exclude + include_state) | Done | 2026-03-02 |
| 5 | IPv4 address parsing + CIDR matching | Done | 2026-03-02 |
| 6 | IPv6 address parsing + CIDR matching | Done | 2026-03-02 |
| 7 | Format, fields, flags parsing + validation | Done | 2026-03-02 |
| 8 | Sysctl profile integration | Done | 2026-03-02 |
| 9 | Comprehensive testing + fuzz harness | Done | 2026-03-02 |

---

## Log Entries

_(Entries are prepended — newest first)_

---

### 2026-03-02 — Phases 1-9 complete

All phases implemented in a single session. Summary of changes:

**Phase 1: Expand filter struct**
- `tcp_stats_kld.h`: Replaced 8-byte v1 `tcpstats_filter` with ~128-byte v2
  struct. Added `TSF_VERSION=2`, per-state exclude flags, mode flags, port
  arrays, IPv4/IPv6 address fields, field_mask, format, spare.
  `_Static_assert` for size <= 256.
- `tcp_stats_kld.h`: Added `#ifdef __FreeBSD__` guard around `<sys/ioccom.h>`
  with ioctl macro stubs for Linux/macOS userspace compilation.
- `tcp_stats_kld.c`: Initialize `sc_filter.version = TSF_VERSION` in open().
  Added version check in `TCPSTATS_SET_FILTER` ioctl.
- `test/read_tcpstats.c`: Updated filter init to set v2 fields.

**Phase 2: Port + address filtering in read path**
- `tcp_stats_kld.c`: Added IP version filter (`TSF_IPV4_ONLY`/`TSF_IPV6_ONLY`),
  simplified state filtering to use `state_mask` only, added local/remote port
  matching, added IPv4 CIDR address matching.
- `test/read_tcpstats.c`: Added `-P port` flag for kernel-side local port
  filter. Combined `-L` and `-P` into single ioctl call.

**Phase 3-7: Parser implementation (all in one)**
- Created `tcp_stats_filter_parse.h`: Parser API, field group bitmasks.
- Created `tcp_stats_filter_parse.c` (~600 lines): Full dual-compile parser
  with all directives: `local_port`, `remote_port`, `exclude`, `include_state`,
  `local_addr`, `remote_addr`, `format`, `fields`, `ipv4_only`, `ipv6_only`.
  Includes IPv4 manual parser with CIDR, IPv6 parser with `::` compression,
  cross-directive validation.
- Created `test/test_filter_parse.c`: 78 test cases covering all positive
  paths, structural rejections, port/state/address/format/fields rejections,
  and value verification (network byte order, bitmask values, address parsing).
- Updated `Makefile`: `SRCS += tcp_stats_filter_parse.c`.

**Phase 8: Sysctl profile integration**
- `tcp_stats_kld.c`: Added profile struct (`tcpstats_profile`), SLIST
  management, sx lock, profile cdevsw + open handler that copies pre-parsed
  filter into per-fd softc. Added `dev.tcpstats.last_error` sysctl,
  `dev.tcpstats.profile_set` sysctl for create/update/delete,
  dynamic per-profile sysctls under `dev.tcpstats.profiles.<name>`.
  Profile device created at `/dev/tcpstats/<name>`. MOD_UNLOAD destroys
  all profiles.

**Phase 9: Fuzz harness + nix integration**
- Created `test/fuzz_filter_parse.c`: AFL++/libFuzzer dual harness.
- `nix/constants.nix`: Added `"aflplusplus"` to `securityTools`.
- `filter-parsing-plan.md`: Updated Phase 9 fuzz commands to use
  `afl-clang-fast` (not deprecated `afl-gcc`) and note `nix develop` provides
  the tools.

**Test results:** 78/78 unit tests pass (compiled and run via `nix develop`).
AFL++ build verified with `afl-clang-fast`.

**Remaining:** VM integration tests (rsync, compile with FreeBSD `make`,
load module, run `read_tcpstats` commands from plan).
