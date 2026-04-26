#include <sys/types.h>
#include <sys/ioctl.h>
#include <sys/socket.h>

#include <arpa/inet.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "tcp_statsdev.h"
#include "tcp_statsdev_filter.h"

#define READBUF_SIZE (4 * 1024 * 1024) /* 4 MB, ~13000 records max */

static const char *tcp_states[] = {
    "CLOSED",
    "LISTEN",
    "SYN_SENT",
    "SYN_RCVD",
    "ESTABLISHED",
    "CLOSE_WAIT",
    "FIN_WAIT_1",
    "CLOSING",
    "LAST_ACK",
    "FIN_WAIT_2",
    "TIME_WAIT",
};

static const char *
state_name(int state)
{
	if (state >= 0 && state <= 10)
		return (tcp_states[state]);
	return ("?");
}

static void
usage(void)
{
	fprintf(stderr,
		"usage: read_tcpstats [-acL] [-f filter] [-p port] [-P port]\n"
		"  -a        read from /dev/tcpstats-full\n"
		"  -c        count-only mode (print matching record count)\n"
		"  -f filter full filter string (mutually exclusive with -L and -P)\n"
		"  -L        exclude LISTEN sockets (kernel filter)\n"
		"  -p port   show only records matching port (userspace filter)\n"
		"  -P port   kernel-side local port filter\n");
	exit(1);
}

int
main(int argc, char *argv[])
{
	struct tcpstats_version ver;
	struct tcpstats_filter filt;
	struct tcp_stats_record *rec;
	char laddr[INET6_ADDRSTRLEN], raddr[INET6_ADDRSTRLEN];
	const char *devpath;
	char *buf;
	ssize_t nbytes;
	int fd, count, matched, i, ch;
	int flag_all = 0;	       /* -a: use /dev/tcpstats-full */
	int flag_count = 0;	       /* -c: count-only output */
	int flag_listen = 0;	       /* -L: exclude LISTEN */
	int filter_port = -1;	       /* -p: port filter (-1 = disabled) */
	int kernel_port = -1;	       /* -P: kernel-side port filter */
	const char *filter_str = NULL; /* -f: full filter string */

	while ((ch = getopt(argc, argv, "acLf:p:P:")) != -1) {
		switch (ch) {
		case 'a':
			flag_all = 1;
			break;
		case 'c':
			flag_count = 1;
			break;
		case 'f':
			filter_str = optarg;
			break;
		case 'L':
			flag_listen = 1;
			break;
		case 'p':
			filter_port = atoi(optarg);
			if (filter_port <= 0 || filter_port > 65535) {
				fprintf(stderr, "invalid port: %s\n", optarg);
				return (1);
			}
			break;
		case 'P':
			kernel_port = atoi(optarg);
			if (kernel_port <= 0 || kernel_port > 65535) {
				fprintf(stderr, "invalid port: %s\n", optarg);
				return (1);
			}
			break;
		default:
			usage();
		}
	}

	/* -f is mutually exclusive with -L and -P */
	if (filter_str != NULL && (flag_listen || kernel_port >= 0)) {
		fprintf(stderr, "error: -f cannot be combined with -L or -P\n");
		return (1);
	}

	devpath = flag_all ? "/dev/tcpstats-full" : "/dev/tcpstats";
	fd = open(devpath, O_RDONLY);
	if (fd < 0) {
		perror(devpath);
		return (1);
	}

	/* Build and apply kernel-side filter */
	if (filter_str != NULL) {
		char errbuf[TSF_ERRBUF_SIZE];
		memset(&filt, 0, sizeof(filt));
		if (tsf_parse_filter_string(filter_str, strlen(filter_str),
					    &filt, errbuf, sizeof(errbuf)) != 0) {
			fprintf(stderr, "filter parse error: %s\n", errbuf);
			close(fd);
			return (1);
		}
		if (ioctl(fd, TCPSTATS_SET_FILTER, &filt) < 0) {
			perror("ioctl TCPSTATS_SET_FILTER");
			close(fd);
			return (1);
		}
	} else if (flag_listen || kernel_port >= 0) {
		memset(&filt, 0, sizeof(filt));
		filt.version = TSF_VERSION;
		filt.state_mask = 0xFFFF;
		if (flag_listen)
			filt.flags |= TSF_EXCLUDE_LISTEN;
		if (kernel_port >= 0) {
			filt.flags |= TSF_LOCAL_PORT_MATCH;
			filt.local_ports[0] = htons((uint16_t)kernel_port);
		}
		if (ioctl(fd, TCPSTATS_SET_FILTER, &filt) < 0) {
			perror("ioctl TCPSTATS_SET_FILTER");
			close(fd);
			return (1);
		}
	}

	if (!flag_count) {
		if (ioctl(fd, TCPSTATS_VERSION_CMD, &ver) < 0) {
			perror("ioctl TCPSTATS_VERSION_CMD");
			close(fd);
			return (1);
		}
		printf("version=%u  record_size=%u  count_hint=%u\n",
		       ver.protocol_version, ver.record_size,
		       ver.record_count_hint);
	}

	buf = malloc(READBUF_SIZE);
	if (buf == NULL) {
		perror("malloc");
		close(fd);
		return (1);
	}

	nbytes = read(fd, buf, READBUF_SIZE);
	if (nbytes < 0) {
		perror("read");
		free(buf);
		close(fd);
		return (1);
	}

	count = nbytes / TCP_STATS_RECORD_SIZE;
	matched = 0;

	for (i = 0; i < count; i++) {
		rec = (struct tcp_stats_record *)(buf + i * TCP_STATS_RECORD_SIZE);

		/* Userspace port filter */
		if (filter_port >= 0 &&
		    rec->tsr_local_port != (uint16_t)filter_port &&
		    rec->tsr_remote_port != (uint16_t)filter_port)
			continue;

		matched++;

		if (flag_count)
			continue;

		if (rec->tsr_af == AF_INET) {
			inet_ntop(AF_INET, &rec->tsr_local_addr.v4,
				  laddr, sizeof(laddr));
			inet_ntop(AF_INET, &rec->tsr_remote_addr.v4,
				  raddr, sizeof(raddr));
		} else if (rec->tsr_af == AF_INET6) {
			inet_ntop(AF_INET6, &rec->tsr_local_addr.v6,
				  laddr, sizeof(laddr));
			inet_ntop(AF_INET6, &rec->tsr_remote_addr.v6,
				  raddr, sizeof(raddr));
		} else {
			strlcpy(laddr, "?", sizeof(laddr));
			strlcpy(raddr, "?", sizeof(raddr));
		}

		printf("[%d] %s:%u -> %s:%u  state=%d(%s)",
		       i, laddr, rec->tsr_local_port,
		       raddr, rec->tsr_remote_port,
		       rec->tsr_state, state_name(rec->tsr_state));

		if (rec->tsr_rtt > 0)
			printf("  rtt=%u us  cwnd=%u  maxseg=%u",
			       rec->tsr_rtt, rec->tsr_snd_cwnd,
			       rec->tsr_maxseg);

		if (rec->tsr_cc[0] != '\0')
			printf("  cc=%s", rec->tsr_cc);
		if (rec->tsr_stack[0] != '\0')
			printf("  stack=%s", rec->tsr_stack);

		printf("  snd_buf=%u/%u  rcv_buf=%u/%u",
		       rec->tsr_snd_buf_cc, rec->tsr_snd_buf_hiwat,
		       rec->tsr_rcv_buf_cc, rec->tsr_rcv_buf_hiwat);

		printf("  uid=%u\n", rec->tsr_uid);
	}

	if (flag_count)
		printf("%d\n", matched);
	else
		printf("total: %d sockets (%d matched)\n", count, matched);

	free(buf);
	close(fd);
	return (0);
}
