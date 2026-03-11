/*
 * bench_filter_parse.c — Benchmark harness for the filter parser.
 *
 * Build:
 *   cc -O2 -o bench_filter_parse bench_filter_parse.c \
 *       ../tcp_stats_filter_parse.c -I..
 *
 * Run:
 *   ./bench_filter_parse [iterations]
 *
 * Default: 1,000,000 iterations per workload.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "../tcp_stats_filter_parse.h"

/* Prevent dead-code elimination of parse results */
static volatile int sink;

struct bench_workload {
	const char *name;
	const char *input;
};

static const struct bench_workload workloads[] = {
    {"empty",
     ""},
    {"single_port",
     "local_port=443"},
    {"multi_port",
     "local_port=443,8443,8080,80,3000,9090,5432,6379"},
    {"exclude_states",
     "exclude=listen,timewait,syn_sent,closing"},
    {"ipv4_cidr",
     "local_addr=10.0.0.0/24 local_port=443"},
    {"ipv6_compressed",
     "remote_addr=2001:db8::1 remote_port=80"},
    {"ipv6_full",
     "local_addr=2001:0db8:0000:0000:0000:0000:0000:0001"},
    {"complex_combo",
     "local_port=443,8443 exclude=listen,timewait "
     "local_addr=10.0.0.0/24 ipv4_only format=full"},
    {"worst_case",
     "local_port=1,2,3,4,5,6,7,8 remote_port=1,2,3,4,5,6,7,8 "
     "exclude=listen,timewait,syn_sent,syn_received,closing "
     "local_addr=10.0.0.0/8 remote_addr=192.168.0.0/16 "
     "format=full fields=all"},
    {"uppercase_stress",
     "LOCAL_PORT=443,8443 EXCLUDE=LISTEN,TIMEWAIT "
     "LOCAL_ADDR=10.0.0.0/24 IPV4_ONLY FORMAT=FULL"},
    {NULL, NULL}};

static double
timespec_to_ms(struct timespec *ts)
{
	return (double)ts->tv_sec * 1000.0 + (double)ts->tv_nsec / 1e6;
}

int
main(int argc, char *argv[])
{
	long iterations = 1000000;
	struct tcpstats_filter filter;
	char errbuf[TSF_ERRBUF_SIZE];

	if (argc > 1) {
		iterations = strtol(argv[1], NULL, 10);
		if (iterations <= 0) {
			fprintf(stderr, "usage: %s [iterations]\n", argv[0]);
			return 1;
		}
	}

	printf("Filter parser benchmark — %ld iterations per workload\n\n",
	       iterations);
	printf("%-20s %10s %10s %12s\n",
	       "WORKLOAD", "TOTAL(ms)", "NS/CALL", "CALLS/SEC");
	printf("%-20s %10s %10s %12s\n",
	       "--------------------", "----------", "----------",
	       "------------");

	for (const struct bench_workload *w = workloads;
	     w->name != NULL; w++) {
		struct timespec start, end;
		size_t len = strlen(w->input);

		clock_gettime(CLOCK_MONOTONIC, &start);

		for (long i = 0; i < iterations; i++) {
			int err = tsf_parse_filter_string(w->input, len,
							  &filter, errbuf, sizeof(errbuf));
			sink = err;
		}

		clock_gettime(CLOCK_MONOTONIC, &end);

		double total_ms = timespec_to_ms(&end) - timespec_to_ms(&start);
		double ns_per_call = (total_ms * 1e6) / (double)iterations;
		double calls_per_sec = (double)iterations / (total_ms / 1000.0);

		printf("%-20s %10.2f %10.1f %12.0f\n",
		       w->name, total_ms, ns_per_call, calls_per_sec);
	}

	printf("\n");
	return 0;
}
