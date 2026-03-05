/*
 * test_filter_parse.c — Userspace unit test harness for the filter parser.
 *
 * Build:
 *   cc -o test_filter_parse test_filter_parse.c ../tcp_stats_filter_parse.c -I..
 *
 * Run:
 *   ./test_filter_parse
 */

#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <arpa/inet.h>

#include "../tcp_stats_filter_parse.h"

/* TCP state constants (matching FreeBSD netinet/tcp_fsm.h) */
#ifndef TCPS_CLOSED
#define TCPS_CLOSED         0
#define TCPS_LISTEN         1
#define TCPS_SYN_SENT       2
#define TCPS_SYN_RECEIVED   3
#define TCPS_ESTABLISHED    4
#define TCPS_CLOSE_WAIT     5
#define TCPS_FIN_WAIT_1     6
#define TCPS_CLOSING        7
#define TCPS_LAST_ACK       8
#define TCPS_FIN_WAIT_2     9
#define TCPS_TIME_WAIT      10
#endif

struct test_case {
	const char *name;
	const char *input;
	int expected_error;
	const char *expected_errmsg;	/* Substring match, or NULL */
};

static const struct test_case cases[] = {
	/* --- Positive: empty/whitespace --- */
	{"empty string resets",
	 "", 0, NULL},
	{"whitespace-only resets",
	 "   ", 0, NULL},

	/* --- Positive: port parsing --- */
	{"single port",
	 "local_port=443", 0, NULL},
	{"multiple ports",
	 "local_port=443,8443,8080", 0, NULL},
	{"max ports (8)",
	 "local_port=1,2,3,4,5,6,7,8", 0, NULL},
	{"remote port",
	 "remote_port=80,443", 0, NULL},
	{"both directions",
	 "local_port=443 remote_port=80", 0, NULL},

	/* --- Positive: state parsing --- */
	{"exclude listen",
	 "exclude=listen", 0, NULL},
	{"exclude multiple",
	 "exclude=listen,timewait", 0, NULL},
	{"include established",
	 "include_state=established", 0, NULL},
	{"include multiple",
	 "include_state=established,syn_sent", 0, NULL},
	{"case insensitive states",
	 "EXCLUDE=LISTEN", 0, NULL},
	{"closewait alias",
	 "exclude=close_wait", 0, NULL},
	{"timewait alias",
	 "exclude=timewait", 0, NULL},

	/* --- Positive: IPv4 parsing --- */
	{"ipv4 exact",
	 "local_addr=10.0.0.1", 0, NULL},
	{"ipv4 /24",
	 "local_addr=10.0.0.0/24", 0, NULL},
	{"ipv4 /32",
	 "local_addr=10.0.0.1/32", 0, NULL},
	{"ipv4 /0",
	 "local_addr=0.0.0.0/0", 0, NULL},
	{"ipv4 remote",
	 "remote_addr=192.168.1.0/24", 0, NULL},
	{"ipv4 combo with port",
	 "local_port=443 local_addr=10.0.0.0/24", 0, NULL},

	/* --- Positive: IPv6 parsing --- */
	{"ipv6 loopback",
	 "local_addr=::1", 0, NULL},
	{"ipv6 all zeros",
	 "local_addr=::", 0, NULL},
	{"ipv6 full",
	 "local_addr=2001:db8:0:0:0:0:0:1", 0, NULL},
	{"ipv6 compressed",
	 "local_addr=2001:db8::1", 0, NULL},
	{"ipv6 link-local cidr",
	 "local_addr=fe80::/10", 0, NULL},
	{"ipv6 /128",
	 "local_addr=::1/128", 0, NULL},
	{"ipv6 /0",
	 "local_addr=::/0", 0, NULL},
	{"ipv6 remote combo",
	 "remote_addr=fe80::/10 local_port=443", 0, NULL},

	/* --- Positive: flags --- */
	{"ipv4_only flag",
	 "ipv4_only", 0, NULL},
	{"ipv6_only flag",
	 "ipv6_only", 0, NULL},

	/* --- Positive: format --- */
	{"format compact",
	 "format=compact", 0, NULL},
	{"format full",
	 "format=full", 0, NULL},

	/* --- Positive: fields --- */
	{"fields single",
	 "fields=rtt", 0, NULL},
	{"fields multiple",
	 "fields=state,rtt,buffers", 0, NULL},
	{"fields all",
	 "fields=all", 0, NULL},
	{"fields default",
	 "fields=default", 0, NULL},

	/* --- Positive: full combo --- */
	{"full combo",
	 "local_port=443 exclude=listen,timewait ipv4_only format=full", 0, NULL},
	{"case insensitive keys",
	 "LOCAL_PORT=443 EXCLUDE=LISTEN", 0, NULL},

	/* --- Structural rejections --- */
	{"non-printable char",
	 "local_port=443\x01", EINVAL, "non-printable"},
	{"unknown directive",
	 "foobar=123", EINVAL, "unknown directive"},
	{"missing value",
	 "local_port", EINVAL, "did you mean"},
	{"empty value",
	 "local_port=", EINVAL, "empty value"},

	/* --- Port rejections --- */
	{"port zero",
	 "local_port=0", EINVAL, "port 0"},
	{"port overflow 65536",
	 "local_port=65536", EINVAL, "exceeds maximum"},
	{"port leading zero",
	 "local_port=0443", EINVAL, "leading zero"},
	{"port non-digit",
	 "local_port=abc", EINVAL, "non-digit"},
	{"port negative",
	 "local_port=-1", EINVAL, "non-digit"},
	{"port duplicate",
	 "local_port=443,443", EINVAL, "duplicate port"},
	{"port too many (9)",
	 "local_port=1,2,3,4,5,6,7,8,9", EINVAL, "too many ports"},
	{"port empty list",
	 "local_port=,,", EINVAL, "empty port list"},
	{"port too many digits",
	 "local_port=100000", EINVAL, "too many digits"},
	{"duplicate port directive",
	 "local_port=443 local_port=80", EINVAL, "duplicate port"},

	/* --- State rejections --- */
	{"unknown state",
	 "exclude=foobar", EINVAL, "unknown state"},
	{"duplicate state",
	 "exclude=listen,listen", EINVAL, "duplicate state"},
	{"exclude+include conflict",
	 "exclude=listen include_state=established", EINVAL,
	 "mutually exclusive"},

	/* --- IPv4 address rejections --- */
	{"ipv4 host bits",
	 "local_addr=10.0.0.1/24", EINVAL, "host bits set"},
	{"ipv4 bad octet",
	 "local_addr=999.1.2.3", EINVAL, "exceeds 255"},
	{"ipv4 missing octets",
	 "local_addr=10.0.0", EINVAL, "octets"},
	{"ipv4 prefix too long",
	 "local_addr=10.0.0.0/33", EINVAL, "exceeds maximum 32"},
	{"ipv4 leading zero octet",
	 "local_addr=010.0.0.0/8", EINVAL, "leading zero"},

	/* --- IPv6 address rejections --- */
	{"ipv6 host bits",
	 "remote_addr=fe80::1/10", EINVAL, "host bits set"},
	{"ipv6 prefix >128",
	 "remote_addr=::/129", EINVAL, "exceeds maximum 128"},
	{"ipv6 multiple ::",
	 "remote_addr=2001::1::2", EINVAL, "multiple '::'"},
	{"ipv6 invalid hex",
	 "remote_addr=gggg::1", EINVAL, "invalid character"},
	{"ipv6 single colon start",
	 "remote_addr=:1", EINVAL, "starts with single"},

	/* --- Conflict rejections --- */
	{"both AF flags",
	 "ipv4_only ipv6_only", EINVAL, "mutually exclusive"},
	{"ipv4_only with value",
	 "ipv4_only=true", EINVAL, "flag"},
	{"ipv4 addr + ipv6_only",
	 "local_addr=10.0.0.1 ipv6_only", EINVAL, "conflicts"},
	{"ipv6 addr + ipv4_only",
	 "local_addr=::1 ipv4_only", EINVAL, "conflicts"},

	/* --- Format/fields rejections --- */
	{"format unknown",
	 "format=json", EINVAL, "unknown format"},
	{"fields unknown",
	 "fields=foobar", EINVAL, "unknown field"},

	{NULL, NULL, 0, NULL}
};

int
main(void)
{
	struct tcpstats_filter filter;
	char errbuf[TSF_ERRBUF_SIZE];
	int pass = 0, fail = 0;

	for (const struct test_case *tc = cases; tc->name != NULL; tc++) {
		memset(&filter, 0, sizeof(filter));
		errbuf[0] = '\0';

		int err = tsf_parse_filter_string(tc->input,
		    tc->input ? strlen(tc->input) : 0,
		    &filter, errbuf, sizeof(errbuf));

		int ok = 1;
		if (err != tc->expected_error) {
			printf("FAIL: %s: expected errno %d, got %d",
			    tc->name, tc->expected_error, err);
			if (errbuf[0] != '\0')
				printf(" (errbuf: '%s')", errbuf);
			printf("\n");
			ok = 0;
		}
		if (tc->expected_errmsg != NULL && err != 0) {
			if (strstr(errbuf, tc->expected_errmsg) == NULL) {
				printf("FAIL: %s: expected '%s' in errbuf, "
				    "got '%s'\n",
				    tc->name, tc->expected_errmsg, errbuf);
				ok = 0;
			}
		}

		if (ok) {
			printf("PASS: %s\n", tc->name);
			pass++;
		} else {
			fail++;
		}
	}

	/* --- Specific value verification tests --- */
	printf("\n--- Value verification tests ---\n");

	/* Test: single port produces correct network-order value */
	{
		memset(&filter, 0, sizeof(filter));
		errbuf[0] = '\0';
		int err = tsf_parse_filter_string("local_port=443",
		    strlen("local_port=443"),
		    &filter, errbuf, sizeof(errbuf));
		int ok = 1;
		if (err != 0) {
			printf("FAIL: port 443 value: parse error %d\n", err);
			ok = 0;
		} else {
			if (filter.local_ports[0] != htons(443)) {
				printf("FAIL: port 443 value: expected %u, "
				    "got %u\n",
				    htons(443), filter.local_ports[0]);
				ok = 0;
			}
			if (!(filter.flags & TSF_LOCAL_PORT_MATCH)) {
				printf("FAIL: port 443 value: "
				    "TSF_LOCAL_PORT_MATCH not set\n");
				ok = 0;
			}
		}
		if (ok) {
			printf("PASS: port 443 network byte order\n");
			pass++;
		} else {
			fail++;
		}
	}

	/* Test: exclude=listen,timewait clears correct bits */
	{
		memset(&filter, 0, sizeof(filter));
		errbuf[0] = '\0';
		int err = tsf_parse_filter_string(
		    "exclude=listen,timewait",
		    strlen("exclude=listen,timewait"),
		    &filter, errbuf, sizeof(errbuf));
		int ok = 1;
		if (err != 0) {
			printf("FAIL: exclude bits: parse error %d\n", err);
			ok = 0;
		} else {
			/* Bits 1 (LISTEN) and 10 (TIME_WAIT) cleared */
			if (filter.state_mask & (1 << TCPS_LISTEN)) {
				printf("FAIL: exclude bits: LISTEN bit "
				    "not cleared\n");
				ok = 0;
			}
			if (filter.state_mask & (1 << TCPS_TIME_WAIT)) {
				printf("FAIL: exclude bits: TIME_WAIT bit "
				    "not cleared\n");
				ok = 0;
			}
			/* Other bits should still be set */
			if (!(filter.state_mask & (1 << TCPS_ESTABLISHED))) {
				printf("FAIL: exclude bits: ESTABLISHED bit "
				    "wrongly cleared\n");
				ok = 0;
			}
		}
		if (ok) {
			printf("PASS: exclude=listen,timewait bits\n");
			pass++;
		} else {
			fail++;
		}
	}

	/* Test: include_state=established sets correct bits */
	{
		memset(&filter, 0, sizeof(filter));
		errbuf[0] = '\0';
		int err = tsf_parse_filter_string(
		    "include_state=established",
		    strlen("include_state=established"),
		    &filter, errbuf, sizeof(errbuf));
		int ok = 1;
		if (err != 0) {
			printf("FAIL: include_state: parse error %d\n", err);
			ok = 0;
		} else {
			if (filter.state_mask != (1 << TCPS_ESTABLISHED)) {
				printf("FAIL: include_state: expected "
				    "mask 0x%x, got 0x%x\n",
				    (1 << TCPS_ESTABLISHED),
				    filter.state_mask);
				ok = 0;
			}
			if (!(filter.flags & TSF_STATE_INCLUDE_MODE)) {
				printf("FAIL: include_state: "
				    "TSF_STATE_INCLUDE_MODE not set\n");
				ok = 0;
			}
		}
		if (ok) {
			printf("PASS: include_state=established\n");
			pass++;
		} else {
			fail++;
		}
	}

	/* Test: IPv4 /24 produces correct mask */
	{
		memset(&filter, 0, sizeof(filter));
		errbuf[0] = '\0';
		int err = tsf_parse_filter_string(
		    "local_addr=10.0.0.0/24",
		    strlen("local_addr=10.0.0.0/24"),
		    &filter, errbuf, sizeof(errbuf));
		int ok = 1;
		if (err != 0) {
			printf("FAIL: ipv4 /24: parse error %d (%s)\n",
			    err, errbuf);
			ok = 0;
		} else {
			struct in_addr expected_addr, expected_mask;
			expected_addr.s_addr = htonl(0x0A000000);
			expected_mask.s_addr = htonl(0xFFFFFF00);
			if (filter.local_addr_v4.s_addr !=
			    expected_addr.s_addr) {
				printf("FAIL: ipv4 /24: wrong address\n");
				ok = 0;
			}
			if (filter.local_mask_v4.s_addr !=
			    expected_mask.s_addr) {
				printf("FAIL: ipv4 /24: wrong mask\n");
				ok = 0;
			}
			if (!(filter.flags & TSF_LOCAL_ADDR_MATCH)) {
				printf("FAIL: ipv4 /24: flag not set\n");
				ok = 0;
			}
		}
		if (ok) {
			printf("PASS: ipv4 10.0.0.0/24 value\n");
			pass++;
		} else {
			fail++;
		}
	}

	/* Test: IPv6 ::1 produces correct address */
	{
		memset(&filter, 0, sizeof(filter));
		errbuf[0] = '\0';
		int err = tsf_parse_filter_string(
		    "local_addr=::1",
		    strlen("local_addr=::1"),
		    &filter, errbuf, sizeof(errbuf));
		int ok = 1;
		if (err != 0) {
			printf("FAIL: ipv6 ::1: parse error %d (%s)\n",
			    err, errbuf);
			ok = 0;
		} else {
			struct in6_addr expected;
			memset(&expected, 0, sizeof(expected));
			expected.s6_addr[15] = 1;
			if (memcmp(&filter.local_addr_v6, &expected,
			    sizeof(expected)) != 0) {
				printf("FAIL: ipv6 ::1: wrong address\n");
				ok = 0;
			}
			if (filter.local_prefix_v6 != 128) {
				printf("FAIL: ipv6 ::1: expected prefix 128, "
				    "got %u\n", filter.local_prefix_v6);
				ok = 0;
			}
		}
		if (ok) {
			printf("PASS: ipv6 ::1 value\n");
			pass++;
		} else {
			fail++;
		}
	}

	/* Test: fields=state,rtt,buffers produces correct mask */
	{
		memset(&filter, 0, sizeof(filter));
		errbuf[0] = '\0';
		int err = tsf_parse_filter_string(
		    "fields=state,rtt,buffers",
		    strlen("fields=state,rtt,buffers"),
		    &filter, errbuf, sizeof(errbuf));
		int ok = 1;
		if (err != 0) {
			printf("FAIL: fields: parse error %d\n", err);
			ok = 0;
		} else {
			uint32_t expected = TSR_FIELDS_STATE |
			    TSR_FIELDS_RTT | TSR_FIELDS_BUFFERS;
			if (filter.field_mask != expected) {
				printf("FAIL: fields: expected 0x%x, "
				    "got 0x%x\n", expected, filter.field_mask);
				ok = 0;
			}
		}
		if (ok) {
			printf("PASS: fields=state,rtt,buffers\n");
			pass++;
		} else {
			fail++;
		}
	}

	/* Test: version is set on empty string */
	{
		memset(&filter, 0, sizeof(filter));
		errbuf[0] = '\0';
		int err = tsf_parse_filter_string("", 0,
		    &filter, errbuf, sizeof(errbuf));
		int ok = 1;
		if (err != 0) {
			printf("FAIL: empty version: parse error %d\n", err);
			ok = 0;
		} else if (filter.version != TSF_VERSION) {
			printf("FAIL: empty version: expected %d, got %u\n",
			    TSF_VERSION, filter.version);
			ok = 0;
		}
		if (ok) {
			printf("PASS: empty string sets version\n");
			pass++;
		} else {
			fail++;
		}
	}

	printf("\n%d passed, %d failed, %d total\n",
	    pass, fail, pass + fail);
	return (fail > 0) ? 1 : 0;
}
