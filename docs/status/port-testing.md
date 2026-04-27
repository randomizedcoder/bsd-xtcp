# FreeBSD Port Testing

## Overview

Two FreeBSD ports are maintained in [randomizedcoder/freebsd-ports](https://github.com/randomizedcoder/freebsd-ports) and submitted via [freebsd/freebsd-ports#497](https://github.com/freebsd/freebsd-ports/pull/497):

| Port | Description | USES |
|------|-------------|------|
| `net/tcpstats-kmod` | Kernel module providing `/dev/tcpstats` character device | `kmod uidfix` |
| `net/tcpstats-reader` | Rust CLI that reads from `/dev/tcpstats` and outputs JSON/protobuf | `cargo` |

Both ports use `USE_GITHUB=yes` pointing at this repo (`randomizedcoder/bsd-xtcp`) with `DISTVERSION=1.0.2`.

## Test infrastructure

Automated port build testing is implemented in `nix/port-test.nix` and runs against 3 FreeBSD VMs:

| VM | IP | FreeBSD Version |
|----|-----|----------------|
| freebsd150 | 192.168.122.41 | 15.0-RELEASE |
| freebsd144 | 192.168.122.85 | 14.4-RELEASE |
| freebsd143 | 192.168.122.27 | 14.3-RELEASE |

### Running the tests

```sh
# Test on all 3 VMs
nix run .#port-test-freebsd

# Test on a single VM
nix run .#port-test-freebsd150
nix run .#port-test-freebsd144
nix run .#port-test-freebsd143

# Override the port source directory
PORT_SRC=../freebsd-ports/net/tcpstats-kmod nix run .#port-test-freebsd144
```

### Test phases (tcpstats-kmod)

The automated test script runs these phases sequentially:

| Phase | What it does |
|-------|-------------|
| A. Ports tree | Ensures `/usr/ports/Mk` exists, installs via git if missing |
| B. Rsync | Syncs port files from host to VM |
| C. makesum | Regenerates `distinfo` (fetches tarball, computes checksums) |
| D. portlint | Runs `portlint -AC` for style/correctness checks |
| E. Build | `make clean stage`, `make stage-qa`, `make check-plist` |
| F. Install | `make install`, `kldload tcpstats`, verify `/dev/tcpstats` exists |
| G. Package | `make package`, `make deinstall` |
| H. Options | Rebuild with `DTRACE`, `STATS`, and `DTRACE+STATS` variants |

### Test phases (tcpstats-reader)

The reader port was tested manually on all 3 VMs through the same phases (excluding kmod-specific steps):

| Phase | What it does |
|-------|-------------|
| stage | `make BATCH=yes stage` (full Rust compilation with 112 crate dependencies) |
| stage-qa | Verify staging correctness |
| check-plist | Verify `PLIST_FILES=bin/tcpstats-reader` matches staged files |
| install | `make BATCH=yes install` |
| verify | `tcpstats-reader --help` produces usage output |
| package | `make BATCH=yes package` |
| deinstall | `make BATCH=yes deinstall` |

## Test results (v1.0.2)

### tcpstats-kmod

| Phase | FreeBSD 15.0 | FreeBSD 14.4 | FreeBSD 14.3 |
|-------|:---:|:---:|:---:|
| makesum | PASS | PASS | PASS |
| portlint | PASS | PASS | PASS |
| stage | PASS | PASS | PASS |
| stage-qa | PASS | PASS | PASS |
| check-plist | PASS | PASS | PASS |
| install | PASS | PASS | PASS |
| kldload + /dev/tcpstats | PASS | PASS | PASS |
| package | PASS | PASS | PASS |
| deinstall | PASS | PASS | PASS |
| DTRACE variant | PASS | PASS | PASS |
| STATS variant | PASS | PASS | PASS |
| DTRACE+STATS variant | PASS | PASS | PASS |

### tcpstats-reader

| Phase | FreeBSD 15.0 | FreeBSD 14.4 | FreeBSD 14.3 |
|-------|:---:|:---:|:---:|
| stage | PASS | PASS | PASS |
| stage-qa | PASS | PASS | PASS |
| check-plist | PASS | PASS | PASS |
| install | PASS | PASS | PASS |
| --help | PASS | PASS | PASS |
| package | PASS | PASS | PASS |
| deinstall | PASS | PASS | PASS |

## Bugs found and fixed

### v1.0.0 tarball directory mismatch

The port Makefile referenced `WRKSRC_SUBDIR=kmod/tcpstats` but the v1.0.0 tarball still had the pre-rename directory `kmod/tcp_stats_kld`. Fixed by tagging v1.0.1 (then v1.0.2) after the upstream rename was complete.

### Man page staging

The kernel module's upstream Makefile declares `MAN=man/tcpstats.4` but FreeBSD's `bsd.kmod.mk` staging mechanism did not automatically install it to `STAGEDIR`. Fixed by adding a `post-install` target:

```makefile
post-install:
	@${MKDIR} ${STAGEDIR}${PREFIX}/share/man/man4
	${INSTALL_MAN} ${WRKSRC}/man/tcpstats.4 ${STAGEDIR}${PREFIX}/share/man/man4/
```

### Man page directory

Initial fix installed to `${PREFIX}/man/man4/` but `stage-qa` warned: "Installing man files in /usr/local/man is no longer supported." Modern FreeBSD requires `${PREFIX}/share/man/man4/`.

### DTrace scripts not in tarball

The `.gitignore` pattern `*.d` was hiding DTrace scripts (`kmod/tcpstats/dtrace/*.d`) from git. Fixed by adding a negation rule: `!kmod/tcpstats/dtrace/*.d`.

### Interactive options dialog blocking builds

`make stage` launched an ncurses dialog for port options. Fixed by adding `BATCH=yes` to all `make` commands in the test script.

### Cargo git discovery in ports tree

When building the Rust `tcpstats-reader` port, cargo's bundled libgit2 walked up from `WRKSRC` and discovered `/usr/ports/.git` (a shallow git clone with an incompatible index format), causing "Signature mismatch" errors. Fixed by creating a minimal `.git` in `WRKSRC` during extraction:

```makefile
CARGO_ENV+=	GIT_CEILING_DIRECTORIES=${WRKDIR}

post-extract:
	@${MKDIR} ${WRKSRC}/.git/refs ${WRKSRC}/.git/objects
	@echo "ref: refs/heads/main" > ${WRKSRC}/.git/HEAD
```

## Port Makefiles

### net/tcpstats-kmod/Makefile

```makefile
PORTNAME=	tcpstats
DISTVERSIONPREFIX=	v
DISTVERSION=	1.0.2
CATEGORIES=	net
PKGNAMESUFFIX=	-kmod

MAINTAINER=	dave.seddon.ca@gmail.com
COMMENT=	Kernel module for system-wide TCP socket statistics
WWW=		https://github.com/randomizedcoder/bsd-xtcp

LICENSE=	MIT
LICENSE_FILE=	${WRKSRC}/../../LICENSE

USES=		kmod uidfix
USE_GITHUB=	yes
GH_ACCOUNT=	randomizedcoder
GH_PROJECT=	bsd-xtcp

WRKSRC_SUBDIR=	kmod/tcpstats

PLIST_FILES=	${KMODDIR}/tcpstats.ko \
		share/man/man4/tcpstats.4.gz

OPTIONS_DEFINE=		DTRACE STATS

DTRACE_DESC=	Enable DTrace SDT probes
STATS_DESC=	Enable per-socket statistics counters

DTRACE_CFLAGS=	-DTCPSTATS_DTRACE -DKDTRACE_HOOKS
STATS_CFLAGS=	-DTCPSTATS_STATS

DTRACE_PLIST_FILES=	share/examples/tcpstats/all_probes.d \
			share/examples/tcpstats/fill_time.d \
			share/examples/tcpstats/filter_skip_reasons.d \
			share/examples/tcpstats/read_latency.d \
			share/examples/tcpstats/read_summary.d

post-install:
	@${MKDIR} ${STAGEDIR}${PREFIX}/share/man/man4
	${INSTALL_MAN} ${WRKSRC}/man/tcpstats.4 ${STAGEDIR}${PREFIX}/share/man/man4/

post-install-DTRACE-on:
	@${MKDIR} ${STAGEDIR}${PREFIX}/share/examples/tcpstats
	${INSTALL_DATA} ${WRKSRC}/dtrace/all_probes.d ${STAGEDIR}${PREFIX}/share/examples/tcpstats/
	${INSTALL_DATA} ${WRKSRC}/dtrace/fill_time.d ${STAGEDIR}${PREFIX}/share/examples/tcpstats/
	${INSTALL_DATA} ${WRKSRC}/dtrace/filter_skip_reasons.d ${STAGEDIR}${PREFIX}/share/examples/tcpstats/
	${INSTALL_DATA} ${WRKSRC}/dtrace/read_latency.d ${STAGEDIR}${PREFIX}/share/examples/tcpstats/
	${INSTALL_DATA} ${WRKSRC}/dtrace/read_summary.d ${STAGEDIR}${PREFIX}/share/examples/tcpstats/

.include <bsd.port.mk>
```

### net/tcpstats-reader/Makefile (key sections)

```makefile
PORTNAME=	tcpstats-reader
DISTVERSIONPREFIX=	v
DISTVERSION=	1.0.2
CATEGORIES=	net

MAINTAINER=	dave.seddon.ca@gmail.com
COMMENT=	TCP socket statistics reader for FreeBSD tcpstats kernel module
WWW=		https://github.com/randomizedcoder/bsd-xtcp

LICENSE=	MIT
LICENSE_FILE=	${WRKSRC}/LICENSE

BUILD_DEPENDS=	protobuf>=3.0:devel/protobuf

USES=		cargo
USE_GITHUB=	yes
GH_ACCOUNT=	randomizedcoder
GH_PROJECT=	bsd-xtcp
CARGO_ENV+=	GIT_CEILING_DIRECTORIES=${WRKDIR}

CARGO_CRATES=	... (112 crates)

post-extract:
	@${MKDIR} ${WRKSRC}/.git/refs ${WRKSRC}/.git/objects
	@echo "ref: refs/heads/main" > ${WRKSRC}/.git/HEAD

PLIST_FILES=	bin/${PORTNAME}

.include <bsd.port.mk>
```

## File inventory

| File | Repo | Purpose |
|------|------|---------|
| `net/tcpstats-kmod/Makefile` | freebsd-ports | Kernel module port |
| `net/tcpstats-kmod/distinfo` | freebsd-ports | Tarball checksums |
| `net/tcpstats-kmod/pkg-descr` | freebsd-ports | Port description |
| `net/tcpstats-reader/Makefile` | freebsd-ports | Rust client port |
| `net/tcpstats-reader/distinfo` | freebsd-ports | Tarball + crate checksums |
| `net/tcpstats-reader/pkg-descr` | freebsd-ports | Port description |
| `net/Makefile` | freebsd-ports | SUBDIR registration for both ports |
| `nix/port-test.nix` | bsd-xtcp | Automated port build testing |
