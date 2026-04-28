KMOD=	tcpstats
SRCS=	tcp_statsdev.c tcp_statsdev_filter.c
CFLAGS+=	-I${.CURDIR}
CFLAGS+=	-I${.OBJDIR}

# Uncomment to enable DTrace SDT probes
#CFLAGS+=	-DTCPSTATS_DTRACE

# Uncomment to enable detailed statistics counters
#CFLAGS+=	-DTCPSTATS_STATS

.if !empty(CFLAGS:M*TCPSTATS_DTRACE*)
CFLAGS+=	-DKDTRACE_HOOKS
.endif

# Create netinet/ include directory so <netinet/tcp_statsdev.h> resolves
# without requiring headers to be pre-installed in /usr/src/sys/netinet
beforebuild:
	@mkdir -p ${.OBJDIR}/netinet
	@ln -sf ${.CURDIR}/tcp_statsdev.h ${.OBJDIR}/netinet/
	@ln -sf ${.CURDIR}/tcp_statsdev_filter.h ${.OBJDIR}/netinet/

MAN=	man/tcpstats.4

.include <bsd.kmod.mk>
