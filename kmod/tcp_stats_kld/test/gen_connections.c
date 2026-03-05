/*
 * gen_connections.c -- Loopback TCP connection generator for benchmarking.
 *
 * Creates N persistent TCP connections to 127.0.0.1 for use with
 * bench_read_tcpstats. Each connection consumes ~1KB kernel memory.
 *
 * Build (FreeBSD):
 *   cc -O2 -o gen_connections gen_connections.c
 *
 * Run:
 *   ./gen_connections [count] [base_port]
 *
 *   count     - number of connections to create (default 1000)
 *   base_port - starting listen port (default 9100)
 *
 * The tool creates listener sockets on ports base_port through
 * base_port+7 (8 ports), then connects to them round-robin.
 * Once all connections are established, it prints a summary and
 * waits for SIGINT (Ctrl-C) to clean up.
 *
 * For large counts, tune system limits first:
 *   sysctl kern.maxfiles=500000
 *   sysctl kern.maxfilesperproc=250000
 *   sysctl net.inet.ip.portrange.first=1024
 *   sysctl net.inet.ip.portrange.last=65535
 */

#include <sys/types.h>
#include <sys/socket.h>
#include <sys/select.h>

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define DEFAULT_COUNT		1000
#define DEFAULT_BASE_PORT	9100
#define NUM_LISTEN_PORTS	8
#define MAX_CONNECTIONS		500000

static volatile int running = 1;

static void
sighandler(int sig)
{
	(void)sig;
	running = 0;
}

static void
usage(void)
{
	fprintf(stderr,
	    "usage: gen_connections [count] [base_port]\n"
	    "  count      number of TCP connections (default %d, max %d)\n"
	    "  base_port  starting listen port (default %d)\n",
	    DEFAULT_COUNT, MAX_CONNECTIONS, DEFAULT_BASE_PORT);
	exit(1);
}

int
main(int argc, char *argv[])
{
	int count = DEFAULT_COUNT;
	int base_port = DEFAULT_BASE_PORT;
	int listen_fds[NUM_LISTEN_PORTS];
	int *client_fds = NULL;
	int *accept_fds = NULL;
	int total_established = 0;
	int i, fd, opt;

	if (argc > 1) {
		count = atoi(argv[1]);
		if (count <= 0 || count > MAX_CONNECTIONS)
			usage();
	}
	if (argc > 2) {
		base_port = atoi(argv[2]);
		if (base_port <= 0 || base_port > 65535 - NUM_LISTEN_PORTS)
			usage();
	}

	signal(SIGINT, sighandler);
	signal(SIGTERM, sighandler);

	/* Allocate fd arrays */
	client_fds = calloc(count, sizeof(int));
	accept_fds = calloc(count, sizeof(int));
	if (client_fds == NULL || accept_fds == NULL) {
		perror("calloc");
		return (1);
	}
	for (i = 0; i < count; i++) {
		client_fds[i] = -1;
		accept_fds[i] = -1;
	}

	/* Create listener sockets */
	printf("Creating %d listeners on ports %d-%d...\n",
	    NUM_LISTEN_PORTS, base_port,
	    base_port + NUM_LISTEN_PORTS - 1);

	for (i = 0; i < NUM_LISTEN_PORTS; i++) {
		struct sockaddr_in addr;

		listen_fds[i] = socket(AF_INET, SOCK_STREAM, 0);
		if (listen_fds[i] < 0) {
			perror("socket(listen)");
			return (1);
		}

		opt = 1;
		setsockopt(listen_fds[i], SOL_SOCKET, SO_REUSEADDR,
		    &opt, sizeof(opt));

		memset(&addr, 0, sizeof(addr));
		addr.sin_family = AF_INET;
		addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
		addr.sin_port = htons(base_port + i);

		if (bind(listen_fds[i], (struct sockaddr *)&addr,
		    sizeof(addr)) < 0) {
			fprintf(stderr, "bind port %d: %s\n",
			    base_port + i, strerror(errno));
			return (1);
		}

		/* Large backlog for batch connects */
		if (listen(listen_fds[i], 1024) < 0) {
			perror("listen");
			return (1);
		}
	}

	/* Create connections */
	printf("Establishing %d connections...\n", count);

	for (i = 0; i < count && running; i++) {
		struct sockaddr_in addr;
		int port_idx = i % NUM_LISTEN_PORTS;

		/* Connect to round-robin listener */
		fd = socket(AF_INET, SOCK_STREAM, 0);
		if (fd < 0) {
			if (errno == EMFILE || errno == ENFILE) {
				fprintf(stderr,
				    "fd limit at %d connections "
				    "(raise kern.maxfiles)\n", i);
				break;
			}
			perror("socket(connect)");
			break;
		}

		memset(&addr, 0, sizeof(addr));
		addr.sin_family = AF_INET;
		addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
		addr.sin_port = htons(base_port + port_idx);

		if (connect(fd, (struct sockaddr *)&addr,
		    sizeof(addr)) < 0) {
			if (errno == EADDRNOTAVAIL) {
				fprintf(stderr,
				    "ephemeral ports exhausted at %d "
				    "connections\n", i);
				close(fd);
				break;
			}
			fprintf(stderr, "connect %d: %s\n",
			    i, strerror(errno));
			close(fd);
			break;
		}
		client_fds[i] = fd;

		/* Accept the corresponding server side */
		fd = accept(listen_fds[port_idx], NULL, NULL);
		if (fd < 0) {
			fprintf(stderr, "accept %d: %s\n",
			    i, strerror(errno));
			break;
		}
		accept_fds[i] = fd;

		total_established++;

		/* Progress reporting */
		if ((i + 1) % 1000 == 0 || i + 1 == count)
			printf("  %d / %d established\r",
			    total_established, count);
	}
	printf("\n");

	printf("%d TCP connections established. "
	    "Press Ctrl-C to close and exit.\n", total_established);

	/* Wait for signal */
	while (running)
		sleep(1);

	printf("\nClosing %d connections...\n", total_established);

	/* Cleanup */
	for (i = 0; i < count; i++) {
		if (client_fds[i] >= 0)
			close(client_fds[i]);
		if (accept_fds[i] >= 0)
			close(accept_fds[i]);
	}
	for (i = 0; i < NUM_LISTEN_PORTS; i++)
		close(listen_fds[i]);

	free(client_fds);
	free(accept_fds);

	printf("Done.\n");
	return (0);
}
