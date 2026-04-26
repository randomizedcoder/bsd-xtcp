#!/usr/sbin/dtrace -s
/*
 * fill_time.d — Histogram of per-record fill time in nanoseconds.
 *
 * The fill-done probe fires after each tcp_stats_record is populated.
 * arg0 = elapsed nanoseconds, arg1 = record size in bytes.
 *
 * Usage: dtrace -s fill_time.d
 *        (Ctrl-C to stop and print histogram)
 */

tcpstats:::fill-done
{
	@fill_ns = quantize(arg0);
}
