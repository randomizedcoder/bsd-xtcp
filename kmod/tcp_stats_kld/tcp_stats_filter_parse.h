#ifndef _TCP_STATS_FILTER_PARSE_H_
#define _TCP_STATS_FILTER_PARSE_H_

#ifdef _KERNEL
#include <sys/param.h>
#include <netinet/in.h>
#else
#include <sys/types.h>
#include <netinet/in.h>
#endif

#include "tcp_stats_kld.h"

/* --- Field group bitmasks --- */
#define TSR_FIELDS_IDENTITY   0x001
#define TSR_FIELDS_STATE      0x002
#define TSR_FIELDS_CONGESTION 0x004
#define TSR_FIELDS_RTT	      0x008
#define TSR_FIELDS_SEQUENCES  0x010
#define TSR_FIELDS_COUNTERS   0x020
#define TSR_FIELDS_TIMERS     0x040
#define TSR_FIELDS_BUFFERS    0x080
#define TSR_FIELDS_ECN	      0x100
#define TSR_FIELDS_NAMES      0x200
#define TSR_FIELDS_ALL	      0x3FF
#define TSR_FIELDS_DEFAULT    0x08F

/* --- Parser API --- */
#define TSF_PARSE_MAXLEN	512
#define TSF_PARSE_MAXDIRECTIVES 16
#define TSF_ERRBUF_SIZE		128

int tsf_parse_filter_string(const char *input, size_t len,
			    struct tcpstats_filter *out, char *errbuf, size_t errbuflen);

#endif /* _TCP_STATS_FILTER_PARSE_H_ */
