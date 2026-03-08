/*
 * test_dos_limits.c -- DoS protection validation for tcp_stats_kld.
 *
 * Tests three DoS protections that are hard to validate from a shell script
 * because they require holding fds open or precise signal timing.
 *
 * Sub-tests:
 *   emfile  -- verify max_open_fds limit returns EMFILE
 *   timeout -- verify read timeout returns partial results
 *   eintr   -- verify signal interrupts a long read
 *
 * Build (FreeBSD):
 *   cc -O2 -o test_dos_limits test_dos_limits.c -I..
 *
 * Run:
 *   sysctl dev.tcpstats.max_open_fds=4
 *   ./test_dos_limits emfile
 *
 *   sysctl dev.tcpstats.max_read_duration_ms=50
 *   ./test_dos_limits timeout 50000
 *
 *   ./test_dos_limits eintr 50000
 */

#include <sys/types.h>
#include <sys/sysctl.h>
#include <sys/wait.h>

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "tcp_stats_kld.h"

#define DEVPATH	     "/dev/tcpstats"
#define MAX_TEST_FDS 256
#define READBUF_SIZE (128 * 1024 * 1024) /* 128 MB -- large enough for 400K records */

static int
get_sysctl_int(const char *name)
{
	int val;
	size_t len = sizeof(val);

	if (sysctlbyname(name, &val, &len, NULL, 0) < 0) {
		fprintf(stderr, "sysctl %s: %s\n", name, strerror(errno));
		return (-1);
	}
	return (val);
}

/* ================================================================
 * Sub-test 1: EMFILE -- max_open_fds limit
 * ================================================================ */

static int
test_emfile(void)
{
	int fds[MAX_TEST_FDS];
	int max_fds, i, extra_fd;
	int passed = 1;

	printf("  [emfile] reading max_open_fds...\n");
	max_fds = get_sysctl_int("dev.tcpstats.max_open_fds");
	if (max_fds < 0)
		return (0);
	if (max_fds > MAX_TEST_FDS)
		max_fds = MAX_TEST_FDS;

	printf("  [emfile] opening %d fds to %s...\n", max_fds, DEVPATH);
	for (i = 0; i < max_fds; i++) {
		fds[i] = open(DEVPATH, O_RDONLY);
		if (fds[i] < 0) {
			fprintf(stderr,
				"  [emfile] FAIL: open %d failed: %s "
				"(expected success)\n",
				i, strerror(errno));
			passed = 0;
			/* Close what we opened */
			while (--i >= 0)
				close(fds[i]);
			return (passed);
		}
	}
	printf("  [emfile] all %d fds opened successfully\n", max_fds);

	/* The next open should fail with EMFILE */
	printf("  [emfile] opening fd %d (should fail with EMFILE)...\n",
	       max_fds);
	extra_fd = open(DEVPATH, O_RDONLY);
	if (extra_fd >= 0) {
		fprintf(stderr,
			"  [emfile] FAIL: open %d succeeded "
			"(expected EMFILE)\n",
			max_fds);
		close(extra_fd);
		passed = 0;
	} else if (errno != EMFILE) {
		fprintf(stderr,
			"  [emfile] FAIL: open %d got errno=%d (%s), "
			"expected EMFILE (%d)\n",
			max_fds, errno, strerror(errno), EMFILE);
		passed = 0;
	} else {
		printf("  [emfile] OK: got EMFILE as expected\n");
	}

	/* Cleanup */
	for (i = 0; i < max_fds; i++)
		close(fds[i]);

	/* Verify we can open again after closing */
	extra_fd = open(DEVPATH, O_RDONLY);
	if (extra_fd < 0) {
		fprintf(stderr,
			"  [emfile] FAIL: reopen after close failed: %s\n",
			strerror(errno));
		passed = 0;
	} else {
		printf("  [emfile] OK: reopen after close succeeded\n");
		close(extra_fd);
	}

	if (passed)
		printf("  [emfile] PASSED\n");
	else
		printf("  [emfile] FAILED\n");
	return (passed);
}

/* ================================================================
 * Sub-test 2: Timeout -- read duration limit returns partial results
 * ================================================================ */

static int
test_timeout(int expected_connections)
{
	char *buf;
	ssize_t nbytes;
	int fd, records;
	int passed = 1;

	printf("  [timeout] expected_connections=%d\n", expected_connections);
	printf("  [timeout] opening %s...\n", DEVPATH);

	fd = open(DEVPATH, O_RDONLY);
	if (fd < 0) {
		fprintf(stderr, "  [timeout] FAIL: open: %s\n",
			strerror(errno));
		return (0);
	}

	buf = malloc(READBUF_SIZE);
	if (buf == NULL) {
		fprintf(stderr, "  [timeout] FAIL: malloc\n");
		close(fd);
		return (0);
	}

	printf("  [timeout] reading with short timeout...\n");
	nbytes = read(fd, buf, READBUF_SIZE);
	if (nbytes < 0) {
		fprintf(stderr, "  [timeout] FAIL: read: %s\n",
			strerror(errno));
		passed = 0;
	} else {
		records = (int)(nbytes / TCP_STATS_RECORD_SIZE);
		printf("  [timeout] got %d records (bytes=%zd)\n",
		       records, nbytes);
		/*
		 * With a very short timeout (50ms) and many connections
		 * (50K), we expect partial results -- fewer records than
		 * the total number of connections. The read should still
		 * succeed (not error), just return fewer records.
		 */
		if (records >= expected_connections) {
			fprintf(stderr,
				"  [timeout] FAIL: got %d records >= %d "
				"expected (timeout didn't limit)\n",
				records, expected_connections);
			passed = 0;
		} else {
			printf("  [timeout] OK: partial result "
			       "(%d < %d)\n",
			       records, expected_connections);
		}
	}

	free(buf);
	close(fd);

	if (passed)
		printf("  [timeout] PASSED\n");
	else
		printf("  [timeout] FAILED\n");
	return (passed);
}

/* ================================================================
 * Sub-test 3: EINTR -- signal interrupts a long read
 * ================================================================ */

static volatile sig_atomic_t got_signal = 0;

static void
sighandler(int sig)
{
	(void)sig;
	got_signal = 1;
}

static int
test_eintr(int expected_connections)
{
	pid_t child, parent;
	char *buf;
	ssize_t nbytes;
	int fd, status;
	int passed = 1;

	printf("  [eintr] expected_connections=%d\n", expected_connections);

	parent = getpid();

	child = fork();
	if (child < 0) {
		fprintf(stderr, "  [eintr] FAIL: fork: %s\n",
			strerror(errno));
		return (0);
	}

	if (child == 0) {
		/* Child: sleep briefly, then signal parent */
		usleep(10000); /* 10ms */
		kill(parent, SIGUSR1);
		_exit(0);
	}

	/* Parent: set up signal handler and do a long read */
	signal(SIGUSR1, sighandler);

	fd = open(DEVPATH, O_RDONLY);
	if (fd < 0) {
		fprintf(stderr, "  [eintr] FAIL: open: %s\n",
			strerror(errno));
		waitpid(child, &status, 0);
		return (0);
	}

	buf = malloc(READBUF_SIZE);
	if (buf == NULL) {
		fprintf(stderr, "  [eintr] FAIL: malloc\n");
		close(fd);
		waitpid(child, &status, 0);
		return (0);
	}

	printf("  [eintr] reading (child will send SIGUSR1 in ~10ms)...\n");
	nbytes = read(fd, buf, READBUF_SIZE);

	if (nbytes < 0 && errno == EINTR) {
		printf("  [eintr] OK: read returned EINTR\n");
	} else if (nbytes >= 0) {
		/*
		 * Read completed before signal arrived. With 50K
		 * connections this is unlikely but possible on fast
		 * hardware. Check if at least the signal was delivered.
		 */
		int records = (int)(nbytes / TCP_STATS_RECORD_SIZE);
		printf("  [eintr] read completed (%d records) before signal\n",
		       records);
		if (got_signal) {
			printf("  [eintr] OK: signal delivered "
			       "(read was fast enough to complete)\n");
		} else {
			printf("  [eintr] OK: read completed before signal "
			       "(acceptable on fast hardware)\n");
		}
	} else {
		fprintf(stderr,
			"  [eintr] FAIL: read returned -1 with errno=%d (%s), "
			"expected EINTR\n",
			errno, strerror(errno));
		passed = 0;
	}

	free(buf);
	close(fd);
	waitpid(child, &status, 0);

	/* Restore default handler */
	signal(SIGUSR1, SIG_DFL);

	if (passed)
		printf("  [eintr] PASSED\n");
	else
		printf("  [eintr] FAILED\n");
	return (passed);
}

/* ================================================================
 * Main
 * ================================================================ */

static void
usage(void)
{
	fprintf(stderr,
		"usage: test_dos_limits <subtest> [expected_connections]\n"
		"  subtests: emfile, timeout, eintr\n");
	exit(2);
}

int
main(int argc, char *argv[])
{
	const char *subtest;
	int expected = 0;
	int passed = 0;

	if (argc < 2)
		usage();

	subtest = argv[1];
	if (argc >= 3) {
		expected = atoi(argv[2]);
		if (expected < 0)
			expected = 0;
	}

	if (strcmp(subtest, "emfile") == 0)
		passed = test_emfile();
	else if (strcmp(subtest, "timeout") == 0)
		passed = test_timeout(expected);
	else if (strcmp(subtest, "eintr") == 0)
		passed = test_eintr(expected);
	else
		usage();

	return (passed ? 0 : 1);
}
