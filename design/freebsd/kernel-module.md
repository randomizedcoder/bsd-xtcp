# FreeBSD Kernel Module Implementation Plan: `tcp_stats_kld`

[Back to designs](../05-kernel-module.md)

## Overview

This document is the **implementation plan** for the `tcp_stats_kld` kernel module
designed in [design/05-kernel-module.md](../05-kernel-module.md). That document
covers the *what* and *why*; this document covers the *how* — concrete steps,
file layout, build instructions, testing strategy, and integration with the
existing `bsd-xtcp` Rust tool.

The module exposes `/dev/tcpstats`, a read-only character device that streams
fixed-size `tcp_stats_record` structs (320 bytes each) for every TCP socket
on the system. It calls `tcp_fill_info()` per-connection, providing system-wide
RTT, RTO, sequence numbers, window scale, SACK state, and TLP counters that
are otherwise only available via `getsockopt(TCP_INFO)` on an owned socket FD.

---

## 1. Target Environment

| Property | Value |
|---|---|
| FreeBSD version | 14.x / 15-CURRENT (main branch) |
| Source tree | `/home/das/Downloads/freebsd-src` (local clone) |
| Kernel headers required | `sys/netinet/tcp_var.h`, `sys/netinet/in_pcb.h`, `sys/netinet/tcp.h` |
| Build method | Out-of-tree KLD via `bsd.kmod.mk` |
| Architecture | amd64 (primary), aarch64 (secondary) |

---

## 2. File Layout

```
bsd-xtcp/
  kmod/
    tcp_stats_kld/
      Makefile                    # KLD build using bsd.kmod.mk
      tcp_stats_kld.c             # Module implementation (single file)
      tcp_stats_kld.h             # Shared header: struct tcp_stats_record, ioctl defs
      test/
        read_tcpstats.c           # Minimal userspace test program
        Makefile                  # Builds the test program
  design/
    freebsd/
      kernel-module.md            # This file
```

The header `tcp_stats_kld.h` is shared between kernel and userspace
(`#ifdef _KERNEL` guards for kernel-only types). The Rust tool will
use this header's layout to define a matching packed struct for parsing.

---

## 3. Implementation Steps

### Phase 1: Shared Header (`tcp_stats_kld.h`)

Define the ABI contract between kernel module and userspace consumers.

```c
#ifndef _TCP_STATS_KLD_H_
#define _TCP_STATS_KLD_H_

#include <sys/types.h>
#include <sys/ioccom.h>

#ifndef _KERNEL
#include <netinet/in.h>
#endif

#define TCP_STATS_VERSION       1
#define TCP_STATS_RECORD_SIZE   320
#define TCP_STATS_CC_MAXLEN     16
#define TCP_STATS_STACK_MAXLEN  16

/* Record flags */
#define TSR_F_IPV6          0x00000001
#define TSR_F_LISTEN        0x00000002
#define TSR_F_SYNCACHE      0x00000004

/*
 * Fixed-size record emitted by /dev/tcpstats for each TCP connection.
 *
 * Layout is stable across the lifetime of a protocol version.
 * All padding is zeroed. No kernel pointers except tsr_so_addr
 * (already exposed by tcp_pcblist sysctl).
 */
struct tcp_stats_record {
    /* Record header (16 bytes) */
    uint32_t    tsr_version;
    uint32_t    tsr_len;
    uint32_t    tsr_flags;
    uint32_t    _tsr_pad0;

    /* Connection identity (48 bytes) */
    uint8_t     tsr_af;
    uint8_t     _tsr_pad1[3];
    uint16_t    tsr_local_port;
    uint16_t    tsr_remote_port;
    union {
        struct in_addr   v4;
        struct in6_addr  v6;
    }           tsr_local_addr;
    union {
        struct in_addr   v4;
        struct in6_addr  v6;
    }           tsr_remote_addr;

    /* TCP state (8 bytes) */
    int32_t     tsr_state;
    uint32_t    tsr_flags_tcp;

    /* Congestion control (52 bytes) */
    uint32_t    tsr_snd_cwnd;
    uint32_t    tsr_snd_ssthresh;
    uint32_t    tsr_snd_wnd;
    uint32_t    tsr_rcv_wnd;
    uint32_t    tsr_maxseg;
    char        tsr_cc[TCP_STATS_CC_MAXLEN];
    char        tsr_stack[TCP_STATS_STACK_MAXLEN];

    /* RTT from tcp_fill_info() (16 bytes) */
    uint32_t    tsr_rtt;
    uint32_t    tsr_rttvar;
    uint32_t    tsr_rto;
    uint32_t    tsr_rttmin;

    /* Window scale + options (4 bytes) */
    uint8_t     tsr_snd_wscale;
    uint8_t     tsr_rcv_wscale;
    uint8_t     tsr_options;
    uint8_t     _tsr_pad2;

    /* Sequence numbers from tcp_fill_info() (20 bytes) */
    uint32_t    tsr_snd_nxt;
    uint32_t    tsr_snd_una;
    uint32_t    tsr_snd_max;
    uint32_t    tsr_rcv_nxt;
    uint32_t    tsr_rcv_adv;

    /* Counters (20 bytes) */
    uint32_t    tsr_snd_rexmitpack;
    uint32_t    tsr_rcv_ooopack;
    uint32_t    tsr_snd_zerowin;
    uint32_t    tsr_dupacks;
    uint32_t    tsr_rcv_numsacks;

    /* ECN (12 bytes) */
    uint32_t    tsr_ecn;
    uint32_t    tsr_delivered_ce;
    uint32_t    tsr_received_ce;

    /* DSACK (8 bytes) */
    uint32_t    tsr_dsack_bytes;
    uint32_t    tsr_dsack_pack;

    /* TLP (12 bytes) */
    uint32_t    tsr_total_tlp;
    uint64_t    tsr_total_tlp_bytes;

    /* Timers in milliseconds, 0 = not running (24 bytes) */
    int32_t     tsr_tt_rexmt;
    int32_t     tsr_tt_persist;
    int32_t     tsr_tt_keep;
    int32_t     tsr_tt_2msl;
    int32_t     tsr_tt_delack;
    int32_t     tsr_rcvtime;

    /* Buffer utilization (16 bytes) */
    uint32_t    tsr_snd_buf_cc;
    uint32_t    tsr_snd_buf_hiwat;
    uint32_t    tsr_rcv_buf_cc;
    uint32_t    tsr_rcv_buf_hiwat;

    /* Socket metadata (20 bytes) */
    uint64_t    tsr_so_addr;
    uint32_t    tsr_uid;
    uint64_t    tsr_inp_gencnt;

    /* Spare for future expansion (32 bytes) */
    uint32_t    _tsr_spare[8];
} __attribute__((packed, aligned(8)));

/* Compile-time size validation */
_Static_assert(sizeof(struct tcp_stats_record) == TCP_STATS_RECORD_SIZE,
    "tcp_stats_record size mismatch");

/* --- Ioctl definitions --- */

struct tcpstats_version {
    uint32_t    protocol_version;
    uint32_t    record_size;
    uint32_t    record_count_hint;
    uint32_t    flags;
};

struct tcpstats_filter {
    uint16_t    state_mask;     /* Bitmask of (1 << TCPS_*) to include; 0xFFFF=all */
    uint16_t    _pad;
    uint32_t    flags;
#define TSF_EXCLUDE_LISTEN   0x01
#define TSF_EXCLUDE_TIMEWAIT 0x02
};

#define TCPSTATS_VERSION_CMD  _IOR('T', 1, struct tcpstats_version)
#define TCPSTATS_SET_FILTER   _IOW('T', 2, struct tcpstats_filter)
#define TCPSTATS_RESET        _IO('T', 3)

#endif /* _TCP_STATS_KLD_H_ */
```

**Verify:** `sizeof(struct tcp_stats_record)` must equal 320. If the packed
layout doesn't reach exactly 320, adjust `_tsr_spare` to compensate. The
`_Static_assert` enforces this at compile time.

### Phase 2: Module Implementation (`tcp_stats_kld.c`)

Single-file implementation. The code follows the pattern established by
`tcp_pcblist()` in `sys/netinet/tcp_subr.c:2617` and `tcp_fill_info()` in
`sys/netinet/tcp_usrreq.c:1569`.

#### 2.1 Includes and Declarations

```c
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/module.h>
#include <sys/conf.h>
#include <sys/uio.h>
#include <sys/malloc.h>
#include <sys/proc.h>
#include <sys/ucred.h>
#include <sys/socket.h>
#include <sys/socketvar.h>
#include <sys/sysctl.h>
#include <net/vnet.h>
#include <netinet/in.h>
#include <netinet/in_pcb.h>
#include <netinet/tcp.h>
#include <netinet/tcp_var.h>
#include <netinet/tcp_fsm.h>
#include <netinet/cc/cc.h>

#include "tcp_stats_kld.h"

MALLOC_DEFINE(M_TCPSTATS, "tcpstats", "TCP stats KLD per-fd state");
```

#### 2.2 Per-FD Soft State

```c
struct tcpstats_softc {
    struct ucred            *sc_cred;
    uint64_t                sc_gen;
    struct inpcb_iterator   sc_iter;
    struct tcpstats_filter  sc_filter;
    int                     sc_started;
    int                     sc_done;
};
```

Allocated in `open()`, freed in `close()` via `devfs_set_cdevpriv()` destructor.
No global mutable state — concurrent readers each get independent iteration.

#### 2.3 open / close

- `open`: reject `FWRITE`, allocate `tcpstats_softc`, cache credential via
  `crhold()`, set default filter (all states), attach via `devfs_set_cdevpriv()`.
- `close` (destructor): `crfree()` the cached credential, `free()` the softc.

#### 2.4 tcpstats_fill_record()

Core logic — merges data from three kernel sources into one record:

1. **Connection identity**: from `inp->inp_laddr/faddr/lport/fport`, `inp_vflag`
2. **TCP state + congestion control**: from `tcpcb` directly (`t_state`,
   `snd_cwnd`, `snd_ssthresh`, `snd_wnd`, `rcv_wnd`, `t_maxseg`,
   `CC_ALGO(tp)->name`, `tp->t_fb->tfb_tcp_block_name`)
3. **RTT, sequences, options**: from `tcp_fill_info()` output (`tcpi_rtt`,
   `tcpi_rttvar`, `tcpi_rto`, `tcpi_rttmin`, `tcpi_snd_nxt`, etc.)
4. **Timers**: reproduced from `tcp_inptoxtp()` pattern in
   `tcp_subr.c:4262` — `getsbinuptime()` then `(timer - now) / SBT_1MS`
5. **Buffers + socket metadata**: from `inp->inp_socket` (`so_snd.sb_ccc`,
   `so_rcv.sb_ccc`, `so_cred->cr_uid`)

Key references in FreeBSD source:
- `tcp_fill_info()`: `sys/netinet/tcp_usrreq.c:1569`
- `tcp_inptoxtp()`: `sys/netinet/tcp_subr.c:4238`
- `tcp_pcblist()`: `sys/netinet/tcp_subr.c:2617`
- RTT conversion: `tcpi_rtt = ((u_int64_t)tp->t_srtt * tick) >> TCP_RTT_SHIFT`
  (already in usec — `tcp_fill_info` handles this)

#### 2.5 tcpstats_read()

Follows the same iteration pattern as `tcp_pcblist()`:

```
INP_ALL_ITERATOR(&V_tcbinfo, INPLOOKUP_RLOCKPCB)
while (inp = inp_next(&sc->sc_iter)):
    skip if inp->inp_gencnt > sc->sc_gen
    skip if cr_canseeinpcb(sc->sc_cred, inp) != 0
    skip if state not in sc->sc_filter.state_mask
    bzero(&rec)
    tcpstats_fill_record(&rec, inp)
    uiomove(&rec, sizeof(rec), uio)
    break if uio->uio_resid < sizeof(rec)
```

**Critical difference from tcp_pcblist**: `inp_next()` already handles the
read lock on each `inpcb` — when we call `inp_next()`, the *previous* inpcb
is unlocked and the *next* one is read-locked. We do **not** call
`INP_RUNLOCK(inp)` manually unless we break out of the loop early (on
`uiomove` error). The FreeBSD `tcp_pcblist` code at line 2663 shows this
pattern — it only calls `INP_RUNLOCK(inp)` on error.

**Buffer sizing**: `uio->uio_resid` tells us how many bytes the user
requested. We emit records as long as `resid >= sizeof(rec)`. If the user
passes a buffer smaller than one record, we return 0 bytes (not EINVAL) —
this matches standard character device semantics where short reads are normal.

#### 2.6 tcpstats_ioctl()

Three commands:

| Command | Direction | Action |
|---|---|---|
| `TCPSTATS_VERSION_CMD` | `_IOR` | Return protocol version, record size, approximate socket count |
| `TCPSTATS_SET_FILTER` | `_IOW` | Set state bitmask filter on per-fd softc |
| `TCPSTATS_RESET` | `_IO` | Reset iteration state (next read starts from beginning) |

#### 2.7 Dual-Device Module Lifecycle

The module creates **two character devices** from a single KLD, following
the same pattern as `sys/dev/null/null.c` which creates `/dev/null`,
`/dev/zero`, and `/dev/full` from one module:

| Device | Record format | Record size | Target use case |
|---|---|---|---|
| `/dev/tcpstats` | Compact | 128 bytes | Production monitoring on busy servers |
| `/dev/tcpstats-full` | Full | 320 bytes | Debugging, incident investigation, low-connection-count systems |

Both devices share the same iteration and security logic. The difference
is which `tcpstats_fill_*` function is called and how much data is
copied per socket.

```c
/* Compact device — production fast path */
static struct cdevsw tcpstats_cdevsw = {
    .d_version = D_VERSION,
    .d_name    = "tcpstats",
    .d_open    = tcpstats_open,
    .d_close   = tcpstats_close,
    .d_read    = tcpstats_read,
    .d_ioctl   = tcpstats_ioctl,
};

/* Full device — complete field set */
static struct cdevsw tcpstats_full_cdevsw = {
    .d_version = D_VERSION,
    .d_name    = "tcpstats-full",
    .d_open    = tcpstats_open,
    .d_close   = tcpstats_close,
    .d_read    = tcpstats_read_full,
    .d_ioctl   = tcpstats_ioctl,
};

static struct cdev *tcpstats_dev;
static struct cdev *tcpstats_full_dev;

DEV_MODULE(tcp_stats_kld, tcpstats_modevent, NULL);
MODULE_VERSION(tcp_stats_kld, 1);
MODULE_DEPEND(tcp_stats_kld, kernel, __FreeBSD_version,
    __FreeBSD_version, __FreeBSD_version);
```

Module event handler:

```c
static int
tcpstats_modevent(module_t mod, int type, void *data)
{
    switch (type) {
    case MOD_LOAD:
        tcpstats_dev = make_dev_credf(MAKEDEV_ETERNAL_KLD,
            &tcpstats_cdevsw, 0, NULL,
            UID_ROOT, GID_NETWORK, 0440, "tcpstats");
        if (tcpstats_dev == NULL)
            return (ENXIO);

        tcpstats_full_dev = make_dev_credf(MAKEDEV_ETERNAL_KLD,
            &tcpstats_full_cdevsw, 0, NULL,
            UID_ROOT, GID_NETWORK, 0440, "tcpstats-full");
        if (tcpstats_full_dev == NULL) {
            destroy_dev(tcpstats_dev);
            return (ENXIO);
        }

        printf("tcp_stats_kld: loaded, /dev/tcpstats (compact) "
            "and /dev/tcpstats-full available\n");
        return (0);

    case MOD_UNLOAD:
        if (tcpstats_full_dev != NULL)
            destroy_dev(tcpstats_full_dev);
        if (tcpstats_dev != NULL)
            destroy_dev(tcpstats_dev);
        printf("tcp_stats_kld: unloaded\n");
        return (0);

    default:
        return (EOPNOTSUPP);
    }
}
```

The `open()` handler records which device was opened so the read path
knows which format to emit:

```c
static int
tcpstats_open(struct cdev *dev, int oflags, int devtype, struct thread *td)
{
    if (__predict_false(oflags & FWRITE))
        return (EPERM);

    struct tcpstats_softc *sc = malloc(sizeof(*sc), M_TCPSTATS,
        M_WAITOK | M_ZERO);
    sc->sc_cred = crhold(td->td_ucred);
    sc->sc_filter.state_mask = 0xFFFF;
    sc->sc_filter.field_mask = TSR_FIELDS_DEFAULT;

    /* Record which device was opened */
    sc->sc_full = (dev->si_devsw == &tcpstats_full_cdevsw);

    devfs_set_cdevpriv(sc, tcpstats_dtor);
    return (0);
}
```

Alternatively, use separate `d_read` handlers (`tcpstats_read` vs
`tcpstats_read_full`) to eliminate the per-read branch on `sc_full`
entirely — the branch is resolved at `open()` time via the cdevsw
dispatch table. This is the approach shown above and is preferred for
the fast path.

### Phase 3: Build System (`Makefile`)

```makefile
# kmod/tcp_stats_kld/Makefile
KMOD    = tcp_stats_kld
SRCS    = tcp_stats_kld.c

# FreeBSD kernel source tree (set via environment or default)
SYSDIR ?= /usr/src/sys

# Include our header directory
CFLAGS += -I${.CURDIR}

.include <bsd.kmod.mk>
```

**Build**:
```sh
cd kmod/tcp_stats_kld
make SYSDIR=/home/das/Downloads/freebsd-src/sys
```

**Install & Load**:
```sh
sudo make install
sudo kldload tcp_stats_kld
ls -la /dev/tcpstats /dev/tcpstats-full
# crw-r-----  1 root  network  ...  /dev/tcpstats
# crw-r-----  1 root  network  ...  /dev/tcpstats-full
```

**Unload**:
```sh
sudo kldunload tcp_stats_kld
```

### Phase 4: Userspace Test Program

Minimal C program to validate the module works:

```c
/* kmod/tcp_stats_kld/test/read_tcpstats.c */
#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <err.h>
#include <sys/ioctl.h>
#include <arpa/inet.h>
#include "../tcp_stats_kld.h"

int main(void)
{
    int fd = open("/dev/tcpstats", O_RDONLY);
    if (fd < 0)
        err(1, "open /dev/tcpstats");

    /* Query version */
    struct tcpstats_version ver;
    if (ioctl(fd, TCPSTATS_VERSION_CMD, &ver) == 0) {
        printf("version=%u record_size=%u count_hint=%u\n",
            ver.protocol_version, ver.record_size,
            ver.record_count_hint);
    }

    /* Read all records */
    struct tcp_stats_record rec;
    int count = 0;
    while (read(fd, &rec, sizeof(rec)) == sizeof(rec)) {
        char local[INET6_ADDRSTRLEN], remote[INET6_ADDRSTRLEN];
        if (rec.tsr_af == AF_INET) {
            inet_ntop(AF_INET, &rec.tsr_local_addr.v4, local, sizeof(local));
            inet_ntop(AF_INET, &rec.tsr_remote_addr.v4, remote, sizeof(remote));
        } else {
            inet_ntop(AF_INET6, &rec.tsr_local_addr.v6, local, sizeof(local));
            inet_ntop(AF_INET6, &rec.tsr_remote_addr.v6, remote, sizeof(remote));
        }
        printf("[%d] %s:%u -> %s:%u  state=%d  rtt=%u us  cwnd=%u\n",
            count, local, rec.tsr_local_port,
            remote, rec.tsr_remote_port,
            rec.tsr_state, rec.tsr_rtt, rec.tsr_snd_cwnd);
        count++;
    }
    printf("total: %d sockets\n", count);
    close(fd);
    return 0;
}
```

---

## 4. Key Implementation Details from FreeBSD Source

These are the critical kernel APIs the module depends on, verified against the
local FreeBSD source tree.

### 4.1 PCB Iteration

From `sys/netinet/in_pcb.h:727` and `sys/netinet/tcp_subr.c:2619`:

```c
struct inpcb_iterator inpi = INP_ALL_ITERATOR(&V_tcbinfo, INPLOOKUP_RLOCKPCB);
struct inpcb *inp;
while ((inp = inp_next(&inpi)) != NULL) {
    // inp is read-locked here
    // inp_next() unlocks the previous inpcb automatically
}
// After loop: last inpcb is already unlocked by inp_next returning NULL
```

`inp_next()` (`sys/netinet/in_pcb.c:1579`) handles all locking. The only
case where manual `INP_RUNLOCK(inp)` is needed is early exit from the loop
(e.g., on `uiomove` error).

### 4.2 tcp_fill_info()

From `sys/netinet/tcp_usrreq.c:1569`:

- Requires `INP_LOCK_ASSERT(tptoinpcb(tp))` — the inpcb must be locked,
  which `inp_next()` with `INPLOOKUP_RLOCKPCB` provides.
- Calls `bzero(ti, sizeof(*ti))` internally.
- RTT conversion: `tcpi_rtt = ((u_int64_t)tp->t_srtt * tick) >> TCP_RTT_SHIFT`
  — output is already in **microseconds** (no further conversion needed).
- Populates: state, options (timestamps/SACK/wscale/ECN/TFO), RTO, RTT,
  rttvar, ssthresh, cwnd, rcv_space, snd_wnd, sequence numbers (snd_nxt,
  snd_una, snd_max, rcv_nxt, rcv_adv), counters (rexmitpack, ooopack,
  zerowin, dupacks, numsacks), ECN (delivered_ce, received_ce), TLP
  (total_tlp, total_tlp_bytes), rttmin.

### 4.3 Credential Check

From `sys/netinet/tcp_subr.c:2657`:

```c
if (cr_canseeinpcb(req->td->td_ucred, inp) == 0) {
    // caller can see this socket
}
```

Returns 0 on success (caller can see), non-zero if hidden.
Enforces `security.bsd.see_other_uids`, jail scoping, and MAC policies.

### 4.4 Timer Extraction

From `sys/netinet/tcp_subr.c:4262` (`tcp_inptoxtp()`):

```c
sbintime_t now = getsbinuptime();
if (tp->t_timers[TT_REXMT] != SBT_MAX)
    xt->tt_rexmt = (tp->t_timers[TT_REXMT] - now) / SBT_1MS;
else
    xt->tt_rexmt = 0;
```

Timer indices: `TT_DELACK`, `TT_REXMT`, `TT_PERSIST`, `TT_KEEP`, `TT_2MSL`.

### 4.5 Socket Buffer Access

From `tcp_inptoxtp()` via `in_pcbtoxinpcb()`:

```c
struct socket *so = inp->inp_socket;
// Send buffer
so->so_snd.sb_ccc     // current byte count
so->so_snd.sb_hiwat   // high watermark
// Recv buffer
so->so_rcv.sb_ccc
so->so_rcv.sb_hiwat
// UID
so->so_cred->cr_uid
```

### 4.6 Congestion Control and Stack Names

From `tcp_subr.c:4279`:

```c
CC_ALGO(tp)->name              // e.g., "cubic", "newreno"
tp->t_fb->tfb_tcp_block_name  // e.g., "freebsd", "rack"
```

Both are NUL-terminated strings. Use `strlcpy()` with the 16-byte buffer.

---

## 5. Security Considerations

The module implements the five-layer security architecture from design/05:

| Layer | Implementation |
|---|---|
| 1. Device permissions | `0444` world-readable; admin can restrict via `devfs.rules` |
| 2. Open-time validation | Reject `FWRITE`; cache credential via `crhold()` |
| 3. Per-socket credential filtering | `cr_canseeinpcb()` on every socket |
| 4. Output sanitization | `bzero()` every record before populating fields |
| 5. No write path | No `d_write`; ioctls only modify per-fd softc |

**VNET awareness**: Use `CURVNET_SET()`/`CURVNET_RESTORE()` if the module
needs to work correctly in VNET jails. For the initial implementation, target
the default VNET only. Add VNET iteration in a follow-up.

---

## 6. Testing Strategy

### 6.1 Compilation Test

Build the module against the FreeBSD source tree on a FreeBSD system (or VM):

```sh
cd kmod/tcp_stats_kld
make SYSDIR=/usr/src/sys
```

Verify: no warnings with `-Wall -Werror` (standard for FreeBSD kernel code).

### 6.2 Load / Unload Test

```sh
sudo kldload ./tcp_stats_kld.ko
ls -la /dev/tcpstats        # Verify device exists
kldstat | grep tcp_stats     # Verify module loaded
sudo kldunload tcp_stats_kld
ls -la /dev/tcpstats        # Verify device removed
```

### 6.3 Basic Read Test

Run the test program while there are active TCP connections (e.g., SSH session):

```sh
cd kmod/tcp_stats_kld/test
make
./read_tcpstats
```

Expected: at least one record printed with valid RTT (non-zero for
ESTABLISHED connections), correct local/remote addresses matching `sockstat`.

### 6.4 Validation Against sockstat/netstat

```sh
# Compare socket counts
sockstat -4 -6 -P tcp | wc -l
./read_tcpstats | grep "^total:"

# Compare individual connections
sockstat -4 -P tcp -c
./read_tcpstats | grep ESTABLISHED
```

### 6.5 Credential Isolation Test

```sh
# As root: should see all sockets
sudo ./read_tcpstats | wc -l

# As unprivileged user: should see only own sockets
./read_tcpstats | wc -l

# Verify no other users' connections appear
./read_tcpstats | grep -v "uid=$(id -u)"  # should be empty
```

### 6.6 Concurrent Read Test

```sh
# Two readers simultaneously
./read_tcpstats &
./read_tcpstats &
wait
# Both should complete without errors or panics
```

### 6.7 Filter Test

Add a small test that exercises ioctl:
- Set `TSF_EXCLUDE_LISTEN` — verify no LISTEN sockets in output
- Set `TSF_EXCLUDE_TIMEWAIT` — verify no TIME_WAIT sockets
- Use `TCPSTATS_RESET` — verify re-reading from beginning works

### 6.8 Stress Test

```sh
# Generate many connections
for i in $(seq 1 100); do nc -w1 example.com 80 & done
# Read while connections are being created/destroyed
while true; do ./read_tcpstats > /dev/null; done
# Should not panic; generation count skips handle races
```

---

## 7. Integration with bsd-xtcp Rust Tool

### 7.1 Rust-Side Reader Module

Create `src/platform/freebsd_kld.rs`:

```rust
use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;

/// Matches the C struct tcp_stats_record layout exactly (320 bytes)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
struct TcpStatsRecord {
    tsr_version: u32,
    tsr_len: u32,
    tsr_flags: u32,
    _pad0: u32,
    tsr_af: u8,
    _pad1: [u8; 3],
    tsr_local_port: u16,
    tsr_remote_port: u16,
    tsr_local_addr: [u8; 16],   // union: 4 bytes IPv4 or 16 bytes IPv6
    tsr_remote_addr: [u8; 16],
    tsr_state: i32,
    tsr_flags_tcp: u32,
    tsr_snd_cwnd: u32,
    tsr_snd_ssthresh: u32,
    tsr_snd_wnd: u32,
    tsr_rcv_wnd: u32,
    tsr_maxseg: u32,
    tsr_cc: [u8; 16],
    tsr_stack: [u8; 16],
    tsr_rtt: u32,
    tsr_rttvar: u32,
    tsr_rto: u32,
    tsr_rttmin: u32,
    tsr_snd_wscale: u8,
    tsr_rcv_wscale: u8,
    tsr_options: u8,
    _pad2: u8,
    tsr_snd_nxt: u32,
    tsr_snd_una: u32,
    tsr_snd_max: u32,
    tsr_rcv_nxt: u32,
    tsr_rcv_adv: u32,
    tsr_snd_rexmitpack: u32,
    tsr_rcv_ooopack: u32,
    tsr_snd_zerowin: u32,
    tsr_dupacks: u32,
    tsr_rcv_numsacks: u32,
    tsr_ecn: u32,
    tsr_delivered_ce: u32,
    tsr_received_ce: u32,
    tsr_dsack_bytes: u32,
    tsr_dsack_pack: u32,
    tsr_total_tlp: u32,
    tsr_total_tlp_bytes: u64,
    tsr_tt_rexmt: i32,
    tsr_tt_persist: i32,
    tsr_tt_keep: i32,
    tsr_tt_2msl: i32,
    tsr_tt_delack: i32,
    tsr_rcvtime: i32,
    tsr_snd_buf_cc: u32,
    tsr_snd_buf_hiwat: u32,
    tsr_rcv_buf_cc: u32,
    tsr_rcv_buf_hiwat: u32,
    tsr_so_addr: u64,
    tsr_uid: u32,
    tsr_inp_gencnt: u64,
    _spare: [u32; 8],
}

const _: () = assert!(std::mem::size_of::<TcpStatsRecord>() == 320);
```

### 7.2 Conversion to RawSocketRecord

Each `TcpStatsRecord` maps directly to a `RawSocketRecord`:

| TcpStatsRecord field | RawSocketRecord field | Notes |
|---|---|---|
| `tsr_af` | `ip_version` | AF_INET=4, AF_INET6=6 |
| `tsr_local_addr` | `local_addr` | First 4 or 16 bytes of union |
| `tsr_local_port` | `local_port` | Direct copy |
| `tsr_state` | `state` | TCPS_* constant (same as kernel) |
| `tsr_rtt` | `rtt_us` | Already in microseconds |
| `tsr_rttvar` | `rttvar_us` | Already in microseconds |
| `tsr_rto` | `rto_us` | Already in microseconds |
| `tsr_rttmin` | (new field) | Map to `rtt_min_us` proto field |
| `tsr_snd_cwnd` | `snd_cwnd` | Direct copy |
| `tsr_snd_nxt` | `snd_nxt` | Direct copy |
| `tsr_so_addr` | `socket_id` | Join key for process mapping |
| `tsr_cc` | (new field) | Map to `cc_algo` proto field |
| `tsr_stack` | (new field) | Map to `tcp_stack` proto field |
| sources | `vec![5]` | `DataSource::FREEBSD_KLD` |

RTT values from the KLD are already in microseconds (the kernel's
`tcp_fill_info()` handles the conversion from raw ticks). **No additional
conversion is needed** — unlike macOS where ticks must be divided by `hz`.

### 7.3 Fallback Strategy

```rust
pub fn collect() -> Result<CollectionResult> {
    match try_kld_collect() {
        Ok(result) => Ok(result),
        Err(_) => {
            // /dev/tcpstats not available — fall back to sysctl
            // (reduced field set: no RTT, no sequences)
            sysctl_collect()
        }
    }
}
```

---

## 8. Known Risks and Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| `tcp_fill_info()` not exported as a symbol | Module won't link | It's declared in `tcp_var.h` and called from `tcp_usrreq.c` (not static). Verify with `nm /boot/kernel/kernel \| grep tcp_fill_info` |
| `V_tcbinfo` not accessible from KLD | Compilation failure | It's a VNET global, accessible via `VNET_*` macros. The sysctl handlers access it directly |
| `CC_ALGO()` macro not available in KLD context | Missing CC algo name | Include `<netinet/cc/cc.h>`. Alternatively read from `tcp_fill_info` output which doesn't include CC name — fall back to empty string |
| `struct tcpcb` opaque to out-of-tree modules | Can't read `snd_cwnd` etc. directly | `tcpcb` is defined in `tcp_var.h` which is a public kernel header. Out-of-tree KLDs can include it |
| ABI breakage across FreeBSD major versions | Module panics on wrong version | `MODULE_DEPEND` pins to `__FreeBSD_version`. Rebuild required per kernel |
| `inp_next()` API changes | Iteration breaks | API has been stable since FreeBSD 14. Pin module to tested versions |

### Verifying Symbol Availability

On a running FreeBSD system:

```sh
# Check that tcp_fill_info is exported
nm /boot/kernel/kernel | grep tcp_fill_info
# Expected: T tcp_fill_info (text section, globally visible)

# Check V_tcbinfo
nm /boot/kernel/kernel | grep tcbinfo
# Expected: vnet_entry_tcbinfo or similar VNET symbol
```

---

## 9. Implementation Order

| Step | Description | Deliverable |
|---|---|---|
| 1 | Create `kmod/tcp_stats_kld/` directory structure | Directory + Makefiles |
| 2 | Write `tcp_stats_kld.h` with record struct + ioctls | Shared header |
| 3 | Write `tcp_stats_kld.c` — module lifecycle (load/unload) only | Loadable module that creates/destroys `/dev/tcpstats` |
| 4 | Add `tcpstats_open()` / close with per-fd state | Per-fd credential caching |
| 5 | Add `tcpstats_read()` — PCB iteration + record population | Streaming records on read |
| 6 | Add `tcpstats_ioctl()` — version, filter, reset | Ioctl interface |
| 7 | Write `test/read_tcpstats.c` | Userspace validation tool |
| 8 | Test on FreeBSD VM (build, load, read, unload) | Passing basic tests |
| 9 | Add `src/platform/freebsd_kld.rs` to Rust tool | KLD reader module |
| 10 | Integrate into `src/platform/freebsd.rs` collector | End-to-end collection |

Steps 1-7 are pure C, targeting the FreeBSD kernel. Steps 8-10 bridge back
to the Rust codebase.

---

## 10. Open Questions

1. **VNET jails**: Should the module iterate all VNETs or only the caller's?
   Initial implementation targets the default VNET. Per-VNET iteration can
   be added via `VNET_FOREACH()` if needed.

2. **Syncache entries**: `tcp_pcblist` includes syncache entries (SYN_RECEIVED
   via `syncache_pcblist()`). Should the KLD include them? They don't have a
   full `tcpcb`, so `tcp_fill_info()` can't be called. Recommendation: skip
   syncache entries initially; add them later with `TSR_F_SYNCACHE` flag and
   limited fields.

3. **FreeBSD ports integration**: Should this be submitted as a FreeBSD port
   (`sysutils/tcp-stats-kld`)? This provides a standard install path via
   `pkg install`. Defer until the module is stable.

4. **Nix cross-compilation**: The KLD must be compiled on FreeBSD (or with
   a FreeBSD kernel source tree). Nix can provide a dev shell with the right
   headers, but actual kernel module compilation requires FreeBSD's
   `bsd.kmod.mk`. Consider a `freebsd-vm` Nix target for CI.

---

## 11. Performance Engineering for High-Connection-Count Servers

**Design target**: Servers with 50,000-200,000+ concurrent TCP connections,
polled at 1-10 Hz, with **< 0.1% CPU overhead** and **zero measurable impact
on the TCP data path**.

### 11.1 The Performance Problem

At 100K connections with a naive 320-byte-per-socket design:

| Factor | Value | Concern |
|---|---|---|
| Data volume per read | 100K × 320 B = 32 MB | Kernel→user copy dominates |
| Cache lines touched | 100K × ~12 lines = 1.2M cache line reads | Evicts hot data-path caches |
| Per-inpcb lock acquires | 100K | Contention with packet processing |
| Wall time (naive) | 100K × ~300 ns = 30 ms | 3% of a core at 1 Hz poll |
| TLB pressure | 100K structs across ~400 MB of heap | iTLB/dTLB thrashing |

The dominant costs are **cache pollution** and **lock contention with the
data path**, not the CPU cycles themselves. A 30 ms iteration at 100K
sockets means we hold successive read locks for 30 ms total — during which
any writer (connection close, retransmit, ACK processing) on each socket
must wait behind us.

### 11.2 The FreeBSD PCB Locking Model

The `inp_next()` iterator (in_pcb.c:1579) uses **SMR (Safe Memory
Reclamation)** for lockless list traversal and per-inpcb reader-writer locks:

```
smr_enter(ipi->ipi_smr)          # Enter SMR section (lockless)
  for each inpcb on CK_LIST:
    inp_trylock(inp, RLOCKPCB)    # Try per-inpcb rwlock (read)
    if failed:
      refcount_acquire → inp_lock (blocking) → restart from anchor
    smr_exit(ipi->ipi_smr)
    # --- inpcb is now read-locked ---
    # caller does work (tcp_fill_info, field extraction)
    # on next inp_next(), current inpcb is unlocked
    smr_enter(ipi->ipi_smr)
```

Key properties:

| Property | Behavior |
|---|---|
| Global lock | **None.** `ipi_lock` is NOT held during iteration. |
| Per-inpcb lock | `inp->inp_lock` as reader (`rw_rlock`). Multiple readers coexist. |
| List traversal | SMR-protected `CK_LIST` — concurrent insert/delete safe. |
| Lock contention | Only with writers on the *same* inpcb. |

### 11.3 Struct Sizes and Cache Layout

From the FreeBSD source (verified against local tree):

- `CACHE_LINE_SIZE` = 64 bytes on amd64 (`sys/amd64/include/param.h:90`)
- `struct inpcb` is cache-line annotated (`in_pcb.h:168`: "Cache line #1",
  "Cache line #2") — the rwlock (`inp_lock`) and list linkage are on
  cache line #1, the connection identity starts on cache line #2.
- `struct tcpcb` **embeds** `struct inpcb` at offset 0 (`t_inpcb` field,
  `tcp_var.h:309`). The tcpcb extends to ~500+ bytes.
- `struct socket` is a separate allocation.

**Per-socket memory touched by our read path:**

| Data | Approx size | Cache lines | Notes |
|---|---|---|---|
| `inp->inp_lock` (rwlock acquire) | 64 B | 1 | Cache line #1 of inpcb. **Shared with data path.** |
| `inp->inp_vflag, inp_inc` (identity) | 64 B | 1 | Cache line #2+. Identity fields. |
| `tp->t_state` through `tp->snd_cwnd` | ~128 B | 2 | Core TCP state. **Hot in data path.** |
| `tp->t_srtt, t_rttvar, t_rxtcur` | ~64 B | 1 | RTT fields. Warm in data path. |
| `tp->t_sndrexmitpack` through `t_dsack_*` | ~64 B | 1 | Counters. Cold unless retransmitting. |
| `so->so_snd.sb_ccc, so_rcv.sb_ccc` | ~64 B | 1 | Socket buffers. Hot in data path. |
| `CC_ALGO(tp)->name` (pointer chase) | 64 B | 1 | Pointer to CC algo struct, then string read. |
| `tp->t_fb->tfb_tcp_block_name` (pointer chase) | 64 B | 1 | Pointer to function block. |
| **Total per socket** | | **~9 cache lines** | **576 bytes of cache touched** |

**At 100K connections: ~900K cache line reads = ~56 MB of L3 working set.**

This is the real problem. A server with a 30 MB L3 cache will see its
*entire* TCP-hot cache evicted during one iteration pass.

### 11.4 Optimization Strategy

#### Opt 1: Compact Record Format ("Fast Mode")

The 320-byte record includes many fields that are rarely needed at high
frequency (DSACK counters, TLP bytes, ECN, CC algorithm name). Define a
compact 128-byte record for high-frequency polling:

```c
struct tcp_stats_record_compact {
    /* Header (8 bytes) */
    uint32_t    tsr_version;    /* High bit set = compact format */
    uint32_t    tsr_len;        /* 128 */

    /* Identity (12 bytes) — hash-based, not full addr */
    uint64_t    tsr_conn_hash;  /* XXH64 of {af, laddr, faddr, lport, fport} */
    uint16_t    tsr_local_port;
    uint16_t    tsr_remote_port;

    /* Core state (40 bytes) — the high-value fields */
    int32_t     tsr_state;
    uint32_t    tsr_snd_cwnd;
    uint32_t    tsr_snd_ssthresh;
    uint32_t    tsr_snd_wnd;
    uint32_t    tsr_rcv_wnd;
    uint32_t    tsr_rtt;        /* usec */
    uint32_t    tsr_rttvar;     /* usec */
    uint32_t    tsr_rto;        /* usec */
    uint32_t    tsr_maxseg;
    uint32_t    tsr_snd_rexmitpack;

    /* Buffers (16 bytes) */
    uint32_t    tsr_snd_buf_cc;
    uint32_t    tsr_snd_buf_hiwat;
    uint32_t    tsr_rcv_buf_cc;
    uint32_t    tsr_rcv_buf_hiwat;

    /* Metadata (16 bytes) */
    uint64_t    tsr_so_addr;    /* join key */
    uint64_t    tsr_inp_gencnt;

    /* Spare (36 bytes) */
    uint32_t    _spare[9];
} __attribute__((packed, aligned(8)));
_Static_assert(sizeof(struct tcp_stats_record_compact) == 128, "compact size");
```

**Impact**: 128 vs 320 bytes → **60% less `uiomove` copying**, 60% less
userspace memory, 60% less cache pollution on the user side. Kernel-side
cache reads drop from ~9 to ~5 cache lines (skip CC name, stack name, ECN,
DSACK, TLP, sequence numbers, timer details).

Selectable via ioctl:

```c
#define TCPSTATS_SET_FORMAT   _IOW('T', 4, uint32_t)
#define TSR_FORMAT_FULL       0
#define TSR_FORMAT_COMPACT    1
```

#### Opt 2: Batched `uiomove()` with Kernel Staging Buffer

Instead of one `uiomove()` per 320-byte record (each requiring a page table
walk and TLB lookup), batch N records into a kernel staging buffer and do
one large `uiomove()`:

```c
#define TCPSTATS_BATCH_PAGES  4                         /* 16 KB */
#define TCPSTATS_BATCH_SIZE   (TCPSTATS_BATCH_PAGES * PAGE_SIZE)
#define TCPSTATS_BATCH_FULL   (TCPSTATS_BATCH_SIZE / sizeof(struct tcp_stats_record))

/* In per-fd softc — allocated once at open() */
char *sc_batch_buf;    /* malloc'd 16 KB batch buffer */
int   sc_batch_used;   /* bytes populated in current batch */
```

Read path becomes:

```
while (sockets remain && uio->uio_resid > 0):
    fill sc_batch_buf with up to TCPSTATS_BATCH_FULL records
    uiomove(sc_batch_buf, sc_batch_used, uio)   # one large copy
```

**Impact**: At 100K sockets with 320-byte records, this reduces `uiomove()`
calls from 100,000 to ~2,000 (50 records per batch). Each `uiomove()` of
16 KB is far more efficient than 50 × 320 B because:
- Single page table walk amortized across 50 records
- `copyin/copyout` uses REP MOVSB or AVX on amd64 for large blocks
- Fewer kernel↔user transitions

#### Opt 3: Prefetching the Next inpcb

The HPTS code (`tcp_hpts.c:1270`) already uses `kern_prefetch()` (Netflix's
contribution) to prefetch the next tcpcb while processing the current one.
We should do the same:

```c
#include <sys/kern_prefetch.h>

while ((inp = inp_next(&sc->sc_iter)) != NULL) {
    /* Prefetch the NEXT inpcb while we process the current one */
    struct inpcb *next_hint = CK_LIST_NEXT(inp, inp_list);
    if (next_hint != NULL) {
        kern_prefetch(next_hint, &prefetch_done);
        /* prefetcht1: fetch into L2, don't pollute L1 */
    }

    /* ... process current inp (tcp_fill_info, etc.) ... */
}
```

**Impact**: Hides the ~100 ns DRAM latency for each cache-cold inpcb behind
the ~200 ns of processing time for the current one. Particularly effective
on NUMA systems where inpcbs may be in remote memory. This is the same
pattern Netflix uses in the HPTS timer wheel.

**Caveat**: We can't prefetch through `inp_next()` easily because it
manages the SMR section. The prefetch should target the *data fields*
(cache lines #2+) of the next inpcb, not the lock (which `inp_next` handles).
A simpler approach — prefetch `intotcpcb(next_inp)` — gets the tcpcb state
fields into cache early.

#### Opt 4: `maybe_yield()` for Latency Fairness

On a busy server, iterating 100K sockets takes ~30 ms. During this time,
the calling thread is in-kernel and non-preemptible (while holding inpcb
locks). Other runnable threads on the same core are delayed.

Insert voluntary preemption points every N sockets:

```c
#define TCPSTATS_YIELD_INTERVAL  1024

int count = 0;
while ((inp = inp_next(&sc->sc_iter)) != NULL) {
    /* ... process socket ... */

    if (++count % TCPSTATS_YIELD_INTERVAL == 0) {
        /* Not holding any locks here (inp_next unlocked the previous) */
        /* Actually: we ARE holding the current inp's read lock until
           the next inp_next() call. We cannot yield while locked. */
    }
}
```

**Problem**: `inp_next()` keeps the *current* inpcb locked until the next
call. We can't yield while holding a lock. The only safe yield point would
be to stop iteration, save state, return to userspace (which yields
implicitly), and resume on the next `read()`.

**Alternative**: Rely on the fact that each `uiomove()` call can sleep
(it accesses user pages). If the user buffer causes a page fault, the
kernel will context-switch to another thread naturally. For non-faulting
cases, the short per-socket hold time (~200 ns) means we never hold a
single lock for long — other cores can make progress on other sockets.

**Recommendation**: Don't add explicit yielding. Instead, ensure the
userspace tool reads in modest chunks (e.g., 64 KB = 200 records per
`read()`), which naturally creates scheduling opportunities between calls.

#### Opt 5: Selective Field Population

The `tcp_fill_info()` call is ~50-100 ns per socket and touches several
additional cache lines in the tcpcb (RTT vars, sequence numbers, options).
For some use cases, the caller only needs state + cwnd + RTT.

Add a field bitmask to the filter ioctl:

```c
struct tcpstats_filter {
    uint16_t    state_mask;
    uint16_t    _pad;
    uint32_t    flags;
    uint32_t    field_mask;     /* Which field groups to populate */
};

#define TSR_FIELDS_IDENTITY     0x001  /* Always included */
#define TSR_FIELDS_STATE        0x002  /* State + flags */
#define TSR_FIELDS_CONGESTION   0x004  /* cwnd, ssthresh, windows, MSS */
#define TSR_FIELDS_RTT          0x008  /* RTT, RTO, rttvar, rttmin */
#define TSR_FIELDS_SEQUENCES    0x010  /* snd_nxt, snd_una, etc. */
#define TSR_FIELDS_COUNTERS     0x020  /* rexmit, ooo, zerowin, dupacks */
#define TSR_FIELDS_TIMERS       0x040  /* Timer values */
#define TSR_FIELDS_BUFFERS      0x080  /* Send/recv buffer utilization */
#define TSR_FIELDS_ECN          0x100  /* ECN counters */
#define TSR_FIELDS_NAMES        0x200  /* CC algo + stack name (pointer chase) */
#define TSR_FIELDS_ALL          0x3FF

/* Default: identity + state + congestion + RTT + buffers */
#define TSR_FIELDS_DEFAULT      0x08F
```

When `TSR_FIELDS_RTT` is clear, skip the `tcp_fill_info()` call entirely —
saves ~100 ns and ~3 cache lines per socket. When `TSR_FIELDS_NAMES` is
clear, skip the `CC_ALGO()->name` and `t_fb->tfb_tcp_block_name` pointer
chases — saves 2 cache line misses per socket.

**Impact at 100K sockets (default field set vs full):**

| Field set | Cache lines/socket | tcp_fill_info? | Pointer chases | Time/socket | Total time |
|---|---|---|---|---|---|
| Full (all fields) | ~9 | Yes | 2 (CC + stack) | ~300 ns | ~30 ms |
| Default (no names, no ECN) | ~7 | Yes | 0 | ~220 ns | ~22 ms |
| Minimal (state+cwnd+RTT+bufs) | ~5 | Yes | 0 | ~170 ns | ~17 ms |
| Ultra-light (state+cwnd, no RTT) | ~3 | **No** | 0 | ~80 ns | ~8 ms |

#### Opt 6: NUMA-Aware Iteration

On multi-socket servers, inpcbs are allocated in the NUMA domain of the
CPU that created them (`inp_numa_domain` field at `in_pcb.h:182`).
Iterating across NUMA domains causes remote memory accesses (~200 ns vs
~80 ns local).

If the monitoring tool is pinned to a specific NUMA domain, most inpcb
accesses will be remote. Options:

1. **Accept the cost** — remote access is hidden by prefetching (Opt 3).
2. **NUMA-local filtering** — add an ioctl to only return sockets from
   a specific NUMA domain. Run one reader per NUMA node.
3. **Defer to userspace** — run the tool with `cpuset -l` to pin to one
   domain and accept that some reads will be remote.

**Recommendation**: Start with prefetching (Opt 3). Add NUMA filtering
only if profiling shows remote access as a significant bottleneck.

#### Opt 7: Avoid Touching Socket Buffer Metadata When Possible

The `so->so_snd.sb_ccc` and `so->so_rcv.sb_ccc` fields are in the
`struct socket`, which is a *separate allocation* from the inpcb/tcpcb.
This means an additional pointer chase + cache line miss per socket.

For the "fast" field set, make buffer stats opt-in. Most high-frequency
monitoring cares about RTT and cwnd, not buffer fill levels. Buffer stats
can be collected at a lower frequency (e.g., every 10th poll).

### 11.5 Projected Performance After Optimizations

**Scenario: 100K connections, 1 Hz polling, compact format, default fields,
batched uiomove, prefetching:**

| Factor | Naive | Optimized | Improvement |
|---|---|---|---|
| Record size | 320 B | 128 B | 2.5× smaller |
| uiomove calls | 100,000 | ~800 (128 per batch) | 125× fewer |
| Cache lines per socket | ~9 | ~5 (skip names, ECN, socket buf) | 44% fewer |
| DRAM latency hidden | None | Prefetch next inpcb | ~100 ns hidden |
| Estimated wall time | ~30 ms | ~12-15 ms | 2× faster |
| Data path interference | 9 hot cache lines evicted/socket | 5 cache lines, L2-only prefetch | Reduced L1 pollution |
| CPU overhead at 1 Hz | ~3% | ~1.2-1.5% | |
| CPU overhead at 0.2 Hz (5s) | ~0.6% | ~0.25% | |

**For the ultra-light mode (no `tcp_fill_info`, just state+cwnd+bufs):**

| Socket count | Wall time | CPU at 1 Hz | CPU at 5s poll |
|---|---|---|---|
| 50,000 | ~4 ms | 0.4% | 0.08% |
| 100,000 | ~8 ms | 0.8% | 0.16% |
| 200,000 | ~16 ms | 1.6% | 0.32% |

### 11.6 Profiling and Benchmarking Plan

All benchmarks must be run on **production-representative hardware** with
**production-representative connection counts**. Lab numbers on a quiet
VM with 50 sockets are meaningless for this use case.

#### Benchmark 1: Baseline Data Path (no module)

Establish the TCP throughput and latency baseline *without* the module loaded:

```sh
# On server (DUT):
iperf3 -s

# On client:
iperf3 -c $SERVER -t 60 -P 16

# Record: throughput (Gbps), retransmits, CPU usage
```

Repeat with `netperf` for latency:

```sh
netperf -H $SERVER -t TCP_RR -l 60 -- -r 1,1
# Record: transaction rate (txn/s), P50/P99 latency
```

#### Benchmark 2: Connection Generation

Create a realistic connection table using `tcpbench`, `wrk`, or a custom
tool:

```sh
# Generate 100K idle ESTABLISHED connections (e.g., via a connection pool)
# Use a tool that opens N connections and holds them open
# Verify: sysctl net.inet.tcp.pcblist | wc -l  (approx count)
```

#### Benchmark 3: Module Overhead — Microbenchmark

Measure per-read cost with DTrace:

```sh
# Load module
kldload tcp_stats_kld

# DTrace: measure read latency distribution
dtrace -n '
fbt::tcpstats_read:entry { self->ts = timestamp; }
fbt::tcpstats_read:return /self->ts/ {
    @read_ns = quantize(timestamp - self->ts);
    @read_count = count();
    self->ts = 0;
}
tick-10s { printa(@read_ns); printa(@read_count); exit(0); }
'

# In parallel: read in a loop
while true; do cat /dev/tcpstats > /dev/null; done
```

#### Benchmark 4: Module Overhead — Data Path Impact

**The critical measurement.** Run the data path benchmark *while the module
is reading*:

```sh
# Terminal 1: iperf3 throughput test
iperf3 -c $SERVER -t 120 -P 16

# Terminal 2: continuous module reads at 1 Hz
while true; do
    dd if=/dev/tcpstats of=/dev/null bs=65536 2>/dev/null
    sleep 1
done

# Compare iperf3 results with Benchmark 1 baseline
# Target: < 0.5% throughput difference
# Target: < 5% P99 latency difference
```

#### Benchmark 5: Lock Contention

```sh
# DTrace: measure time writers wait for our read lock
dtrace -n '
lockstat:::rw-block /arg0 == (uintptr_t)&tcpstats_magic/ {
    @block_ns = quantize(arg1);
}
'

# Alternative: use lockstat(1)
lockstat -A -s 512 sleep 10
# Look for rwlock contention on inp_lock addresses
```

#### Benchmark 6: Cache Impact

```sh
# PMC (Performance Monitoring Counters) via hwpmc(4)
# Measure LLC misses during module read vs baseline

pmcstat -S LLC-LOAD-MISSES -O /tmp/pmc_baseline.out iperf3 -c $SERVER -t 30
pmcstat -S LLC-LOAD-MISSES -O /tmp/pmc_module.out \
    sh -c 'iperf3 -c $SERVER -t 30 &
           while true; do cat /dev/tcpstats > /dev/null; sleep 1; done'

# Compare LLC-LOAD-MISSES between the two runs
# Target: < 10% increase in LLC misses
```

#### Benchmark 7: Scaling Test

Vary connection count and measure overhead:

```sh
for N in 1000 5000 10000 50000 100000 200000; do
    # Create N connections
    establish_connections $N

    # Measure read time
    TIME=$(dtrace_measure_read_time)
    CPU=$(measure_cpu_during_read)

    echo "$N connections: ${TIME}ms, ${CPU}% CPU"
done

# Expect: linear scaling (O(N) in socket count)
# Red flag: super-linear scaling indicates lock contention
```

#### Benchmark 8: Compact vs Full Format

```sh
# Compare read times for compact (128 B) vs full (320 B) format
# at 100K connections

# Full format
ioctl(fd, TCPSTATS_SET_FORMAT, TSR_FORMAT_FULL);
time read_all(fd);  # → expect ~30 ms

# Compact format
ioctl(fd, TCPSTATS_SET_FORMAT, TSR_FORMAT_COMPACT);
time read_all(fd);  # → expect ~15 ms
```

#### Benchmark 9: Field Mask Impact

```sh
# Measure the impact of skipping tcp_fill_info()
# (TSR_FIELDS_ALL vs TSR_FIELDS_DEFAULT vs TSR_FIELDS_STATE only)

for MASK in 0x3FF 0x08F 0x002; do
    ioctl(fd, TCPSTATS_SET_FILTER, {.field_mask = $MASK});
    time read_all(fd);
done

# The gap between 0x08F (with RTT) and 0x002 (no RTT) measures
# the exact cost of tcp_fill_info() per socket
```

#### Profiling Tools Summary

| Tool | What it measures | FreeBSD command |
|---|---|---|
| DTrace fbt | Function entry/return latency | `dtrace -n 'fbt::tcpstats_read:...'` |
| DTrace lockstat | Lock contention time | `dtrace -n 'lockstat:::rw-block...'` |
| lockstat(1) | Lock hold time + contention | `lockstat -A sleep 10` |
| hwpmc / pmcstat | Cache misses, instructions/cycle | `pmcstat -S LLC-LOAD-MISSES` |
| top -SH | Per-thread CPU usage | `top -SHp $(pgrep bsd-xtcp)` |
| vmstat -i | Interrupt rate (network overhead) | `vmstat -i 1` |
| sysctl | Socket count, TCP state dist | `sysctl net.inet.tcp.states` |
| kgdb | Struct size verification | `p sizeof(struct tcpcb)` |

---

## 12. Socket Filtering — Reducing Iteration Scope

### 12.1 The Opportunity

On a CDN cache node with 100K TCP connections, an operator typically cares
about specific subsets:

- **Client-facing**: remote:ephemeral → local:443 (HTTPS from end users)
- **Upstream origin**: local:ephemeral → remote:443 (fetches to parent caches)
- **Health checks**: remote:* → local:80 (monitoring probes)
- **Active only**: Exclude LISTEN, TIME_WAIT, CLOSE_WAIT noise

If only 30K of 100K connections match the filter, we skip 70K sockets —
eliminating 70% of the cache pollution, lock acquires, and `tcp_fill_info()`
calls.

### 12.2 SMR-Level Pre-Filtering (Zero Lock Overhead)

FreeBSD's `inp_next()` supports an **optional match callback** that runs
inside the SMR section — **before acquiring the per-inpcb read lock**:

```c
/* From in_pcb.h:707 */
typedef bool inp_match_t(const struct inpcb *, void *);

/* From in_pcb.h:719 */
#define INP_ITERATOR(_ipi, _lock, _match, _ctx) ...

/* From in_pcb.c:1594 (first call) and 1627 (subsequent calls):  */
if (match != NULL && (match)(inp, ctx) == false)
    continue;   /* Skip without acquiring lock */
```

The match function can safely read **immutable fields** of the inpcb in SMR
context. For established TCP connections, the connection identity — ports
and addresses — is set at connection establishment and never changes:

- `inp->inp_inc.inc_lport` — local port (network byte order)
- `inp->inp_inc.inc_fport` — foreign port (network byte order)
- `inp->inp_inc.inc_laddr` / `inc_faddr` — IPv4 addresses
- `inp->inp_inc.inc6_laddr` / `inc6_faddr` — IPv6 addresses
- `inp->inp_vflag` — IP version flags (`INP_IPV4`, `INP_IPV6`)

This means port-based and address-based filtering happens **with zero lock
overhead** — non-matching sockets are skipped without touching their rwlock
or any of their tcpcb/socket cache lines. Only the list linkage cache line
(inpcb cache line #1) is touched during SMR traversal.

**Cost comparison per non-matching socket:**

| Approach | Cache lines touched | Lock acquired | Cost |
|---|---|---|---|
| No filter (skip in read loop) | ~9 | Yes (rwlock read) | ~300 ns |
| Post-lock state filter | ~3 (lock + state) | Yes | ~50 ns |
| SMR match callback (port filter) | ~1 (list linkage + ports) | **No** | ~5 ns |

### 12.3 Filter Specification

```c
/*
 * Socket filter — configurable via ioctl or sysctl-created named profiles.
 *
 * All conditions are ANDed. Empty/zero fields mean "match any".
 * Port arrays use network byte order. A port value of 0 means "unused slot".
 *
 * Version 2: adds CIDR masks, include_state mode, expanded exclude flags.
 * See filter-parsing.md section 4 for the full struct documentation.
 */
#define TSF_VERSION             2
#define TSF_MAX_PORTS           8

struct tcpstats_filter {
    /* Version for forward compatibility */
    uint32_t    version;                /* Must be TSF_VERSION */

    /* State filter */
    uint16_t    state_mask;             /* Bitmask of (1 << TCPS_*); 0xFFFF = all */
    uint16_t    _pad0;
    uint32_t    flags;

/* --- Exclude flags (one per TCP state) --- */
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

/* --- Mode flags --- */
#define TSF_STATE_INCLUDE_MODE  0x00001000  /* include_state= used (exclusive with exclude=) */
#define TSF_LOCAL_PORT_MATCH    0x00002000  /* Filter on local ports */
#define TSF_REMOTE_PORT_MATCH   0x00004000  /* Filter on remote ports */
#define TSF_LOCAL_ADDR_MATCH    0x00008000  /* Filter on local address (CIDR) */
#define TSF_REMOTE_ADDR_MATCH   0x00010000  /* Filter on remote address (CIDR) */
#define TSF_IPV4_ONLY           0x00020000
#define TSF_IPV6_ONLY           0x00040000

    /* Port filters — match if socket port is ANY of the listed ports */
    uint16_t    local_ports[TSF_MAX_PORTS];     /* Network byte order; 0 = unused */
    uint16_t    remote_ports[TSF_MAX_PORTS];    /* Network byte order; 0 = unused */

    /* IPv4 address filters with CIDR mask */
    struct in_addr  local_addr_v4;      /* Match if non-zero */
    struct in_addr  local_mask_v4;      /* Netmask (e.g., 0xFFFFFF00 for /24) */
    struct in_addr  remote_addr_v4;
    struct in_addr  remote_mask_v4;

    /* IPv6 address filters with prefix length */
    struct in6_addr local_addr_v6;      /* Match if non-zero */
    uint8_t         local_prefix_v6;    /* Prefix length (0-128); 0 = exact match */
    uint8_t         _pad1[3];
    struct in6_addr remote_addr_v6;
    uint8_t         remote_prefix_v6;
    uint8_t         _pad2[3];

    /* Field mask (which field groups to populate) */
    uint32_t    field_mask;

    /* Record format selection */
    uint32_t    format;                 /* 0 = compact (default), 1 = full */
#define TSF_FORMAT_COMPACT      0
#define TSF_FORMAT_FULL         1

    /* Spare for future expansion */
    uint32_t    _spare[4];
};

_Static_assert(sizeof(struct tcpstats_filter) <= 256,
    "tcpstats_filter exceeds maximum profile size");
```

### 12.4 SMR Match Callback Implementation

```c
/*
 * SMR-safe match function. Called with NO locks held.
 * Can only read immutable inpcb fields (ports, addresses, vflag).
 *
 * Returns true if this socket should be included (will be locked and read).
 * Returns false to skip (no lock acquired, minimal cache impact).
 */
static bool
tcpstats_match(const struct inpcb *inp, void *ctx)
{
    const struct tcpstats_filter *f = ctx;

    /* IP version filter — check before anything else */
    if (__predict_false(f->flags & TSF_IPV4_ONLY)) {
        if (!(inp->inp_vflag & INP_IPV4))
            return (false);
    }
    if (__predict_false(f->flags & TSF_IPV6_ONLY)) {
        if (!(inp->inp_vflag & INP_IPV6))
            return (false);
    }

    /* Local port filter */
    if (__predict_false(f->flags & TSF_LOCAL_PORT_MATCH)) {
        uint16_t lport = inp->inp_inc.inc_lport;  /* network byte order */
        bool found = false;
        for (int i = 0; i < TSF_MAX_PORTS && f->local_ports[i] != 0; i++) {
            if (__predict_true(lport == f->local_ports[i])) {
                found = true;
                break;
            }
        }
        if (__predict_false(!found))
            return (false);
    }

    /* Remote port filter */
    if (__predict_false(f->flags & TSF_REMOTE_PORT_MATCH)) {
        uint16_t fport = inp->inp_inc.inc_fport;
        bool found = false;
        for (int i = 0; i < TSF_MAX_PORTS && f->remote_ports[i] != 0; i++) {
            if (__predict_true(fport == f->remote_ports[i])) {
                found = true;
                break;
            }
        }
        if (__predict_false(!found))
            return (false);
    }

    /* Local address filter with CIDR mask */
    if (__predict_false(f->flags & TSF_LOCAL_ADDR_MATCH)) {
        if (inp->inp_vflag & INP_IPV4) {
            /* IPv4: single AND + CMP — no performance impact */
            if (f->local_addr_v4.s_addr != INADDR_ANY &&
                (inp->inp_inc.inc_laddr.s_addr & f->local_mask_v4.s_addr)
                != (f->local_addr_v4.s_addr & f->local_mask_v4.s_addr))
                return (false);
        } else if (inp->inp_vflag & INP_IPV6) {
            /* IPv6: byte-wise prefix comparison
             * (see filter-parsing.md section 13.2 for implementation) */
            if (!IN6_IS_ADDR_UNSPECIFIED(&f->local_addr_v6) &&
                !tsf_match_v6_prefix(&inp->inp_inc.inc6_laddr,
                    &f->local_addr_v6, f->local_prefix_v6))
                return (false);
        }
    }

    /* Remote address filter with CIDR mask */
    if (__predict_false(f->flags & TSF_REMOTE_ADDR_MATCH)) {
        if (inp->inp_vflag & INP_IPV4) {
            if (f->remote_addr_v4.s_addr != INADDR_ANY &&
                (inp->inp_inc.inc_faddr.s_addr & f->remote_mask_v4.s_addr)
                != (f->remote_addr_v4.s_addr & f->remote_mask_v4.s_addr))
                return (false);
        } else if (inp->inp_vflag & INP_IPV6) {
            if (!IN6_IS_ADDR_UNSPECIFIED(&f->remote_addr_v6) &&
                !tsf_match_v6_prefix(&inp->inp_inc.inc6_faddr,
                    &f->remote_addr_v6, f->remote_prefix_v6))
                return (false);
        }
    }

    return (true);
}
```

Usage in the iterator initialization:

```c
/* With filter: use INP_ITERATOR with match callback */
if (sc->sc_filter.flags & (TSF_LOCAL_PORT_MATCH | TSF_REMOTE_PORT_MATCH |
    TSF_LOCAL_ADDR_MATCH | TSF_REMOTE_ADDR_MATCH |
    TSF_IPV4_ONLY | TSF_IPV6_ONLY))
{
    sc->sc_iter = (struct inpcb_iterator)INP_ITERATOR(
        &V_tcbinfo, INPLOOKUP_RLOCKPCB,
        tcpstats_match, &sc->sc_filter);
} else {
    /* No port/addr filter — use faster INP_ALL_ITERATOR (no callback) */
    sc->sc_iter = (struct inpcb_iterator)INP_ALL_ITERATOR(
        &V_tcbinfo, INPLOOKUP_RLOCKPCB);
}
```

State filtering (`TCPS_ESTABLISHED`, exclude `TCPS_TIME_WAIT`, etc.) still
happens **after** acquiring the lock, because `tp->t_state` is in the tcpcb
and is mutable. But this is still fast — the state check is a single
comparison on a field that's already in cache from the lock acquire.

### 12.5 Named Filter Profiles via sysctl

Operators create named profiles that generate device nodes. This is the
operational interface — no code changes needed for common filter patterns.

```sh
# Create a named filter profile via sysctl
sysctl dev.tcpstats.profiles.cdn_clients="local_port=443 exclude=listen,timewait"
# Module creates: /dev/tcpstats/cdn_clients

sysctl dev.tcpstats.profiles.cdn_upstream="remote_port=443,80 exclude=listen,timewait"
# Module creates: /dev/tcpstats/cdn_upstream

sysctl dev.tcpstats.profiles.all_active="exclude=listen,timewait,closewait"
# Module creates: /dev/tcpstats/all_active
```

**Implementation**: The module registers a sysctl node
`dev.tcpstats.profiles` with a handler that parses the filter string,
validates it, stores it in a list, and calls `make_dev_credf()` to create
the device. Each profile device has its own `cdevsw` (or a shared one
with the profile name as `si_drv1` context).

**Limit**: Maximum of 16 named profiles to bound kernel memory.

### 12.6 Built-In Filter Devices

The module creates a set of common built-in profiles without sysctl
configuration:

| Device | Filter | Use case |
|---|---|---|
| `/dev/tcpstats` | Compact format, no filter, all states | General production monitoring |
| `/dev/tcpstats-full` | Full format, no filter, all states | Deep debugging |
| `/dev/tcpstats/active` | Compact, exclude LISTEN+TIME_WAIT+CLOSE_WAIT | Active connections only |

The sysctl-based profiles extend this with operator-defined filters:

| Device (sysctl-created) | Filter | CDN example |
|---|---|---|
| `/dev/tcpstats/cdn_clients` | local_port=443, exclude LISTEN/TIME_WAIT | Client-facing HTTPS |
| `/dev/tcpstats/cdn_upstream` | remote_port=443,80, exclude LISTEN/TIME_WAIT | Origin/parent fetches |
| `/dev/tcpstats/health` | local_port=80,8080, exclude TIME_WAIT | Health check endpoints |

### 12.7 Filter String Syntax

See [filter-parsing.md](filter-parsing.md) for the complete filter grammar
(EBNF), parser design, security analysis, exhaustive input validation
tables, and operator recipes.

Summary: filter strings are space-separated directives (`key=value` or
bare flags), parsed at sysctl write time. The parsed `tcpstats_filter`
struct is stored with the device and copied into the per-fd softc at
`open()` time. Invalid syntax returns `EINVAL` with a human-readable
error in `dev.tcpstats.last_error`.

### 12.8 CDN Cache Node Example

A CDN cache node typically has:
- 50K client connections (remote:ephemeral → local:443)
- 5K upstream connections (local:ephemeral → remote:443)
- 2K health check connections (remote:ephemeral → local:80)
- 10K connections in TIME_WAIT
- 200 LISTEN sockets

```sh
# System setup (e.g., in /etc/rc.local or loader.conf):
kldload tcp_stats_kld

# Create operator-defined profiles:
sysctl dev.tcpstats.profiles.clients="local_port=443 exclude=listen,timewait"
sysctl dev.tcpstats.profiles.upstream="remote_port=443,80 exclude=listen,timewait"

# Monitoring tool reads only the relevant subset:
bsd-xtcp --source /dev/tcpstats/clients --interval 1s   # 50K sockets
bsd-xtcp --source /dev/tcpstats/upstream --interval 5s   # 5K sockets

# Quick operational check:
cat /dev/tcpstats/clients | head -20    # First 20 client connections
cat /dev/tcpstats/active | wc -c        # Count all active connections
```

**Performance impact of filtering:**

| Reader | Sockets matched | Sockets skipped (SMR only) | Lock acquires saved | Time saved |
|---|---|---|---|---|
| `/dev/tcpstats` (unfiltered) | 67K | 0 | 0 | baseline |
| `/dev/tcpstats/clients` | 50K | 17K | 17K | ~25% faster |
| `/dev/tcpstats/upstream` | 5K | 62K | 62K | ~93% faster |
| `/dev/tcpstats/active` | 57K (post-lock state filter) | 0 (state not in SMR) | 0 | ~15% faster (less uiomove) |

The upstream reader is 93% faster because port matching in SMR eliminates
lock acquisition for 62K sockets — each skipped at ~5 ns instead of ~300 ns.

### 12.9 Ioctl-Based Filter (Programmatic)

For the Rust tool, the ioctl interface provides the same filtering without
requiring sysctl profile creation:

```c
#define TCPSTATS_SET_FILTER   _IOW('T', 2, struct tcpstats_filter)
```

```rust
// Rust tool: set filter programmatically after open()
let fd = open("/dev/tcpstats", O_RDONLY)?;

let mut filter = tcpstats_filter::default();
filter.flags = TSF_LOCAL_PORT_MATCH | TSF_EXCLUDE_TIMEWAIT;
filter.local_ports[0] = 443u16.to_be();  // Network byte order
ioctl(fd, TCPSTATS_SET_FILTER, &filter)?;

// First read() initializes the iterator with the filter
let records = read_all(fd)?;
```

The ioctl stores the filter in the per-fd softc. The iterator is
initialized on the first `read()`, so setting the filter before reading
takes effect correctly.

### 12.10 Filter Safety

| Concern | Mitigation |
|---|---|
| Filter must not cause match function to access freed memory | SMR section guarantees inpcb is valid during match. Only immutable fields accessed. |
| Port values in filter must be network byte order | Document clearly. Provide userspace helper macro `TSF_PORT(n)` = `htons(n)`. |
| Race between state change and state filter | State filter runs post-lock — sees consistent state. Socket may transition between match and read — acceptable (same as tcp_pcblist). |
| Too many sysctl profiles exhaust kernel memory | Hard limit of 16 profiles. Each profile is ~256 bytes + one `cdev`. |
| Profile deletion while device is open | `destroy_dev()` waits for all fds to close. Profile deletion deferred until safe. |
| Filter string parsing rejects malformed input | See [filter-parsing.md](filter-parsing.md) section 8 for exhaustive input validation tables covering structural, key, port, state, address, and conflict rejections. |
| Integer overflow in port parsing | Pre-validated digit-only input, max 5 digits, range check. See [filter-parsing.md](filter-parsing.md) section 7 for security analysis. |
| IPv6 address parser bounded execution | Max 8 groups, max 4 hex digits per group, single `::` enforced. See [filter-parsing.md](filter-parsing.md) section 6. |
| CIDR host bits validation | Parser rejects addresses with host bits set (e.g., `10.0.0.1/24`). See [filter-parsing.md](filter-parsing.md) section 6.4. |

---

## 13. Compiler Hints and Low-Level Optimization

FreeBSD kernel code makes extensive use of compiler hints. The `inp_next()`
implementation we depend on uses `__predict_true`/`__predict_false` on
nearly every branch (in_pcb.c:1496, 1497, 1506, 1509, 1630, 1631, 1653,
1656, 1663). Our module should follow the same discipline.

### 12.1 Branch Prediction Hints

From `sys/sys/cdefs.h:341`:
```c
#define __predict_true(exp)   __builtin_expect((exp), 1)
#define __predict_false(exp)  __builtin_expect((exp), 0)
```

Applied throughout the hot read path:

```c
static int
tcpstats_read(struct cdev *dev, struct uio *uio, int ioflag)
{
    struct tcpstats_softc *sc;
    struct inpcb *inp;
    int error;

    error = devfs_get_cdevpriv((void **)&sc);
    if (__predict_false(error != 0))
        return (error);

    /* EOF — already iterated all sockets */
    if (__predict_false(sc->sc_done))
        return (0);

    /* Initialize iteration on first read */
    if (__predict_false(!sc->sc_started)) {
        sc->sc_gen = V_tcbinfo.ipi_gencnt;
        sc->sc_iter = (struct inpcb_iterator)INP_ALL_ITERATOR(
            &V_tcbinfo, INPLOOKUP_RLOCKPCB);
        sc->sc_started = 1;
    }

    /* --- Inner loop: hot path --- */
    while (__predict_true(uio->uio_resid >= (ssize_t)sizeof(rec))) {
        inp = inp_next(&sc->sc_iter);
        if (__predict_false(inp == NULL)) {
            sc->sc_done = 1;
            break;
        }

        /* Generation check — new sockets rare during iteration */
        if (__predict_false(inp->inp_gencnt > sc->sc_gen))
            continue;

        /* Credential check — root sees all, fast path */
        if (__predict_false(cr_canseeinpcb(sc->sc_cred, inp) != 0))
            continue;

        /* State filter — most sockets pass */
        if (__predict_true(sc->sc_filter.state_mask == 0xFFFF))
            goto populate;  /* No filter — skip the check entirely */
        {
            struct tcpcb *tp = intotcpcb(inp);
            if (__predict_false(
                !(sc->sc_filter.state_mask & (1 << tp->t_state))))
                continue;
        }

populate:
        /* Populate and emit record */
        bzero(&rec, sizeof(rec));
        tcpstats_fill_record_compact(&rec, inp);

        error = uiomove(&rec, sizeof(rec), uio);
        if (__predict_false(error != 0)) {
            INP_RUNLOCK(inp);
            return (error);
        }
    }

    return (0);
}
```

**Rationale for each hint:**

| Branch | Prediction | Why |
|---|---|---|
| `devfs_get_cdevpriv` error | `__predict_false` | Only fails on invalid fd — never happens in normal operation |
| `sc->sc_done` | `__predict_false` | True only on the final read call of the session |
| `!sc->sc_started` | `__predict_false` | True only on the first read call |
| `uio->uio_resid >= sizeof(rec)` | `__predict_true` | User typically provides a large buffer; loop runs many times |
| `inp == NULL` | `__predict_false` | Only true at end of list — once per session |
| `inp_gencnt > sc_gen` | `__predict_false` | New sockets during iteration are rare |
| `cr_canseeinpcb != 0` | `__predict_false` | Root caller sees all; non-root typically sees most of their own |
| `state_mask == 0xFFFF` | `__predict_true` | Default is no filter — most callers don't filter |
| `state_mask & (1 << t_state)` | `__predict_false` (negated) | When filtering is active, most sockets pass |
| `uiomove error` | `__predict_false` | User buffer faults are extremely rare |

### 12.2 Function Inlining

```c
/* The record fill functions are called once per socket in the inner loop.
 * Force-inline the compact version (small) to avoid function call overhead.
 * The full version is larger — let the compiler decide. */
static __always_inline void
tcpstats_fill_record_compact(struct tcp_stats_record_compact *rec,
    struct inpcb *inp)
{
    struct tcpcb *tp = intotcpcb(inp);
    struct tcp_info ti;

    tcp_fill_info(tp, &ti);  /* Not inlined — kernel function */

    rec->tsr_version = TCP_STATS_VERSION | TSR_VERSION_COMPACT;
    rec->tsr_len = sizeof(*rec);
    /* ... populate ~15 fields ... */
}

/* Full version: too large to inline profitably */
static __noinline void
tcpstats_fill_record_full(struct tcp_stats_record *rec,
    struct inpcb *inp)
{
    /* ... populate ~40 fields + pointer chases for CC/stack names ... */
}
```

### 12.3 Data Section Placement

```c
/* Read-mostly globals — placed in .data.read_mostly to avoid
 * false sharing with writable data on the same cache line. */
static struct cdev * __read_mostly tcpstats_dev;
static struct cdev * __read_mostly tcpstats_full_dev;
```

From `sys/sys/systm.h:89`:
```c
#define __read_mostly     __section(".data.read_mostly")
#define __read_frequently __section(".data.read_frequently")
```

These section attributes tell the kernel linker to group read-mostly data
together, minimizing cache line sharing with frequently-written data.
The `cdev *` pointers are written once at `MOD_LOAD` and read on every
`open()` — ideal for `__read_mostly`.

### 12.4 Prefetch with Cache Level Control

From `sys/sys/kern_prefetch.h` (Netflix contribution, used by HPTS):

```c
#include <sys/kern_prefetch.h>

/* In the iteration loop, after processing the current inpcb,
 * prefetch the next tcpcb's data fields into L2.
 *
 * prefetcht1 loads into L2 (not L1), reducing L1 pollution of
 * the data path's hot working set while still eliminating the
 * ~100 ns DRAM fetch latency for the next socket.
 */
static __always_inline void
tcpstats_prefetch_next(struct inpcb *inp)
{
    struct inpcb *next = CK_LIST_NEXT(inp, inp_list);
    int32_t dummy;

    if (__predict_true(next != NULL)) {
        struct tcpcb *next_tp = intotcpcb(next);

        /* Prefetch tcpcb state fields (cache line containing t_state,
         * snd_cwnd, snd_nxt, etc.) — offset ~340 bytes from inpcb start */
        kern_prefetch(&next_tp->t_state, &dummy);

        /* Prefetch the RTT fields — ~60 bytes further */
        kern_prefetch(&next_tp->t_srtt, &dummy);
    }
}
```

**Cache level rationale**: `prefetcht1` fetches into L2 (and L3) but not
L1. This is deliberate — the TCP data path's hot working set (ACK
processing, cwnd updates) lives in L1. Our monitoring prefetch should
warm L2 so our read is fast (~5 ns L2 hit vs ~100 ns DRAM) without
evicting the data path's L1 entries.

### 12.5 Avoiding Pointer Chases in the Compact Path

The full record path does two pointer chases per socket that each cause
a cache miss:

1. `CC_ALGO(tp)->name` — follows `tp->t_cc` pointer to `struct cc_algo`,
   then reads the `name` field
2. `tp->t_fb->tfb_tcp_block_name` — follows `tp->t_fb` pointer to
   `struct tcp_function_block`, then reads the name string

The compact record skips these entirely. For the full path, if these
names don't change during a connection's lifetime (they're set at
connection establishment), consider caching them in the per-fd softc
indexed by `inp_gencnt` — but this adds complexity. Initial
implementation should just accept the cache misses on the full path.

### 12.6 Batch Buffer Alignment

```c
/* Align the batch staging buffer to a cache line boundary to ensure
 * uiomove (which may use REP MOVSB or AVX) gets optimal throughput */
sc->sc_batch_buf = malloc(TCPSTATS_BATCH_SIZE, M_TCPSTATS,
    M_WAITOK | M_ZERO | M_ALIGNBUF);

/* If M_ALIGNBUF is not available, allocate with extra space: */
sc->sc_batch_raw = malloc(TCPSTATS_BATCH_SIZE + CACHE_LINE_SIZE,
    M_TCPSTATS, M_WAITOK | M_ZERO);
sc->sc_batch_buf = (char *)roundup2((uintptr_t)sc->sc_batch_raw,
    CACHE_LINE_SIZE);
```

### 12.7 Summary of Compiler/Low-Level Hints Used

| Technique | FreeBSD API | Where applied |
|---|---|---|
| Branch prediction | `__predict_true`, `__predict_false` | Every branch in the read loop |
| Force inline | `__always_inline` | Compact record fill function |
| Prevent inline | `__noinline` | Full record fill function (too large) |
| Cache prefetch (L2) | `kern_prefetch()` / `prefetcht1` | Next inpcb's tcpcb data fields |
| Data section placement | `__read_mostly` | Device pointers, module globals |
| Cache-aligned alloc | `roundup2(ptr, CACHE_LINE_SIZE)` | Batch staging buffer |

---

## 13. Alternative Interfaces to Character Devices

### 12.1 Comparison

| Interface | Advantages | Disadvantages |
|---|---|---|
| **Character device** (`/dev/tcpstats`) | Streaming (no kernel buffer alloc), ioctl for config, per-fd state via cdevpriv, standard UNIX semantics | Requires module, slightly higher syscall overhead (open+read+close vs single sysctl) |
| **Sysctl node** | No module needed if upstreamed, existing pattern (tcp_pcblist) | Must allocate N×record_size contiguous kernel buffer, ENOMEM risk at scale, no per-client state, no ioctl |
| **Netlink socket** | Event-driven possible (notifications), Linux-compatible API pattern, message-based | FreeBSD netlink has **no inet_diag/sock_diag equivalent** (checked — only CARP uses netlink in netinet). Would require writing a new netlink family from scratch. Substantially more complex than chardev. |
| **kqueue/kevent** | Could notify on socket creation/deletion events | Not suitable for bulk snapshot reads. Good as future *complement* to polling, not a replacement |
| **Shared memory (mmap)** | Zero-copy, lowest possible latency | Complex: kernel must manage ring buffer, user must handle wrap-around, synchronization is tricky, no credential filtering possible per-record |

### 12.2 Recommendation: Character Device with Batched Reads

The character device remains the best choice. However, we can optimize the
read path for throughput:

**Batch uiomove()**: Instead of calling `uiomove()` once per record (320
bytes each), accumulate a batch of records into a kernel-side buffer (e.g.,
16 KB = 51 records), then do a single `uiomove()` per batch. This amortizes
the per-syscall overhead.

```c
#define TCPSTATS_BATCH_SIZE    (16 * 1024)  /* 16 KB kernel batch buffer */
#define TCPSTATS_BATCH_RECORDS (TCPSTATS_BATCH_SIZE / sizeof(struct tcp_stats_record))

/* In the per-fd softc: */
struct tcp_stats_record sc_batch[TCPSTATS_BATCH_RECORDS];
int sc_batch_count;
```

Trade-off: Uses ~16 KB of kernel memory per open fd (allocated at open time,
not per-read). At 10 concurrent readers, that's 160 KB — negligible.

**Userspace-side batching**: The Rust tool should issue reads with large
buffers (e.g., 64 KB = 204 records per read). This reduces syscall count
from N (one per socket) to N/204.

### 12.3 Future: Netlink as an Upstream Path

If the module proves useful, the long-term path is to propose a
`NETLINK_SOCK_DIAG` family for FreeBSD (similar to Linux's `inet_diag`).
This would make TCP socket enumeration a first-class kernel facility.
The character device is the pragmatic starting point that doesn't require
kernel tree changes.

---

## 14. Security Model — Revised

### 13.1 Device Ownership: `root:network` with `0440`

The FreeBSD `etc/group` file defines a standard `network` group (GID 69).
This is more appropriate than world-readable:

```c
tcpstats_dev = make_dev_credf(MAKEDEV_ETERNAL_KLD, &tcpstats_cdevsw, 0,
    NULL, UID_ROOT, GID_NETWORK, 0440, "tcpstats");
```

Where `GID_NETWORK` is 69 (define in the module or use the numeric literal).

| Permissions | Who can read | Security posture |
|---|---|---|
| `0444` (world-readable) | Everyone | Relies entirely on `cr_canseeinpcb()` — users see only their own sockets |
| `0440` (root + group) | root and `network` group members | Defense in depth: device access requires group membership AND `cr_canseeinpcb()` filters per-socket |
| `0400` (root only) | root | Maximum restriction but defeats unprivileged use case |

**Recommendation: `0440` with `root:network`.**

- Administrators add monitoring users to the `network` group.
- `cr_canseeinpcb()` still enforces per-socket visibility — a `network`
  group member who is not root still only sees their own sockets unless
  `security.bsd.see_other_uids=1` (the default).
- For root-level monitoring (see all sockets), run as root or with a
  privileged helper.
- Administrators can override via `devfs.rules(5)` for custom policies.

### 13.2 Credential Model Details

FreeBSD's `cr_canseeinpcb()` checks these sysctls:

| Sysctl | Default | Effect |
|---|---|---|
| `security.bsd.see_other_uids` | 1 | If 0: non-root users can only see own-UID sockets |
| `security.bsd.see_other_gids` | 1 | If 0: non-root users can only see own-GID sockets |
| `security.bsd.see_jail_proc` | 1 | If 0: jails cannot see host sockets |

On a hardened system with `see_other_uids=0`, even a `network` group member
sees only their own sockets — the device permission grants access to the
*mechanism*, but `cr_canseeinpcb()` controls *which data* they receive.

### 13.3 Additional Security Hardening

| Hardening | Implementation | Notes |
|---|---|---|
| Rate limiting | `ratecheck()` on open | Prevent rapid re-open as a DoS vector. Limit to e.g., 10 opens/sec per UID. |
| Max concurrent FDs | Counter in softc | Limit to e.g., 32 concurrent open fds to the device. Prevents fd exhaustion. |
| `securelevel` check | On MOD_LOAD | Refuse to load if `securelevel >= 2` (same as standard KLD policy). |
| Audit trail | `AUDIT_ARG_DEV` | Emit audit record on open for systems running `auditd`. |
| MAC integration | `mac_device_check_open()` | Already provided by devfs for MAC-aware systems. |
| Pointer sanitization | Hash `tsr_so_addr` | Instead of raw kernel pointer, use `XXH64(so_addr, secret)`. Breaks `kern.file` join — only enable via sysctl tunable. |

---

## 15. Reliability Considerations

### 14.1 Locking Safety

| Risk | Cause | Mitigation |
|---|---|---|
| Deadlock | Module holds lock A, kernel path holds lock B, each waits for the other | Our module only acquires inpcb read locks via `inp_next()`, which is the *same* code path used by `tcp_pcblist`. If `tcp_pcblist` is deadlock-free, we are too. No additional locks introduced. |
| Lock ordering violation | Acquiring locks in wrong order | We acquire exactly one lock at a time (per-inpcb rwlock) via the iterator. No nesting. |
| Reader starvation | Writer holds inpcb wlock indefinitely | FreeBSD rwlocks are fair — readers don't starve. TCP write-lock hold times are bounded (packet processing). |
| Iteration during module unload | Reader is mid-iteration when `kldunload` runs | `destroy_dev()` waits for all open fds to close before returning. Concurrent reads complete normally. |

### 14.2 Memory Safety

| Risk | Mitigation |
|---|---|
| Kernel heap overflow | Fixed-size record (320 bytes), `_Static_assert` enforces size at compile time. No variable-length copies. |
| Use-after-free on inpcb | SMR protects traversal — freed inpcbs remain safe to dereference within SMR section. `inp_next()` handles `INP_FREED` flag. |
| Leaked credentials | `crhold()` in open, `crfree()` in close destructor. `devfs_set_cdevpriv()` guarantees destructor runs even on process crash. |
| Per-fd state leak | `devfs_set_cdevpriv()` destructor frees `tcpstats_softc` on close/exit/signal. |

### 14.3 Error Handling

| Scenario | Behavior |
|---|---|
| `uiomove()` returns error (e.g., bad user pointer) | Return error immediately. The current inpcb is still locked by `inp_next()` — we must call `INP_RUNLOCK(inp)` before returning. |
| `malloc()` fails in open | Return `ENOMEM`. No state to clean up. |
| Module loaded on system with no TCP sockets | First `inp_next()` returns NULL immediately → `read()` returns 0 (EOF). |
| Socket destroyed during iteration | Generation check skips it. If it was the *current* inpcb, `inp_next()` handles `INP_FREED` via SMR restart. |
| VNET destroyed during iteration | Only iterate the default VNET initially. Future VNET support requires `CURVNET_SET()` per-VNET with proper locking. |

### 14.4 Graceful Degradation

The Rust userspace tool should handle these cases:

| Condition | Tool behavior |
|---|---|
| `/dev/tcpstats` doesn't exist | Fall back to `sysctl net.inet.tcp.pcblist` (reduced fields, no RTT) |
| `open()` returns `EACCES` | User not in `network` group. Log warning, fall back to sysctl |
| `read()` returns 0 on first call | No visible sockets (credential filtering). Normal — emit empty batch |
| Record `tsr_version` doesn't match | Module/tool version mismatch. Log error, refuse to parse |
| `tsr_len` doesn't match `sizeof` | ABI change. Same as above |
| `ioctl(TCPSTATS_VERSION_CMD)` fails | Old module without ioctl. Proceed with defaults (all states, no filter) |

---

## 16. Testing Strategy — Expanded

### 15.1 Kernel-Side Tests

| Test | Method | Validates |
|---|---|---|
| **Record size assertion** | `_Static_assert(sizeof(struct tcp_stats_record) == 320)` | ABI stability at compile time |
| **bzero coverage** | Manual audit: every field in the record must be either populated or left as zero | No kernel memory leakage through uninitialized fields |
| **Lock balance** | `witness(4)` with `LOCK_PROFILING` kernel | No lock ordering violations, no unreleased locks |
| **Memory leak** | `kldload`, run test, `kldunload`, check `vmstat -m \| grep tcpstats` | All allocations freed on unload |
| **Panic resistance** | Load module, kill -9 reader mid-iteration, verify system stable | `devfs_set_cdevpriv` destructor handles cleanup |

### 15.2 Functional Tests

| Test | Command | Expected |
|---|---|---|
| **Basic read** | `./read_tcpstats` | Prints records for active connections |
| **RTT populated** | `./read_tcpstats \| grep rtt` | Non-zero RTT for ESTABLISHED connections |
| **Matches sockstat** | `diff <(sockstat -4c -P tcp \| wc -l) <(./read_tcpstats \| grep -c ESTABLISHED)` | Counts match (approximately) |
| **IPv6 support** | Create IPv6 connection, verify `tsr_af=28` | IPv6 addresses correct |
| **LISTEN sockets** | `./read_tcpstats \| grep state=2` | Shows listening sockets with RTT=0 |
| **Filter: exclude LISTEN** | Set `TSF_EXCLUDE_LISTEN`, read | No state=2 records |
| **Filter: exclude TIME_WAIT** | Set `TSF_EXCLUDE_TIMEWAIT`, read | No state=11 records |
| **Reset** | Read half, ioctl RESET, read again | Gets all records from beginning |
| **Empty system** | Test on system with no TCP connections | `read()` returns 0 immediately |

### 15.3 Security Tests

| Test | Method | Expected |
|---|---|---|
| **Non-root visibility** | Run as unprivileged user | Only sees own sockets |
| **Group enforcement** | Remove user from `network` group, try open | `EACCES` |
| **see_other_uids=0** | `sysctl security.bsd.see_other_uids=0`, read as non-root | Only own sockets visible |
| **Jail isolation** | Read from inside a jail | Only jail-scoped sockets |
| **Write rejected** | `open("/dev/tcpstats", O_RDWR)` | Returns `EPERM` |
| **No padding leak** | Read records, check bytes at known padding offsets | All zero |
| **Pointer stability** | Compare `tsr_so_addr` with `sockstat -v` output | Values match (same kernel pointer) |

### 15.4 Performance Tests

| Test | Tool | Target |
|---|---|---|
| **Latency** | `dtrace -n 'fbt::tcpstats_read:entry { self->ts = timestamp; } fbt::tcpstats_read:return /self->ts/ { @=quantize(timestamp - self->ts); }'` | < 1 ms for 500 sockets |
| **Throughput** | Read in tight loop, measure records/sec | > 100,000 records/sec |
| **CPU overhead** | `top -P` while reading at 1 Hz with 1000 sockets | < 0.5% single core |
| **Lock contention** | `lockstat` during concurrent read + heavy TCP traffic | < 1% read-lock contention on per-inpcb locks |
| **Data path impact** | `iperf3` benchmark with and without module reading | < 1% throughput difference |

### 15.5 Regression Tests (for CI)

A simple script that can run in a FreeBSD VM:

```sh
#!/bin/sh
# test_tcp_stats_kld.sh — Automated regression test

set -e

echo "=== Build ==="
cd kmod/tcp_stats_kld && make clean && make

echo "=== Load ==="
sudo kldload ./tcp_stats_kld.ko
test -c /dev/tcpstats || (echo "FAIL: device not created" && exit 1)

echo "=== Version ioctl ==="
./test/read_tcpstats --version-only | grep "version=1" || \
    (echo "FAIL: version mismatch" && exit 1)

echo "=== Basic read ==="
NREC=$(./test/read_tcpstats | grep -c '^\[')
echo "Read $NREC records"
test "$NREC" -gt 0 || echo "WARN: no records (no TCP connections?)"

echo "=== Non-root read ==="
NREC_USER=$(sudo -u nobody ./test/read_tcpstats 2>/dev/null | grep -c '^\[' || true)
echo "Non-root saw $NREC_USER records (expected 0 on hardened system)"

echo "=== Write rejected ==="
# Attempt to open for writing should fail
! ./test/read_tcpstats --write-test 2>/dev/null || \
    (echo "FAIL: write open succeeded" && exit 1)

echo "=== Concurrent read ==="
./test/read_tcpstats > /dev/null &
./test/read_tcpstats > /dev/null &
wait

echo "=== Unload ==="
sudo kldunload tcp_stats_kld
test ! -c /dev/tcpstats || (echo "FAIL: device not removed" && exit 1)

echo "=== PASS ==="
```
