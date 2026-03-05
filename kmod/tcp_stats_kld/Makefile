KMOD	= tcp_stats_kld
SRCS	= tcp_stats_kld.c tcp_stats_filter_parse.c
SYSDIR	?= /usr/src/sys
CFLAGS	+= -I${.CURDIR}

# Pass extra compile flags from the command line (e.g., make EXTRA_CFLAGS="-DTCPSTATS_STATS")
EXTRA_CFLAGS ?=
CFLAGS	+= ${EXTRA_CFLAGS}

# Uncomment for DTrace SDT probes (adds per-probe overhead when DTrace attached)
# CFLAGS+= -DTCPSTATS_DTRACE

# Uncomment for detailed per-socket statistics (adds overhead in hot loop)
# CFLAGS+= -DTCPSTATS_STATS

# Enable both for full profiling:
# CFLAGS+= -DTCPSTATS_DTRACE -DTCPSTATS_STATS

# KDTRACE_HOOKS is required for SDT macros in sys/sdt.h to emit linker set
# entries.  GENERIC kernels define this, but out-of-tree builds do not get it
# in opt_global.h (it is in GENERIC, not DEFAULTS).  Automatically add it
# when TCPSTATS_DTRACE is requested via EXTRA_CFLAGS or uncommented above.
.if !empty(CFLAGS:M*TCPSTATS_DTRACE*)
CFLAGS+= -DKDTRACE_HOOKS
.endif

.include <bsd.kmod.mk>
