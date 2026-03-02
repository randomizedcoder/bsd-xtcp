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

    /* Spare for future expansion (52 bytes) */
    uint32_t    _tsr_spare[13];
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
