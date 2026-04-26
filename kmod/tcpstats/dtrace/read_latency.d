#!/usr/sbin/dtrace -s
/*
 * read_latency.d — Histogram of read() call duration in microseconds.
 *
 * Usage: dtrace -s read_latency.d
 *        (Ctrl-C to stop and print histogram)
 */

tcpstats:::read-entry
{
	self->ts = timestamp;
}

tcpstats:::read-done
/self->ts/
{
	@read_us = quantize((timestamp - self->ts) / 1000);
	self->ts = 0;
}
