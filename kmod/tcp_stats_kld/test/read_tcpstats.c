#include <sys/types.h>
#include <sys/ioctl.h>
#include <sys/socket.h>

#include <arpa/inet.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "tcp_stats_kld.h"

#define	READBUF_SIZE	(1024 * 1024)	/* 1 MB, ~3200 records max */

static const char *tcp_states[] = {
	"CLOSED",      "LISTEN",      "SYN_SENT",    "SYN_RCVD",
	"ESTABLISHED", "CLOSE_WAIT",  "FIN_WAIT_1",  "CLOSING",
	"LAST_ACK",    "FIN_WAIT_2",  "TIME_WAIT",
};

static const char *
state_name(int state)
{
	if (state >= 0 && state <= 10)
		return (tcp_states[state]);
	return ("?");
}

int
main(int argc, char *argv[])
{
	struct tcpstats_version ver;
	struct tcp_stats_record *rec;
	char laddr[INET6_ADDRSTRLEN], raddr[INET6_ADDRSTRLEN];
	char *buf;
	ssize_t nbytes;
	int fd, count, i;

	fd = open("/dev/tcpstats", O_RDONLY);
	if (fd < 0) {
		perror("open /dev/tcpstats");
		return (1);
	}

	if (ioctl(fd, TCPSTATS_VERSION_CMD, &ver) < 0) {
		perror("ioctl TCPSTATS_VERSION_CMD");
		close(fd);
		return (1);
	}

	printf("version=%u  record_size=%u  count_hint=%u\n",
	    ver.protocol_version, ver.record_size, ver.record_count_hint);

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
	for (i = 0; i < count; i++) {
		rec = (struct tcp_stats_record *)(buf + i * TCP_STATS_RECORD_SIZE);

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

		printf("  uid=%u\n", rec->tsr_uid);
	}

	printf("total: %d sockets\n", count);
	free(buf);
	close(fd);
	return (0);
}
