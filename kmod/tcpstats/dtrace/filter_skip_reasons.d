#!/usr/sbin/dtrace -s
/*
 * filter_skip_reasons.d — Count of filter skip reasons by category.
 *
 * Reason codes (from tcp_statsdev.c):
 *   0=gencnt  1=cred  2=ipver  3=state  4=port  5=addr  6=timeout
 *
 * Usage: dtrace -s filter_skip_reasons.d
 *        (Ctrl-C to stop and print counts)
 */

tcpstats:::filter-skip
{
	@reasons[arg1 == 0 ? "gencnt" :
	         arg1 == 1 ? "cred" :
	         arg1 == 2 ? "ipver" :
	         arg1 == 3 ? "state" :
	         arg1 == 4 ? "port" :
	         arg1 == 5 ? "addr" :
	         arg1 == 6 ? "timeout" : "unknown"] = count();
}
