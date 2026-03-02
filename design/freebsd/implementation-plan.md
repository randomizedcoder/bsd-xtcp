# FreeBSD `tcp_stats_kld` -- Incremental Implementation Plan

[Back to kernel module design](kernel-module.md)

## Overview

This document is the **step-by-step build plan** for the `tcp_stats_kld` kernel
module designed in [kernel-module.md](kernel-module.md). Each step adds exactly
one capability, includes specific validation commands to run on the FreeBSD VM,
and describes what can go wrong. The goal is continuous progress with early and
frequent correctness checks.

Progress is tracked in [implementation-log.md](implementation-log.md).

---

## Prerequisites

Before Step 1, the FreeBSD VM must have:

| Requirement | How to verify |
|---|---|
| FreeBSD 14.x or 15-CURRENT | `uname -r` |
| SSH access from dev host | `ssh freebsd-vm uname -r` |
| Kernel source tree | `ls /usr/src/sys/netinet/tcp_var.h` |
| `tcp_fill_info` exported | `nm /boot/kernel/kernel \| grep tcp_fill_info` (expect uppercase `T`) |
| `inp_next` exported | `nm /boot/kernel/kernel \| grep inp_next` |
| `cr_canseeinpcb` exported | `nm /boot/kernel/kernel \| grep cr_canseeinpcb` |
| Transfer mechanism | `scp` or `rsync` working to the VM |

If kernel source is missing: `pkg install git && git clone https://git.freebsd.org/src.git /usr/src`

### File layout (final state)

```
kmod/tcp_stats_kld/
    Makefile                # KLD build using bsd.kmod.mk
    tcp_stats_kld.h         # Shared header (kernel + userspace)
    tcp_stats_kld.c         # Module implementation (single file)
    test/
        Makefile            # Test program build
        read_tcpstats.c     # Userspace validation tool
```

---

## Step 1: Bare Module Load/Unload

**Goal:** Simplest possible KLD. Proves the build toolchain, `kldload`, and
`kldunload` work. Zero functionality beyond `printf`.

**Files to create:**

- `kmod/tcp_stats_kld/Makefile`
- `kmod/tcp_stats_kld/tcp_stats_kld.c`

**Implementation:**

`Makefile`:
```makefile
KMOD    = tcp_stats_kld
SRCS    = tcp_stats_kld.c
SYSDIR ?= /usr/src/sys
CFLAGS += -I${.CURDIR}
.include <bsd.kmod.mk>
```

`tcp_stats_kld.c`: Minimal module using `DECLARE_MODULE` with a `modevent`
handler that calls `printf` on `MOD_LOAD` and `MOD_UNLOAD`, returns
`EOPNOTSUPP` for everything else.

**Includes:** `<sys/param.h>`, `<sys/module.h>`, `<sys/kernel.h>`, `<sys/systm.h>`

**Validate:**
```sh
cd /root/bsd-xtcp/kmod/tcp_stats_kld
make clean && make                    # compiles, produces tcp_stats_kld.ko
sudo kldload ./tcp_stats_kld.ko       # no error
dmesg | tail -3                       # "tcp_stats_kld: loaded"
kldstat | grep tcp_stats              # module listed with size
sudo kldunload tcp_stats_kld
dmesg | tail -3                       # "tcp_stats_kld: unloaded"
kldstat | grep tcp_stats              # no output
```

**What can go wrong:**
- `SYSDIR` path wrong -> `make SYSDIR=/usr/src/sys` explicitly
- `DEV_MODULE` needs `<sys/conf.h>` -> use `DECLARE_MODULE` + `moduledata_t` instead for this step
- Kernel/source version mismatch -> `kldload` returns "Exec format error"; ensure source matches `uname -r`

**Risk:** Low

---

## Step 2: Create `/dev/tcpstats` Device Node

**Goal:** On load, create a character device at `/dev/tcpstats`. On unload,
destroy it. No operations yet.

**Files to modify:** `tcp_stats_kld.c`

**Implementation:**

Add `#include <sys/conf.h>`. Define a minimal `struct cdevsw` with only
`.d_version = D_VERSION` and `.d_name = "tcpstats"`. In `MOD_LOAD`, call
`make_dev_credf(MAKEDEV_ETERNAL_KLD, &tcpstats_cdevsw, 0, NULL, UID_ROOT,
GID_WHEEL, 0444, "tcpstats")`. In `MOD_UNLOAD`, call `destroy_dev()`.

Use `GID_WHEEL` and `0444` initially for easy testing (tightened in Step 13).

**Validate:**
```sh
make clean && make
sudo kldload ./tcp_stats_kld.ko
ls -la /dev/tcpstats                  # crw-r--r--  1 root  wheel  ...
cat /dev/tcpstats                     # "Operation not supported" (no d_read)
sudo kldunload tcp_stats_kld
ls /dev/tcpstats 2>&1                 # "No such file or directory"
```

**What can go wrong:**
- `make_dev_credf` returns NULL -> check `dmesg`; usually means `D_VERSION` mismatch
- `MAKEDEV_ETERNAL_KLD` undefined on older FreeBSD -> use `0` instead
- Device node persists after unload -> `destroy_dev()` was not called; check unload path

**Risk:** Low

---

## Step 3: Shared Header (`tcp_stats_kld.h`)

**Goal:** Define the ABI contract: `struct tcp_stats_record` (320 bytes,
packed), ioctl commands, constants. Compile-time `_Static_assert` enforces
size.

**Files to create:** `tcp_stats_kld.h`

**Implementation:**

Full header from [kernel-module.md Section 3, Phase 1](kernel-module.md#phase-1-shared-header-tcp_stats_kldh):
- `TCP_STATS_VERSION`, `TCP_STATS_RECORD_SIZE`, flag defines
- `struct tcp_stats_record` with `__attribute__((packed, aligned(8)))`
- `_Static_assert(sizeof(struct tcp_stats_record) == 320, ...)`
- `struct tcpstats_version`, `struct tcpstats_filter`
- `TCPSTATS_VERSION_CMD`, `TCPSTATS_SET_FILTER`, `TCPSTATS_RESET` ioctl macros
- `#ifdef _KERNEL` guard for kernel-only includes

Add `#include "tcp_stats_kld.h"` to `tcp_stats_kld.c`.

**Validate:**
```sh
make clean && make                                       # _Static_assert passes
cc -fsyntax-only -I/usr/include tcp_stats_kld.h          # valid in userspace
```

**What can go wrong:**
- `_Static_assert` fires with size != 320 -> count bytes manually, adjust `_tsr_spare` array
- `struct in6_addr` not found in kernel context -> ensure `<netinet/in.h>` is included
  (unconditionally, or via the `.c` file before the header)

**Risk:** Low (compile-time only)

---

## Step 4: `open()` / `close()` with Per-FD State

**Goal:** Allocate per-fd state on open, free on close. Reject write opens.
Validates `devfs_set_cdevpriv()` destructor pattern.

**Files to modify:** `tcp_stats_kld.c`

**Implementation:**

New includes: `<sys/malloc.h>`, `<sys/proc.h>`, `<sys/ucred.h>`

Define `MALLOC_DEFINE(M_TCPSTATS, ...)` and `struct tcpstats_softc` with
`sc_cred`, `sc_gen`, `sc_started`, `sc_done`, `sc_filter`.

`tcpstats_open()`: reject `FWRITE` with `EPERM`, `malloc` softc with
`M_WAITOK | M_ZERO`, `crhold(td->td_ucred)`, set default filter
(`state_mask = 0xFFFF`), `devfs_set_cdevpriv(sc, tcpstats_dtor)`.

`tcpstats_dtor()`: `crfree(sc->sc_cred)`, `free(sc, M_TCPSTATS)`.

Add `.d_open = tcpstats_open` to `cdevsw`.

**Validate:**
```sh
make clean && make
sudo kldload ./tcp_stats_kld.ko
cat /dev/tcpstats                     # no crash (no d_read yet, returns error)
echo test > /dev/tcpstats             # "Permission denied"
vmstat -m | grep tcpstats             # after close: InUse=0 (no leak)
sudo kldunload tcp_stats_kld
```

**What can go wrong:**
- `FWRITE` not defined -> add `<sys/fcntl.h>`
- `devfs_set_cdevpriv` not found -> on modern FreeBSD (14+), available via `<sys/conf.h>`
- Credential leak if destructor not called -> verify with `vmstat -m`

**Risk:** Low

---

## Step 5: `read()` with Dummy Records

**Goal:** Implement `tcpstats_read()` returning 3 hardcoded records via
`uiomove()`. Validates the record size, the read loop, and EOF semantics
(second read returns 0 bytes) without touching kernel networking state.

**Files to modify:** `tcp_stats_kld.c`

**Implementation:**

Add `#include <sys/uio.h>`. Implement `tcpstats_read()`: get softc via
`devfs_get_cdevpriv()`, check `sc_done`, loop 3 times emitting `bzero`'d
records with `tsr_version`, `tsr_len`, `tsr_af`, `tsr_local_port` set.
Set `sc_done = 1` after loop. Add `.d_read = tcpstats_read` to `cdevsw`.

**Validate:**
```sh
make clean && make
sudo kldunload tcp_stats_kld 2>/dev/null; sudo kldload ./tcp_stats_kld.ko
dd if=/dev/tcpstats bs=960 count=1 2>/dev/null | wc -c   # 960 (3 x 320)
hexdump -C /dev/tcpstats | head -5
# First bytes: 01 00 00 00 (version=1)  40 01 00 00 (len=320)
```

**What can go wrong:**
- `uiomove` not found -> include `<sys/uio.h>`
- Second read doesn't return 0 -> check `sc_done` flag is set and checked

**Risk:** Low

---

## Step 6: Real PCB Iteration (Identity Only)

**Goal:** Replace dummy records with actual PCB iteration using
`INP_ALL_ITERATOR` + `inp_next()`. For each inpcb, emit a record with only
AF and ports. Includes credential check via `cr_canseeinpcb()` and generation
counter skip.

**Files to modify:** `tcp_stats_kld.c`

**Implementation:**

New includes: `<sys/socket.h>`, `<sys/socketvar.h>`, `<net/vnet.h>`,
`<netinet/in.h>`, `<netinet/in_pcb.h>`, `<netinet/tcp_var.h>`

Add `struct inpcb_iterator sc_iter` to `tcpstats_softc`.

In `tcpstats_read()`: on first read, snapshot `V_tcbinfo.ipi_gencnt` and
initialize `INP_ALL_ITERATOR(&V_tcbinfo, INPLOOKUP_RLOCKPCB)`. Loop with
`inp_next()`, skip if `inp->inp_gencnt > sc_gen`, skip if
`cr_canseeinpcb() != 0`. Populate AF + ports via `ntohs(inp->inp_inc.inc_lport)`.
On `uiomove` error only: `INP_RUNLOCK(inp)` (early exit with lock held).

**Critical locking note:** `inp_next()` auto-unlocks the previous inpcb and
locks the next one. Manual `INP_RUNLOCK` is needed only on early loop exit
(uiomove error).

**Validate:**
```sh
make clean && make
sudo kldunload tcp_stats_kld 2>/dev/null; sudo kldload ./tcp_stats_kld.ko
dd if=/dev/tcpstats bs=65536 2>/dev/null | wc -c      # N * 320
sockstat -4 -6 -P tcp | tail -n +2 | wc -l             # ~same count
# Stability test:
for i in $(seq 1 20); do dd if=/dev/tcpstats of=/dev/null bs=65536 2>/dev/null; done
echo "20 iterations OK"
```

**What can go wrong:**
- `V_tcbinfo` undefined -> needs `<netinet/tcp_var.h>` + `<net/vnet.h>`
- `INP_ALL_ITERATOR` macro missing -> try `INP_ITERATOR(&V_tcbinfo, INPLOOKUP_RLOCKPCB, NULL, NULL)`
- `inp_next` symbol not found at load time -> check `nm /boot/kernel/kernel | grep inp_next`
- Kernel panic -> have VM snapshot ready; verify locking flags are correct

**Risk:** **Medium** -- first kernel data access

---

## Step 7: Full Connection Identity Fields

**Goal:** Populate IPv4/IPv6 addresses, TCP state, socket metadata (uid,
so_addr, inp_gencnt). First touch of `tcpcb` via `intotcpcb()`.

**Files to modify:** `tcp_stats_kld.c`

**Implementation:**

New includes: `<netinet/tcp.h>`, `<netinet/tcp_fsm.h>`

New function `tcpstats_fill_identity()`: set addresses from `inp->inp_inc`
(IPv4: `inc_laddr`/`inc_faddr`, IPv6: `inc6_laddr`/`inc6_faddr`), state from
`intotcpcb(inp)->t_state`, flags (`TSR_F_IPV6`, `TSR_F_LISTEN`), uid from
`inp->inp_socket->so_cred->cr_uid`, `so_addr`, `inp_gencnt`.

Guard `intotcpcb()` and `inp->inp_socket` for NULL.

**Validate:**
```sh
# Python or C: decode records, print addresses/ports/state
# Verify SSH connection visible (port 22, state=4 ESTABLISHED)
# Cross-check: sockstat -4 -P tcp -c
```

**What can go wrong:**
- `intotcpcb(inp)` returns NULL -> guarded
- `inp->inp_socket` is NULL (teardown race) -> guarded
- Byte order: ports stored in network order in inpcb, converted with `ntohs()`
- Addresses stored as-is (binary network order), correct for `inet_ntop` in userspace

**Risk:** Medium

---

## Step 8: `tcp_fill_info()` -- RTT, Sequences, Windows

**Goal:** Call `tcp_fill_info(tp, &ti)` for each non-LISTEN socket. Populate
RTT (already in usec), rttvar, RTO, rttmin, window scale, options, sequence
numbers, cwnd, ssthresh, snd_wnd, rcv_wnd, maxseg.

This is the **key differentiator** -- data only available via `getsockopt(TCP_INFO)`
on owned sockets.

**Files to modify:** `tcp_stats_kld.c`

**Implementation:**

New function `tcpstats_fill_record()` wrapping `tcpstats_fill_identity()` plus:
- Call `tcp_fill_info(tp, &ti)` (requires inpcb locked -- `inp_next` provides this)
- Skip for LISTEN sockets (`tp->t_state == TCPS_LISTEN`)
- Copy RTT fields from `struct tcp_info` (values already in microseconds)
- Copy congestion fields from `tcpcb` directly (`tp->snd_cwnd`, etc.)

**Validate:**
```sh
# Read records, find ESTABLISHED connection
# Verify non-zero RTT for SSH session (expect 1000-50000us on LAN)
# Generate traffic: fetch -o /dev/null http://example.com/ then re-read
```

**What can go wrong:**
- **`tcp_fill_info` not exported** (biggest risk): `nm /boot/kernel/kernel | grep tcp_fill_info`
  must show uppercase `T`. If lowercase `t` (static), must extract RTT from tcpcb directly
  (`tp->t_srtt * tick >> TCP_RTT_SHIFT`)
- **Lock assertion panic**: `tcp_fill_info` calls `INP_LOCK_ASSERT()`. Ensure
  `INPLOOKUP_RLOCKPCB` was passed to the iterator
- Field names differ between versions -> `grep` the source tree for correct names
- `struct tcp_info` field names -> check `/usr/src/sys/netinet/tcp.h`

**Risk:** **High** -- biggest single risk of the project

---

## Step 9: Complete Record Population

**Goal:** Populate all remaining fields: retransmission counters, ECN, DSACK,
TLP, timer values, buffer utilization, CC algo name, TCP stack name.

**Files to modify:** `tcp_stats_kld.c`

**Implementation:**

New include: `<netinet/cc/cc.h>`

Extend `tcpstats_fill_record()` with:

- **Counters:** `tp->t_sndrexmitpack`, `t_rcvoopack`, `t_sndzerowin`, `t_dupacks`, `rcv_numsacks`
- **ECN:** `ti.tcpi_ecn`, `ti.tcpi_delivered_ce`, `ti.tcpi_received_ce`
- **DSACK:** `tp->t_dsack_bytes`, `tp->t_dsack_pack`
- **TLP:** `ti.tcpi_total_tlp`, `ti.tcpi_total_tlp_bytes`
- **Timers:** `getsbinuptime()` + `(tp->t_timers[TT_*] - now) / SBT_1MS`, 0 if `SBT_MAX`
- **Buffers:** `so->so_snd.sb_ccc/sb_hiwat`, `so->so_rcv.sb_ccc/sb_hiwat`
- **Names:** `strlcpy` from `CC_ALGO(tp)->name`, `tp->t_fb->tfb_tcp_block_name`

**Validate:**
```sh
# Full field dump: cc=cubic, stack=freebsd, buffer sizes, timers
# Verify fields change after generating retransmissions
```

**What can go wrong:**
- Field names differ between FreeBSD versions -> `grep` each name in `/usr/src/sys/netinet/tcp_var.h`
- `tp->t_timers[]` array format changed in FreeBSD 14 -> check header
- `CC_ALGO` macro missing -> include `<netinet/cc/cc.h>`, or access `tp->t_cc` directly
- `SBT_1MS` missing -> include `<sys/time.h>`
- DSACK/TLP fields may not exist on all versions -> guard with `#ifdef` or set to 0
- Negative timer values (expired) are expected -> `int32_t` handles correctly

**Risk:** Medium (many names to get right, but each is a compile-time fix)

---

## Step 10: Ioctl Support

**Goal:** Implement `tcpstats_ioctl()` with three commands: version query,
filter set, and iteration reset.

**Files to modify:** `tcp_stats_kld.c`

**Implementation:**

`tcpstats_ioctl()`:
- `TCPSTATS_VERSION_CMD` (`_IOR`): return `protocol_version`, `record_size`, `ipi_count` hint
- `TCPSTATS_SET_FILTER` (`_IOW`): copy filter to `sc->sc_filter`
- `TCPSTATS_RESET` (`_IO`): set `sc_started = 0`, `sc_done = 0`

Add `.d_ioctl = tcpstats_ioctl` to `cdevsw`.

Add state filtering in read loop:
```c
if (sc->sc_filter.state_mask != 0xFFFF) {
    struct tcpcb *tp = intotcpcb(inp);
    if (tp != NULL && !(sc->sc_filter.state_mask & (1 << tp->t_state)))
        continue;
}
```

Plus `TSF_EXCLUDE_LISTEN` and `TSF_EXCLUDE_TIMEWAIT` flag checks.

**Validate:**
```sh
# Python: ioctl TCPSTATS_VERSION_CMD -> version=1, record_size=320
# Python: ioctl TCPSTATS_RESET -> second read returns ~same count
# Python: SET_FILTER excluding LISTEN -> verify no LISTEN sockets in output
```

**What can go wrong:**
- Ioctl command number mismatch between header macros and test calculation
- `ENOTTY` from ioctl -> command number wrong; recalculate per `sys/ioccom.h`

**Risk:** Low

---

## Step 11: Userspace Test Program

**Goal:** `test/read_tcpstats.c` -- standalone C program that opens
`/dev/tcpstats`, queries version ioctl, reads all records, and prints
human-readable output.

**Files to create:** `test/read_tcpstats.c`, `test/Makefile`

**Implementation:**

From [kernel-module.md Section 3, Phase 4](kernel-module.md#phase-4-userspace-test-program):
- Open `/dev/tcpstats` with `O_RDONLY`
- `ioctl(fd, TCPSTATS_VERSION_CMD, &ver)` -> print version info
- Loop: `read(fd, &rec, sizeof(rec))` -> `inet_ntop` + print
- Count and print total

Build: `cc -I.. -o read_tcpstats read_tcpstats.c`

**Validate:**
```sh
./read_tcpstats
# version=1 record_size=320 count_hint=15
# [0] 10.0.2.15:22 -> 10.0.2.2:54321  state=4  rtt=1234 us  cwnd=65536
# total: 15 sockets

sudo ./read_tcpstats | tail -1    # all sockets (root)
./read_tcpstats | tail -1          # own sockets only
```

**Risk:** Low

---

## Step 12: Dual Device (`/dev/tcpstats-full`)

**Goal:** Second character device sharing open/close/ioctl but with its own
cdevsw. Both use full 320-byte format for now -- the infrastructure for two
devices is what matters.

**Files to modify:** `tcp_stats_kld.c`

**Implementation:**

Add second `struct cdevsw tcpstats_full_cdevsw` with `.d_name = "tcpstats-full"`.
Second `make_dev_credf()` in `MOD_LOAD`, second `destroy_dev()` in `MOD_UNLOAD`
(with rollback if second create fails). Add `sc_full` flag to softc, set in
`tcpstats_open()` via `dev->si_devsw == &tcpstats_full_cdevsw`.

**Validate:**
```sh
ls -la /dev/tcpstats /dev/tcpstats-full    # both exist
# Read from both, verify same record counts
sudo kldunload tcp_stats_kld
ls /dev/tcpstats /dev/tcpstats-full 2>&1   # both gone
```

**Risk:** Low

---

## Step 13: Security Hardening

**Goal:** Production-appropriate permissions: `0440 root:network`. Add
`MODULE_DEPEND` for kernel version pinning.

**Files to modify:** `tcp_stats_kld.c`

**Implementation:**

Switch `make_dev_credf` calls to `GID_NETWORK` (69) and `0440`. Define
`GID_NETWORK` if not in headers. Add `MODULE_DEPEND(tcp_stats_kld, kernel,
__FreeBSD_version, __FreeBSD_version, __FreeBSD_version)`.

**Validate:**
```sh
ls -la /dev/tcpstats                    # crw-r-----  root  network
su -m nobody -c 'cat /dev/tcpstats'     # Permission denied
sudo ./test/read_tcpstats               # works (root)
```

**What can go wrong:**
- Group `network` doesn't exist -> `pw groupadd network -g 69`

**Risk:** Low

---

## Step 14: Stress Testing

**Goal:** Validate stability under stress. No code changes.

**Tests to run:**

| Test | Command | Expected |
|---|---|---|
| 10 concurrent readers | `for i in $(seq 1 10); do dd if=/dev/tcpstats of=/dev/null bs=65536 &; done; wait` | All complete, no panic |
| 100 rapid open/close | `for i in $(seq 1 100); do dd if=/dev/tcpstats of=/dev/null bs=320 count=1 2>/dev/null; done` | `vmstat -m`: InUse=0 |
| Connection churn | Read continuously while running `nc -w1 -z 8.8.8.8 53` in parallel | No panic |
| Kill -9 mid-read | `./read_tcpstats & sleep 0.1; kill -9 $!` | Destructor cleans up |
| 10 load/unload cycles | `for i in $(seq 1 10); do sudo kldload/kldunload; done` | All succeed |

**Risk:** None (validation only)

---

## Step 15: Performance Baseline

**Goal:** Measure read latency and throughput for future optimization reference.
No code changes.

**Measurements:**

```sh
# Wall-clock read time
time sudo dd if=/dev/tcpstats of=/dev/null bs=65536

# Records per second (Python timing loop)

# DTrace latency histogram (if available)
sudo kldload dtraceall 2>/dev/null
sudo dtrace -n '
  fbt::tcpstats_read:entry { self->ts = timestamp; }
  fbt::tcpstats_read:return /self->ts/ {
    @ns = quantize(timestamp - self->ts); self->ts = 0;
  }
  tick-5s { printa(@ns); exit(0); }
'

# Symbol check for future optimizations
nm /boot/kernel/kernel | grep kern_prefetch
```

**Risk:** None (measurement only)

---

## Summary Table

| Step | Capability | Key API | Risk |
|------|-----------|---------|------|
| 1 | Module load/unload | `DECLARE_MODULE` | Low |
| 2 | `/dev/tcpstats` device node | `make_dev_credf` | Low |
| 3 | Shared header + size assert | `_Static_assert` | Low |
| 4 | Per-fd state open/close | `devfs_set_cdevpriv`, `crhold` | Low |
| 5 | Read returns dummy records | `uiomove` | Low |
| 6 | Real PCB iteration | `INP_ALL_ITERATOR`, `inp_next`, `cr_canseeinpcb` | **Medium** |
| 7 | Connection identity fields | `intotcpcb`, `inp->inp_inc` | Medium |
| 8 | RTT + sequences | `tcp_fill_info` | **High** |
| 9 | All remaining fields | Timers, `CC_ALGO`, socket buffers | Medium |
| 10 | Ioctl interface | `d_ioctl`, state filtering | Low |
| 11 | Userspace test program | `read()`, `ioctl()`, `inet_ntop` | Low |
| 12 | Dual device nodes | Second `make_dev_credf` | Low |
| 13 | Security hardening | `GID_NETWORK`, `MODULE_DEPEND` | Low |
| 14 | Stress testing | (validation only) | None |
| 15 | Performance baseline | (measurement only) | None |

---

## Future Work (After Step 15)

These are deferred to separate implementation phases:

- **Compact 128-byte record format** for `/dev/tcpstats` (see kernel-module.md Section 11.4 Opt 1)
- **Batched `uiomove()`** with 16KB kernel staging buffer (Opt 2)
- **`kern_prefetch()`** for next inpcb -- Netflix HPTS pattern (Opt 3)
- **Selective field population** via `field_mask` ioctl (Opt 5)
- **SMR match callback** for zero-lock port/address pre-filtering (Section 12)
- **Named filter profiles** via sysctl `dev.tcpstats.profiles.*` (Section 12.5)
- **Rust integration** -- `src/platform/freebsd_kld.rs` reader module (Section 7)
