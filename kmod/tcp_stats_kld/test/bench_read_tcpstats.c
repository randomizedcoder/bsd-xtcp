/*
 * bench_read_tcpstats.c -- Read-path microbenchmark for tcp_stats_kld.
 *
 * Measures read() throughput and latency against /dev/tcpstats with
 * various filter configurations, buffer sizes, and concurrency levels.
 *
 * Build (FreeBSD):
 *   cc -O2 -lpthread -o bench_read_tcpstats bench_read_tcpstats.c -I..
 *
 * Run:
 *   # Ensure tcp_stats_kld is loaded and connections exist:
 *   #   kldload ./tcp_stats_kld.ko
 *   #   ./gen_connections 1000
 *   ./bench_read_tcpstats [iterations]
 *
 * For concurrent reader tests with >16 threads, first raise the limit:
 *   sysctl dev.tcpstats.max_open_fds=64
 *
 * Output: CSV-like table for each workload.
 */

#include <sys/types.h>
#include <sys/ioctl.h>

#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "../tcp_stats_kld.h"
#include "../tcp_stats_filter_parse.h"

#define DEVPATH	      "/dev/tcpstats"
#define DEFAULT_ITERS 10
#define DEFAULT_BUFSZ (4 * 1024 * 1024)

static int global_iterations;

/* ================================================================
 * Timing helpers
 * ================================================================ */

static double
timespec_to_us(struct timespec *ts)
{
	return (double)ts->tv_sec * 1e6 + (double)ts->tv_nsec / 1e3;
}

/* ================================================================
 * Single-threaded read benchmark
 * ================================================================ */

struct bench_result {
	int records;
	double time_us;
};

static struct bench_result
do_single_read(const char *devpath, struct tcpstats_filter *filt,
	       size_t bufsz)
{
	struct bench_result res;
	struct timespec start, end;
	char *buf;
	ssize_t nbytes;
	int fd;

	res.records = 0;
	res.time_us = 0.0;

	fd = open(devpath, O_RDONLY);
	if (fd < 0)
		return (res);

	if (filt != NULL) {
		if (ioctl(fd, TCPSTATS_SET_FILTER, filt) < 0) {
			close(fd);
			return (res);
		}
	}

	buf = malloc(bufsz);
	if (buf == NULL) {
		close(fd);
		return (res);
	}

	clock_gettime(CLOCK_MONOTONIC, &start);
	nbytes = read(fd, buf, bufsz);
	clock_gettime(CLOCK_MONOTONIC, &end);

	if (nbytes > 0)
		res.records = (int)(nbytes / TCP_STATS_RECORD_SIZE);
	res.time_us = timespec_to_us(&end) - timespec_to_us(&start);

	free(buf);
	close(fd);
	return (res);
}

static void
run_bench(const char *name, struct tcpstats_filter *filt, size_t bufsz)
{
	double total_us = 0.0;
	double min_us = 1e18, max_us = 0.0;
	int total_records = 0;

	for (int i = 0; i < global_iterations; i++) {
		struct bench_result r = do_single_read(DEVPATH, filt, bufsz);
		total_us += r.time_us;
		if (r.time_us < min_us) min_us = r.time_us;
		if (r.time_us > max_us) max_us = r.time_us;
		total_records += r.records;
	}

	double avg_us = total_us / global_iterations;
	int avg_records = total_records / global_iterations;
	double ns_per_rec = (avg_records > 0) ? (avg_us * 1000.0 / avg_records) : 0.0;
	double recs_per_sec = (avg_us > 0) ? (avg_records / (avg_us / 1e6)) : 0.0;

	printf("  %-28s  %6d recs  %10.1f us avg  "
	       "[%8.1f - %8.1f]  %8.1f ns/rec  %10.0f rec/s\n",
	       name, avg_records, avg_us, min_us, max_us,
	       ns_per_rec, recs_per_sec);
}

/* ================================================================
 * Concurrent reader benchmark
 * ================================================================ */

struct thread_ctx {
	int nreads;
	size_t bufsz;
	double total_us;
	int total_records;
	int errors;
};

static void *
reader_thread(void *arg)
{
	struct thread_ctx *ctx = arg;

	for (int i = 0; i < ctx->nreads; i++) {
		struct bench_result r = do_single_read(DEVPATH, NULL,
						       ctx->bufsz);
		if (r.time_us == 0.0 && r.records == 0)
			ctx->errors++;
		else {
			ctx->total_us += r.time_us;
			ctx->total_records += r.records;
		}
	}

	return (NULL);
}

static void
run_concurrent(int nthreads, int reads_per_thread)
{
	pthread_t *threads;
	struct thread_ctx *ctxs;
	struct timespec start, end;
	double wall_us;

	threads = calloc(nthreads, sizeof(pthread_t));
	ctxs = calloc(nthreads, sizeof(struct thread_ctx));
	if (threads == NULL || ctxs == NULL) {
		fprintf(stderr, "  alloc failed for %d threads\n", nthreads);
		free(threads);
		free(ctxs);
		return;
	}

	for (int i = 0; i < nthreads; i++) {
		ctxs[i].nreads = reads_per_thread;
		ctxs[i].bufsz = DEFAULT_BUFSZ;
	}

	clock_gettime(CLOCK_MONOTONIC, &start);
	for (int i = 0; i < nthreads; i++)
		pthread_create(&threads[i], NULL, reader_thread, &ctxs[i]);
	for (int i = 0; i < nthreads; i++)
		pthread_join(threads[i], NULL);
	clock_gettime(CLOCK_MONOTONIC, &end);

	wall_us = timespec_to_us(&end) - timespec_to_us(&start);

	int total_errors = 0;
	double total_thread_us = 0.0;
	int total_records = 0;
	for (int i = 0; i < nthreads; i++) {
		total_errors += ctxs[i].errors;
		total_thread_us += ctxs[i].total_us;
		total_records += ctxs[i].total_records;
	}

	int total_reads = nthreads * reads_per_thread;
	double avg_per_read = total_thread_us / (total_reads - total_errors);

	printf("  %2d threads x %d reads: %10.1f us wall  "
	       "%8.1f us/read avg  %d errors  %d total recs\n",
	       nthreads, reads_per_thread, wall_us,
	       avg_per_read, total_errors, total_records);

	free(threads);
	free(ctxs);
}

/* ================================================================
 * Workload definitions
 * ================================================================ */

int
main(int argc, char *argv[])
{
	struct tcpstats_filter filt;
	struct tcpstats_version ver;
	int fd;

	global_iterations = DEFAULT_ITERS;
	if (argc > 1) {
		global_iterations = atoi(argv[1]);
		if (global_iterations <= 0)
			global_iterations = DEFAULT_ITERS;
	}

	/* Probe device */
	fd = open(DEVPATH, O_RDONLY);
	if (fd < 0) {
		fprintf(stderr, "cannot open %s: %s\n"
				"  (is tcp_stats_kld loaded?)\n",
			DEVPATH, strerror(errno));
		return (1);
	}
	if (ioctl(fd, TCPSTATS_VERSION_CMD, &ver) == 0) {
		printf("tcp_stats_kld v%u, record_size=%u, "
		       "connection_hint=%u\n",
		       ver.protocol_version, ver.record_size,
		       ver.record_count_hint);
	}
	close(fd);

	printf("\nRead-path benchmark -- %d iterations per workload\n",
	       global_iterations);
	printf("==========================================================\n");

	/* Workload 1: Baseline (no filter) */
	printf("\n[1] Baseline: no filter, 4MB buffer\n");
	run_bench("no filter", NULL, DEFAULT_BUFSZ);

	/* Workload 2: Port filter selectivity */
	printf("\n[2] Port filter selectivity\n");
	{
		/* Match port 22 (likely exists) */
		memset(&filt, 0, sizeof(filt));
		filt.version = TSF_VERSION;
		filt.state_mask = 0xFFFF;
		filt.field_mask = TSR_FIELDS_DEFAULT;
		filt.flags = TSF_LOCAL_PORT_MATCH;
		filt.local_ports[0] = htons(22);
		run_bench("local_port=22", &filt, DEFAULT_BUFSZ);

		/* Match port 443 */
		filt.local_ports[0] = htons(443);
		run_bench("local_port=443", &filt, DEFAULT_BUFSZ);

		/* Match port 99999 (unlikely) */
		filt.local_ports[0] = htons(1);
		run_bench("local_port=1 (no match)", &filt, DEFAULT_BUFSZ);
	}

	/* Workload 3: State filter */
	printf("\n[3] State filter\n");
	{
		memset(&filt, 0, sizeof(filt));
		filt.version = TSF_VERSION;
		filt.state_mask = (1 << 4); /* ESTABLISHED only */
		filt.field_mask = TSR_FIELDS_DEFAULT;
		filt.flags = TSF_STATE_INCLUDE_MODE;
		run_bench("established only", &filt, DEFAULT_BUFSZ);

		filt.state_mask = 0xFFFF & ~(1 << 1) & ~(1 << 10);
		filt.flags = 0;
		run_bench("exclude listen+timewait", &filt, DEFAULT_BUFSZ);
	}

	/* Workload 4: field_mask gating */
	printf("\n[4] Field mask gating\n");
	{
		memset(&filt, 0, sizeof(filt));
		filt.version = TSF_VERSION;
		filt.state_mask = 0xFFFF;

		filt.field_mask = TSR_FIELDS_ALL;
		run_bench("fields=all", &filt, DEFAULT_BUFSZ);

		filt.field_mask = TSR_FIELDS_IDENTITY | TSR_FIELDS_STATE;
		run_bench("fields=identity,state", &filt, DEFAULT_BUFSZ);

		filt.field_mask = TSR_FIELDS_DEFAULT;
		run_bench("fields=default", &filt, DEFAULT_BUFSZ);
	}

	/* Workload 5: Buffer size sweep */
	printf("\n[5] Buffer size sweep\n");
	{
		size_t sizes[] = {320, 4096, 65536, 1024 * 1024, 4 * 1024 * 1024};
		const char *names[] = {
		    "320B (1 rec)", "4KB", "64KB", "1MB", "4MB"};
		for (int i = 0; i < 5; i++)
			run_bench(names[i], NULL, sizes[i]);
	}

	/* Workload 6: Concurrent readers */
	printf("\n[6] Concurrent readers\n");
	{
		int concurrency[] = {1, 2, 4, 8, 16};
		for (int i = 0; i < 5; i++)
			run_concurrent(concurrency[i],
				       global_iterations);
	}

	/* Workload 7: IPv4 only filter */
	printf("\n[7] IP version filter\n");
	{
		memset(&filt, 0, sizeof(filt));
		filt.version = TSF_VERSION;
		filt.state_mask = 0xFFFF;
		filt.field_mask = TSR_FIELDS_DEFAULT;
		filt.flags = TSF_IPV4_ONLY;
		run_bench("ipv4_only", &filt, DEFAULT_BUFSZ);

		filt.flags = TSF_IPV6_ONLY;
		run_bench("ipv6_only", &filt, DEFAULT_BUFSZ);
	}

	printf("\n==========================================================\n");
	printf("Benchmark complete.\n");

	return (0);
}
