#!/usr/sbin/dtrace -s
/*
 * read_summary.d — Per-read summary: records emitted, elapsed time, errors.
 *
 * Prints a line for each read() completion and aggregates totals.
 *
 * Usage: dtrace -s read_summary.d
 *        (Ctrl-C to stop and print aggregations)
 */

tcpstats:::read-entry
{
	self->ts = timestamp;
}

tcpstats:::read-done
/self->ts/
{
	printf("read: error=%d records=%d elapsed_ns=%d\n",
	    arg0, arg1, arg2);
	@reads = count();
	@records = sum(arg1);
	@latency_us = quantize((timestamp - self->ts) / 1000);
	self->ts = 0;
}
