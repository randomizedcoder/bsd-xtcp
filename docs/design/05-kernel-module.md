[← Back to README](../../README.md)

# Kernel Module: `tcpstats` — System-Wide `tcp_info` Without File Descriptors

## Table of Contents

- [11.1 The Problem](#111-the-problem)
- [11.2 Design Principles](#112-design-principles)
- [11.3 Interface: `/dev/tcpstats` Character Device](#113-interface-devtcpstats-character-device)
- [11.4 Wire Protocol](#114-wire-protocol)
- [11.5 Read Semantics](#115-read-semantics)
- [11.6 Ioctl Interface](#116-ioctl-interface)
- [11.7 Security Architecture](#117-security-architecture)
- [11.8 Threat Model](#118-threat-model)
- [11.9 Kernel Module Implementation](#119-kernel-module-implementation)
- [11.10 Read Path Implementation](#1110-read-path-implementation)
- [11.11 Module Lifecycle](#1111-module-lifecycle)
- [11.12 Build System](#1112-build-system)
- [11.13 Userspace Consumer Pattern](#1113-userspace-consumer-pattern)
- [11.14 Impact on Architecture](#1114-impact-on-architecture)

---

## 11.1 The Problem

The single biggest gap in the sysctl-only approach is that **RTT, RTO, rttvar, rttmin, sequence numbers, window scale, and SACK state are only available via `getsockopt(TCP_INFO)`**, which requires an open file descriptor to the target socket. Userspace cannot obtain an FD to another process's socket. This means the most important diagnostic field — round-trip time — is unavailable for system-wide monitoring.

The existing `tcp_pcblist` sysctl already iterates every TCP connection in kernel space, holding a read lock on each `inpcb`. The function `tcp_fill_info()` (which populates the full `tcp_info` struct) requires exactly the same lock. The data is right there — the existing code simply doesn't call the function.

A kernel loadable module (KLD) that iterates the same PCB list but calls `tcp_fill_info()` for each connection closes this gap completely: every field that `getsockopt(TCP_INFO)` provides, for every socket on the system, in a single kernel pass.

## 11.2 Design Principles

1. **Read-only.** The module never modifies kernel state. It reads TCP control blocks and copies sanitized data to userspace. No socket options are set, no routes are changed, no timers are modified.

2. **Strict data filtering.** The module exposes a character device (`/dev/tcpstats`) with a narrow protocol. The only operation is "return all TCP socket statistics." No arbitrary memory reads, no file descriptor passing, no kernel address disclosure beyond what existing sysctls already expose (e.g., `xso_so` in `xtcpcb`). The output struct is a fixed-layout, fully-defined record with no pointers and no variable-length fields that could leak kernel memory.

3. **Credential enforcement.** Every read applies the same `cr_canseeinpcb()` check that the existing `tcp_pcblist` sysctl uses. Unprivileged users see only their own sockets. Root sees all sockets. Jail-scoped users see only jail-local sockets. MAC policies are honored.

4. **No new attack surface beyond existing sysctls.** The data returned is a strict superset of what `net.inet.tcp.pcblist` already returns (which is world-readable), plus the `tcp_info` fields that `getsockopt(TCP_INFO)` returns (which requires only an FD to the socket, not any privilege). The module does not grant access to data that wasn't already accessible — it makes *existing* data accessible *more efficiently*.

5. **Minimal kernel footprint.** No new kernel threads, no persistent allocations, no callbacks. The module registers a device and a sysctl; all work happens synchronously in the read path.

## 11.3 Interface: `/dev/tcpstats` Character Device

**Why a character device instead of a sysctl?**

The sysctl interface has a structural limitation: the kernel must serialize the entire response into a contiguous buffer before returning it to userspace. For `tcp_pcblist`, this means allocating `N * sizeof(xtcpcb)` bytes in kernel memory. With the addition of `tcp_info` per socket, each record grows significantly. At 1,000 sockets, this is ~500 KB of kernel allocation per read — acceptable, but wasteful.

A character device with `d_read` uses `uiomove()` to stream records directly to the userspace buffer one at a time. The kernel never allocates more than one record's worth of memory. This is more efficient for large socket counts and eliminates the risk of `ENOMEM` on busy systems.

The device also provides a clean ioctl interface for configuration (filtering, field selection) without overloading sysctl semantics.

**Device properties:**

| Property | Value |
|---|---|
| Path | `/dev/tcpstats` |
| Owner | `root:wheel` |
| Permissions | `0444` (world-readable; credential filtering handles visibility) |
| Operations | `open`, `read`, `ioctl`, `close` |
| Write | Not supported (`EOPNOTSUPP`) |
| Mmap | Not supported |
| Concurrent reads | Supported (each open FD gets independent iteration state) |

## 11.4 Wire Protocol

Each `read()` call returns zero or more complete records. The module never returns a partial record — if the userspace buffer is too small for even one record, `read()` returns `EINVAL`. This simplifies userspace parsing (no reassembly needed).

**Record format: `struct tcp_stats_record`**

```c
/*
 * Fixed-size record emitted by /dev/tcpstats for each TCP connection.
 * No pointers. No variable-length fields. Fully defined layout.
 * All padding bytes are zeroed before copyout to prevent kernel memory leaks.
 */
struct tcp_stats_record {
    /* Record header */
    uint32_t    tsr_version;        /* Protocol version (1) */
    uint32_t    tsr_len;            /* sizeof(struct tcp_stats_record) */
    uint32_t    tsr_flags;          /* Record flags (TSR_F_*) */
    uint32_t    _tsr_pad0;

    /* Connection identity (from xinpcb) */
    uint8_t     tsr_af;             /* AF_INET (2) or AF_INET6 (28) */
    uint8_t     _tsr_pad1[3];
    uint16_t    tsr_local_port;     /* Local port (host byte order) */
    uint16_t    tsr_remote_port;    /* Remote port (host byte order) */
    union {
        struct in_addr   v4;
        struct in6_addr  v6;
    }           tsr_local_addr;     /* Local IP */
    union {
        struct in_addr   v4;
        struct in6_addr  v6;
    }           tsr_remote_addr;    /* Remote IP */

    /* TCP state (from xtcpcb) */
    int32_t     tsr_state;          /* TCP FSM state (TCPS_*) */
    uint32_t    tsr_flags_tcp;      /* TCP flags (TF_*) */

    /* Congestion control (from xtcpcb + tcp_info) */
    uint32_t    tsr_snd_cwnd;       /* Congestion window (bytes) */
    uint32_t    tsr_snd_ssthresh;   /* Slow start threshold (bytes) */
    uint32_t    tsr_snd_wnd;        /* Send window (bytes) */
    uint32_t    tsr_rcv_wnd;        /* Receive window (bytes) */
    uint32_t    tsr_maxseg;         /* Maximum segment size */
    char        tsr_cc[16];         /* CC algorithm name, NUL-terminated */
    char        tsr_stack[16];      /* TCP stack name, NUL-terminated */

    /* === Fields from tcp_fill_info() — the whole reason this module exists === */
    uint32_t    tsr_rtt;            /* Smoothed RTT (usec) */
    uint32_t    tsr_rttvar;         /* RTT variance (usec) */
    uint32_t    tsr_rto;            /* Retransmission timeout (usec) */
    uint32_t    tsr_rttmin;         /* Minimum observed RTT (usec) */
    uint8_t     tsr_snd_wscale;     /* Send window scale factor */
    uint8_t     tsr_rcv_wscale;     /* Receive window scale factor */
    uint8_t     tsr_options;        /* Negotiated options (TCPI_OPT_*) */
    uint8_t     _tsr_pad2;

    /* Sequence numbers (from tcp_info) */
    uint32_t    tsr_snd_nxt;        /* Next send sequence number */
    uint32_t    tsr_snd_una;        /* Unacknowledged send sequence */
    uint32_t    tsr_snd_max;        /* Highest sequence number sent */
    uint32_t    tsr_rcv_nxt;        /* Next receive sequence number */
    uint32_t    tsr_rcv_adv;        /* Peer advertised window */

    /* Counters (from xtcpcb) */
    uint32_t    tsr_snd_rexmitpack; /* Retransmitted packets (cumulative) */
    uint32_t    tsr_rcv_ooopack;    /* Out-of-order packets (cumulative) */
    uint32_t    tsr_snd_zerowin;    /* Zero-window probes sent */
    uint32_t    tsr_dupacks;        /* Consecutive duplicate ACKs */
    uint32_t    tsr_rcv_numsacks;   /* Distinct SACK blocks received */

    /* ECN (from xtcpcb + tcp_info) */
    uint32_t    tsr_ecn;            /* ECN flags */
    uint32_t    tsr_delivered_ce;   /* CE marks delivered */
    uint32_t    tsr_received_ce;    /* CE marks received */

    /* DSACK (from xtcpcb) */
    uint32_t    tsr_dsack_bytes;    /* DSACK bytes received */
    uint32_t    tsr_dsack_pack;     /* DSACK packets received */

    /* Tail loss probes (from tcp_info) */
    uint32_t    tsr_total_tlp;      /* TLP probes sent */
    uint64_t    tsr_total_tlp_bytes;/* TLP bytes sent */

    /* Timers (from xtcpcb, milliseconds, 0 = not running) */
    int32_t     tsr_tt_rexmt;       /* Retransmit timer */
    int32_t     tsr_tt_persist;     /* Persist timer */
    int32_t     tsr_tt_keep;        /* Keepalive timer */
    int32_t     tsr_tt_2msl;        /* 2MSL timer */
    int32_t     tsr_tt_delack;      /* Delayed ACK timer */
    int32_t     tsr_rcvtime;        /* Time since last data received (ms) */

    /* Buffer utilization (from xsocket) */
    uint32_t    tsr_snd_buf_cc;     /* Send buffer bytes in use */
    uint32_t    tsr_snd_buf_hiwat;  /* Send buffer high watermark */
    uint32_t    tsr_rcv_buf_cc;     /* Recv buffer bytes in use */
    uint32_t    tsr_rcv_buf_hiwat;  /* Recv buffer high watermark */

    /* Socket metadata (from xsocket — for process mapping join key) */
    uint64_t    tsr_so_addr;        /* Kernel socket address (join key) */
    uid_t       tsr_uid;            /* Socket owner UID */

    /* Generation tracking */
    uint64_t    tsr_inp_gencnt;     /* inpcb generation count */

    uint32_t    _tsr_spare[8];      /* Future expansion, zeroed */
} __attribute__((packed, aligned(8)));
```

**Record size:** 320 bytes (fixed). At 1,000 sockets, one full read is 320 KB.

**Flags (`tsr_flags`):**

```c
#define TSR_F_IPV6          0x00000001  /* IPv6 connection */
#define TSR_F_LISTEN        0x00000002  /* Listening socket (no tcp_info) */
#define TSR_F_SYNCACHE      0x00000004  /* SYN_RECEIVED via syncache */
#define TSR_F_CREDENTIAL    0x00000008  /* Credential check filtered some fields */
```

## 11.5 Read Semantics

```
open("/dev/tcpstats", O_RDONLY)
  → Allocates per-fd state: generation snapshot, credential cache
  → Returns fd

read(fd, buf, bufsize)
  → First read after open (or after lseek to 0):
      1. Read generation count from V_tcbinfo.ipi_gencnt
      2. Begin iteration via INP_ALL_ITERATOR(&V_tcbinfo, INPLOOKUP_RLOCKPCB)
      3. For each inpcb:
         a. Check inp->inp_gencnt <= saved generation (skip new connections)
         b. Check cr_canseeinpcb(td->td_ucred, inp) == 0 (skip invisible)
         c. Lock tcpcb, call tcp_fill_info() to populate tcp_info
         d. Read xtcpcb-equivalent fields from tcpcb and xsocket
         e. bzero a tcp_stats_record, populate all fields
         f. uiomove() the record to userspace
         g. If userspace buffer is full, save iterator position, return
      4. When iteration completes, return 0 (EOF)
  → Subsequent reads continue from saved position

close(fd)
  → Frees per-fd state
```

**Atomicity guarantee:** Each `read()` call returns only complete records. The module tracks how many bytes of the current record have been transferred; if `uiomove()` would split a record across two `read()` calls, it stops before that record and returns what it has. The userspace tool issues reads with buffers that are multiples of `tsr_len`.

**Consistency guarantee:** The generation count snapshot taken at the first `read()` ensures a consistent view — connections created during iteration are skipped (same as `tcp_pcblist`). The userspace tool detects this by comparing the total record count with `V_tcbinfo.ipi_count` and retries if significantly divergent.

## 11.6 Ioctl Interface

```c
/* Get module version and record size */
#define TCPSTATS_VERSION    _IOR('T', 1, struct tcpstats_version)

struct tcpstats_version {
    uint32_t    protocol_version;   /* Wire protocol version */
    uint32_t    record_size;        /* sizeof(tcp_stats_record) */
    uint32_t    record_count_hint;  /* Approximate current socket count */
    uint32_t    flags;              /* Module capability flags */
};

/* Set state filter (only return sockets in these states) */
#define TCPSTATS_SET_FILTER _IOW('T', 2, struct tcpstats_filter)

struct tcpstats_filter {
    uint16_t    state_mask;         /* Bitmask of TCPS_* states to include */
                                    /* 0xFFFF = all states (default) */
    uint16_t    _pad;
    uint32_t    flags;              /* Filter flags */
};
#define TSF_EXCLUDE_LISTEN  0x01    /* Skip LISTEN sockets (no meaningful tcp_info) */
#define TSF_EXCLUDE_TIMEWAIT 0x02   /* Skip TIME_WAIT (usually noise) */

/* Reset iteration (seek to beginning for next read) */
#define TCPSTATS_RESET      _IO('T', 3)
```

The ioctl interface is deliberately minimal. Filtering reduces the data volume for common cases (excluding LISTEN and TIME_WAIT sockets, which have no meaningful RTT data), but the default is to return everything.

## 11.7 Security Architecture

The security design has five layers, each independently sufficient to prevent data leakage:

**Layer 1: Device permissions**

```c
tcpstats_dev = make_dev(&tcpstats_cdevsw, 0,
    UID_ROOT, GID_WHEEL, 0444, "tcpstats");
```

The device is world-readable (`0444`). This is intentional — the credential filtering in Layer 3 handles per-user visibility. Making the device root-only would prevent the unprivileged use case (developer seeing their own sockets). The administrator can restrict this further via `devfs.rules` if desired.

**Layer 2: Open-time validation**

```c
static int
tcpstats_open(struct cdev *dev, int oflags, int devtype, struct thread *td)
{
    /* Reject write opens */
    if (oflags & FWRITE)
        return (EPERM);

    /* Allocate per-fd state, cache credential reference */
    struct tcpstats_softc *sc = malloc(sizeof(*sc), M_TCPSTATS, M_WAITOK | M_ZERO);
    sc->sc_cred = crhold(td->td_ucred);     /* Reference caller's credential */
    devfs_set_cdevpriv(sc, tcpstats_close_private);
    return (0);
}
```

Write opens are rejected. The caller's credential is captured at open time and used for all subsequent reads (preventing credential changes between open and read from affecting visibility).

**Layer 3: Per-socket credential filtering**

Every socket is checked against the opener's credential before its data is emitted:

```c
/* Inside the iteration loop */
if (cr_canseeinpcb(sc->sc_cred, inp) != 0) {
    /* Caller cannot see this socket — skip silently */
    continue;
}
```

This enforces:
- **UID isolation:** Non-root users see only sockets owned by their UID (controlled by `security.bsd.see_other_uids`)
- **GID isolation:** Controlled by `security.bsd.see_other_gids`
- **Jail scoping:** Jail inmates see only sockets within their jail (`prison_check()`)
- **MAC policy:** If MAC is compiled in, `mac_inpcb_check_visible()` is called, honoring Biba, MLS, LOMAC, or any custom policy

**Layer 4: Output sanitization**

The `tcp_stats_record` struct is `bzero()`'d before any field is populated:

```c
struct tcp_stats_record rec;
bzero(&rec, sizeof(rec));    /* Zero ALL bytes including padding */
rec.tsr_version = 1;
rec.tsr_len = sizeof(rec);
/* ... populate fields ... */
```

This prevents kernel heap data from leaking through padding bytes or uninitialized fields. The struct uses `__attribute__((packed, aligned(8)))` to eliminate compiler-inserted padding, and the `_tsr_spare` array is explicitly zeroed.

**No kernel pointers in output.** The `tsr_so_addr` field (socket kernel address) is the same value already exposed by the existing `xsocket.xso_so` in the world-readable `tcp_pcblist` sysctl. It is needed as a join key for process mapping. If KASLR/pointer sanitization is a concern, this field can be replaced with a hash or omitted via an ioctl flag. On stock FreeBSD, this address is already public via `sockstat -v`.

**Layer 5: No write path**

The module has no `d_write`, no `d_ioctl` that modifies kernel state, and no `d_mmap`. There is no mechanism to inject data into the kernel through this module. The ioctl commands use `_IOR` (read from kernel) and `_IOW` (write to kernel) correctly — the "write" ioctls (`TCPSTATS_SET_FILTER`, `TCPSTATS_RESET`) only modify the per-fd softc state, never kernel networking state.

## 11.8 Threat Model

| Threat | Mitigation |
|---|---|
| Unprivileged user reads other users' socket data | `cr_canseeinpcb()` enforces UID/GID/jail/MAC visibility per socket |
| Kernel memory leak through padding bytes | `bzero()` on every record before population; packed struct eliminates compiler padding |
| Kernel pointer disclosure enabling KASLR bypass | `tsr_so_addr` matches existing `xso_so` in world-readable `tcp_pcblist` sysctl; can be hashed or omitted |
| Module used to scan for listening services | LISTEN sockets have no meaningful `tcp_info`; filter flag `TSF_EXCLUDE_LISTEN` available; same data already in `tcp_pcblist` |
| Race condition between credential check and data read | Credential cached at `open()` time; inpcb read-locked during field extraction; generation count prevents stale data |
| Denial of service via repeated reads | No kernel allocations per-read beyond one `tcp_stats_record` on stack; iteration is O(N) in socket count, same as existing `tcp_pcblist` |
| Module used to bypass jail isolation | `prison_check()` inside `cr_canseeinpcb()` enforces jail scoping; module inherits existing jail security model |
| Attacker loads malicious module masquerading as `tcpstats` | Module loading requires root + `securelevel < 1`; standard KLD security model |
| Leaked sequence numbers enable TCP injection | Sequence numbers are already exposed via `getsockopt(TCP_INFO)` on owned sockets; `tcp_pcblist` does not expose them but root can access via kvm; risk is equivalent to existing attack surface |

## 11.9 Kernel Module Implementation

```c
/* tcp_statsdev.c — FreeBSD kernel module for system-wide TCP statistics */

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

MALLOC_DEFINE(M_TCPSTATS, "tcpstats", "TCP stats kernel module");

/* --- Per-open-fd state --- */
struct tcpstats_softc {
    struct ucred        *sc_cred;       /* Cached opener credential */
    uint64_t            sc_gen;         /* Generation snapshot */
    struct inpcb_iterator sc_iter;      /* PCB iterator state */
    struct tcpstats_filter sc_filter;   /* Active filter */
    int                 sc_started;     /* Iteration in progress? */
    int                 sc_done;        /* Iteration complete? */
};

/* --- Record population (the core logic) --- */

/*
 * Populate a tcp_stats_record from an inpcb + tcpcb.
 *
 * Called with inp read-locked.  Merges fields from three sources:
 *   1. xtcpcb-equivalent fields (timers, CC name, stack name, DSACK)
 *   2. tcp_info fields via tcp_fill_info() (RTT, RTO, sequences, options)
 *   3. xsocket fields (buffer utilization, UID, kernel socket address)
 *
 * The record is bzero'd before entry.  No kernel pointers are copied
 * except tsr_so_addr (which matches existing xso_so in tcp_pcblist).
 */
static void
tcpstats_fill_record(struct tcp_stats_record *rec, struct inpcb *inp)
{
    struct tcpcb *tp = intotcpcb(inp);
    struct socket *so = inp->inp_socket;
    struct tcp_info ti;

    /* --- tcp_info fields (RTT, RTO, sequences, options) --- */
    tcp_fill_info(tp, &ti);

    rec->tsr_version = 1;
    rec->tsr_len = sizeof(*rec);

    /* Connection identity */
    if (inp->inp_vflag & INP_IPV6) {
        rec->tsr_af = AF_INET6;
        rec->tsr_flags |= TSR_F_IPV6;
        rec->tsr_local_addr.v6 = inp->in6p_laddr;
        rec->tsr_remote_addr.v6 = inp->in6p_faddr;
    } else {
        rec->tsr_af = AF_INET;
        rec->tsr_local_addr.v4 = inp->inp_laddr;
        rec->tsr_remote_addr.v4 = inp->inp_faddr;
    }
    rec->tsr_local_port = ntohs(inp->inp_lport);
    rec->tsr_remote_port = ntohs(inp->inp_fport);

    /* TCP state */
    rec->tsr_state = tp->t_state;
    rec->tsr_flags_tcp = tp->t_flags;

    /* Congestion control — from tcpcb directly */
    rec->tsr_snd_cwnd = tp->snd_cwnd;
    rec->tsr_snd_ssthresh = tp->snd_ssthresh;
    rec->tsr_snd_wnd = tp->snd_wnd;
    rec->tsr_rcv_wnd = tp->rcv_wnd;
    rec->tsr_maxseg = tp->t_maxseg;
    strlcpy(rec->tsr_cc, CC_ALGO(tp)->name, sizeof(rec->tsr_cc));
    strlcpy(rec->tsr_stack, tp->t_fb->tfb_tcp_block_name,
        sizeof(rec->tsr_stack));

    /* RTT and timing — from tcp_fill_info() result */
    rec->tsr_rtt = ti.tcpi_rtt;
    rec->tsr_rttvar = ti.tcpi_rttvar;
    rec->tsr_rto = ti.tcpi_rto;
    rec->tsr_rttmin = ti.tcpi_rttmin;
    rec->tsr_snd_wscale = ti.tcpi_snd_wscale;
    rec->tsr_rcv_wscale = ti.tcpi_rcv_wscale;
    rec->tsr_options = ti.tcpi_options;

    /* Sequence numbers — from tcp_fill_info() */
    rec->tsr_snd_nxt = ti.tcpi_snd_nxt;
    rec->tsr_snd_una = ti.tcpi_snd_una;
    rec->tsr_snd_max = ti.tcpi_snd_max;
    rec->tsr_rcv_nxt = ti.tcpi_rcv_nxt;
    rec->tsr_rcv_adv = ti.tcpi_rcv_adv;

    /* Counters */
    rec->tsr_snd_rexmitpack = tp->t_sndrexmitpack;
    rec->tsr_rcv_ooopack = tp->t_rcvoopack;
    rec->tsr_snd_zerowin = tp->t_sndzerowin;
    rec->tsr_dupacks = ti.tcpi_dupacks;
    rec->tsr_rcv_numsacks = ti.tcpi_rcv_numsacks;

    /* ECN */
    rec->tsr_ecn = (tp->t_flags2 & TF2_ECN_PERMIT) ? 1 : 0;
    rec->tsr_delivered_ce = ti.tcpi_delivered_ce;
    rec->tsr_received_ce = ti.tcpi_received_ce;

    /* DSACK */
    rec->tsr_dsack_bytes = tp->t_dsack_bytes;
    rec->tsr_dsack_pack = tp->t_dsack_pack;

    /* TLP */
    rec->tsr_total_tlp = ti.tcpi_total_tlp;
    rec->tsr_total_tlp_bytes = ti.tcpi_total_tlp_bytes;

    /* Timers — from tcp_inptoxtp() pattern */
    /* (reproduced here because tcp_inptoxtp fills xtcpcb, not our struct) */
    {
        sbintime_t now = getsbinuptime();
        if (tp->t_timers[TT_REXMT] != SBT_MAX)
            rec->tsr_tt_rexmt = (tp->t_timers[TT_REXMT] - now) / SBT_1MS;
        if (tp->t_timers[TT_PERSIST] != SBT_MAX)
            rec->tsr_tt_persist = (tp->t_timers[TT_PERSIST] - now) / SBT_1MS;
        if (tp->t_timers[TT_KEEP] != SBT_MAX)
            rec->tsr_tt_keep = (tp->t_timers[TT_KEEP] - now) / SBT_1MS;
        if (tp->t_timers[TT_2MSL] != SBT_MAX)
            rec->tsr_tt_2msl = (tp->t_timers[TT_2MSL] - now) / SBT_1MS;
        if (tp->t_timers[TT_DELACK] != SBT_MAX)
            rec->tsr_tt_delack = (tp->t_timers[TT_DELACK] - now) / SBT_1MS;
    }
    rec->tsr_rcvtime = 1000 * (ticks - tp->t_rcvtime) / hz;

    /* Buffer utilization — from socket */
    if (so != NULL) {
        rec->tsr_snd_buf_cc = so->so_snd.sb_ccc;
        rec->tsr_snd_buf_hiwat = so->so_snd.sb_hiwat;
        rec->tsr_rcv_buf_cc = so->so_rcv.sb_ccc;
        rec->tsr_rcv_buf_hiwat = so->so_rcv.sb_hiwat;
        rec->tsr_so_addr = (uint64_t)(uintptr_t)so;
        rec->tsr_uid = so->so_cred->cr_uid;
    }

    /* Generation count */
    rec->tsr_inp_gencnt = inp->inp_gencnt;
}
```

## 11.10 Read Path Implementation

```c
static int
tcpstats_read(struct cdev *dev, struct uio *uio, int ioflag)
{
    struct tcpstats_softc *sc;
    struct inpcb *inp;
    struct tcp_stats_record rec;
    int error;

    error = devfs_get_cdevpriv((void **)&sc);
    if (error != 0)
        return (error);

    /* If iteration is complete, return EOF */
    if (sc->sc_done)
        return (0);

    /* Initialize iteration on first read */
    if (!sc->sc_started) {
        sc->sc_gen = V_tcbinfo.ipi_gencnt;
        sc->sc_iter = (struct inpcb_iterator)INP_ALL_ITERATOR(
            &V_tcbinfo, INPLOOKUP_RLOCKPCB);
        sc->sc_started = 1;
    }

    /* Stream records until userspace buffer is full or iteration ends */
    while (uio->uio_resid >= (ssize_t)sizeof(rec)) {
        inp = inp_next(&sc->sc_iter);
        if (inp == NULL) {
            sc->sc_done = 1;
            break;
        }

        /* Generation check: skip connections created after snapshot */
        if (inp->inp_gencnt > sc->sc_gen)
            continue;

        /* Credential check: skip sockets caller cannot see */
        if (cr_canseeinpcb(sc->sc_cred, inp) != 0)
            continue;

        /* State filter */
        if (sc->sc_filter.state_mask != 0xFFFF) {
            struct tcpcb *tp = intotcpcb(inp);
            if (!(sc->sc_filter.state_mask & (1 << tp->t_state)))
                continue;
        }

        /* Populate record */
        bzero(&rec, sizeof(rec));
        tcpstats_fill_record(&rec, inp);

        /* Copy to userspace */
        error = uiomove(&rec, sizeof(rec), uio);
        if (error != 0)
            return (error);
    }

    return (0);
}
```

## 11.11 Module Lifecycle

```c
static struct cdev *tcpstats_dev;

static struct cdevsw tcpstats_cdevsw = {
    .d_version = D_VERSION,
    .d_name    = "tcpstats",
    .d_flags   = 0,            /* No Giant lock needed; inpcb locking is sufficient */
    .d_open    = tcpstats_open,
    .d_close   = tcpstats_close,
    .d_read    = tcpstats_read,
    .d_ioctl   = tcpstats_ioctl,
};

static int
tcpstats_modevent(module_t mod, int type, void *data)
{
    switch (type) {
    case MOD_LOAD:
        tcpstats_dev = make_dev(&tcpstats_cdevsw, 0,
            UID_ROOT, GID_WHEEL, 0444, "tcpstats");
        if (tcpstats_dev == NULL)
            return (ENXIO);
        printf("tcpstats: loaded, /dev/tcpstats available\n");
        return (0);

    case MOD_UNLOAD:
        if (tcpstats_dev != NULL)
            destroy_dev(tcpstats_dev);
        printf("tcpstats: unloaded\n");
        return (0);

    default:
        return (EOPNOTSUPP);
    }
}

DEV_MODULE(tcpstats, tcpstats_modevent, NULL);
MODULE_VERSION(tcpstats, 1);
MODULE_DEPEND(tcpstats, kernel, __FreeBSD_version,
    __FreeBSD_version, __FreeBSD_version);
```

## 11.12 Build System

```makefile
# Makefile for tcpstats kernel module

KMOD=   tcpstats
SRCS=   tcp_statsdev.c

CFLAGS+= -I${SYSDIR}

.include <bsd.kmod.mk>
```

```
# Build:
make -C /path/to/tcpstats SYSDIR=/usr/src/sys

# Load:
kldload ./tcpstats.ko

# Verify:
ls -la /dev/tcpstats
kldstat | grep tcp_stats

# Unload:
kldunload tcpstats
```

## 11.13 Userspace Consumer Pattern

The userspace tool reads from `/dev/tcpstats` instead of calling multiple sysctls:

```rust
// Rust userspace reader (simplified)
use std::fs::File;
use std::io::Read;

const RECORD_SIZE: usize = 320; // sizeof(tcp_stats_record)

fn read_all_sockets() -> Vec<TcpStatsRecord> {
    let mut f = File::open("/dev/tcpstats").expect("open /dev/tcpstats");
    let mut buf = vec![0u8; RECORD_SIZE * 1024]; // Read up to 1024 sockets at a time
    let mut records = Vec::new();

    loop {
        let n = f.read(&mut buf).expect("read");
        if n == 0 { break; } // EOF — iteration complete

        assert!(n % RECORD_SIZE == 0, "partial record");
        for chunk in buf[..n].chunks_exact(RECORD_SIZE) {
            records.push(TcpStatsRecord::from_bytes(chunk));
        }
    }
    records
}
```

**Performance comparison with sysctl approach:**

| Metric | sysctl `tcp_pcblist` (Tier 1 only) | `/dev/tcpstats` (full data) |
|---|---|---|
| RTT data | No | Yes |
| Kernel passes | 1 (pcblist) + 1 (kern.file) | 1 (single iteration) |
| Kernel allocations | N * sizeof(xtcpcb) contiguous | 1 record on stack per iteration |
| Userspace syscalls | 2+ (pcblist + kern.file + proc) | 1 open + K reads + 1 close |
| Per-socket overhead | ~400 bytes (xtcpcb) | ~320 bytes (tcp_stats_record) |
| Process mapping | Requires separate kern.file join | Still requires kern.file (tsr_so_addr is the join key) |

## 11.14 Impact on Architecture

With the kernel module, the tiered polling model from [Section 3.2](02-architecture.md#32-polling-architecture) simplifies:

| Tier | Interval | Data Source |
|---|---|---|
| **Fast** | 1s | `/dev/tcpstats` — full record including RTT |
| **Standard** | 30s | `/dev/tcpstats` + `kern.file` for process mapping |
| **Slow** | 60s | Same + `tcp.states` for state distribution |
| **Aggregate** | 300s | Same + `tcp.stats` for system-wide counters |

The Tier 1/Tier 2 split from [Section 2](01-freebsd-data-sources.md) is eliminated. Every poll, at every interval, gets the full field set. The "~20 fields" limitation cited in the business plan becomes "~35 fields" — still less than Linux's 60, but now including RTT, RTO, rttvar, rttmin, window scale, sequence numbers, SACK state, and TLP counters, which are the fields that matter most for developer-facing TCP profiling.
