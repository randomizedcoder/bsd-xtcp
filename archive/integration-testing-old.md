# FreeBSD `tcp_stats_kld` -- Integration Testing

[Back to implementation plan](implementation-plan.md) | [Back to implementation log](implementation-log.md)

## Overview

Integration tests using [`tcp-echo`](../../utils/tcp-echo/) to generate known TCP socket load,
then verifying the kernel module (`/dev/tcpstats`) reports them correctly via the enhanced
`read_tcpstats` test program.

---

## Environment

| Property | Value |
|---|---|
| FreeBSD version | 15.0-RELEASE (GENERIC) |
| VM type | libvirt/KVM |
| SSH access | `ssh root@192.168.122.41` |
| Rust version (on VM) | 1.92.0 (via `pkg install rust`) |
| Kernel module | `tcp_stats_kld.ko` (steps 1-15 complete) |
| tcp-echo version | 0.1.0 (built natively on VM) |

---

## Test Tool Enhancements

`kmod/tcp_stats_kld/test/read_tcpstats.c` was enhanced with the following flags:

| Flag | Description |
|---|---|
| `-p PORT` | Userspace port filter -- only count/display records where `tsr_local_port == PORT` or `tsr_remote_port == PORT` |
| `-c` | Count-only mode -- print just the matching record count, no per-socket detail |
| `-a` | Read from `/dev/tcpstats-full` instead of `/dev/tcpstats` |
| `-L` | Exclude LISTEN sockets via `TCPSTATS_SET_FILTER` ioctl with `TSF_EXCLUDE_LISTEN` |

Read buffer increased from 1 MB to 4 MB (~13000 records max).

---

## Test 1: tcp-echo Smoke Test

**Objective**: Verify tcp-echo builds and runs correctly on FreeBSD VM.

```sh
# Build
cd /root/bsd-xtcp && cargo build --release -p tcp-echo

# Run
/root/bsd-xtcp/target/release/tcp-echo server --ports 9001 &
/root/bsd-xtcp/target/release/tcp-echo client --ports 9001 --connections 5 --duration 3
pkill tcp-echo
```

| Field | Value |
|---|---|
| Status | **Pass** |
| Build time | 6.12s (release, first build with dependency download) |
| Connections established | 2 of 5 (ramp rate limited to 1/s over 10s default ramp, 3s duration) |
| Bytes echoed | 3072 written, 3072 read |
| Notes | Default ramp period (10s) means only 2 connections open in 3s. Working as designed. |

---

## Test 2: read_tcpstats Flag Verification (Baseline)

**Objective**: Verify new `-c`, `-p`, `-L` flags work with SSH-only baseline.

```sh
read_tcpstats -c        # total count
read_tcpstats -p 22 -c  # port 22 only
read_tcpstats -L -c     # exclude LISTEN
```

| Measurement | Expected | Actual | Pass? |
|---|---|---|---|
| Total sockets | 4 (2 ESTABLISHED + 2 LISTEN) | 4 | Yes |
| Port 22 filter | 4 (all are SSH) | 4 | Yes |
| Exclude LISTEN | 2 (ESTABLISHED only) | 2 | Yes |

---

## Test 3: Socket Count Visibility (200 Connections)

**Objective**: tcp-echo server on ports 80 and 443, client opens 200 connections (100 per port, round-robin). Verify kernel module reports correct counts.

```sh
# Server
/root/bsd-xtcp/target/release/tcp-echo server --ports 80,443 &

# Client: 200 connections, hold open 60s
/root/bsd-xtcp/target/release/tcp-echo client --ports 80,443 --connections 200 --rate 10000 --duration 60 &

# Wait for ramp-up, then count
sleep 15
read_tcpstats -c
read_tcpstats -p 80 -c
read_tcpstats -p 443 -c
read_tcpstats -L -c
read_tcpstats -a -c    # via /dev/tcpstats-full
```

### Results

| Measurement | Expected | Actual | Pass? |
|---|---|---|---|
| Total (`-c`) | ~406 | 406 | Yes |
| Port 80 (`-p 80 -c`) | ~201 | 201 | Yes |
| Port 443 (`-p 443 -c`) | ~201 | 201 | Yes |
| No LISTEN (`-L -c`) | ~402 | 402 | Yes |
| Full device (`-a -c`) | same as total | 406 | Yes |

### Count Breakdown

| Category | Count |
|---|---|
| tcp-echo connections (200 x 2 sides) | 400 |
| tcp-echo LISTEN sockets (ports 80, 443) | 2 |
| SSH ESTABLISHED (2 sessions) | 2 |
| SSH LISTEN (IPv4 + IPv6) | 2 |
| **Total** | **406** |

### Additivity Check

| Check | Expected | Actual | Pass? |
|---|---|---|---|
| port80 + port443 + SSH baseline | 201 + 201 + 4 = 406 | 406 | Yes |
| total - no_listen = LISTEN count | 406 - 402 = 4 | 4 | Yes |

---

## Test 4: State Filtering (LISTEN Exclusion)

**Objective**: Verify `TSF_EXCLUDE_LISTEN` ioctl correctly excludes LISTEN sockets under load.

```sh
read_tcpstats -c       # full count
read_tcpstats -L -c    # exclude LISTEN
```

| Measurement | Value |
|---|---|
| Total sockets | 406 |
| Without LISTEN | 402 |
| Difference (LISTEN count) | 4 |
| Expected LISTEN sockets | 4 (2 tcp-echo + 2 sshd) |
| **Pass?** | **Yes** |

---

## Test 5: Buffer Utilization Sanity Check

**Objective**: Verify buffer fields (`tsr_snd_buf_cc`, `tsr_snd_buf_hiwat`, `tsr_rcv_buf_cc`, `tsr_rcv_buf_hiwat`) are populated correctly under tcp-echo traffic.

```sh
/root/bsd-xtcp/target/release/tcp-echo server --ports 80 &
/root/bsd-xtcp/target/release/tcp-echo client --ports 80 --connections 50 --rate 50000 --duration 20 &
sleep 10
read_tcpstats -p 80
```

### Results

| Socket Type | snd_buf_cc | snd_buf_hiwat | rcv_buf_cc | rcv_buf_hiwat | Pass? |
|---|---|---|---|---|---|
| ESTABLISHED | 0 (consumed) | 49032 | 0 (consumed) | 81720 | Yes |
| LISTEN | 0 | 0 | 0 | 0 | Yes |

**Notes**: On loopback, tcp-echo consumes data faster than it accumulates, so `cc` (current content) is typically 0 at sampling time. The non-zero `hiwat` (high water mark) values confirm the buffer fields are correctly populated. LISTEN sockets correctly show 0/0 for all buffer fields.

---

## Memory Leak Check

```sh
pkill tcp-echo; sleep 2; vmstat -m | grep tcpstats
```

| Field | Value |
|---|---|
| InUse | 0 |
| Requests | 19 |
| Allocation size | 64 bytes |
| **Leak?** | **No** |

---

## Summary

| Test | Result |
|---|---|
| tcp-echo smoke test | **Pass** |
| read_tcpstats flag verification | **Pass** |
| Socket count visibility (200 connections) | **Pass** |
| Port filter additivity | **Pass** |
| State filtering (LISTEN exclusion) | **Pass** |
| Buffer utilization sanity | **Pass** |
| Memory leak check | **Pass** |

All tests passed. The kernel module correctly enumerates TCP sockets, supports state-based filtering, and port filtering works correctly in userspace.

---

## Future Work

Items needed for complete filter testing (see also [filter-parsing.md](filter-parsing.md)):

1. **Kernel-side port filtering**: Implement port filter ioctl in the kernel module to avoid reading all records for port-specific queries. Currently port matching is done in userspace after reading all records.

2. **Port filter ioctl**: Add a new ioctl command (e.g., `TCPSTATS_SET_PORT_FILTER`) that accepts a port number and filters at the kernel level during PCB iteration.

3. **Performance comparison**: Re-run tests 3b/3c with kernel-side filtering and measure the reduction in data transferred through `/dev/tcpstats`.

4. **Non-loopback buffer test**: Test buffer utilization over a real network link where `cc` values are more likely to be non-zero at sampling time.

5. **Scale testing**: Test with 1000+ connections to verify the 4 MB read buffer and kernel iteration performance at higher socket counts.
