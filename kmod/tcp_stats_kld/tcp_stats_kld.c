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
#include <sys/time.h>
#include <netinet/tcp.h>
#include <netinet/tcp_fsm.h>
#include <netinet/tcp_var.h>
#include <netinet/cc/cc.h>

#include "tcp_stats_kld.h"

#ifndef GID_NETWORK
#define	GID_NETWORK	69
#endif

MALLOC_DEFINE(M_TCPSTATS, "tcpstats", "tcp_stats_kld per-fd state");

struct tcpstats_softc {
	struct ucred		*sc_cred;
	uint64_t		sc_gen;
	int			sc_started;
	int			sc_done;
	struct tcpstats_filter	sc_filter;
	int			sc_full;
};

static d_open_t		tcpstats_open;
static d_read_t		tcpstats_read;
static d_ioctl_t	tcpstats_ioctl;

static void		tcpstats_dtor(void *data);

static struct cdev *tcpstats_dev;
static struct cdev *tcpstats_full_dev;

static struct cdevsw tcpstats_cdevsw = {
	.d_version = D_VERSION,
	.d_name    = "tcpstats",
	.d_open    = tcpstats_open,
	.d_read    = tcpstats_read,
	.d_ioctl   = tcpstats_ioctl,
};

static struct cdevsw tcpstats_full_cdevsw = {
	.d_version = D_VERSION,
	.d_name    = "tcpstats-full",
	.d_open    = tcpstats_open,
	.d_read    = tcpstats_read,
	.d_ioctl   = tcpstats_ioctl,
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
	sc->sc_full = (dev->si_devsw == &tcpstats_full_cdevsw);

	error = devfs_set_cdevpriv(sc, tcpstats_dtor);
	if (error != 0) {
		crfree(sc->sc_cred);
		free(sc, M_TCPSTATS);
		return (error);
	}

	return (0);
}

static void
tcpstats_fill_identity(struct tcp_stats_record *rec, struct inpcb *inp)
{
	struct tcpcb *tp;
	struct socket *so;

	rec->tsr_version = TCP_STATS_VERSION;
	rec->tsr_len = TCP_STATS_RECORD_SIZE;

	/* AF and addresses */
	if (inp->inp_vflag & INP_IPV6) {
		rec->tsr_af = AF_INET6;
		rec->tsr_flags |= TSR_F_IPV6;
		rec->tsr_local_addr.v6 = inp->inp_inc.inc6_laddr;
		rec->tsr_remote_addr.v6 = inp->inp_inc.inc6_faddr;
	} else {
		rec->tsr_af = AF_INET;
		rec->tsr_local_addr.v4 = inp->inp_inc.inc_laddr;
		rec->tsr_remote_addr.v4 = inp->inp_inc.inc_faddr;
	}

	rec->tsr_local_port = ntohs(inp->inp_lport);
	rec->tsr_remote_port = ntohs(inp->inp_fport);

	/* TCP state from tcpcb */
	tp = intotcpcb(inp);
	if (tp != NULL) {
		rec->tsr_state = tp->t_state;
		rec->tsr_flags_tcp = tp->t_flags;
		if (tp->t_state == TCPS_LISTEN)
			rec->tsr_flags |= TSR_F_LISTEN;
	}

	/* Socket metadata */
	so = inp->inp_socket;
	if (so != NULL) {
		rec->tsr_so_addr = (uint64_t)(uintptr_t)so;
		if (so->so_cred != NULL)
			rec->tsr_uid = so->so_cred->cr_uid;
	}

	rec->tsr_inp_gencnt = inp->inp_gencnt;
}

/*
 * Fill RTT, sequence numbers, congestion, and window fields.
 * Replicates tcp_fill_info() logic since that function is static
 * on FreeBSD 15.0-RELEASE.
 */
static void
tcpstats_fill_record(struct tcp_stats_record *rec, struct inpcb *inp)
{
	struct tcpcb *tp;

	tcpstats_fill_identity(rec, inp);

	tp = intotcpcb(inp);
	if (tp == NULL || tp->t_state == TCPS_LISTEN)
		return;

	/* RTT -- replicate tcp_fill_info() conversion to usec */
	rec->tsr_rtt = ((uint64_t)tp->t_srtt * tick) >> TCP_RTT_SHIFT;
	rec->tsr_rttvar = ((uint64_t)tp->t_rttvar * tick) >> TCP_RTTVAR_SHIFT;
	rec->tsr_rto = tp->t_rxtcur * tick;
	rec->tsr_rttmin = tp->t_rttlow;

	/* Window scale and options */
	if ((tp->t_flags & TF_REQ_SCALE) && (tp->t_flags & TF_RCVD_SCALE)) {
		rec->tsr_options |= TCPI_OPT_WSCALE;
		rec->tsr_snd_wscale = tp->snd_scale;
		rec->tsr_rcv_wscale = tp->rcv_scale;
	}
	if ((tp->t_flags & TF_REQ_TSTMP) && (tp->t_flags & TF_RCVD_TSTMP))
		rec->tsr_options |= TCPI_OPT_TIMESTAMPS;
	if (tp->t_flags & TF_SACK_PERMIT)
		rec->tsr_options |= TCPI_OPT_SACK;

	/* Sequence numbers */
	rec->tsr_snd_nxt = tp->snd_nxt;
	rec->tsr_snd_una = tp->snd_una;
	rec->tsr_snd_max = tp->snd_max;
	rec->tsr_rcv_nxt = tp->rcv_nxt;
	rec->tsr_rcv_adv = tp->rcv_adv;

	/* Congestion */
	rec->tsr_snd_cwnd = tp->snd_cwnd;
	rec->tsr_snd_ssthresh = tp->snd_ssthresh;
	rec->tsr_snd_wnd = tp->snd_wnd;
	rec->tsr_rcv_wnd = tp->rcv_wnd;
	rec->tsr_maxseg = tp->t_maxseg;

	/* CC algo and TCP stack names */
	if (CC_ALGO(tp) != NULL)
		strlcpy(rec->tsr_cc, CC_ALGO(tp)->name,
		    sizeof(rec->tsr_cc));
	if (tp->t_fb != NULL)
		strlcpy(rec->tsr_stack, tp->t_fb->tfb_tcp_block_name,
		    sizeof(rec->tsr_stack));

	/* Counters */
	rec->tsr_snd_rexmitpack = tp->t_sndrexmitpack;
	rec->tsr_rcv_ooopack = tp->t_rcvoopack;
	rec->tsr_snd_zerowin = tp->t_sndzerowin;
	rec->tsr_dupacks = tp->t_dupacks;
	rec->tsr_rcv_numsacks = tp->rcv_numsacks;

	/* ECN */
	if ((tp->t_flags2 & (TF2_ECN_PERMIT | TF2_ACE_PERMIT)) ==
	    (TF2_ECN_PERMIT | TF2_ACE_PERMIT))
		rec->tsr_delivered_ce = tp->t_scep - 5;
	else
		rec->tsr_delivered_ce = tp->t_scep;
	rec->tsr_received_ce = tp->t_rcep;
	rec->tsr_ecn = (tp->t_flags2 & TF2_ECN_PERMIT) ? 1 : 0;

	/* DSACK */
	rec->tsr_dsack_bytes = tp->t_dsack_bytes;
	rec->tsr_dsack_pack = tp->t_dsack_pack;

	/* TLP */
	rec->tsr_total_tlp = tp->t_sndtlppack;
	rec->tsr_total_tlp_bytes = tp->t_sndtlpbyte;

	/* Timers -- remaining time in ms, 0 if not running */
	{
		sbintime_t now = getsbinuptime();
		int i;
		int32_t *timer_fields[] = {
			&rec->tsr_tt_rexmt,
			&rec->tsr_tt_persist,
			&rec->tsr_tt_keep,
			&rec->tsr_tt_2msl,
			&rec->tsr_tt_delack,
		};
		for (i = 0; i < TT_N; i++) {
			if (tp->t_timers[i] == SBT_MAX ||
			    tp->t_timers[i] == 0)
				*timer_fields[i] = 0;
			else
				*timer_fields[i] = (int32_t)(
				    (tp->t_timers[i] - now) / SBT_1MS);
		}
	}
	rec->tsr_rcvtime = ((uint32_t)ticks - tp->t_rcvtime) * tick / 1000;

	/* Buffer utilization */
	{
		struct socket *so = inp->inp_socket;
		if (so != NULL) {
			rec->tsr_snd_buf_cc = so->so_snd.sb_ccc;
			rec->tsr_snd_buf_hiwat = so->so_snd.sb_hiwat;
			rec->tsr_rcv_buf_cc = so->so_rcv.sb_ccc;
			rec->tsr_rcv_buf_hiwat = so->so_rcv.sb_hiwat;
		}
	}
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

		/* State filtering. */
		{
			struct tcpcb *tp = intotcpcb(inp);
			if (tp != NULL) {
				if (sc->sc_filter.state_mask != 0xFFFF &&
				    !(sc->sc_filter.state_mask &
				    (1 << tp->t_state)))
					continue;
				if ((sc->sc_filter.flags &
				    TSF_EXCLUDE_LISTEN) &&
				    tp->t_state == TCPS_LISTEN)
					continue;
				if ((sc->sc_filter.flags &
				    TSF_EXCLUDE_TIMEWAIT) &&
				    tp->t_state == TCPS_TIME_WAIT)
					continue;
			}
		}

		bzero(&rec, sizeof(rec));
		tcpstats_fill_record(&rec, inp);

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

static int
tcpstats_ioctl(struct cdev *dev, u_long cmd, caddr_t data, int fflag,
    struct thread *td)
{
	struct tcpstats_softc *sc;
	int error;

	error = devfs_get_cdevpriv((void **)&sc);
	if (error != 0)
		return (error);

	switch (cmd) {
	case TCPSTATS_VERSION_CMD:
	{
		struct tcpstats_version *ver = (struct tcpstats_version *)data;

		CURVNET_SET(TD_TO_VNET(td));
		ver->protocol_version = TCP_STATS_VERSION;
		ver->record_size = TCP_STATS_RECORD_SIZE;
		ver->record_count_hint = V_tcbinfo.ipi_count;
		ver->flags = 0;
		CURVNET_RESTORE();
		return (0);
	}
	case TCPSTATS_SET_FILTER:
	{
		struct tcpstats_filter *filt = (struct tcpstats_filter *)data;

		sc->sc_filter = *filt;
		return (0);
	}
	case TCPSTATS_RESET:
		sc->sc_done = 0;
		return (0);
	default:
		return (ENOTTY);
	}
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
		    &tcpstats_cdevsw, 0, NULL, UID_ROOT, GID_NETWORK,
		    0440, "tcpstats");
		if (tcpstats_dev == NULL) {
			printf("tcp_stats_kld: make_dev_credf failed\n");
			return (ENXIO);
		}
		tcpstats_full_dev = make_dev_credf(MAKEDEV_ETERNAL_KLD,
		    &tcpstats_full_cdevsw, 0, NULL, UID_ROOT, GID_NETWORK,
		    0440, "tcpstats-full");
		if (tcpstats_full_dev == NULL) {
			destroy_dev(tcpstats_dev);
			tcpstats_dev = NULL;
			printf("tcp_stats_kld: make_dev_credf (full) failed\n");
			return (ENXIO);
		}
		printf("tcp_stats_kld: loaded\n");
		return (0);
	case MOD_UNLOAD:
		if (tcpstats_full_dev != NULL)
			destroy_dev(tcpstats_full_dev);
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
