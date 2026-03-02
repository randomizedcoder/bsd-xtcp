#!/usr/bin/env python3
"""Decode binary records from /dev/tcpstats.

Usage:
    dd if=/dev/tcpstats of=/tmp/tcpstats.bin bs=65536 2>/dev/null
    python3 decode_tcpstats.py /tmp/tcpstats.bin

Or via pipe (on the VM or after scp):
    dd if=/dev/tcpstats bs=65536 2>/dev/null | python3 decode_tcpstats.py -
"""
import struct
import socket
import sys

RECORD_SIZE = 320

STATES = {
    0: "CLOSED", 1: "LISTEN", 2: "SYN_SENT", 3: "SYN_RCVD",
    4: "ESTABLISHED", 5: "CLOSE_WAIT", 6: "FIN_WAIT_1", 7: "CLOSING",
    8: "LAST_ACK", 9: "FIN_WAIT_2", 10: "TIME_WAIT",
}

# Packed struct offsets (see tcp_stats_kld.h)
OFF_VERSION     = 0
OFF_LEN         = 4
OFF_FLAGS       = 8
OFF_AF          = 16
OFF_LPORT       = 20
OFF_RPORT       = 22
OFF_LADDR       = 24
OFF_RADDR       = 40
OFF_STATE       = 56
OFF_FLAGS_TCP   = 60
OFF_SND_CWND    = 64
OFF_SND_SSTHRESH = 68
OFF_SND_WND     = 72
OFF_RCV_WND     = 76
OFF_MAXSEG      = 80
OFF_CC          = 84
OFF_STACK       = 100
OFF_RTT         = 116
OFF_RTTVAR      = 120
OFF_RTO         = 124
OFF_RTTMIN      = 128
OFF_SND_WSCALE  = 132
OFF_RCV_WSCALE  = 133
OFF_SND_NXT     = 136
OFF_SND_UNA     = 140
OFF_SND_MAX     = 144
OFF_RCV_NXT     = 148
OFF_REXMITPACK  = 156
OFF_OOOPACK     = 160
OFF_ZEROWIN     = 164
OFF_DUPACKS     = 168
OFF_SND_BUF_CC  = 232
OFF_SND_BUF_HI  = 236
OFF_RCV_BUF_CC  = 240
OFF_RCV_BUF_HI  = 244
OFF_SO_ADDR     = 248
OFF_UID         = 256
OFF_GENCNT      = 260


def decode_record(r, idx):
    ver = struct.unpack_from("<I", r, OFF_VERSION)[0]
    rlen = struct.unpack_from("<I", r, OFF_LEN)[0]
    flags = struct.unpack_from("<I", r, OFF_FLAGS)[0]
    af = r[OFF_AF]
    lport = struct.unpack_from("<H", r, OFF_LPORT)[0]
    rport = struct.unpack_from("<H", r, OFF_RPORT)[0]
    state = struct.unpack_from("<i", r, OFF_STATE)[0]
    flags_tcp = struct.unpack_from("<I", r, OFF_FLAGS_TCP)[0]
    cwnd = struct.unpack_from("<I", r, OFF_SND_CWND)[0]
    ssthresh = struct.unpack_from("<I", r, OFF_SND_SSTHRESH)[0]
    snd_wnd = struct.unpack_from("<I", r, OFF_SND_WND)[0]
    rcv_wnd = struct.unpack_from("<I", r, OFF_RCV_WND)[0]
    maxseg = struct.unpack_from("<I", r, OFF_MAXSEG)[0]
    rtt = struct.unpack_from("<I", r, OFF_RTT)[0]
    rttvar = struct.unpack_from("<I", r, OFF_RTTVAR)[0]
    rto = struct.unpack_from("<I", r, OFF_RTO)[0]
    cc = r[OFF_CC:OFF_CC+16].split(b"\x00")[0].decode("ascii", errors="replace")
    stack = r[OFF_STACK:OFF_STACK+16].split(b"\x00")[0].decode("ascii", errors="replace")
    so_addr = struct.unpack_from("<Q", r, OFF_SO_ADDR)[0]
    uid = struct.unpack_from("<I", r, OFF_UID)[0]
    gencnt = struct.unpack_from("<Q", r, OFF_GENCNT)[0]
    snd_buf_cc = struct.unpack_from("<I", r, OFF_SND_BUF_CC)[0]
    snd_buf_hi = struct.unpack_from("<I", r, OFF_SND_BUF_HI)[0]
    rcv_buf_cc = struct.unpack_from("<I", r, OFF_RCV_BUF_CC)[0]
    rcv_buf_hi = struct.unpack_from("<I", r, OFF_RCV_BUF_HI)[0]

    # FreeBSD AF_INET=2, AF_INET6=28; Linux AF_INET6=10
    BSD_AF_INET = 2
    BSD_AF_INET6 = 28
    if af == BSD_AF_INET:
        laddr = socket.inet_ntop(socket.AF_INET, r[OFF_LADDR:OFF_LADDR+4])
        raddr = socket.inet_ntop(socket.AF_INET, r[OFF_RADDR:OFF_RADDR+4])
    elif af == BSD_AF_INET6:
        laddr = socket.inet_ntop(socket.AF_INET6, r[OFF_LADDR:OFF_LADDR+16])
        raddr = socket.inet_ntop(socket.AF_INET6, r[OFF_RADDR:OFF_RADDR+16])
    else:
        laddr = "?"
        raddr = "?"

    sname = STATES.get(state, str(state))
    tag = ""
    if flags & 0x2:
        tag += " LISTEN"
    if flags & 0x1:
        tag += " IPv6"

    print(f"[{idx}] {laddr}:{lport} -> {raddr}:{rport}  "
          f"state={state}({sname}){tag}  uid={uid}  gen={gencnt}")

    if rtt or cwnd or cc:
        print(f"     rtt={rtt}us  rttvar={rttvar}us  rto={rto}us  "
              f"cwnd={cwnd}  ssthresh={ssthresh}  maxseg={maxseg}")
        print(f"     cc={cc!r}  stack={stack!r}  "
              f"snd_wnd={snd_wnd}  rcv_wnd={rcv_wnd}")
        print(f"     snd_buf={snd_buf_cc}/{snd_buf_hi}  "
              f"rcv_buf={rcv_buf_cc}/{rcv_buf_hi}")


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "-"
    if path == "-":
        data = sys.stdin.buffer.read()
    else:
        with open(path, "rb") as f:
            data = f.read()

    n = len(data) // RECORD_SIZE
    print(f"Total: {n} records ({len(data)} bytes)")
    for i in range(n):
        decode_record(data[i * RECORD_SIZE:(i + 1) * RECORD_SIZE], i)


if __name__ == "__main__":
    main()
