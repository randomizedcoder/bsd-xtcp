#include <sys/param.h>
#include <sys/module.h>
#include <sys/kernel.h>
#include <sys/systm.h>
#include <sys/conf.h>
#include <netinet/in.h>

#include "tcp_stats_kld.h"

static struct cdev *tcpstats_dev;

static struct cdevsw tcpstats_cdevsw = {
	.d_version = D_VERSION,
	.d_name    = "tcpstats",
};

static int
tcp_stats_kld_modevent(module_t mod, int type, void *arg)
{

	switch (type) {
	case MOD_LOAD:
		tcpstats_dev = make_dev_credf(MAKEDEV_ETERNAL_KLD,
		    &tcpstats_cdevsw, 0, NULL, UID_ROOT, GID_WHEEL,
		    0444, "tcpstats");
		if (tcpstats_dev == NULL) {
			printf("tcp_stats_kld: make_dev_credf failed\n");
			return (ENXIO);
		}
		printf("tcp_stats_kld: loaded\n");
		return (0);
	case MOD_UNLOAD:
		if (tcpstats_dev != NULL)
			destroy_dev(tcpstats_dev);
		printf("tcp_stats_kld: unloaded\n");
		return (0);
	default:
		return (EOPNOTSUPP);
	}
}

static moduledata_t tcp_stats_kld_mod = {
	"tcp_stats_kld",
	tcp_stats_kld_modevent,
	NULL
};

DECLARE_MODULE(tcp_stats_kld, tcp_stats_kld_mod, SI_SUB_DRIVERS, SI_ORDER_MIDDLE);
