#!/usr/sbin/dtrace -s
/*
 * all_probes.d — Verbose trace of every tcpstats probe firing.
 *
 * Useful for debugging probe arguments and verifying that probes
 * fire as expected. Produces high-volume output under load.
 *
 * Usage: dtrace -s all_probes.d
 *        (Ctrl-C to stop)
 */

tcpstats:::read-entry
{
	printf("read-entry: resid=%d flags=0x%x\n", arg0, arg1);
}

tcpstats:::read-done
{
	printf("read-done: err=%d records=%d ns=%d\n", arg0, arg1, arg2);
}

tcpstats:::filter-skip
{
	printf("filter-skip: inp=%p reason=%d\n", arg0, arg1);
}

tcpstats:::filter-match
{
	printf("filter-match: inp=%p\n", arg0);
}

tcpstats:::fill-done
{
	printf("fill-done: ns=%d size=%d\n", arg0, arg1);
}

tcpstats:::profile-create
{
	printf("profile-create: %s\n", copyinstr(arg0));
}

tcpstats:::profile-destroy
{
	printf("profile-destroy: %s\n", copyinstr(arg0));
}
