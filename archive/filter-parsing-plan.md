# FreeBSD `tcp_stats_kld` Filter Parsing — Incremental Implementation Plan

[Back to filter parsing design](filter-parsing.md) | [Back to kernel module design](kernel-module.md)

## Overview

This document is the **step-by-step build plan** for adding filter parsing to
the `tcp_stats_kld` kernel module. The design is specified in
[filter-parsing.md](filter-parsing.md). Each phase adds exactly one capability,
includes specific validation commands, unit tests, and integration tests.

Progress is tracked in [filter-parsing-log.md](filter-parsing-log.md).

---

## Prerequisites

All prerequisites from [implementation-plan.md](implementation-plan.md) still
apply. Additionally:

| Requirement | How to verify |
|---|---|
| Steps 1-15 complete (module functional) | `ssh root@192.168.122.41 'kldstat \| grep tcp_stats'` |
| tcp-echo built on VM | `ssh root@192.168.122.41 'ls /root/bsd-xtcp/target/release/tcp-echo'` |
| read_tcpstats built on VM | `ssh root@192.168.122.41 'ls /root/bsd-xtcp/kmod/tcp_stats_kld/test/read_tcpstats'` |

**VM details:**
- **Host:** `root@192.168.122.41` (libvirt/KVM)
- **OS:** FreeBSD 15.0-RELEASE (GENERIC)
- **Access:** `ssh root@192.168.122.41`

---

## Development Workflow

All development happens on the dev host; the FreeBSD VM is used only for
compilation, loading, and testing.

### Sync source to VM

```sh
rsync -av --delete kmod/ root@192.168.122.41:/root/bsd-xtcp/kmod/
```

### Compile on VM

```sh
ssh root@192.168.122.41 'cd /root/bsd-xtcp/kmod/tcp_stats_kld && make clean && make'
```

### Load and test on VM

```sh
ssh root@192.168.122.41 'kldunload tcp_stats_kld 2>/dev/null; kldload /root/bsd-xtcp/kmod/tcp_stats_kld/tcp_stats_kld.ko && dmesg | tail -5'
```

### One-liner: sync + build + deploy

```sh
rsync -av --delete kmod/ root@192.168.122.41:/root/bsd-xtcp/kmod/ && \
ssh root@192.168.122.41 'cd /root/bsd-xtcp/kmod/tcp_stats_kld && make clean && make && kldunload tcp_stats_kld 2>/dev/null; kldload ./tcp_stats_kld.ko && dmesg | tail -5'
```

### Userspace parser testing (no VM required for Phases 3-7)

```sh
cd kmod/tcp_stats_kld
cc -o test_filter_parse test/test_filter_parse.c tcp_stats_filter_parse.c -I. -DTEST_HARNESS
./test_filter_parse
```

### tcp-echo setup for integration tests

```sh
# Build tcp-echo on VM (first time only)
ssh root@192.168.122.41 'cd /root/bsd-xtcp && cargo build --release -p tcp-echo'

# Start server on test ports
ssh root@192.168.122.41 '/root/bsd-xtcp/target/release/tcp-echo server --ports 9001,9002,9003 &'

# Create 60 connections (20 per port, round-robin)
ssh root@192.168.122.41 '/root/bsd-xtcp/target/release/tcp-echo client --ports 9001,9002,9003 --connections 60 --rate 10000 --duration 60 &'

# Cleanup
ssh root@192.168.122.41 'pkill tcp-echo'
```

---

## File Layout

### Current state

```
kmod/tcp_stats_kld/
    Makefile                    # SRCS = tcp_stats_kld.c
    tcp_stats_kld.h             # Shared header (149 lines)
    tcp_stats_kld.c             # Module implementation (410 lines)
    test/
        Makefile                # Builds read_tcpstats
        read_tcpstats.c         # Userspace test reader (189 lines)
    tools/
        decode_tcpstats.py      # Binary decoder
```

### Final state (after all phases)

```
kmod/tcp_stats_kld/
    Makefile                    # SRCS += tcp_stats_filter_parse.c
    tcp_stats_kld.h             # Updated: record struct unchanged, filter struct moved out
    tcp_stats_kld.c             # Updated: expanded filtering, profile support (~700 lines)
    tcp_stats_filter_parse.h    # NEW: parser API, expanded filter struct, field bitmasks
    tcp_stats_filter_parse.c    # NEW: dual-compile parser (~600 lines)
    test/
        Makefile                # Updated: builds test_filter_parse too
        read_tcpstats.c         # Updated: v2 filter support
        test_filter_parse.c     # NEW: userspace unit test harness
        fuzz_filter_parse.c     # NEW: AFL/libFuzzer harness
    tools/
        decode_tcpstats.py      # Unchanged
```

---

## Phase 1: Expand `tcpstats_filter` Struct + Update Ioctl

**Goal:** Replace the 8-byte v1 `tcpstats_filter` struct with the ~128-byte v2
struct. Update the ioctl handler to accept v2 and reject v1. This is a clean
break — old tools must be recompiled.

### Files to modify

| File | Changes |
|---|---|
| `tcp_stats_kld.h:137-143` | Replace `tcpstats_filter` struct with v2 (from filter-parsing.md §4) |
| `tcp_stats_kld.c:32-39` | `tcpstats_softc.sc_filter` grows automatically |
| `tcp_stats_kld.c:66-88` | `tcpstats_open()`: set `sc_filter.version = TSF_VERSION`, `state_mask = 0xFFFF` |
| `tcp_stats_kld.c:343-348` | `TCPSTATS_SET_FILTER`: add version check |
| `test/read_tcpstats.c:89-98` | Update filter setup to use v2 struct |

### Implementation

**`tcp_stats_kld.h`** — replace the filter struct and defines (lines 137-148):

```c
/* --- Filter struct v2 --- */
#define TSF_VERSION             2
#define TSF_MAX_PORTS           8

struct tcpstats_filter {
    uint32_t    version;                /* Must be TSF_VERSION */
    uint16_t    state_mask;             /* Bitmask of (1 << TCPS_*); 0xFFFF = all */
    uint16_t    _pad0;
    uint32_t    flags;

/* Exclude flags (one per TCP state) */
#define TSF_EXCLUDE_CLOSED      0x00000001
#define TSF_EXCLUDE_LISTEN      0x00000002
#define TSF_EXCLUDE_SYN_SENT    0x00000004
#define TSF_EXCLUDE_SYN_RCVD    0x00000008
#define TSF_EXCLUDE_ESTABLISHED 0x00000010
#define TSF_EXCLUDE_CLOSE_WAIT  0x00000020
#define TSF_EXCLUDE_FIN_WAIT_1  0x00000040
#define TSF_EXCLUDE_CLOSING     0x00000080
#define TSF_EXCLUDE_LAST_ACK    0x00000100
#define TSF_EXCLUDE_FIN_WAIT_2  0x00000200
#define TSF_EXCLUDE_TIME_WAIT   0x00000400

/* Mode flags */
#define TSF_STATE_INCLUDE_MODE  0x00001000
#define TSF_LOCAL_PORT_MATCH    0x00002000
#define TSF_REMOTE_PORT_MATCH   0x00004000
#define TSF_LOCAL_ADDR_MATCH    0x00008000
#define TSF_REMOTE_ADDR_MATCH   0x00010000
#define TSF_IPV4_ONLY           0x00020000
#define TSF_IPV6_ONLY           0x00040000

    /* Port filters */
    uint16_t    local_ports[TSF_MAX_PORTS];
    uint16_t    remote_ports[TSF_MAX_PORTS];

    /* IPv4 address filters with CIDR mask */
    struct in_addr  local_addr_v4;
    struct in_addr  local_mask_v4;
    struct in_addr  remote_addr_v4;
    struct in_addr  remote_mask_v4;

    /* IPv6 address filters with prefix length */
    struct in6_addr local_addr_v6;
    uint8_t         local_prefix_v6;
    uint8_t         _pad1[3];
    struct in6_addr remote_addr_v6;
    uint8_t         remote_prefix_v6;
    uint8_t         _pad2[3];

    /* Field mask and format */
    uint32_t    field_mask;
    uint32_t    format;
#define TSF_FORMAT_COMPACT      0
#define TSF_FORMAT_FULL         1

    /* Spare for future expansion */
    uint32_t    _spare[4];
};

_Static_assert(sizeof(struct tcpstats_filter) <= 256,
    "tcpstats_filter exceeds maximum profile size");
```

**`tcp_stats_kld.c`** — update ioctl handler (replace lines 343-348):

```c
case TCPSTATS_SET_FILTER:
{
    struct tcpstats_filter *filt = (struct tcpstats_filter *)data;

    if (filt->version != TSF_VERSION) {
        printf("tcp_stats_kld: filter version %u unsupported (expected %u)\n",
            filt->version, TSF_VERSION);
        return (ENOTSUP);
    }
    sc->sc_filter = *filt;
    return (0);
}
```

**`tcp_stats_kld.c`** — update open to initialize v2 defaults (line 77):

```c
sc->sc_filter.version = TSF_VERSION;
sc->sc_filter.state_mask = 0xFFFF;
```

**`test/read_tcpstats.c`** — update filter setup (lines 89-98):

```c
if (flag_listen) {
    memset(&filt, 0, sizeof(filt));
    filt.version = TSF_VERSION;
    filt.state_mask = 0xFFFF;
    filt.flags = TSF_EXCLUDE_LISTEN;
    if (ioctl(fd, TCPSTATS_SET_FILTER, &filt) < 0) {
        perror("ioctl TCPSTATS_SET_FILTER");
        close(fd);
        return (1);
    }
}
```

### Definition of Done

1. `make clean && make` succeeds on VM (no warnings)
2. `_Static_assert` passes (struct ≤ 256 bytes)
3. Module loads and creates both `/dev/tcpstats` and `/dev/tcpstats-full`
4. `read_tcpstats -c` returns a count (basic read works with expanded softc)
5. `read_tcpstats -L -c` works (LISTEN exclusion via v2 struct)
6. Existing state filtering still works (state_mask + exclude flags)

### Integration Tests

```sh
# Sync, build, load
rsync -av --delete kmod/ root@192.168.122.41:/root/bsd-xtcp/kmod/ && \
ssh root@192.168.122.41 'cd /root/bsd-xtcp/kmod/tcp_stats_kld && make clean && make'

# Build updated read_tcpstats
ssh root@192.168.122.41 'cd /root/bsd-xtcp/kmod/tcp_stats_kld/test && make clean && make'

# Load module
ssh root@192.168.122.41 'kldunload tcp_stats_kld 2>/dev/null; kldload /root/bsd-xtcp/kmod/tcp_stats_kld/tcp_stats_kld.ko'

# Basic functionality
ssh root@192.168.122.41 '/root/bsd-xtcp/kmod/tcp_stats_kld/test/read_tcpstats -c'
# Expected: non-zero count (SSH + system sockets)

# LISTEN exclusion still works
ssh root@192.168.122.41 '/root/bsd-xtcp/kmod/tcp_stats_kld/test/read_tcpstats -L -c'
# Expected: fewer sockets than without -L
```

### What Can Go Wrong

- `sizeof(struct tcpstats_filter)` changes the ioctl command number (since `_IOW` encodes size) — rebuild `read_tcpstats` to match
- `struct in6_addr` not available in header — already guarded by `#ifndef _KERNEL` + `#include <netinet/in.h>`
- Alignment issues with the expanded struct on the ioctl boundary — `_Static_assert` catches size, manual check for padding

### Risk: **Medium** (ABI change, but contained to header + ioctl)

---

## Phase 2: Port Filtering in Read Path

**Goal:** Add local and remote port matching to `tcpstats_read()`. When
`TSF_LOCAL_PORT_MATCH` or `TSF_REMOTE_PORT_MATCH` flags are set, only sockets
matching a listed port pass through. Update `read_tcpstats` to use kernel-side
port filtering.

### Files to modify

| File | Changes |
|---|---|
| `tcp_stats_kld.c:283-300` | Add port matching after state filtering |
| `test/read_tcpstats.c:56-76` | Add `-P port` flag for kernel-side port filter (rename old `-p` to userspace) |

### Implementation

**`tcp_stats_kld.c`** — add after the state filtering block (after line 300):

```c
/* Port filtering. */
if (sc->sc_filter.flags & TSF_LOCAL_PORT_MATCH) {
    uint16_t lport = inp->inp_inc.inc_lport;
    bool found = false;
    for (int i = 0; i < TSF_MAX_PORTS &&
        sc->sc_filter.local_ports[i] != 0; i++) {
        if (lport == sc->sc_filter.local_ports[i]) {
            found = true;
            break;
        }
    }
    if (!found)
        continue;
}
if (sc->sc_filter.flags & TSF_REMOTE_PORT_MATCH) {
    uint16_t fport = inp->inp_inc.inc_fport;
    bool found = false;
    for (int i = 0; i < TSF_MAX_PORTS &&
        sc->sc_filter.remote_ports[i] != 0; i++) {
        if (fport == sc->sc_filter.remote_ports[i]) {
            found = true;
            break;
        }
    }
    if (!found)
        continue;
}
```

**Note:** Ports in `inp->inp_inc.inc_lport` are in **network byte order**.
The filter struct stores ports in network byte order too (set via `htons()`
in the parser or userspace tool). This means comparison is a direct `==`.

**`test/read_tcpstats.c`** — add `-P port` flag for kernel-side filtering:

```c
int kernel_port = -1;   /* -P: kernel-side port filter */

/* In getopt: add 'P:' */
case 'P':
    kernel_port = atoi(optarg);
    if (kernel_port <= 0 || kernel_port > 65535) {
        fprintf(stderr, "invalid port: %s\n", optarg);
        return (1);
    }
    break;

/* After fd open, before read: */
if (kernel_port >= 0) {
    memset(&filt, 0, sizeof(filt));
    filt.version = TSF_VERSION;
    filt.state_mask = 0xFFFF;
    filt.flags = TSF_LOCAL_PORT_MATCH;
    filt.local_ports[0] = htons((uint16_t)kernel_port);
    if (ioctl(fd, TCPSTATS_SET_FILTER, &filt) < 0) {
        perror("ioctl TCPSTATS_SET_FILTER");
        close(fd);
        return (1);
    }
}
```

### Definition of Done

1. Module compiles and loads
2. `read_tcpstats -P 22 -c` returns exactly the SSH socket count
3. With tcp-echo on ports 9001,9002,9003 (60 connections total), `-P 9001 -c` returns ~20
4. `-P 9999 -c` on a port with no sockets returns 0
5. Old `-p` (userspace filter) and `-L` still work

### Integration Tests

```sh
# Setup: tcp-echo with known ports
ssh root@192.168.122.41 'pkill tcp-echo 2>/dev/null; sleep 1'
ssh root@192.168.122.41 '/root/bsd-xtcp/target/release/tcp-echo server --ports 9001,9002,9003 &'
sleep 2
ssh root@192.168.122.41 '/root/bsd-xtcp/target/release/tcp-echo client --ports 9001,9002,9003 --connections 60 --rate 10000 --duration 120 &'
sleep 10

# Test: total sockets (should include tcp-echo + SSH + system)
ssh root@192.168.122.41 '/root/bsd-xtcp/kmod/tcp_stats_kld/test/read_tcpstats -c'

# Test: kernel-side port filter for 9001 only
ssh root@192.168.122.41 '/root/bsd-xtcp/kmod/tcp_stats_kld/test/read_tcpstats -P 9001 -c'
# Expected: ~20 (60 connections / 3 ports) + 1 listener

# Test: port with no sockets
ssh root@192.168.122.41 '/root/bsd-xtcp/kmod/tcp_stats_kld/test/read_tcpstats -P 9999 -c'
# Expected: 0

# Test: SSH port
ssh root@192.168.122.41 '/root/bsd-xtcp/kmod/tcp_stats_kld/test/read_tcpstats -P 22 -c'
# Expected: >=1 (our SSH session)

# Cleanup
ssh root@192.168.122.41 'pkill tcp-echo'
```

### What Can Go Wrong

- Byte order mismatch: `inp_lport` is network order, filter port must also be network order — use `htons()` when populating
- `bool` type not available in kernel C — use `int` or include `<sys/types.h>` (FreeBSD provides `bool` via `<sys/param.h>`)

### Risk: **Low** (simple array scan, well-understood data)

---

## Phase 3: Dual-Compile Parser Scaffold + Port Parsing

**Goal:** Create the parser source files with dual-compilation support
(`#ifdef _KERNEL` guards). Implement the top-level dispatcher, port number
parser, and port list parser. Create the userspace test harness. At the end
of this phase, the parser can parse `"local_port=443,8443"` into a
`tcpstats_filter` struct, tested entirely in userspace.

### Files to create

| File | Purpose |
|---|---|
| `tcp_stats_filter_parse.h` | Parser API, expanded struct (moved from header), field bitmask defines |
| `tcp_stats_filter_parse.c` | Dual-compile parser implementation |
| `test/test_filter_parse.c` | Userspace unit test harness |

### Files to modify

| File | Changes |
|---|---|
| `Makefile:2` | `SRCS = tcp_stats_kld.c tcp_stats_filter_parse.c` |
| `tcp_stats_kld.h` | Move filter struct to `tcp_stats_filter_parse.h`, `#include` it |

### Implementation

**`tcp_stats_filter_parse.h`**:

```c
#ifndef _TCP_STATS_FILTER_PARSE_H_
#define _TCP_STATS_FILTER_PARSE_H_

#ifdef _KERNEL
#include <sys/param.h>
#include <netinet/in.h>
#else
#include <sys/types.h>
#include <netinet/in.h>
#endif

/* --- Filter struct (moved from tcp_stats_kld.h) --- */
#define TSF_VERSION             2
#define TSF_MAX_PORTS           8
/* ... full struct definition ... */

/* --- Field group bitmasks --- */
#define TSR_FIELDS_IDENTITY     0x001
#define TSR_FIELDS_STATE        0x002
#define TSR_FIELDS_CONGESTION   0x004
#define TSR_FIELDS_RTT          0x008
#define TSR_FIELDS_SEQUENCES    0x010
#define TSR_FIELDS_COUNTERS     0x020
#define TSR_FIELDS_TIMERS       0x040
#define TSR_FIELDS_BUFFERS      0x080
#define TSR_FIELDS_ECN          0x100
#define TSR_FIELDS_NAMES        0x200
#define TSR_FIELDS_ALL          0x3FF
#define TSR_FIELDS_DEFAULT      0x08F

/* --- Parser API --- */
#define TSF_PARSE_MAXLEN        512
#define TSF_PARSE_MAXDIRECTIVES 16
#define TSF_ERRBUF_SIZE         128

int tsf_parse_filter_string(const char *input, size_t len,
    struct tcpstats_filter *out, char *errbuf, size_t errbuflen);

#endif /* _TCP_STATS_FILTER_PARSE_H_ */
```

**`tcp_stats_filter_parse.c`** — dual-compile preamble + port parsing:

```c
#ifdef _KERNEL
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/libkern.h>
#include <netinet/in.h>
#else
/* Userspace shims */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <ctype.h>
#include <arpa/inet.h>
#include <errno.h>

#define log(level, fmt, ...)    fprintf(stderr, fmt, ##__VA_ARGS__)
#ifndef strlcpy
#define strlcpy(dst, src, len)  snprintf(dst, len, "%s", src)
#endif
#ifndef bzero
#define bzero(ptr, len)         memset(ptr, 0, len)
#endif
#ifndef bcopy
#define bcopy(src, dst, len)    memcpy(dst, src, len)
#endif
#endif /* _KERNEL */

#include "tcp_stats_filter_parse.h"

/* Implementation of:
 * - tsf_parse_filter_string()     (top-level, two-pass)
 * - tsf_parse_directive()         (per-directive dispatcher)
 * - tsf_parse_port_number()       (single port string → uint16_t)
 * - tsf_parse_port_list()         (comma-separated ports → array)
 *
 * See filter-parsing.md §5 for full pseudocode.
 */
```

The functions are implemented exactly as specified in filter-parsing.md §5.2
(`tsf_parse_filter_string`), §5.3 (`tsf_parse_directive`), §7.1
(`tsf_parse_port_number`), and §7.2 (`tsf_parse_port_list`).

**`test/test_filter_parse.c`**:

```c
#include "../tcp_stats_filter_parse.h"
#include <stdio.h>
#include <string.h>
#include <errno.h>

struct test_case {
    const char *name;
    const char *input;
    int expected_error;
    const char *expected_errmsg;    /* Substring match, or NULL */
};

static const struct test_case cases[] = {
    /* --- Positive cases --- */
    {"empty string resets",          "", 0, NULL},
    {"whitespace-only resets",       "   ", 0, NULL},
    {"single port",                  "local_port=443", 0, NULL},
    {"multiple ports",               "local_port=443,8443,8080", 0, NULL},
    {"max ports (8)",                "local_port=1,2,3,4,5,6,7,8", 0, NULL},
    {"remote port",                  "remote_port=80,443", 0, NULL},
    {"both directions",              "local_port=443 remote_port=80", 0, NULL},

    /* --- Structural rejections --- */
    {"non-printable char",           "local_port=443\x01", EINVAL, "non-printable"},
    {"unknown directive",            "foobar=123", EINVAL, "unknown directive"},
    {"missing value",                "local_port", EINVAL, "did you mean"},
    {"empty value",                  "local_port=", EINVAL, "empty value"},

    /* --- Port rejections (§8.3) --- */
    {"port zero",                    "local_port=0", EINVAL, "port 0"},
    {"port overflow 65536",          "local_port=65536", EINVAL, "exceeds maximum"},
    {"port leading zero",            "local_port=0443", EINVAL, "leading zero"},
    {"port non-digit",               "local_port=abc", EINVAL, "non-digit"},
    {"port negative",                "local_port=-1", EINVAL, "non-digit"},
    {"port duplicate",               "local_port=443,443", EINVAL, "duplicate port"},
    {"port too many (9)",            "local_port=1,2,3,4,5,6,7,8,9", EINVAL, "too many ports"},
    {"port empty list",              "local_port=,,", EINVAL, "empty port list"},
    {"port too many digits",         "local_port=100000", EINVAL, "too many digits"},
    {"duplicate port directive",     "local_port=443 local_port=80", EINVAL, "duplicate port"},

    {NULL, NULL, 0, NULL}
};

int main(void) { /* ... test runner loop ... */ }
```

### Definition of Done

1. `cc -o test_filter_parse test/test_filter_parse.c tcp_stats_filter_parse.c -I.` compiles on dev host (Linux/macOS)
2. `./test_filter_parse` passes all ~20 test cases (0 failures)
3. `make clean && make` succeeds on VM (kernel compilation with new SRCS)
4. Module loads and all Phase 1-2 functionality still works
5. Port values in parsed struct are in network byte order (verified in test)

### Unit Tests

| # | Test name | Input | Expected |
|---|---|---|---|
| 1 | empty string resets | `""` | errno=0, state_mask=0xFFFF |
| 2 | whitespace-only resets | `"   "` | errno=0, state_mask=0xFFFF |
| 3 | single port | `"local_port=443"` | errno=0, local_ports[0]=htons(443), TSF_LOCAL_PORT_MATCH set |
| 4 | multiple ports | `"local_port=443,8443,8080"` | errno=0, 3 ports populated |
| 5 | max ports (8) | `"local_port=1,2,3,4,5,6,7,8"` | errno=0, 8 ports |
| 6 | remote port | `"remote_port=80,443"` | errno=0, TSF_REMOTE_PORT_MATCH set |
| 7 | both directions | `"local_port=443 remote_port=80"` | errno=0, both flags set |
| 8 | non-printable | `"local_port=443\x01"` | EINVAL, "non-printable" |
| 9 | unknown directive | `"foobar=123"` | EINVAL, "unknown directive" |
| 10 | port zero | `"local_port=0"` | EINVAL, "port 0" |
| 11 | port 65536 | `"local_port=65536"` | EINVAL, "exceeds maximum" |
| 12 | leading zero | `"local_port=0443"` | EINVAL, "leading zero" |
| 13 | non-digit | `"local_port=abc"` | EINVAL, "non-digit" |
| 14 | duplicate port | `"local_port=443,443"` | EINVAL, "duplicate port" |
| 15 | too many ports | `"local_port=1,2,3,4,5,6,7,8,9"` | EINVAL, "too many ports" |
| 16 | empty list | `"local_port=,,"` | EINVAL, "empty port list" |
| 17 | too many digits | `"local_port=100000"` | EINVAL, "too many digits" |
| 18 | duplicate directive | `"local_port=443 local_port=80"` | EINVAL, "duplicate port" |
| 19 | missing value | `"local_port"` | EINVAL, "did you mean" |
| 20 | empty value | `"local_port="` | EINVAL, "empty value" |

### What Can Go Wrong

- `strsep()` not available in kernel — it is available on FreeBSD (`<sys/libkern.h>`)
- `tolower()` not available in kernel — FreeBSD provides it in `<sys/libkern.h>`
- `strtoul()` not available in kernel — FreeBSD provides it in `<sys/libkern.h>`
- `errno` constants differ kernel vs userspace — on FreeBSD they are the same (`<sys/errno.h>`)
- `htons()` in kernel — available via `<netinet/in.h>`

### Risk: **Medium** (first dual-compile file, kernel/userspace shim boundary)

---

## Phase 4: State Parsing (exclude + include_state)

**Goal:** Add state name parsing to the filter parser. Implement `exclude=`
and `include_state=` directives. Add cross-directive validation (mutual
exclusion). Update the read path to handle `TSF_STATE_INCLUDE_MODE`.

### Files to modify

| File | Changes |
|---|---|
| `tcp_stats_filter_parse.c` | Add `tsf_parse_exclude_list()`, `tsf_parse_include_state_list()`, state name table |
| `tcp_stats_kld.c:283-300` | Update state filtering for include mode |
| `test/test_filter_parse.c` | Add ~10 state parsing test cases |

### Implementation

**State name → constant mapping table:**

```c
static const struct {
    const char  *name;
    int          state;
} tsf_state_names[] = {
    { "closed",        TCPS_CLOSED },
    { "listen",        TCPS_LISTEN },
    { "syn_sent",      TCPS_SYN_SENT },
    { "syn_received",  TCPS_SYN_RECEIVED },
    { "established",   TCPS_ESTABLISHED },
    { "close_wait",    TCPS_CLOSE_WAIT },
    { "fin_wait_1",    TCPS_FIN_WAIT_1 },
    { "closing",       TCPS_CLOSING },
    { "last_ack",      TCPS_LAST_ACK },
    { "fin_wait_2",    TCPS_FIN_WAIT_2 },
    { "time_wait",     TCPS_TIME_WAIT },
    { NULL, 0 }
};
```

Note: In userspace test, define `TCPS_*` constants locally since
`<netinet/tcp_fsm.h>` is not available on the dev host:

```c
#ifndef _KERNEL
#ifndef TCPS_CLOSED
#define TCPS_CLOSED         0
#define TCPS_LISTEN         1
#define TCPS_SYN_SENT       2
#define TCPS_SYN_RECEIVED   3
#define TCPS_ESTABLISHED    4
#define TCPS_CLOSE_WAIT     5
#define TCPS_FIN_WAIT_1     6
#define TCPS_CLOSING        7
#define TCPS_LAST_ACK       8
#define TCPS_FIN_WAIT_2     9
#define TCPS_TIME_WAIT      10
#endif
#endif
```

**`tsf_parse_exclude_list()`** — parse `exclude=listen,timewait`:
- Split on `,`, look up each name in `tsf_state_names`
- Clear corresponding bit in `state_mask`
- Check for `TSF_STATE_INCLUDE_MODE` conflict (return EINVAL)

**`tsf_parse_include_state_list()`** — parse `include_state=established`:
- Start with `state_mask = 0` (include nothing)
- Split on `,`, look up each name, set corresponding bit
- Set `TSF_STATE_INCLUDE_MODE` flag
- Check for exclude conflict

**`tcp_stats_kld.c`** — update state filtering (replace lines 283-300):

```c
/* State filtering. */
{
    struct tcpcb *tp = intotcpcb(inp);
    if (tp != NULL) {
        if (sc->sc_filter.state_mask != 0xFFFF &&
            !(sc->sc_filter.state_mask & (1 << tp->t_state)))
            continue;
    }
}
```

This simplified check works for both modes because:
- `exclude=` clears bits in state_mask during parsing
- `include_state=` sets only the desired bits during parsing
- Either way, `state_mask & (1 << state)` does the right thing

### Definition of Done

1. Userspace `test_filter_parse` passes all state test cases
2. `"exclude=listen,timewait"` → state_mask has bits 1 and 10 cleared
3. `"include_state=established"` → state_mask has only bit 4 set, `TSF_STATE_INCLUDE_MODE` flag set
4. `"exclude=listen include_state=established"` → EINVAL "mutually exclusive"
5. Module loads and filtering works on VM

### Unit Tests (additions)

| # | Test name | Input | Expected |
|---|---|---|---|
| 21 | exclude listen | `"exclude=listen"` | errno=0, state_mask bit 1 cleared |
| 22 | exclude multiple | `"exclude=listen,timewait"` | errno=0, bits 1,10 cleared |
| 23 | include established | `"include_state=established"` | errno=0, state_mask=(1<<4), TSF_STATE_INCLUDE_MODE |
| 24 | include multiple | `"include_state=established,syn_sent"` | errno=0, bits 2,4 set |
| 25 | unknown state | `"exclude=foobar"` | EINVAL, "unknown state" |
| 26 | duplicate state | `"exclude=listen,listen"` | EINVAL, "duplicate state" |
| 27 | exclude+include conflict | `"exclude=listen include_state=established"` | EINVAL, "mutually exclusive" |
| 28 | case insensitive | `"EXCLUDE=LISTEN"` | errno=0 |
| 29 | closewait alias | `"exclude=close_wait"` | errno=0, bit 5 cleared |
| 30 | all states exclude | `"exclude=listen,timewait,closewait"` | errno=0 |

### Integration Tests

```sh
# Start tcp-echo for socket variety
ssh root@192.168.122.41 'pkill tcp-echo 2>/dev/null; sleep 1'
ssh root@192.168.122.41 '/root/bsd-xtcp/target/release/tcp-echo server --ports 9001 &'
sleep 2
ssh root@192.168.122.41 '/root/bsd-xtcp/target/release/tcp-echo client --ports 9001 --connections 20 --rate 5000 --duration 60 &'
sleep 5

# Count all sockets
ssh root@192.168.122.41 '/root/bsd-xtcp/kmod/tcp_stats_kld/test/read_tcpstats -c'

# Count with LISTEN excluded (should be fewer)
ssh root@192.168.122.41 '/root/bsd-xtcp/kmod/tcp_stats_kld/test/read_tcpstats -L -c'

# Cleanup
ssh root@192.168.122.41 'pkill tcp-echo'
```

### Risk: **Low** (string lookup in bounded table, well-defined semantics)

---

## Phase 5: IPv4 Address Parsing + CIDR Matching

**Goal:** Parse `local_addr=10.0.0.0/24` into the filter struct. Add IPv4
CIDR matching to the read path. This is the first address filtering
capability.

### Files to modify

| File | Changes |
|---|---|
| `tcp_stats_filter_parse.c` | Add `tsf_parse_ipv4_addr()`, `tsf_parse_addr()` (AF auto-detect), `tsf_parse_prefix_length()` |
| `tcp_stats_kld.c` | Add IPv4 CIDR match block in read path (after port filtering) |
| `test/test_filter_parse.c` | Add ~12 IPv4 test cases |

### Implementation

**`tsf_parse_ipv4_addr()`** — parse dotted-decimal with optional `/prefix`:

```c
static int
tsf_parse_ipv4_addr(const char *str, struct in_addr *addr,
    struct in_addr *mask, char *errbuf, size_t errbuflen)
{
    const char *slash;
    char addrbuf[16];
    uint32_t octets[4];
    int prefix = 32;

    /* Separate address from prefix */
    slash = strchr(str, '/');
    if (slash != NULL) {
        size_t addrlen = slash - str;
        if (addrlen >= sizeof(addrbuf)) {
            snprintf(errbuf, errbuflen, "IPv4 address too long");
            return (EINVAL);
        }
        strlcpy(addrbuf, str, addrlen + 1);
        /* Parse prefix length */
        /* ... validate 0-32, no leading zeros ... */
    } else {
        strlcpy(addrbuf, str, sizeof(addrbuf));
    }

    /* Parse 4 octets manually (no inet_pton in kernel) */
    /* ... validate each 0-255, no leading zeros except "0" itself ... */

    /* Compute netmask from prefix */
    if (prefix == 0)
        mask->s_addr = 0;
    else
        mask->s_addr = htonl(~((1U << (32 - prefix)) - 1));

    /* Validate host bits are zero */
    if ((addr->s_addr & ~mask->s_addr) != 0) {
        snprintf(errbuf, errbuflen,
            "host bits set in IPv4 CIDR (prefix /%d)", prefix);
        return (EINVAL);
    }

    return (0);
}
```

**`tsf_parse_addr()`** — auto-detect AF by presence of `:`:

```c
static int
tsf_parse_addr(char *value, struct tcpstats_filter *f, uint32_t flag_bit,
    char *errbuf, size_t errbuflen)
{
    if (strchr(value, ':') != NULL) {
        /* IPv6 — deferred to Phase 6 */
        snprintf(errbuf, errbuflen, "IPv6 addresses not yet supported");
        return (ENOTSUP);
    }

    /* IPv4 */
    if (flag_bit == TSF_LOCAL_ADDR_MATCH)
        return tsf_parse_ipv4_addr(value, &f->local_addr_v4,
            &f->local_mask_v4, errbuf, errbuflen);
    else
        return tsf_parse_ipv4_addr(value, &f->remote_addr_v4,
            &f->remote_mask_v4, errbuf, errbuflen);
}
```

**`tcp_stats_kld.c`** — IPv4 CIDR match in read path:

```c
/* IPv4 local address filtering. */
if (sc->sc_filter.flags & TSF_LOCAL_ADDR_MATCH) {
    if (inp->inp_vflag & INP_IPV4) {
        if ((inp->inp_inc.inc_laddr.s_addr &
            sc->sc_filter.local_mask_v4.s_addr) !=
            (sc->sc_filter.local_addr_v4.s_addr &
            sc->sc_filter.local_mask_v4.s_addr))
            continue;
    }
}
/* IPv4 remote address filtering. */
if (sc->sc_filter.flags & TSF_REMOTE_ADDR_MATCH) {
    if (inp->inp_vflag & INP_IPV4) {
        if ((inp->inp_inc.inc_faddr.s_addr &
            sc->sc_filter.remote_mask_v4.s_addr) !=
            (sc->sc_filter.remote_addr_v4.s_addr &
            sc->sc_filter.remote_mask_v4.s_addr))
            continue;
    }
}
```

### Definition of Done

1. Userspace test passes all IPv4 parsing cases
2. `"local_addr=10.0.0.0/24"` → `local_addr_v4` = 10.0.0.0, `local_mask_v4` = 255.255.255.0
3. `"local_addr=10.0.0.1/24"` → EINVAL "host bits set"
4. `"local_addr=127.0.0.1"` → exact match (mask = 255.255.255.255)
5. On VM: filtering by `local_addr=127.0.0.0/8` returns only loopback sockets

### Unit Tests (additions)

| # | Test name | Input | Expected |
|---|---|---|---|
| 31 | ipv4 exact | `"local_addr=10.0.0.1"` | errno=0, addr=10.0.0.1, mask=255.255.255.255 |
| 32 | ipv4 /24 | `"local_addr=10.0.0.0/24"` | errno=0, mask=255.255.255.0 |
| 33 | ipv4 /32 | `"local_addr=10.0.0.1/32"` | errno=0, exact match |
| 34 | ipv4 /0 | `"local_addr=0.0.0.0/0"` | errno=0, match all |
| 35 | ipv4 host bits | `"local_addr=10.0.0.1/24"` | EINVAL, "host bits set" |
| 36 | ipv4 bad octet | `"local_addr=999.1.2.3"` | EINVAL, "exceeds 255" |
| 37 | ipv4 missing octets | `"local_addr=10.0.0"` | EINVAL, "3 octets" |
| 38 | ipv4 extra octets | `"local_addr=10.0.0.0.1"` | EINVAL, "trailing" |
| 39 | ipv4 prefix too long | `"local_addr=10.0.0.0/33"` | EINVAL, "exceeds maximum 32" |
| 40 | ipv4 remote | `"remote_addr=192.168.1.0/24"` | errno=0, TSF_REMOTE_ADDR_MATCH |
| 41 | ipv4 leading zero octet | `"local_addr=010.0.0.0/8"` | EINVAL, "leading zero" |
| 42 | ipv4 combo | `"local_port=443 local_addr=10.0.0.0/24"` | errno=0, both flags |

### Integration Tests

```sh
# On VM: filter for loopback only
# (need a loopback socket — connect tcp-echo to 127.0.0.1)
ssh root@192.168.122.41 '/root/bsd-xtcp/target/release/tcp-echo server --ports 9001 --bind 127.0.0.1 &'
sleep 2
ssh root@192.168.122.41 '/root/bsd-xtcp/target/release/tcp-echo client --host 127.0.0.1 --ports 9001 --connections 10 --rate 5000 --duration 60 &'
sleep 5

# All sockets
ssh root@192.168.122.41 '/root/bsd-xtcp/kmod/tcp_stats_kld/test/read_tcpstats -c'

# Loopback only (would need custom tool or updated read_tcpstats with -A flag)
# For now, verify via ioctl in a small test program

ssh root@192.168.122.41 'pkill tcp-echo'
```

### Risk: **Medium** (manual IPv4 parser, CIDR math, but well-understood)

---

## Phase 6: IPv6 Address Parsing + CIDR Matching

**Goal:** Implement the kernel-side IPv6 address parser. Handle `::` compression,
leading zero omission, mixed IPv4-mapped notation, and CIDR prefix validation.
This is the most complex parsing phase.

### Files to modify

| File | Changes |
|---|---|
| `tcp_stats_filter_parse.c` | Add `tsf_parse_ipv6_addr()`, `tsf_validate_v6_cidr()`, `tsf_match_v6_prefix()`, update `tsf_parse_addr()` to dispatch to v6 |
| `tcp_stats_kld.c` | Add IPv6 CIDR match block in read path |
| `test/test_filter_parse.c` | Add ~15 IPv6 test cases |

### Implementation

The full IPv6 parser is implemented as specified in filter-parsing.md §6.2.
Key functions:

- `tsf_parse_ipv6_addr()` — ~100 lines, handles 8 forms listed in §6.1
- `tsf_validate_v6_cidr()` — from §6.4, validates host bits are zero
- `tsf_match_v6_prefix()` — `__always_inline`, from §13.2

Update `tsf_parse_addr()` to remove the `ENOTSUP` stub and dispatch to the
real IPv6 parser when `:` is detected.

**`tcp_stats_kld.c`** — IPv6 match in read path:

```c
/* IPv6 local address filtering. */
if (sc->sc_filter.flags & TSF_LOCAL_ADDR_MATCH) {
    if (inp->inp_vflag & INP_IPV6) {
        if (!IN6_IS_ADDR_UNSPECIFIED(&sc->sc_filter.local_addr_v6)) {
            if (!tsf_match_v6_prefix(
                &inp->inp_inc.inc6_laddr,
                &sc->sc_filter.local_addr_v6,
                sc->sc_filter.local_prefix_v6))
                continue;
        }
    }
}
```

### Definition of Done

1. Userspace test passes all IPv6 parsing cases
2. All RFC 5952 forms parse correctly: full, compressed, loopback, all-zeros, link-local
3. `::` can appear at start, middle, or end
4. Multiple `::` is rejected
5. CIDR host bits validation works for all prefix lengths
6. Module compiles and loads on VM
7. IPv6 socket filtering works (if IPv6 test sockets available)

### Unit Tests (additions)

| # | Test name | Input | Expected |
|---|---|---|---|
| 43 | ipv6 loopback | `"local_addr=::1"` | errno=0, addr=::1, prefix=128 |
| 44 | ipv6 all zeros | `"local_addr=::"` | errno=0, addr=::, prefix=128 |
| 45 | ipv6 full | `"local_addr=2001:db8:0:0:0:0:0:1"` | errno=0 |
| 46 | ipv6 compressed | `"local_addr=2001:db8::1"` | errno=0, same as above |
| 47 | ipv6 link-local | `"local_addr=fe80::/10"` | errno=0, prefix=10 |
| 48 | ipv6 /128 | `"local_addr=::1/128"` | errno=0, exact match |
| 49 | ipv6 /0 | `"local_addr=::/0"` | errno=0, match all |
| 50 | ipv6 host bits | `"remote_addr=fe80::1/10"` | EINVAL, "host bits set" |
| 51 | ipv6 prefix >128 | `"remote_addr=::/129"` | EINVAL, "exceeds maximum 128" |
| 52 | ipv6 multiple :: | `"remote_addr=2001::1::2"` | EINVAL, "multiple '::'" |
| 53 | ipv6 invalid hex | `"remote_addr=gggg::1"` | EINVAL, "invalid character" |
| 54 | ipv6 too many groups | `"remote_addr=1:2:3:4:5:6:7:8:9"` | EINVAL |
| 55 | ipv6 single colon start | `"remote_addr=:1"` | EINVAL, "starts with single" |
| 56 | ipv6 empty group | `"remote_addr=2001:db8:::1"` | EINVAL |
| 57 | ipv6 combo | `"remote_addr=fe80::/10 local_port=443"` | errno=0, both flags |

### What Can Go Wrong

- `::` expansion logic off-by-one — test extensively with the 8 forms in §6.1
- Mixed IPv4-mapped notation (`::ffff:192.168.1.1`) — can defer to future if complex
- `__always_inline` not available in userspace — use `static inline` behind `#ifdef`

### Risk: **High** (most complex parser, `::` compression is tricky)

---

## Phase 7: Format, Fields, Flags Parsing + Cross-Validation

**Goal:** Implement the remaining directives: `format=`, `fields=`,
`ipv4_only`, `ipv6_only`. Implement `tsf_validate_filter()` for cross-directive
conflict detection. Add `ipv4_only`/`ipv6_only` filtering to the read path.

### Files to modify

| File | Changes |
|---|---|
| `tcp_stats_filter_parse.c` | Add `tsf_parse_format()`, `tsf_parse_field_list()`, `tsf_validate_filter()` |
| `tcp_stats_kld.c` | Add AF filtering (ipv4_only/ipv6_only) at top of filter block |
| `test/test_filter_parse.c` | Add ~10 test cases for flags, format, fields, conflicts |

### Implementation

**`tsf_parse_format()`:**

```c
static int
tsf_parse_format(char *value, struct tcpstats_filter *f,
    char *errbuf, size_t errbuflen)
{
    /* Normalize to lowercase (already done by dispatcher) */
    if (strcmp(value, "compact") == 0)
        f->format = TSF_FORMAT_COMPACT;
    else if (strcmp(value, "full") == 0)
        f->format = TSF_FORMAT_FULL;
    else {
        snprintf(errbuf, errbuflen,
            "unknown format '%s' (expected 'compact' or 'full')", value);
        return (EINVAL);
    }
    return (0);
}
```

**`tsf_parse_field_list()`** — parse comma-separated field group names:

```c
static const struct {
    const char  *name;
    uint32_t     mask;
} tsf_field_groups[] = {
    { "identity",    TSR_FIELDS_IDENTITY },
    { "state",       TSR_FIELDS_STATE },
    { "congestion",  TSR_FIELDS_CONGESTION },
    { "rtt",         TSR_FIELDS_RTT },
    { "sequences",   TSR_FIELDS_SEQUENCES },
    { "counters",    TSR_FIELDS_COUNTERS },
    { "timers",      TSR_FIELDS_TIMERS },
    { "buffers",     TSR_FIELDS_BUFFERS },
    { "ecn",         TSR_FIELDS_ECN },
    { "names",       TSR_FIELDS_NAMES },
    { "all",         TSR_FIELDS_ALL },
    { "default",     TSR_FIELDS_DEFAULT },
    { NULL, 0 }
};
```

**`tsf_validate_filter()`** — cross-directive conflict detection:

```c
static int
tsf_validate_filter(struct tcpstats_filter *f,
    char *errbuf, size_t errbuflen)
{
    /* ipv4_only + ipv6_only conflict */
    if ((f->flags & TSF_IPV4_ONLY) && (f->flags & TSF_IPV6_ONLY)) {
        snprintf(errbuf, errbuflen,
            "'ipv4_only' and 'ipv6_only' are mutually exclusive");
        return (EINVAL);
    }

    /* exclude + include_state conflict (already caught in parsers, but defense in depth) */

    /* IPv4 address + ipv6_only conflict */
    if ((f->flags & TSF_LOCAL_ADDR_MATCH) &&
        f->local_addr_v4.s_addr != INADDR_ANY &&
        (f->flags & TSF_IPV6_ONLY)) {
        snprintf(errbuf, errbuflen,
            "IPv4 address conflicts with ipv6_only flag");
        return (EINVAL);
    }

    /* IPv6 address + ipv4_only conflict */
    if ((f->flags & TSF_LOCAL_ADDR_MATCH) &&
        !IN6_IS_ADDR_UNSPECIFIED(&f->local_addr_v6) &&
        (f->flags & TSF_IPV4_ONLY)) {
        snprintf(errbuf, errbuflen,
            "IPv6 address conflicts with ipv4_only flag");
        return (EINVAL);
    }

    /* Same checks for remote_addr ... */

    return (0);
}
```

**`tcp_stats_kld.c`** — AF filtering (top of filter block, cheapest check first):

```c
/* IP version filter. */
if ((sc->sc_filter.flags & TSF_IPV4_ONLY) &&
    !(inp->inp_vflag & INP_IPV4))
    continue;
if ((sc->sc_filter.flags & TSF_IPV6_ONLY) &&
    !(inp->inp_vflag & INP_IPV6))
    continue;
```

### Definition of Done

1. All new test cases pass in userspace
2. `"ipv4_only ipv6_only"` → EINVAL "mutually exclusive"
3. `"local_addr=10.0.0.1 ipv6_only"` → EINVAL "conflicts"
4. `"format=compact"` → format=TSF_FORMAT_COMPACT
5. `"fields=state,rtt,buffers"` → field_mask = 0x08A
6. `"fields=all"` → field_mask = 0x3FF
7. On VM: `ipv4_only` filter returns only IPv4 sockets

### Unit Tests (additions)

| # | Test name | Input | Expected |
|---|---|---|---|
| 58 | ipv4_only flag | `"ipv4_only"` | errno=0, TSF_IPV4_ONLY set |
| 59 | ipv6_only flag | `"ipv6_only"` | errno=0, TSF_IPV6_ONLY set |
| 60 | both AF flags | `"ipv4_only ipv6_only"` | EINVAL, "mutually exclusive" |
| 61 | ipv4_only with value | `"ipv4_only=true"` | EINVAL, "flag" |
| 62 | format compact | `"format=compact"` | errno=0, format=0 |
| 63 | format full | `"format=full"` | errno=0, format=1 |
| 64 | format unknown | `"format=json"` | EINVAL, "unknown format" |
| 65 | fields single | `"fields=rtt"` | errno=0, field_mask=0x008 |
| 66 | fields multiple | `"fields=state,rtt,buffers"` | errno=0, field_mask=0x08A |
| 67 | fields all | `"fields=all"` | errno=0, field_mask=0x3FF |
| 68 | fields default | `"fields=default"` | errno=0, field_mask=0x08F |
| 69 | fields unknown | `"fields=foobar"` | EINVAL, "unknown field" |
| 70 | ipv4 addr + ipv6_only | `"local_addr=10.0.0.1 ipv6_only"` | EINVAL, "conflicts" |
| 71 | ipv6 addr + ipv4_only | `"local_addr=::1 ipv4_only"` | EINVAL, "conflicts" |
| 72 | full combo | `"local_port=443 exclude=listen,timewait ipv4_only format=full"` | errno=0 |
| 73 | case insensitive keys | `"LOCAL_PORT=443 EXCLUDE=LISTEN"` | errno=0 |

### Integration Tests

```sh
# ipv4_only filter on VM
# (Extend read_tcpstats with -4 flag for ipv4_only, or use a test script)
# Verify no IPv6 sockets in output
```

### Risk: **Low** (simple enum lookups and flag checks)

---

## Phase 8: Sysctl Profile Integration

**Goal:** Add `dev.tcpstats.profiles.<name>` sysctl handler. Writing a filter
string to a profile sysctl creates a `/dev/tcpstats/<name>` device with the
parsed filter. Reading the profile sysctl returns the original filter string.
Writing an empty string deletes the profile. Add `dev.tcpstats.last_error`
sysctl for interactive error debugging.

### Files to modify

| File | Changes |
|---|---|
| `tcp_stats_kld.c` | Major additions: profile struct, sysctl handler, profile open, sx lock, error sysctl (~200 lines new) |

### Implementation

**Profile data structure:**

```c
#define TSF_MAX_PROFILES        16
#define TSF_PROFILE_NAME_MAX    32

struct tcpstats_profile {
    char                    name[TSF_PROFILE_NAME_MAX];
    char                    filter_str[TSF_PARSE_MAXLEN];
    struct tcpstats_filter  filter;
    struct cdev             *dev;
    SLIST_ENTRY(tcpstats_profile) link;
};

static SLIST_HEAD(, tcpstats_profile) tcpstats_profiles =
    SLIST_HEAD_INITIALIZER(tcpstats_profiles);
static int tcpstats_nprofiles;
static struct sx tcpstats_profile_lock;

/* Error reporting */
static char tcpstats_last_error[TSF_ERRBUF_SIZE];
```

**Sysctl registration (in MOD_LOAD):**

```c
SYSCTL_NODE(_dev, OID_AUTO, tcpstats, CTLFLAG_RD, 0,
    "tcp_stats_kld");
SYSCTL_STRING(_dev_tcpstats, OID_AUTO, last_error, CTLFLAG_RD,
    tcpstats_last_error, sizeof(tcpstats_last_error),
    "Last filter parse error");
SYSCTL_NODE(_dev_tcpstats, OID_AUTO, profiles, CTLFLAG_RW, 0,
    "Named filter profiles");
```

**Profile sysctl handler** — as specified in filter-parsing.md §9.2 and §12.1:

1. Validate profile name (`[a-z0-9_]+`, max 32 chars)
2. Call `tsf_parse_filter_string()` to parse the filter
3. On error: store in `last_error`, log to dmesg, return errno
4. On success: allocate profile, create `/dev/tcpstats/<name>`, store in list

**Profile open handler:**

```c
static int
tcpstats_profile_open(struct cdev *dev, int oflags, int devtype,
    struct thread *td)
{
    struct tcpstats_profile *prof = dev->si_drv1;
    struct tcpstats_softc *sc;

    if (oflags & FWRITE)
        return (EPERM);

    sc = malloc(sizeof(*sc), M_TCPSTATS, M_WAITOK | M_ZERO);
    sc->sc_cred = crhold(td->td_ucred);
    bcopy(&prof->filter, &sc->sc_filter, sizeof(sc->sc_filter));
    sc->sc_full = (prof->filter.format == TSF_FORMAT_FULL);

    return devfs_set_cdevpriv(sc, tcpstats_dtor);
}
```

**Module unload** — destroy all profile devices + free profile structs.

### Definition of Done

1. `sysctl dev.tcpstats.profiles.web="local_port=9001 exclude=listen"` succeeds
2. `ls -la /dev/tcpstats/web` shows the device (crw-r----- root network)
3. `cat /dev/tcpstats/web | wc -c` returns filtered record data
4. `sysctl dev.tcpstats.profiles.web` returns the original filter string
5. `sysctl dev.tcpstats.profiles.web=""` deletes the device
6. `sysctl dev.tcpstats.profiles.bad="foobar=123"` → "Invalid argument"
7. `sysctl dev.tcpstats.last_error` → "unknown directive 'foobar'"
8. `dmesg | tail -1` → "tcp_stats_kld: filter parse error: unknown directive 'foobar'"
9. Max 16 profiles enforced
10. Module unload destroys all profile devices

### Integration Tests

```sh
# Setup
ssh root@192.168.122.41 'pkill tcp-echo 2>/dev/null; sleep 1'
ssh root@192.168.122.41 '/root/bsd-xtcp/target/release/tcp-echo server --ports 9001,9002,9003 &'
sleep 2
ssh root@192.168.122.41 '/root/bsd-xtcp/target/release/tcp-echo client --ports 9001,9002,9003 --connections 60 --rate 10000 --duration 120 &'
sleep 10

# Create a profile filtering for port 9001
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.port9001="local_port=9001 exclude=listen,timewait"'

# Verify device exists
ssh root@192.168.122.41 'ls -la /dev/tcpstats/port9001'
# Expected: crw-r-----  root  network

# Read from the profile device
ssh root@192.168.122.41 'dd if=/dev/tcpstats/port9001 bs=65536 2>/dev/null | wc -c'
# Expected: ~20 * 320 = ~6400 bytes (20 connections on port 9001)

# Read profile value back
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.port9001'
# Expected: dev.tcpstats.profiles.port9001: local_port=9001 exclude=listen,timewait

# Error case
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.bad="foobar=123" 2>&1'
# Expected: sysctl: dev.tcpstats.profiles.bad: Invalid argument

ssh root@192.168.122.41 'sysctl dev.tcpstats.last_error'
# Expected: dev.tcpstats.last_error: unknown directive 'foobar'

# Delete profile
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.port9001=""'
ssh root@192.168.122.41 'ls /dev/tcpstats/port9001 2>&1'
# Expected: No such file or directory

# Multiple profiles simultaneously
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.p1="local_port=9001"'
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.p2="local_port=9002"'
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.p3="local_port=9003"'
ssh root@192.168.122.41 'ls /dev/tcpstats/'
# Expected: p1  p2  p3

# Cleanup
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.p1=""'
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.p2=""'
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.p3=""'
ssh root@192.168.122.41 'pkill tcp-echo'
```

### What Can Go Wrong

- Dynamic sysctl node creation is complex — `SYSCTL_ADD_PROC()` for dynamic children
- `make_dev_credf` for `/dev/tcpstats/<name>` needs directory support — may need `make_dev_credf` with path format or pre-create the directory
- `sx_xlock` in sysctl handler — must not sleep holding other locks
- `destroy_dev()` blocks until all fds close — profile deletion may hang if readers are active
- `SLIST` not safe for concurrent iteration — `sx` lock protects all access

### Risk: **High** (most kernel API surface: sysctl, cdev, sx locks, profile lifecycle)

---

## Phase 9: Comprehensive Testing + Fuzz Harness

**Goal:** Full integration test matrix, fuzz harness, stress testing. No new
features — validation and hardening only.

### Files to create

| File | Purpose |
|---|---|
| `test/fuzz_filter_parse.c` | AFL/libFuzzer dual harness |

### Fuzz Harness

```c
/* test/fuzz_filter_parse.c */
#include "../tcp_stats_filter_parse.h"

#ifdef __AFL_FUZZ_TESTCASE_LEN
__AFL_FUZZ_INIT();
int main(void) {
    __AFL_INIT();
    unsigned char *buf = __AFL_FUZZ_TESTCASE_BUF;
    while (__AFL_LOOP(1000)) {
        int len = __AFL_FUZZ_TESTCASE_LEN;
        if (len > 0 && len < TSF_PARSE_MAXLEN) {
            char input[TSF_PARSE_MAXLEN];
            memcpy(input, buf, len);
            input[len] = '\0';
            struct tcpstats_filter filter;
            char errbuf[TSF_ERRBUF_SIZE];
            tsf_parse_filter_string(input, len,
                &filter, errbuf, sizeof(errbuf));
        }
    }
    return 0;
}
#else
int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size) {
    if (size == 0 || size >= TSF_PARSE_MAXLEN) return 0;
    char input[TSF_PARSE_MAXLEN];
    memcpy(input, data, size);
    input[size] = '\0';
    struct tcpstats_filter filter;
    char errbuf[TSF_ERRBUF_SIZE];
    tsf_parse_filter_string(input, size,
        &filter, errbuf, sizeof(errbuf));
    return 0;
}
#endif
```

### Integration Test Matrix

| Test | tcp-echo Setup | Filter | Expected |
|---|---|---|---|
| Single port | server --ports 9001, client --connections 30 | `local_port=9001` | 30 connections + 1 listener |
| Multi-port | server --ports 9001,9002,9003, client --connections 60 | `local_port=9001` | ~20 connections |
| Port + state | same as above | `local_port=9001 exclude=listen` | ~20 connections (no listener) |
| Port + addr | server --bind 127.0.0.1 --ports 9001, client --host 127.0.0.1 | `local_port=9001 local_addr=127.0.0.0/8` | all connections |
| ipv4_only | server --ports 9001, client --connections 20 | `ipv4_only local_port=9001` | IPv4 only |
| include_state | server --ports 9001, client --connections 20 | `include_state=established` | only ESTABLISHED |
| format=full | server --ports 9001 | `local_port=9001 format=full` | records via full device |
| empty filter | (baseline) | `""` | default behavior (all sockets) |
| max ports | server --ports 9001,...,9008 | `local_port=9001,9002,...,9008` | all 8 ports |

### Stress Tests

```sh
# 10 concurrent readers from profile device
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.stress="local_port=9001"'
ssh root@192.168.122.41 'for i in $(seq 1 10); do dd if=/dev/tcpstats/stress of=/dev/null bs=65536 2>/dev/null & done; wait'
# Expected: all complete, no panic

# Rapid profile create/delete
ssh root@192.168.122.41 'for i in $(seq 1 50); do sysctl dev.tcpstats.profiles.tmp="local_port=9001" && sysctl dev.tcpstats.profiles.tmp=""; done'
# Expected: all succeed

# Profile read during delete
ssh root@192.168.122.41 'sysctl dev.tcpstats.profiles.race="local_port=9001"'
ssh root@192.168.122.41 'dd if=/dev/tcpstats/race bs=1 2>/dev/null & sleep 0.1; sysctl dev.tcpstats.profiles.race=""; wait'
# Expected: dd exits cleanly, profile destroyed after fd closes

# 10 load/unload cycles with profiles
ssh root@192.168.122.41 'for i in $(seq 1 10); do kldunload tcp_stats_kld 2>/dev/null; kldload /root/bsd-xtcp/kmod/tcp_stats_kld/tcp_stats_kld.ko; done; dmesg | tail -3'
# Expected: all succeed

# Memory leak check
ssh root@192.168.122.41 'vmstat -m | grep tcpstats'
# Expected: InUse=0 after all fds closed and profiles deleted
```

### Fuzz Testing Commands

`nix develop` provides `afl-clang-fast` and `afl-fuzz` via the `aflplusplus`
package (added to `nix/constants.nix` `securityTools`).

```sh
# Enter dev shell (provides afl-clang-fast, afl-fuzz from aflplusplus)
nix develop

# Build with AFL++ (on Linux dev host)
cd kmod/tcp_stats_kld
afl-clang-fast -o fuzz_filter test/fuzz_filter_parse.c tcp_stats_filter_parse.c -I.
mkdir -p seeds findings
echo "local_port=443" > seeds/basic
echo "local_port=443,8443 exclude=listen,timewait" > seeds/combo
echo "local_addr=10.0.0.0/24 ipv4_only" > seeds/cidr
echo "remote_addr=fe80::/10 local_port=443" > seeds/ipv6
echo "include_state=established format=full fields=all" > seeds/full
afl-fuzz -i seeds/ -o findings/ -- ./fuzz_filter

# Build with libFuzzer (clang, also available via nix develop)
clang -fsanitize=fuzzer,address -o fuzz_filter \
    test/fuzz_filter_parse.c tcp_stats_filter_parse.c -I.
./fuzz_filter -max_len=512 -runs=10000000 corpus/
```

### Definition of Done

1. All 73+ unit tests pass
2. All 9 integration test matrix entries pass
3. All 5 stress tests pass (no panic, no leak, no hang)
4. AFL runs 10M+ iterations with no crashes
5. libFuzzer with ASAN runs 10M+ iterations with no findings
6. `vmstat -m | grep tcpstats` shows InUse=0 after cleanup

### Risk: **Low** (validation only, no new code)

---

## Summary Table

| Phase | Capability | Key API | Risk | Files |
|-------|-----------|---------|------|-------|
| 1 | Expand filter struct | `_Static_assert`, ioctl version check | Medium | `tcp_stats_kld.h`, `tcp_stats_kld.c`, `read_tcpstats.c` |
| 2 | Port filtering in read path | `inp_lport`/`inp_fport` comparison | Low | `tcp_stats_kld.c`, `read_tcpstats.c` |
| 3 | Parser scaffold + port parsing | `strsep`, `strtoul`, `htons` | Medium | NEW: `filter_parse.{h,c}`, `test_filter_parse.c` |
| 4 | State parsing | State name table lookup | Low | `filter_parse.c`, `tcp_stats_kld.c` |
| 5 | IPv4 + CIDR matching | AND+CMP mask comparison | Medium | `filter_parse.c`, `tcp_stats_kld.c` |
| 6 | IPv6 + CIDR matching | `::` compression parser | **High** | `filter_parse.c`, `tcp_stats_kld.c` |
| 7 | Format, fields, flags | Enum lookups, conflict detection | Low | `filter_parse.c`, `tcp_stats_kld.c` |
| 8 | Sysctl profiles | `SYSCTL_ADD_PROC`, `make_dev_credf`, `sx` | **High** | `tcp_stats_kld.c` (major) |
| 9 | Testing + fuzzing | AFL, libFuzzer, ASAN | Low | NEW: `fuzz_filter_parse.c` |
