/*
 * fuzz_filter_parse.c — AFL++/libFuzzer dual harness for the filter parser.
 *
 * Build with AFL++ (inside nix develop):
 *   afl-clang-fast -o fuzz_filter test/fuzz_filter_parse.c \
 *       tcp_stats_filter_parse.c -I.
 *   mkdir -p seeds findings
 *   echo "local_port=443" > seeds/basic
 *   echo "local_port=443,8443 exclude=listen,timewait" > seeds/combo
 *   echo "local_addr=10.0.0.0/24 ipv4_only" > seeds/cidr
 *   echo "remote_addr=fe80::/10 local_port=443" > seeds/ipv6
 *   echo "include_state=established format=full fields=all" > seeds/full
 *   afl-fuzz -i seeds/ -o findings/ -- ./fuzz_filter
 *
 * Build with libFuzzer (clang):
 *   clang -fsanitize=fuzzer,address -o fuzz_filter \
 *       test/fuzz_filter_parse.c tcp_stats_filter_parse.c -I.
 *   ./fuzz_filter -max_len=512 -runs=10000000 corpus/
 */

#include <stdint.h>
#include <string.h>
#include <unistd.h>

#include "../tcp_stats_filter_parse.h"

#ifdef __AFL_FUZZ_TESTCASE_LEN
/* AFL++ persistent mode */
__AFL_FUZZ_INIT();

int
main(void)
{
	__AFL_INIT();
	unsigned char *buf = __AFL_FUZZ_TESTCASE_BUF;

	while (__AFL_LOOP(1000)) {
		int len = __AFL_FUZZ_TESTCASE_LEN;
		if (len > 0 && len < TSF_PARSE_MAXLEN) {
			char input[TSF_PARSE_MAXLEN];
			memcpy(input, buf, len);
			input[len] = '\0';

			struct tcpstats_filter filter;
			char errbuf[TSF_ERRBUF_SIZE];
			tsf_parse_filter_string(input, len,
			    &filter, errbuf, sizeof(errbuf));
		}
	}
	return 0;
}

#else
/* libFuzzer entry point */
int
LLVMFuzzerTestOneInput(const uint8_t *data, size_t size)
{
	if (size == 0 || size >= TSF_PARSE_MAXLEN)
		return 0;

	char input[TSF_PARSE_MAXLEN];
	memcpy(input, data, size);
	input[size] = '\0';

	struct tcpstats_filter filter;
	char errbuf[TSF_ERRBUF_SIZE];
	tsf_parse_filter_string(input, size,
	    &filter, errbuf, sizeof(errbuf));
	return 0;
}
#endif
