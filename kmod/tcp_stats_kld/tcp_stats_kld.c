#include <sys/param.h>
#include <sys/module.h>
#include <sys/kernel.h>
#include <sys/systm.h>
#include <sys/conf.h>
#include <sys/fcntl.h>
#include <sys/malloc.h>
#include <sys/proc.h>
#include <sys/uio.h>
#include <sys/ucred.h>
#include <sys/jail.h>
#include <sys/socket.h>
#include <sys/socketvar.h>
#include <net/vnet.h>
#include <netinet/in.h>
#include <netinet/in_systm.h>
#include <netinet/in_pcb.h>
#include <netinet/tcp_var.h>

#include "tcp_stats_kld.h"

MALLOC_DEFINE(M_TCPSTATS, "tcpstats", "tcp_stats_kld per-fd state");

struct tcpstats_softc {
	struct ucred		*sc_cred;
	uint64_t		sc_gen;
	int			sc_started;
	int			sc_done;
	struct tcpstats_filter	sc_filter;
};

static d_open_t		tcpstats_open;
static d_read_t		tcpstats_read;

static void		tcpstats_dtor(void *data);

static struct cdev *tcpstats_dev;

static struct cdevsw tcpstats_cdevsw = {
	.d_version = D_VERSION,
	.d_name    = "tcpstats",
	.d_open    = tcpstats_open,
	.d_read    = tcpstats_read,
};

static int
tcpstats_open(struct cdev *dev, int oflags, int devtype, struct thread *td)
{
	struct tcpstats_softc *sc;
	int error;

	if (oflags & FWRITE)
		return (EPERM);

	sc = malloc(sizeof(*sc), M_TCPSTATS, M_WAITOK | M_ZERO);
	sc->sc_cred = crhold(td->td_ucred);
	sc->sc_filter.state_mask = 0xFFFF;

	error = devfs_set_cdevpriv(sc, tcpstats_dtor);
	if (error != 0) {
		crfree(sc->sc_cred);
		free(sc, M_TCPSTATS);
		return (error);
	}

	return (0);
}

static int
tcpstats_read(struct cdev *dev, struct uio *uio, int ioflag)
{
	struct tcpstats_softc *sc;
	struct tcp_stats_record rec;
	struct inpcb *inp;
	uint64_t gencnt;
	int error;

	error = devfs_get_cdevpriv((void **)&sc);
	if (error != 0)
		return (error);

	if (sc->sc_done)
		return (0);

	CURVNET_SET(TD_TO_VNET(curthread));

	struct inpcb_iterator inpi = INP_ALL_ITERATOR(&V_tcbinfo,
	    INPLOOKUP_RLOCKPCB);
	gencnt = V_tcbinfo.ipi_gencnt;

	while ((inp = inp_next(&inpi)) != NULL) {
		if (uio->uio_resid < (ssize_t)sizeof(rec)) {
			INP_RUNLOCK(inp);
			break;
		}

		/* Skip entries added after our snapshot. */
		if (inp->inp_gencnt > gencnt)
			continue;

		/* Credential visibility check. */
		if (cr_canseeinpcb(sc->sc_cred, inp) != 0)
			continue;

		bzero(&rec, sizeof(rec));
		rec.tsr_version = TCP_STATS_VERSION;
		rec.tsr_len = TCP_STATS_RECORD_SIZE;

		if (inp->inp_vflag & INP_IPV6) {
			rec.tsr_af = AF_INET6;
			rec.tsr_flags |= TSR_F_IPV6;
		} else {
			rec.tsr_af = AF_INET;
		}

		rec.tsr_local_port = ntohs(inp->inp_lport);
		rec.tsr_remote_port = ntohs(inp->inp_fport);

		error = uiomove(&rec, sizeof(rec), uio);
		if (error != 0) {
			INP_RUNLOCK(inp);
			CURVNET_RESTORE();
			return (error);
		}
	}

	CURVNET_RESTORE();

	sc->sc_done = 1;
	return (0);
}

static void
tcpstats_dtor(void *data)
{
	struct tcpstats_softc *sc = data;

	crfree(sc->sc_cred);
	free(sc, M_TCPSTATS);
}

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
