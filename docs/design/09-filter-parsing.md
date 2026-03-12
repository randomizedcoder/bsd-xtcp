# Filter String Parsing Design for `tcp_stats_kld`

[← Back to README](../../README.md) | [Back to kernel module design](05-kernel-module.md)

## 1. Overview & Architectural Decision

The `tcp_stats_kld` module supports named filter profiles created via sysctl:

```sh
sysctl dev.tcpstats.profiles.cdn_clients="local_port=443 exclude=listen,timewait"
```

The filter string is **parsed in kernel space** at sysctl write time. This
is a deliberate architectural choice over a userspace helper.

### Why Kernel-Side Parsing

| Approach | Pros | Cons |
|---|---|---|
| **Kernel parser** (chosen) | Direct `sysctl` from shell, works in `/etc/sysctl.conf`, Ansible `sysctl:` module, no helper binary needed | Parser complexity in kernel, must be hardened |
| **Userspace helper** | Parser in safe language (Rust), easier testing | Requires helper binary installed, can't use `sysctl.conf` directly, two-step workflow |
| **Binary-only ioctl** | No parsing at all, struct passed directly | Unusable from shell, requires custom tool for every operation |

The kernel parser runs **only on the sysctl write path** — a low-frequency
administrative operation (profile creation/deletion). It is never invoked
on the hot data path (socket iteration, record emission). The parser
processes at most 512 bytes of input at most 16 times (max profiles), so
even an inefficient parser has negligible performance impact.

For programmatic use (the Rust `bsd-xtcp` tool), the binary
`tcpstats_filter` struct is available via the `TCPSTATS_SET_FILTER` ioctl,
bypassing the string parser entirely.

---

## 2. Formal Filter Grammar (EBNF)

```ebnf
filter_string  = { directive } ;

directive      = key_value | flag ;

key_value      = key , "=" , value ;
flag           = "ipv4_only" | "ipv6_only" ;

key            = "local_port" | "remote_port" | "exclude"
               | "include_state" | "local_addr" | "remote_addr"
               | "format" | "fields" ;

value          = list_value | single_value ;
list_value     = single_value , { "," , single_value } ;
single_value   = port_number | state_name | field_group
               | addr_cidr | format_name ;

port_number    = nonzero_digit , { digit } ;          (* 1-65535, no leading zeros *)
nonzero_digit  = "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" ;
digit          = "0" | nonzero_digit ;

state_name     = "listen" | "syn_sent" | "syn_received" | "established"
               | "close_wait" | "fin_wait_1" | "fin_wait_2" | "closing"
               | "last_ack" | "time_wait" | "closed" ;

field_group    = "identity" | "state" | "congestion" | "rtt" | "sequences"
               | "counters" | "timers" | "buffers" | "ecn" | "names"
               | "all" | "default" ;

addr_cidr      = ipv4_cidr | ipv6_cidr ;
ipv4_cidr      = ipv4_addr , [ "/" , prefix_len_v4 ] ;
ipv6_cidr      = ipv6_addr , [ "/" , prefix_len_v6 ] ;

ipv4_addr      = octet , "." , octet , "." , octet , "." , octet ;
octet          = digit , [ digit , [ digit ] ] ;       (* 0-255 *)

ipv6_addr      = (* RFC 5952 canonical form, see section 6 *) ;
prefix_len_v4  = digit , [ digit ] ;                   (* 0-32 *)
prefix_len_v6  = digit , [ digit , [ digit ] ] ;       (* 0-128 *)

format_name    = "compact" | "full" ;
```

### Lexical Rules

- **Whitespace**: Directives are separated by one or more ASCII space characters
  (0x20) or horizontal tabs (0x09). Leading and trailing whitespace is stripped.
- **Case**: All keywords are **case-insensitive**. Internally normalized to
  lowercase during tokenization. Address strings preserve case (hex digits).
- **Quoting**: No quoting mechanism. Values must not contain spaces, `=`, or `,`.
- **Comments**: Not supported (this is a kernel sysctl value, not a config file).
- **Maximum length**: 512 bytes total (including NUL terminator).
- **Maximum directives**: 16 per filter string.
- **No port ranges**: Port values are explicit numbers only (no `1000-2000`).
  Operators list ports individually; `TSF_MAX_PORTS=8` per direction is sufficient.

### Examples

```sh
# CDN client-facing connections
"local_port=443 exclude=listen,timewait"

# Upstream origin fetches with CIDR source filter
"remote_port=443,80 local_addr=10.0.1.0/24 exclude=listen,timewait"

# IPv6-only monitoring
"ipv6_only local_port=443,8443 exclude=listen,timewait,closewait"

# Database connections (PostgreSQL + MySQL)
"remote_port=5432,3306 fields=state,congestion,rtt,buffers"

# Dual-stack microservice mesh
"local_port=8080,8443 remote_port=8080,8443 exclude=listen,timewait format=full"

# Full-format debugging on a specific subnet
"local_addr=192.168.1.0/24 format=full fields=all"

# Include only ESTABLISHED state (positive match)
"include_state=established"
```

---

## 3. Directive Reference Table

### 3.1 Complete Directive Reference

| Directive | Syntax | Struct field(s) | Flag set | Notes |
|---|---|---|---|---|
| `local_port` | `local_port=443` or `local_port=443,8443` | `local_ports[]` | `TSF_LOCAL_PORT_MATCH` | Max 8 ports, network byte order, no duplicates |
| `remote_port` | `remote_port=80,443` | `remote_ports[]` | `TSF_REMOTE_PORT_MATCH` | Max 8 ports, network byte order, no duplicates |
| `exclude` | `exclude=listen,timewait` | `exclude_states` | `TSF_EXCLUDE_*` per state | Sets per-state exclude flags; clears corresponding bits in `state_mask` |
| `include_state` | `include_state=established` | `state_mask` | `TSF_STATE_INCLUDE_MODE` | Positive match — only listed states are included. Mutually exclusive with `exclude`. |
| `local_addr` | `local_addr=10.0.0.1` or `local_addr=10.0.0.0/24` | `local_addr_v4` + `local_mask_v4` or `local_addr_v6` + `local_prefix_v6` | `TSF_LOCAL_ADDR_MATCH` | CIDR or exact match. AF detected from format. |
| `remote_addr` | `remote_addr=fe80::/10` | `remote_addr_v6` + `remote_prefix_v6` | `TSF_REMOTE_ADDR_MATCH` | CIDR or exact match |
| `format` | `format=compact` or `format=full` | `format` | — | Selects record format (128 or 320 bytes) |
| `fields` | `fields=state,rtt,congestion` | `field_mask` | — | Comma-separated field group names, OR'd into bitmask |
| `ipv4_only` | `ipv4_only` (bare flag) | — | `TSF_IPV4_ONLY` | No value accepted. Mutually exclusive with `ipv6_only`. |
| `ipv6_only` | `ipv6_only` (bare flag) | — | `TSF_IPV6_ONLY` | No value accepted. Mutually exclusive with `ipv4_only`. |

### 3.2 State Name to Constant Mapping

| Filter name | FreeBSD constant | Value | Description |
|---|---|---|---|
| `closed` | `TCPS_CLOSED` | 0 | Connection closed |
| `listen` | `TCPS_LISTEN` | 1 | Listening for connections |
| `syn_sent` | `TCPS_SYN_SENT` | 2 | SYN sent, awaiting SYN-ACK |
| `syn_received` | `TCPS_SYN_RECEIVED` | 3 | SYN received, sent SYN-ACK |
| `established` | `TCPS_ESTABLISHED` | 4 | Connection established |
| `close_wait` | `TCPS_CLOSE_WAIT` | 5 | Remote closed, awaiting local close |
| `fin_wait_1` | `TCPS_FIN_WAIT_1` | 6 | Local closed, FIN sent |
| `closing` | `TCPS_CLOSING` | 7 | Both sides closing simultaneously |
| `last_ack` | `TCPS_LAST_ACK` | 8 | Awaiting final ACK |
| `fin_wait_2` | `TCPS_FIN_WAIT_2` | 9 | FIN acknowledged, awaiting remote FIN |
| `time_wait` | `TCPS_TIME_WAIT` | 10 | 2MSL timer running |

### 3.3 Field Group Bitmask Mapping

| Field group | Bitmask | Struct fields included |
|---|---|---|
| `identity` | `TSR_FIELDS_IDENTITY` (0x001) | af, addresses, ports (always included) |
| `state` | `TSR_FIELDS_STATE` (0x002) | t_state, flags_tcp |
| `congestion` | `TSR_FIELDS_CONGESTION` (0x004) | cwnd, ssthresh, snd_wnd, rcv_wnd, maxseg |
| `rtt` | `TSR_FIELDS_RTT` (0x008) | rtt, rttvar, rto, rttmin |
| `sequences` | `TSR_FIELDS_SEQUENCES` (0x010) | snd_nxt, snd_una, snd_max, rcv_nxt, rcv_adv |
| `counters` | `TSR_FIELDS_COUNTERS` (0x020) | rexmitpack, ooopack, zerowin, dupacks, numsacks |
| `timers` | `TSR_FIELDS_TIMERS` (0x040) | tt_rexmt, tt_persist, tt_keep, tt_2msl, tt_delack, rcvtime |
| `buffers` | `TSR_FIELDS_BUFFERS` (0x080) | snd_buf_cc, snd_buf_hiwat, rcv_buf_cc, rcv_buf_hiwat |
| `ecn` | `TSR_FIELDS_ECN` (0x100) | ecn, delivered_ce, received_ce |
| `names` | `TSR_FIELDS_NAMES` (0x200) | cc algo name, tcp stack name |
| `all` | `TSR_FIELDS_ALL` (0x3FF) | All of the above |
| `default` | `TSR_FIELDS_DEFAULT` (0x08F) | identity + state + congestion + rtt + buffers |

### 3.4 Operator Recipes

#### CDN Cache Node

```sh
# Client-facing HTTPS (50K connections)
sysctl dev.tcpstats.profiles.clients="local_port=443 exclude=listen,timewait"

# Upstream origin fetches (5K connections)
sysctl dev.tcpstats.profiles.upstream="remote_port=443,80 exclude=listen,timewait"

# Health checks (low frequency, full detail)
sysctl dev.tcpstats.profiles.health="local_port=80,8080 exclude=timewait format=full"
```

#### Web Server

```sh
# All HTTP/HTTPS traffic
sysctl dev.tcpstats.profiles.web="local_port=80,443 exclude=listen,timewait,closewait"

# Identify slow clients (need RTT + buffers)
sysctl dev.tcpstats.profiles.slow="local_port=443 include_state=established fields=identity,state,rtt,buffers,congestion"
```

#### Database Server

```sh
# PostgreSQL connections from app tier
sysctl dev.tcpstats.profiles.pg="local_port=5432 local_addr=10.0.1.0/24 exclude=listen"

# MySQL replication
sysctl dev.tcpstats.profiles.repl="remote_port=3306 include_state=established fields=state,congestion,rtt,sequences"
```

#### Microservice Mesh

```sh
# Service-to-service on mesh ports
sysctl dev.tcpstats.profiles.mesh="local_port=8080,8443 remote_port=8080,8443 exclude=listen,timewait"
```

#### Multi-Homed Host

```sh
# Traffic on the public interface only
sysctl dev.tcpstats.profiles.public="local_addr=203.0.113.10 exclude=listen,timewait"

# Traffic on the management VLAN
sysctl dev.tcpstats.profiles.mgmt="local_addr=10.255.0.0/16 format=full"
```

#### Dual-Stack IPv6 Transition

```sh
# IPv6-only monitoring to track v6 adoption
sysctl dev.tcpstats.profiles.v6="ipv6_only local_port=443 exclude=listen,timewait"

# IPv4 legacy traffic
sysctl dev.tcpstats.profiles.v4="ipv4_only local_port=443 exclude=listen,timewait"
```

---

## 4. Expanded `tcpstats_filter` Struct

This is the updated filter struct incorporating CIDR support, expanded state
filtering, and forward-compatibility versioning. See [kernel-module.md
section 12.3](kernel-module.md#123-filter-specification) for the in-context
definition.

```c
/*
 * Socket filter — configurable via ioctl or sysctl-created named profiles.
 *
 * All conditions are ANDed. Empty/zero fields mean "match any".
 * Port arrays use network byte order. A port value of 0 means "unused slot".
 *
 * Version 2: adds CIDR masks, include_state mode, expanded exclude flags.
 */
#define TSF_VERSION             2
#define TSF_MAX_PORTS           8

struct tcpstats_filter {
    /* Version for forward compatibility */
    uint32_t    version;                /* Must be TSF_VERSION */

    /* State filter */
    uint16_t    state_mask;             /* Bitmask of (1 << TCPS_*); 0xFFFF = all */
    uint16_t    _pad0;
    uint32_t    flags;

/* --- Exclude flags (one per TCP state) --- */
#define TSF_EXCLUDE_CLOSED      0x00000001
#define TSF_EXCLUDE_LISTEN      0x00000002
#define TSF_EXCLUDE_SYN_SENT    0x00000004
#define TSF_EXCLUDE_SYN_RCVD    0x00000008
#define TSF_EXCLUDE_ESTABLISHED 0x00000010
#define TSF_EXCLUDE_CLOSE_WAIT  0x00000020
#define TSF_EXCLUDE_FIN_WAIT_1  0x00000040
#define TSF_EXCLUDE_CLOSING     0x00000080
#define TSF_EXCLUDE_LAST_ACK    0x00000100
#define TSF_EXCLUDE_FIN_WAIT_2  0x00000200
#define TSF_EXCLUDE_TIME_WAIT   0x00000400

/* --- Mode flags --- */
#define TSF_STATE_INCLUDE_MODE  0x00001000  /* include_state= used (exclusive with exclude=) */
#define TSF_LOCAL_PORT_MATCH    0x00002000  /* Filter on local ports */
#define TSF_REMOTE_PORT_MATCH   0x00004000  /* Filter on remote ports */
#define TSF_LOCAL_ADDR_MATCH    0x00008000  /* Filter on local address (CIDR) */
#define TSF_REMOTE_ADDR_MATCH   0x00010000  /* Filter on remote address (CIDR) */
#define TSF_IPV4_ONLY           0x00020000
#define TSF_IPV6_ONLY           0x00040000

    /* Port filters — match if socket port is ANY of the listed ports */
    uint16_t    local_ports[TSF_MAX_PORTS];     /* Network byte order; 0 = unused */
    uint16_t    remote_ports[TSF_MAX_PORTS];    /* Network byte order; 0 = unused */

    /* IPv4 address filters with CIDR mask */
    struct in_addr  local_addr_v4;      /* Match if non-zero */
    struct in_addr  local_mask_v4;      /* Netmask (e.g., 0xFFFFFF00 for /24) */
    struct in_addr  remote_addr_v4;
    struct in_addr  remote_mask_v4;

    /* IPv6 address filters with prefix length */
    struct in6_addr local_addr_v6;      /* Match if non-zero */
    uint8_t         local_prefix_v6;    /* Prefix length (0-128); 0 = exact match */
    uint8_t         _pad1[3];
    struct in6_addr remote_addr_v6;
    uint8_t         remote_prefix_v6;
    uint8_t         _pad2[3];

    /* Field mask (which field groups to populate) */
    uint32_t    field_mask;

    /* Record format selection */
    uint32_t    format;                 /* 0 = compact (default), 1 = full */
#define TSF_FORMAT_COMPACT      0
#define TSF_FORMAT_FULL         1

    /* Spare for future expansion */
    uint32_t    _spare[4];
};

/* Compile-time validation */
_Static_assert(sizeof(struct tcpstats_filter) <= 256,
    "tcpstats_filter exceeds maximum profile size");
```

### Struct Layout Summary

| Field group | Size (bytes) | Purpose |
|---|---|---|
| Version + state + flags | 12 | Filter mode and state bitmask |
| Port arrays (2 x 8 x uint16) | 32 | Local and remote port lists |
| IPv4 addresses + masks | 16 | CIDR-aware v4 address filtering |
| IPv6 addresses + prefixes | 40 | CIDR-aware v6 address filtering |
| Field mask + format | 8 | Output field selection |
| Spare | 16 | Future expansion without ABI break |
| **Total** | **~128** | Fits in 2 cache lines |

### Key Design Decisions

- **`TSF_MAX_PORTS=8`**: No port ranges means operators list ports explicitly.
  Real-world deployments rarely monitor more than 4-5 distinct ports per
  profile. 8 provides comfortable headroom.

- **`version` field**: Allows the kernel to reject filters from newer
  userspace tools gracefully (return `ENOTSUP`) rather than misinterpreting
  fields. Old tools sending version=0 (uninitialized) are handled by
  treating version 0 as version 1 (original layout without CIDR).

- **`_spare[4]`**: 16 bytes of reserved space. Future additions (e.g.,
  UID filtering, NUMA domain filter, connection age threshold) can use
  these without changing the struct size, preserving ABI compatibility
  within version 2.

- **Separate IPv4 mask vs IPv6 prefix length**: IPv4 uses a precomputed
  netmask (`0xFFFFFF00` for /24) for single-instruction AND+CMP in the
  match callback. IPv6 uses a prefix length because precomputing a 128-bit
  mask offers no advantage over byte-wise comparison with length.

---

## 5. Kernel Parser Implementation

### 5.1 Two-Pass Architecture

The parser uses a **two-pass design** to ensure atomicity — the filter
struct is never left in a partially-modified state on error:

```
Pass 1: Tokenize
  - Split input into directive tokens on whitespace
  - Validate directive count <= 16
  - For each token, identify key and value (split on '=')
  - Check for unknown keys, duplicate directives, flags with values
  - NO struct modification in this pass

Pass 2: Parse and populate
  - Allocate temporary tcpstats_filter on stack, bzero it
  - For each validated token, call the appropriate tsf_parse_*() function
  - Each parse function validates its value and populates the temp struct
  - After all directives: run tsf_validate_filter() for cross-directive checks
  - On success: copy temp struct to output
  - On failure: output struct unchanged, error reported
```

### 5.2 Top-Level Dispatcher

```c
/*
 * Parse a filter string into a tcpstats_filter struct.
 *
 * Returns 0 on success, errno on failure.
 * On failure, errbuf contains a human-readable error message and
 * the output filter struct is unchanged.
 *
 * The input string is copied and modified in place (tokenized with strsep).
 * The original sysctl buffer is not modified.
 */
#define TSF_PARSE_MAXLEN        512
#define TSF_PARSE_MAXDIRECTIVES 16
#define TSF_ERRBUF_SIZE         128

int
tsf_parse_filter_string(const char *input, size_t len,
    struct tcpstats_filter *out, char *errbuf, size_t errbuflen)
{
    struct tcpstats_filter tmp;
    char buf[TSF_PARSE_MAXLEN];
    char *tokens[TSF_PARSE_MAXDIRECTIVES];
    int ntokens, error;

    /* --- Input validation --- */
    if (len == 0 || input[0] == '\0') {
        /* Empty string = reset to default (all states, no filter) */
        bzero(out, sizeof(*out));
        out->version = TSF_VERSION;
        out->state_mask = 0xFFFF;
        out->field_mask = TSR_FIELDS_DEFAULT;
        return (0);
    }

    if (len > TSF_PARSE_MAXLEN - 1) {
        snprintf(errbuf, errbuflen, "filter string too long (%zu > %d)",
            len, TSF_PARSE_MAXLEN - 1);
        return (ENAMETOOLONG);
    }

    /* Validate: no non-printable characters */
    for (size_t i = 0; i < len; i++) {
        if (input[i] != '\0' && (input[i] < 0x20 || input[i] > 0x7E)) {
            snprintf(errbuf, errbuflen,
                "non-printable character 0x%02x at offset %zu",
                (unsigned char)input[i], i);
            return (EINVAL);
        }
    }

    /* Work on a mutable copy */
    strlcpy(buf, input, sizeof(buf));

    /* --- Pass 1: Tokenize --- */
    ntokens = 0;
    char *p = buf;
    char *tok;
    while ((tok = strsep(&p, " \t")) != NULL) {
        if (tok[0] == '\0')
            continue;   /* Skip consecutive whitespace */
        if (ntokens >= TSF_PARSE_MAXDIRECTIVES) {
            snprintf(errbuf, errbuflen,
                "too many directives (%d > %d)",
                ntokens + 1, TSF_PARSE_MAXDIRECTIVES);
            return (EINVAL);
        }
        tokens[ntokens++] = tok;
    }

    if (ntokens == 0) {
        /* Whitespace-only string = same as empty */
        bzero(out, sizeof(*out));
        out->version = TSF_VERSION;
        out->state_mask = 0xFFFF;
        out->field_mask = TSR_FIELDS_DEFAULT;
        return (0);
    }

    /* --- Pass 2: Parse into temporary struct --- */
    bzero(&tmp, sizeof(tmp));
    tmp.version = TSF_VERSION;
    tmp.state_mask = 0xFFFF;
    tmp.field_mask = TSR_FIELDS_DEFAULT;

    for (int i = 0; i < ntokens; i++) {
        error = tsf_parse_directive(tokens[i], &tmp, errbuf, errbuflen);
        if (error != 0)
            return (error);
    }

    /* --- Cross-directive validation --- */
    error = tsf_validate_filter(&tmp, errbuf, errbuflen);
    if (error != 0)
        return (error);

    /* --- Success: atomic copy to output --- */
    bcopy(&tmp, out, sizeof(*out));
    return (0);
}
```

### 5.3 Per-Directive Dispatcher

```c
/*
 * Parse a single directive token ("key=value" or bare "flag").
 * Modifies the filter struct in place.
 */
static int
tsf_parse_directive(char *token, struct tcpstats_filter *f,
    char *errbuf, size_t errbuflen)
{
    char *key, *value;

    /* Split on '=' */
    key = token;
    value = strchr(token, '=');
    if (value != NULL) {
        *value = '\0';
        value++;
        if (*value == '\0') {
            snprintf(errbuf, errbuflen,
                "directive '%s' has empty value", key);
            return (EINVAL);
        }
    }

    /* Normalize key to lowercase (case-insensitive) */
    for (char *c = key; *c != '\0'; c++)
        *c = tolower((unsigned char)*c);

    /* --- Flags (no value) --- */
    if (strcmp(key, "ipv4_only") == 0) {
        if (value != NULL) {
            snprintf(errbuf, errbuflen,
                "'ipv4_only' is a flag and does not accept a value");
            return (EINVAL);
        }
        f->flags |= TSF_IPV4_ONLY;
        return (0);
    }
    if (strcmp(key, "ipv6_only") == 0) {
        if (value != NULL) {
            snprintf(errbuf, errbuflen,
                "'ipv6_only' is a flag and does not accept a value");
            return (EINVAL);
        }
        f->flags |= TSF_IPV6_ONLY;
        return (0);
    }

    /* --- Key=value directives (value required) --- */
    if (value == NULL) {
        snprintf(errbuf, errbuflen,
            "unknown flag '%s' (did you mean '%s=...'?)", key, key);
        return (EINVAL);
    }

    if (strcmp(key, "local_port") == 0)
        return tsf_parse_port_list(value, f->local_ports, TSF_MAX_PORTS,
            &f->flags, TSF_LOCAL_PORT_MATCH, errbuf, errbuflen);

    if (strcmp(key, "remote_port") == 0)
        return tsf_parse_port_list(value, f->remote_ports, TSF_MAX_PORTS,
            &f->flags, TSF_REMOTE_PORT_MATCH, errbuf, errbuflen);

    if (strcmp(key, "exclude") == 0)
        return tsf_parse_exclude_list(value, f, errbuf, errbuflen);

    if (strcmp(key, "include_state") == 0)
        return tsf_parse_include_state_list(value, f, errbuf, errbuflen);

    if (strcmp(key, "local_addr") == 0)
        return tsf_parse_addr(value, f, TSF_LOCAL_ADDR_MATCH,
            errbuf, errbuflen);

    if (strcmp(key, "remote_addr") == 0)
        return tsf_parse_addr(value, f, TSF_REMOTE_ADDR_MATCH,
            errbuf, errbuflen);

    if (strcmp(key, "format") == 0)
        return tsf_parse_format(value, f, errbuf, errbuflen);

    if (strcmp(key, "fields") == 0)
        return tsf_parse_field_list(value, f, errbuf, errbuflen);

    snprintf(errbuf, errbuflen, "unknown directive '%s'", key);
    return (EINVAL);
}
```

### 5.4 Function Decomposition

| Function | Input | Output | Security-critical? |
|---|---|---|---|
| `tsf_parse_filter_string()` | Raw sysctl string | Populated `tcpstats_filter` | Yes — entry point from untrusted input |
| `tsf_parse_directive()` | Single token | Dispatches to specific parsers | Yes — validates key names |
| `tsf_parse_port_number()` | Port string (e.g., `"443"`) | `uint16_t` in network byte order | **Yes** — integer overflow, octal injection |
| `tsf_parse_port_list()` | Comma-separated ports | Port array + flag | Yes — array bounds |
| `tsf_parse_ipv4_addr()` | Dotted-decimal + optional `/prefix` | `in_addr` + `in_addr` mask | Yes — format validation |
| `tsf_parse_ipv6_addr()` | RFC 5952 string + optional `/prefix` | `in6_addr` + prefix length | **Yes** — complex format, `::` compression |
| `tsf_parse_addr()` | Address string (auto-detect AF) | Dispatches to v4 or v6 parser | Yes — AF detection |
| `tsf_parse_state_list()` | Comma-separated state names | State bitmask | Moderate — bounded enum |
| `tsf_parse_exclude_list()` | Comma-separated state names | Exclude flags + state_mask | Moderate |
| `tsf_parse_include_state_list()` | Comma-separated state names | state_mask + include mode flag | Moderate |
| `tsf_parse_field_list()` | Comma-separated field groups | `field_mask` bitmask | Moderate — bounded enum |
| `tsf_parse_format()` | `"compact"` or `"full"` | `format` field | Low — two valid values |
| `tsf_validate_filter()` | Complete `tcpstats_filter` | 0 or EINVAL | Yes — conflict detection |

---

## 6. IPv6 Kernel Address Parser

FreeBSD's kernel does **not export `inet_pton()`** — it is a libc function
not available in kernel space. The module must implement its own IPv6 address
parser.

### 6.1 RFC 5952 Compliance

The parser handles the following IPv6 address forms:

| Form | Example | Description |
|---|---|---|
| Full | `2001:0db8:0000:0000:0000:0000:0000:0001` | 8 groups of 4 hex digits |
| Compressed | `2001:db8::1` | `::` replaces consecutive zero groups |
| Leading zero omission | `2001:db8:0:0:0:0:0:1` | Leading zeros within groups omitted |
| Loopback | `::1` | `::` at start |
| All zeros | `::` | Unspecified address |
| IPv4-mapped | `::ffff:192.168.1.1` | Last 32 bits as dotted-decimal |
| Link-local | `fe80::1` | Common in practice |
| With CIDR | `fe80::/10` | Prefix length appended |

### 6.2 Parser Implementation

```c
/*
 * Parse an IPv6 address string into a struct in6_addr.
 * Handles :: compression and mixed IPv4 notation.
 *
 * Returns 0 on success, EINVAL on invalid format.
 * If prefix_out is non-NULL and the string contains "/N",
 * the prefix length is stored there (0-128).
 */
static int
tsf_parse_ipv6_addr(const char *str, struct in6_addr *addr,
    uint8_t *prefix_out, char *errbuf, size_t errbuflen)
{
    uint16_t groups[8];
    int ngroups_before = 0;     /* Groups before :: */
    int ngroups_after = 0;      /* Groups after :: */
    int double_colon = -1;      /* Position of :: (-1 = not seen) */
    const char *p = str;
    const char *slash;
    char addrbuf[48];           /* Max IPv6 string: 45 chars + NUL */
    int total_groups;

    bzero(groups, sizeof(groups));
    bzero(addr, sizeof(*addr));

    /* Separate address from prefix length */
    slash = strchr(str, '/');
    if (slash != NULL) {
        size_t addrlen = slash - str;
        if (addrlen >= sizeof(addrbuf)) {
            snprintf(errbuf, errbuflen,
                "IPv6 address too long before '/'");
            return (EINVAL);
        }
        strlcpy(addrbuf, str, addrlen + 1);
        p = addrbuf;
    }

    /* Parse groups with :: detection */
    int gi = 0;         /* Current group index */
    int seen_digits;
    uint32_t val;

    while (*p != '\0' && gi < 8) {
        /* Check for :: */
        if (p[0] == ':' && p[1] == ':') {
            if (double_colon >= 0) {
                snprintf(errbuf, errbuflen,
                    "multiple '::' in IPv6 address");
                return (EINVAL);
            }
            double_colon = gi;
            ngroups_before = gi;
            p += 2;
            if (*p == '\0')
                break;
            continue;
        }

        /* Skip single colon separator (not at start) */
        if (*p == ':') {
            if (gi == 0 && double_colon < 0) {
                snprintf(errbuf, errbuflen,
                    "IPv6 address starts with single ':'");
                return (EINVAL);
            }
            p++;
        }

        /* Parse hex group (1-4 hex digits) */
        seen_digits = 0;
        val = 0;
        while (*p != '\0' && *p != ':' && *p != '/' && seen_digits < 4) {
            char c = tolower((unsigned char)*p);
            if (c >= '0' && c <= '9')
                val = (val << 4) | (c - '0');
            else if (c >= 'a' && c <= 'f')
                val = (val << 4) | (c - 'a' + 10);
            else {
                /* Check for IPv4-mapped notation */
                if (c == '.' && gi >= 6) {
                    /* Reparse last group + remaining as IPv4 */
                    /* (implementation deferred to helper) */
                    return tsf_parse_ipv6_v4mapped(
                        p - seen_digits, addr, gi,
                        groups, ngroups_before, double_colon,
                        errbuf, errbuflen);
                }
                snprintf(errbuf, errbuflen,
                    "invalid character '%c' in IPv6 group", *p);
                return (EINVAL);
            }
            seen_digits++;
            p++;
        }

        if (seen_digits == 0) {
            snprintf(errbuf, errbuflen, "empty group in IPv6 address");
            return (EINVAL);
        }
        if (val > 0xFFFF) {
            snprintf(errbuf, errbuflen,
                "IPv6 group value 0x%x exceeds 0xFFFF", val);
            return (EINVAL);
        }

        groups[gi++] = (uint16_t)val;
    }

    /* Determine total groups and expand :: */
    if (double_colon >= 0) {
        ngroups_after = gi - ngroups_before;
        total_groups = ngroups_before + ngroups_after;
        if (total_groups > 7) {
            snprintf(errbuf, errbuflen,
                "too many groups (%d) with '::' in IPv6 address",
                total_groups);
            return (EINVAL);
        }

        /* Shift after-groups to the end of the 8-group array */
        int zero_fill = 8 - total_groups;
        for (int i = ngroups_after - 1; i >= 0; i--)
            groups[ngroups_before + zero_fill + i] =
                groups[ngroups_before + i];
        for (int i = 0; i < zero_fill; i++)
            groups[ngroups_before + i] = 0;
    } else {
        if (gi != 8) {
            snprintf(errbuf, errbuflen,
                "IPv6 address has %d groups (expected 8, or use '::')", gi);
            return (EINVAL);
        }
    }

    /* Convert groups to in6_addr (network byte order) */
    for (int i = 0; i < 8; i++) {
        addr->s6_addr[i * 2]     = (groups[i] >> 8) & 0xFF;
        addr->s6_addr[i * 2 + 1] = groups[i] & 0xFF;
    }

    /* Parse prefix length if present */
    if (prefix_out != NULL && slash != NULL) {
        const char *pstr = slash + 1;  /* Points into original str */
        return tsf_parse_prefix_length(pstr, 128, prefix_out,
            errbuf, errbuflen);
    } else if (prefix_out != NULL) {
        *prefix_out = 128;     /* No prefix = exact match (/128) */
    }

    return (0);
}
```

### 6.3 Security Properties

| Property | Implementation |
|---|---|
| Bounded iteration | Outer loop: max 8 groups. Inner hex loop: max 4 digits. Total iterations bounded at 8 * 4 = 32. |
| No buffer overflows | `groups[8]` array indexed by `gi < 8` check. `addrbuf[48]` bounded by `addrlen < sizeof(addrbuf)`. |
| Strict character validation | Only `[0-9a-fA-F:./]` accepted. Any other character → immediate EINVAL. |
| Single `::` enforcement | `double_colon >= 0` check prevents multiple `::` sequences. |
| Group count validation | With `::`: total groups <= 7. Without `::`: exactly 8 groups. |
| No dependency on libc | Pure kernel implementation — no `inet_pton()`, no `sscanf()`. |

### 6.4 CIDR Host Bits Validation

When a prefix length is specified, the parser validates that host bits
are zero in the network address:

```c
/*
 * Validate that host bits are zero in a CIDR address.
 * For example, fe80::1/10 is rejected (host bits set).
 * fe80::/10 is accepted.
 */
static int
tsf_validate_v6_cidr(const struct in6_addr *addr, uint8_t prefix,
    char *errbuf, size_t errbuflen)
{
    if (prefix == 128)
        return (0);             /* Exact match, no host bits */
    if (prefix == 0)
        return (0);             /* Match all, any address valid */

    int full_bytes = prefix / 8;
    int remainder_bits = prefix % 8;

    /* Check partial byte */
    if (remainder_bits > 0) {
        uint8_t mask = (uint8_t)(0xFF << (8 - remainder_bits));
        if ((addr->s6_addr[full_bytes] & ~mask) != 0) {
            snprintf(errbuf, errbuflen,
                "host bits set in IPv6 CIDR (prefix /%u)", prefix);
            return (EINVAL);
        }
        full_bytes++;
    }

    /* Check remaining bytes (must all be zero) */
    for (int i = full_bytes; i < 16; i++) {
        if (addr->s6_addr[i] != 0) {
            snprintf(errbuf, errbuflen,
                "host bits set in IPv6 CIDR (prefix /%u)", prefix);
            return (EINVAL);
        }
    }

    return (0);
}
```

---

## 7. Port Number Parsing (Security Analysis)

Port number parsing is security-critical because it converts untrusted
string input into a `uint16_t` used in the SMR match callback. Any
parsing bug here could cause incorrect filtering (security bypass) or
integer overflow (undefined behavior in kernel).

### 7.1 Implementation

```c
/*
 * Parse a single port number string to uint16_t in network byte order.
 *
 * Rejects: leading zeros, negative numbers, overflow >65535, port 0,
 * trailing non-digits, empty strings.
 *
 * Returns 0 on success with *port_out set, EINVAL on failure.
 */
static int
tsf_parse_port_number(const char *str, uint16_t *port_out,
    char *errbuf, size_t errbuflen)
{
    unsigned long val;
    size_t len;

    if (str == NULL || str[0] == '\0') {
        snprintf(errbuf, errbuflen, "empty port number");
        return (EINVAL);
    }

    len = strnlen(str, 6);     /* Max 5 digits + check for 6th */

    /* Reject leading zeros (octal confusion) */
    if (len > 1 && str[0] == '0') {
        snprintf(errbuf, errbuflen,
            "port '%s' has leading zero (octal not supported)", str);
        return (EINVAL);
    }

    /* Reject non-digit characters before calling strtoul */
    for (size_t i = 0; i < len && str[i] != '\0'; i++) {
        if (str[i] < '0' || str[i] > '9') {
            snprintf(errbuf, errbuflen,
                "port '%s' contains non-digit character '%c'",
                str, str[i]);
            return (EINVAL);
        }
    }

    /* Reject if more than 5 digits (> 65535 guaranteed) */
    if (len > 5) {
        snprintf(errbuf, errbuflen,
            "port '%s' too many digits (max 65535)", str);
        return (EINVAL);
    }

    /* Safe to call strtoul — input is validated digits only */
    val = strtoul(str, NULL, 10);

    /* Range check */
    if (val == 0) {
        snprintf(errbuf, errbuflen, "port 0 is not valid");
        return (EINVAL);
    }
    if (val > 65535) {
        snprintf(errbuf, errbuflen,
            "port %lu exceeds maximum 65535", val);
        return (EINVAL);
    }

    *port_out = htons((uint16_t)val);
    return (0);
}
```

### 7.2 Port List Parser with Duplicate Detection

```c
static int
tsf_parse_port_list(char *value, uint16_t *ports, int maxports,
    uint32_t *flags, uint32_t flag_bit,
    char *errbuf, size_t errbuflen)
{
    char *tok, *p;
    int count = 0;
    uint16_t port;
    int error;

    if (*flags & flag_bit) {
        snprintf(errbuf, errbuflen, "duplicate port directive");
        return (EINVAL);
    }

    p = value;
    while ((tok = strsep(&p, ",")) != NULL) {
        if (tok[0] == '\0')
            continue;   /* Skip empty tokens (e.g., "443,,80") */

        if (count >= maxports) {
            snprintf(errbuf, errbuflen,
                "too many ports (max %d per direction)", maxports);
            return (EINVAL);
        }

        error = tsf_parse_port_number(tok, &port, errbuf, errbuflen);
        if (error != 0)
            return (error);

        /* Duplicate detection */
        for (int i = 0; i < count; i++) {
            if (ports[i] == port) {
                snprintf(errbuf, errbuflen,
                    "duplicate port %u", ntohs(port));
                return (EINVAL);
            }
        }

        ports[count++] = port;
    }

    if (count == 0) {
        snprintf(errbuf, errbuflen, "empty port list");
        return (EINVAL);
    }

    *flags |= flag_bit;
    return (0);
}
```

### 7.3 Rejection Analysis

| Input | Rejection reason | Check |
|---|---|---|
| `""` | Empty string | `str[0] == '\0'` |
| `"0"` | Port 0 not valid | `val == 0` |
| `"00"` | Leading zero | `len > 1 && str[0] == '0'` |
| `"08080"` | Leading zero | Same check |
| `"65536"` | Exceeds maximum | `val > 65535` |
| `"99999"` | Exceeds maximum | Same (99999 > 65535) |
| `"100000"` | Too many digits | `len > 5` |
| `"-1"` | Non-digit character | `str[i] < '0'` |
| `"443abc"` | Non-digit character | `str[i] > '9'` |
| `"0x1BB"` | Non-digit character | `str[1] == 'x'` |
| `" 443"` | Non-digit character (space) | `str[0] < '0'` |
| `"443 "` | Non-digit character (space) | Trailing space check |

---

## 8. Input Validation — Exhaustive Rejection Tables

### 8.1 Structural Rejections

| Input | Error | errno | Message |
|---|---|---|---|
| String > 511 bytes | Too long | `ENAMETOOLONG` | `"filter string too long (N > 511)"` |
| Contains byte 0x01-0x1F (except 0x09) | Non-printable | `EINVAL` | `"non-printable character 0xNN at offset N"` |
| Contains byte 0x7F-0xFF | Non-printable | `EINVAL` | Same |
| > 16 directives | Too many | `EINVAL` | `"too many directives (N > 16)"` |
| Empty string | Reset (not error) | 0 | Filter reset to defaults |
| Whitespace-only | Reset (not error) | 0 | Filter reset to defaults |

### 8.2 Key Rejections

| Input | Error | errno | Message |
|---|---|---|---|
| `foobar=123` | Unknown key | `EINVAL` | `"unknown directive 'foobar'"` |
| `local_port` (no `=value`) | Missing value | `EINVAL` | `"unknown flag 'local_port' (did you mean 'local_port=...'?)"` |
| `local_port=443 local_port=80` | Duplicate directive | `EINVAL` | `"duplicate port directive"` |
| `ipv4_only=true` | Flag with value | `EINVAL` | `"'ipv4_only' is a flag and does not accept a value"` |
| `local_port=` | Empty value | `EINVAL` | `"directive 'local_port' has empty value"` |
| `=443` | Empty key | `EINVAL` | `"unknown directive ''"` |

### 8.3 Port Rejections

| Input | Error | errno | Message |
|---|---|---|---|
| `local_port=0` | Port 0 invalid | `EINVAL` | `"port 0 is not valid"` |
| `local_port=65536` | Exceeds max | `EINVAL` | `"port 65536 exceeds maximum 65535"` |
| `local_port=08080` | Leading zero | `EINVAL` | `"port '08080' has leading zero (octal not supported)"` |
| `local_port=abc` | Non-digit | `EINVAL` | `"port 'abc' contains non-digit character 'a'"` |
| `local_port=-1` | Non-digit (dash) | `EINVAL` | `"port '-1' contains non-digit character '-'"` |
| `local_port=443,443` | Duplicate | `EINVAL` | `"duplicate port 443"` |
| `local_port=1,2,3,4,5,6,7,8,9` | Too many (>8) | `EINVAL` | `"too many ports (max 8 per direction)"` |
| `local_port=,,` | Empty list | `EINVAL` | `"empty port list"` |
| `local_port=100000` | Too many digits | `EINVAL` | `"port '100000' too many digits (max 65535)"` |

### 8.4 State Rejections

| Input | Error | errno | Message |
|---|---|---|---|
| `exclude=foobar` | Unknown state | `EINVAL` | `"unknown state name 'foobar'"` |
| `exclude=listen,listen` | Duplicate state | `EINVAL` | `"duplicate state 'listen' in exclude list"` |
| `exclude=listen include_state=established` | Conflict | `EINVAL` | `"'exclude' and 'include_state' are mutually exclusive"` |
| `include_state=` | Empty list | `EINVAL` | `"directive 'include_state' has empty value"` |

### 8.5 Address Rejections

| Input | Error | errno | Message |
|---|---|---|---|
| `local_addr=999.1.2.3` | Invalid octet | `EINVAL` | `"IPv4 octet 999 exceeds 255"` |
| `local_addr=10.0.0.0/33` | Prefix too long | `EINVAL` | `"IPv4 prefix length 33 exceeds maximum 32"` |
| `local_addr=10.0.0.1/24` | Host bits set | `EINVAL` | `"host bits set in IPv4 CIDR (prefix /24)"` |
| `local_addr=10.0.0` | Missing octets | `EINVAL` | `"IPv4 address has 3 octets (expected 4)"` |
| `local_addr=10.0.0.0.1` | Extra octets | `EINVAL` | `"trailing characters after IPv4 address"` |
| `remote_addr=fe80::1/10` | Host bits set (v6) | `EINVAL` | `"host bits set in IPv6 CIDR (prefix /10)"` |
| `remote_addr=fe80::/129` | Prefix too long (v6) | `EINVAL` | `"IPv6 prefix length 129 exceeds maximum 128"` |
| `remote_addr=gggg::1` | Invalid hex | `EINVAL` | `"invalid character 'g' in IPv6 group"` |
| `remote_addr=2001:db8::1::2` | Multiple `::` | `EINVAL` | `"multiple '::' in IPv6 address"` |
| `local_addr=10.0.0.1 ipv6_only` | AF conflict | `EINVAL` | `"IPv4 address conflicts with ipv6_only flag"` |
| `local_addr=fe80::1 ipv4_only` | AF conflict | `EINVAL` | `"IPv6 address conflicts with ipv4_only flag"` |

### 8.6 Conflict Rejections

| Input | Error | errno | Message |
|---|---|---|---|
| `ipv4_only ipv6_only` | Mutual exclusion | `EINVAL` | `"'ipv4_only' and 'ipv6_only' are mutually exclusive"` |
| `exclude=listen include_state=established` | Mutual exclusion | `EINVAL` | `"'exclude' and 'include_state' are mutually exclusive"` |
| `format=compact format=full` | Duplicate directive | `EINVAL` | `"duplicate 'format' directive"` |
| `fields=all fields=rtt` | Duplicate directive | `EINVAL` | `"duplicate 'fields' directive"` |

---

## 9. Error Reporting

### 9.1 Three Error Channels

Errors from the filter parser are reported through three channels to
ensure operators can always diagnose configuration failures:

| Channel | Audience | Persistence | Example |
|---|---|---|---|
| **errno return** | Programmatic (sysctl write returns errno) | Per-call | `sysctl: dev.tcpstats.profiles.foo: Invalid argument` |
| **`dev.tcpstats.last_error` sysctl** | Interactive operator | Until next write | `"port '08080' has leading zero (octal not supported)"` |
| **`dmesg` via `log(LOG_NOTICE)`** | System audit trail | Persistent (kernel log) | `tcp_stats_kld: filter parse error: port '08080' has leading zero` |

### 9.2 Error Buffer Convention

Every `tsf_parse_*` function takes `errbuf` and `errbuflen` parameters.
On error, the function populates `errbuf` with a human-readable message
using `snprintf()` (never `sprintf()`), then returns an appropriate errno.

The top-level sysctl handler copies the errbuf to the `dev.tcpstats.last_error`
sysctl and logs it to dmesg:

```c
static int
tcpstats_profile_handler(SYSCTL_HANDLER_ARGS)
{
    char input[TSF_PARSE_MAXLEN];
    char errbuf[TSF_ERRBUF_SIZE];
    struct tcpstats_filter filter;
    int error;

    /* ... read input from sysctl handler ... */

    errbuf[0] = '\0';
    error = tsf_parse_filter_string(input, strlen(input),
        &filter, errbuf, sizeof(errbuf));

    if (error != 0) {
        /* Channel 2: store in last_error sysctl */
        strlcpy(tcpstats_last_error, errbuf, sizeof(tcpstats_last_error));

        /* Channel 3: log to dmesg */
        log(LOG_NOTICE, "tcp_stats_kld: filter parse error: %s\n", errbuf);

        /* Channel 1: return errno to caller */
        return (error);
    }

    /* Success — clear last error */
    tcpstats_last_error[0] = '\0';

    /* ... create device with parsed filter ... */
    return (0);
}
```

### 9.3 Return Code Conventions

| errno | Meaning | When used |
|---|---|---|
| `EINVAL` | Parse error (bad syntax, unknown key, invalid value) | Most parse failures |
| `ENAMETOOLONG` | Filter string exceeds 512-byte limit | Input length check |
| `ENOSPC` | Maximum 16 profiles already exist | Profile creation limit |
| `EEXIST` | Profile name already exists | Duplicate profile name |
| `ENOTSUP` | Filter struct version too new for this kernel | Version mismatch |

### 9.4 Example Operator Workflow

```sh
# Attempt to create a profile with a typo
$ sysctl dev.tcpstats.profiles.web="local_port=443 exlude=listen"
sysctl: dev.tcpstats.profiles.web: Invalid argument

# Check what went wrong
$ sysctl dev.tcpstats.last_error
dev.tcpstats.last_error: unknown directive 'exlude'

# Also visible in dmesg
$ dmesg | tail -1
tcp_stats_kld: filter parse error: unknown directive 'exlude'

# Fix the typo
$ sysctl dev.tcpstats.profiles.web="local_port=443 exclude=listen"
dev.tcpstats.profiles.web: local_port=443 exclude=listen

# Verify the device was created
$ ls -la /dev/tcpstats/web
crw-r-----  1 root  network  0x... /dev/tcpstats/web
```

---

## 10. Security Hardening Checklist

Every item in this checklist addresses a specific attack surface of the
kernel string parser. The parser runs with kernel privileges on
input provided by a privileged user (root or sysctl access), but defense
in depth requires hardening against malformed input regardless of caller
privilege.

### 10.1 Input Bounds

| Control | Implementation | Rationale |
|---|---|---|
| Maximum string length | `len > TSF_PARSE_MAXLEN - 1` → `ENAMETOOLONG` | Prevents unbounded kernel stack/heap usage |
| Maximum directive count | `ntokens >= TSF_PARSE_MAXDIRECTIVES` → `EINVAL` | Prevents O(N^2) in duplicate detection |
| Maximum port array size | `count >= TSF_MAX_PORTS` → `EINVAL` | Prevents array overflow in port list |
| Maximum port digit count | `strnlen(str, 6) > 5` → `EINVAL` | Pre-validates before `strtoul()` |
| Maximum IPv6 groups | `gi < 8` loop bound | Prevents buffer overflow in `groups[8]` |
| Maximum hex digits per group | `seen_digits < 4` loop bound | Prevents integer overflow in group value |
| Maximum prefix length | 32 (IPv4) or 128 (IPv6) | Prevents out-of-range shift/mask |

### 10.2 String Safety

| Control | Implementation | Rationale |
|---|---|---|
| No `sprintf()` | All formatting uses `snprintf(errbuf, errbuflen, ...)` | Prevents buffer overflow in error messages |
| No `strcpy()` | All copies use `strlcpy()` | Bounded copy with NUL termination |
| No `strlen()` on untrusted input | Use `strnlen(input, maxlen)` | Prevents scanning past buffer end |
| Mutable copy of input | `strlcpy(buf, input, sizeof(buf))` before tokenization | Protects original sysctl buffer from modification |
| Non-printable rejection | Character range check `0x20-0x7E` | Prevents control characters in kernel strings |

### 10.3 Integer Safety

| Control | Implementation | Rationale |
|---|---|---|
| Leading zero rejection | `len > 1 && str[0] == '0'` | Prevents octal interpretation by `strtoul()` |
| Pre-validated digits | Character-by-character `[0-9]` check before `strtoul()` | Prevents sign character injection (`-`, `+`) |
| Digit count limit | Max 5 digits checked before conversion | `strtoul()` cannot overflow `unsigned long` with 5 digits |
| Range check after conversion | `val > 65535` | Defense in depth after digit count limit |
| Network byte order storage | `htons((uint16_t)val)` | Prevents byte order bugs in match callback |

### 10.4 Memory Safety

| Control | Implementation | Rationale |
|---|---|---|
| `bzero` on temp struct | `bzero(&tmp, sizeof(tmp))` before population | No uninitialized fields in output |
| Atomic output update | `bcopy(&tmp, out, sizeof(*out))` only on success | No partial state on error |
| Stack-allocated buffers | `char buf[512]`, `char errbuf[128]` | No heap allocation in parser (no leak path) |
| `_Static_assert` on struct sizes | Compile-time validation | Catches struct layout drift |
| Bounded `for` loops | All loops have explicit upper bounds (no `while(1)`) | Guaranteed termination |

### 10.5 Audit Trail

| Control | Implementation | Rationale |
|---|---|---|
| All errors logged to dmesg | `log(LOG_NOTICE, ...)` on every parse failure | Administrator audit trail |
| Success logged | `log(LOG_NOTICE, "profile '%s' created")` | Track profile lifecycle |
| Deletion logged | `log(LOG_NOTICE, "profile '%s' deleted")` | Track profile lifecycle |
| Error stored in sysctl | `dev.tcpstats.last_error` | Interactive debugging |

---

## 11. Dual-Compilation Testing Pattern

The filter parser is designed to compile as both kernel module code and
a userspace test program. This enables comprehensive testing — including
fuzzing — without loading a kernel module.

### 11.1 Source File Organization

```
kmod/tcp_stats_kld/
  tcp_stats_filter_parse.c    # Parser implementation (dual-compile)
  tcp_stats_filter_parse.h    # Parser API + struct definition
  tcp_stats_kld.c             # Module code (includes parser)
  test/
    test_filter_parse.c       # Userspace test harness
    Makefile                  # Builds userspace test
```

### 11.2 Conditional Compilation Guards

```c
/* tcp_stats_filter_parse.c */

#ifdef _KERNEL
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/libkern.h>
#include <netinet/in.h>
#else
/* Userspace shims */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <ctype.h>
#include <arpa/inet.h>
#include <errno.h>

/* Shim kernel functions */
#define log(level, fmt, ...)    fprintf(stderr, fmt, ##__VA_ARGS__)
#define strlcpy(dst, src, len)  snprintf(dst, len, "%s", src)

/* bzero is available on BSD but not all platforms */
#ifndef bzero
#define bzero(ptr, len)         memset(ptr, 0, len)
#endif
#ifndef bcopy
#define bcopy(src, dst, len)    memcpy(dst, src, len)
#endif
#endif /* _KERNEL */
```

### 11.3 Test Harness

The test harness exercises every entry in the rejection tables from
section 8, plus positive test cases for all valid grammar productions:

```c
/* test/test_filter_parse.c */

#include "../tcp_stats_filter_parse.h"

struct test_case {
    const char *name;
    const char *input;
    int expected_error;         /* 0 = success, EINVAL, ENAMETOOLONG, etc. */
    const char *expected_errmsg; /* Substring match in errbuf, or NULL */
};

static const struct test_case cases[] = {
    /* --- Positive cases --- */
    {"empty string resets",
     "", 0, NULL},
    {"single port",
     "local_port=443", 0, NULL},
    {"multiple ports",
     "local_port=443,8443,8080", 0, NULL},
    {"exclude states",
     "exclude=listen,timewait", 0, NULL},
    {"include states",
     "include_state=established", 0, NULL},
    {"ipv4 exact address",
     "local_addr=10.0.0.1", 0, NULL},
    {"ipv4 cidr",
     "local_addr=10.0.0.0/24", 0, NULL},
    {"ipv6 loopback",
     "local_addr=::1", 0, NULL},
    {"ipv6 cidr",
     "remote_addr=fe80::/10", 0, NULL},
    {"full combo",
     "local_port=443 exclude=listen,timewait ipv4_only format=full", 0, NULL},
    {"case insensitive",
     "LOCAL_PORT=443 EXCLUDE=LISTEN", 0, NULL},
    {"ipv4_only flag",
     "ipv4_only", 0, NULL},

    /* --- Structural rejections --- */
    {"non-printable char",
     "local_port=443\x01", EINVAL, "non-printable"},
    {"too many directives",
     /* 17 directives */
     "a=1 b=2 c=3 d=4 e=5 f=6 g=7 h=8 i=9 j=10 k=11 l=12 m=13 n=14 o=15 p=16 q=17",
     EINVAL, "too many directives"},

    /* --- Port rejections --- */
    {"port zero",
     "local_port=0", EINVAL, "port 0"},
    {"port overflow",
     "local_port=65536", EINVAL, "exceeds maximum"},
    {"port leading zero",
     "local_port=0443", EINVAL, "leading zero"},
    {"port non-digit",
     "local_port=abc", EINVAL, "non-digit"},
    {"port duplicate",
     "local_port=443,443", EINVAL, "duplicate port"},
    {"port too many",
     "local_port=1,2,3,4,5,6,7,8,9", EINVAL, "too many ports"},

    /* --- State rejections --- */
    {"unknown state",
     "exclude=foobar", EINVAL, "unknown state"},

    /* --- Conflict rejections --- */
    {"ipv4_only + ipv6_only",
     "ipv4_only ipv6_only", EINVAL, "mutually exclusive"},
    {"exclude + include_state",
     "exclude=listen include_state=established", EINVAL, "mutually exclusive"},

    /* --- Address rejections --- */
    {"ipv4 host bits",
     "local_addr=10.0.0.1/24", EINVAL, "host bits set"},
    {"ipv6 host bits",
     "remote_addr=fe80::1/10", EINVAL, "host bits set"},
    {"ipv6 multiple ::",
     "remote_addr=2001::1::2", EINVAL, "multiple '::'"},

    {NULL, NULL, 0, NULL}       /* Sentinel */
};

int main(void)
{
    struct tcpstats_filter filter;
    char errbuf[TSF_ERRBUF_SIZE];
    int pass = 0, fail = 0;

    for (const struct test_case *tc = cases; tc->name != NULL; tc++) {
        bzero(&filter, sizeof(filter));
        errbuf[0] = '\0';

        int err = tsf_parse_filter_string(tc->input,
            tc->input ? strlen(tc->input) : 0,
            &filter, errbuf, sizeof(errbuf));

        int ok = 1;
        if (err != tc->expected_error) {
            printf("FAIL: %s: expected errno %d, got %d\n",
                tc->name, tc->expected_error, err);
            ok = 0;
        }
        if (tc->expected_errmsg != NULL && err != 0) {
            if (strstr(errbuf, tc->expected_errmsg) == NULL) {
                printf("FAIL: %s: expected '%s' in errbuf, got '%s'\n",
                    tc->name, tc->expected_errmsg, errbuf);
                ok = 0;
            }
        }

        if (ok) {
            pass++;
        } else {
            fail++;
        }
    }

    printf("\n%d passed, %d failed, %d total\n",
        pass, fail, pass + fail);
    return (fail > 0) ? 1 : 0;
}
```

### 11.4 Fuzz Testing

The dual-compilation design enables fuzzing with standard userspace
fuzzers:

```c
/* test/fuzz_filter_parse.c — AFL/libFuzzer harness */

#include "../tcp_stats_filter_parse.h"

#ifdef __AFL_FUZZ_TESTCASE_LEN
/* AFL persistent mode */
__AFL_FUZZ_INIT();

int main(void)
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
int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size)
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
```

Build and run:

```sh
# With AFL
afl-gcc -o fuzz_filter test/fuzz_filter_parse.c tcp_stats_filter_parse.c
afl-fuzz -i seeds/ -o findings/ -- ./fuzz_filter

# With libFuzzer (clang)
clang -fsanitize=fuzzer,address -o fuzz_filter \
    test/fuzz_filter_parse.c tcp_stats_filter_parse.c
./fuzz_filter -max_len=512 corpus/
```

---

## 12. Sysctl Profile Handler Integration

### 12.1 Profile Creation

When an operator writes to `dev.tcpstats.profiles.<name>`, the sysctl
handler:

1. Validates the profile name (alphanumeric + underscore, max 32 chars)
2. Checks the profile count limit (max 16)
3. Calls `tsf_parse_filter_string()` to parse the filter
4. Allocates a profile struct and stores the parsed filter
5. Calls `make_dev_credf()` to create `/dev/tcpstats/<name>`
6. Stores the original filter string for readback

```c
#define TSF_MAX_PROFILES        16
#define TSF_PROFILE_NAME_MAX    32

struct tcpstats_profile {
    char                    name[TSF_PROFILE_NAME_MAX];
    char                    filter_str[TSF_PARSE_MAXLEN]; /* Original string */
    struct tcpstats_filter  filter;                        /* Parsed filter */
    struct cdev             *dev;
    SLIST_ENTRY(tcpstats_profile) link;
};

static SLIST_HEAD(, tcpstats_profile) tcpstats_profiles =
    SLIST_HEAD_INITIALIZER(tcpstats_profiles);
static int tcpstats_nprofiles;
static struct sx tcpstats_profile_lock;

/*
 * Validate profile name: [a-z0-9_]+, max 32 chars.
 * No uppercase (normalize on input), no special chars.
 */
static int
tsf_validate_profile_name(const char *name, char *errbuf, size_t errbuflen)
{
    size_t len = strnlen(name, TSF_PROFILE_NAME_MAX + 1);

    if (len == 0) {
        snprintf(errbuf, errbuflen, "empty profile name");
        return (EINVAL);
    }
    if (len > TSF_PROFILE_NAME_MAX) {
        snprintf(errbuf, errbuflen,
            "profile name too long (%zu > %d)", len, TSF_PROFILE_NAME_MAX);
        return (ENAMETOOLONG);
    }

    for (size_t i = 0; i < len; i++) {
        char c = name[i];
        if (!((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_')) {
            snprintf(errbuf, errbuflen,
                "invalid character '%c' in profile name "
                "(allowed: a-z, 0-9, _)", c);
            return (EINVAL);
        }
    }

    return (0);
}
```

### 12.2 Profile Deletion

Writing an empty string to a profile sysctl deletes the profile:

```sh
# Delete a profile
sysctl dev.tcpstats.profiles.web=""
```

The handler calls `destroy_dev()` which waits for all open file
descriptors to close before destroying the device. This ensures no
in-flight reads are interrupted.

### 12.3 Profile Listing

Reading a profile sysctl returns the original filter string:

```sh
$ sysctl dev.tcpstats.profiles.web
dev.tcpstats.profiles.web: local_port=80,443 exclude=listen,timewait
```

### 12.4 Device Creation with Filter Context

Each profile device stores its filter in the `si_drv1` pointer of the
`struct cdev`, accessible from the open/read handlers:

```c
static int
tcpstats_profile_create(const char *name, const char *filter_str,
    struct tcpstats_filter *filter, char *errbuf, size_t errbuflen)
{
    struct tcpstats_profile *prof;

    sx_xlock(&tcpstats_profile_lock);

    if (tcpstats_nprofiles >= TSF_MAX_PROFILES) {
        sx_xunlock(&tcpstats_profile_lock);
        snprintf(errbuf, errbuflen,
            "maximum profiles reached (%d)", TSF_MAX_PROFILES);
        return (ENOSPC);
    }

    /* Check for duplicate */
    SLIST_FOREACH(prof, &tcpstats_profiles, link) {
        if (strcmp(prof->name, name) == 0) {
            sx_xunlock(&tcpstats_profile_lock);
            snprintf(errbuf, errbuflen,
                "profile '%s' already exists", name);
            return (EEXIST);
        }
    }

    prof = malloc(sizeof(*prof), M_TCPSTATS, M_WAITOK | M_ZERO);
    strlcpy(prof->name, name, sizeof(prof->name));
    strlcpy(prof->filter_str, filter_str, sizeof(prof->filter_str));
    bcopy(filter, &prof->filter, sizeof(prof->filter));

    /* Create /dev/tcpstats/<name> */
    prof->dev = make_dev_credf(MAKEDEV_ETERNAL_KLD,
        &tcpstats_profile_cdevsw, 0, NULL,
        UID_ROOT, GID_NETWORK, 0440, "tcpstats/%s", name);
    if (prof->dev == NULL) {
        free(prof, M_TCPSTATS);
        sx_xunlock(&tcpstats_profile_lock);
        snprintf(errbuf, errbuflen,
            "failed to create device for profile '%s'", name);
        return (ENXIO);
    }

    /* Store filter as device context for open() */
    prof->dev->si_drv1 = prof;

    SLIST_INSERT_HEAD(&tcpstats_profiles, prof, link);
    tcpstats_nprofiles++;

    sx_xunlock(&tcpstats_profile_lock);

    log(LOG_NOTICE, "tcp_stats_kld: profile '%s' created: %s\n",
        name, filter_str);
    return (0);
}
```

The `open()` handler for profile devices copies the profile's filter
into the per-fd softc:

```c
static int
tcpstats_profile_open(struct cdev *dev, int oflags, int devtype,
    struct thread *td)
{
    struct tcpstats_profile *prof = dev->si_drv1;
    struct tcpstats_softc *sc;

    if (__predict_false(oflags & FWRITE))
        return (EPERM);

    sc = malloc(sizeof(*sc), M_TCPSTATS, M_WAITOK | M_ZERO);
    sc->sc_cred = crhold(td->td_ucred);

    /* Copy the profile's pre-parsed filter */
    bcopy(&prof->filter, &sc->sc_filter, sizeof(sc->sc_filter));
    sc->sc_full = (prof->filter.format == TSF_FORMAT_FULL);

    devfs_set_cdevpriv(sc, tcpstats_dtor);
    return (0);
}
```

---

## 13. SMR Match Callback Updates

The `tcpstats_match()` function is updated to support CIDR-masked address
comparison. This function runs in the SMR section with **no locks held**
and can only read immutable fields of the inpcb.

### 13.1 Updated Match Function

```c
/*
 * SMR-safe match function with CIDR address support.
 * Called with NO locks held — reads only immutable inpcb fields.
 *
 * Returns true if this socket should be included.
 * Returns false to skip (no lock acquired, minimal cache impact).
 */
static bool
tcpstats_match(const struct inpcb *inp, void *ctx)
{
    const struct tcpstats_filter *f = ctx;

    /* IP version filter — cheapest check first */
    if (__predict_false(f->flags & TSF_IPV4_ONLY)) {
        if (!(inp->inp_vflag & INP_IPV4))
            return (false);
    }
    if (__predict_false(f->flags & TSF_IPV6_ONLY)) {
        if (!(inp->inp_vflag & INP_IPV6))
            return (false);
    }

    /* Local port filter */
    if (__predict_false(f->flags & TSF_LOCAL_PORT_MATCH)) {
        uint16_t lport = inp->inp_inc.inc_lport;
        bool found = false;
        for (int i = 0; i < TSF_MAX_PORTS && f->local_ports[i] != 0; i++) {
            if (__predict_true(lport == f->local_ports[i])) {
                found = true;
                break;
            }
        }
        if (__predict_false(!found))
            return (false);
    }

    /* Remote port filter */
    if (__predict_false(f->flags & TSF_REMOTE_PORT_MATCH)) {
        uint16_t fport = inp->inp_inc.inc_fport;
        bool found = false;
        for (int i = 0; i < TSF_MAX_PORTS && f->remote_ports[i] != 0; i++) {
            if (__predict_true(fport == f->remote_ports[i])) {
                found = true;
                break;
            }
        }
        if (__predict_false(!found))
            return (false);
    }

    /* Local address filter with CIDR mask */
    if (__predict_false(f->flags & TSF_LOCAL_ADDR_MATCH)) {
        if (inp->inp_vflag & INP_IPV4) {
            /* IPv4: single AND + CMP — no performance impact */
            if (f->local_addr_v4.s_addr != INADDR_ANY) {
                if ((inp->inp_inc.inc_laddr.s_addr & f->local_mask_v4.s_addr)
                    != (f->local_addr_v4.s_addr & f->local_mask_v4.s_addr))
                    return (false);
            }
        } else if (inp->inp_vflag & INP_IPV6) {
            if (!IN6_IS_ADDR_UNSPECIFIED(&f->local_addr_v6)) {
                if (!tsf_match_v6_prefix(
                    &inp->inp_inc.inc6_laddr,
                    &f->local_addr_v6, f->local_prefix_v6))
                    return (false);
            }
        }
    }

    /* Remote address filter with CIDR mask */
    if (__predict_false(f->flags & TSF_REMOTE_ADDR_MATCH)) {
        if (inp->inp_vflag & INP_IPV4) {
            if (f->remote_addr_v4.s_addr != INADDR_ANY) {
                if ((inp->inp_inc.inc_faddr.s_addr & f->remote_mask_v4.s_addr)
                    != (f->remote_addr_v4.s_addr & f->remote_mask_v4.s_addr))
                    return (false);
            }
        } else if (inp->inp_vflag & INP_IPV6) {
            if (!IN6_IS_ADDR_UNSPECIFIED(&f->remote_addr_v6)) {
                if (!tsf_match_v6_prefix(
                    &inp->inp_inc.inc6_faddr,
                    &f->remote_addr_v6, f->remote_prefix_v6))
                    return (false);
            }
        }
    }

    return (true);
}
```

### 13.2 IPv6 Prefix Match Helper

```c
/*
 * Compare two IPv6 addresses under a prefix length.
 * Returns true if they match within the prefix.
 *
 * This runs in the SMR section — must be fast and branch-free
 * on the common path.
 */
static __always_inline bool
tsf_match_v6_prefix(const struct in6_addr *a, const struct in6_addr *b,
    uint8_t prefix)
{
    int full_bytes, remainder_bits;

    if (prefix == 128) {
        /* Exact match — compare all 16 bytes */
        return (memcmp(a, b, 16) == 0);
    }
    if (prefix == 0) {
        /* Match all */
        return (true);
    }

    full_bytes = prefix / 8;
    remainder_bits = prefix % 8;

    /* Compare full bytes */
    if (full_bytes > 0 && memcmp(a, b, full_bytes) != 0)
        return (false);

    /* Compare partial byte */
    if (remainder_bits > 0) {
        uint8_t mask = (uint8_t)(0xFF << (8 - remainder_bits));
        if ((a->s6_addr[full_bytes] & mask) !=
            (b->s6_addr[full_bytes] & mask))
            return (false);
    }

    return (true);
}
```

### 13.3 State Filtering (Post-Lock)

State filtering remains **after** lock acquisition because `tp->t_state`
is mutable. The updated logic handles both `exclude` and `include_state`
modes:

```c
/* In tcpstats_read(), after inp_next() acquires the read lock: */

struct tcpcb *tp = intotcpcb(inp);
int state = tp->t_state;

if (f->flags & TSF_STATE_INCLUDE_MODE) {
    /* Positive match: only include listed states */
    if (!(f->state_mask & (1 << state)))
        continue;
} else {
    /* Negative match: exclude listed states */
    if (!(f->state_mask & (1 << state)))
        continue;

    /* Legacy exclude flags (also modify state_mask during parse) */
    /* Already handled by state_mask — exclude flags clear bits */
}
```

### 13.4 Performance Impact of CIDR Matching

| Operation | IPv4 | IPv6 |
|---|---|---|
| Exact match | 1 CMP (32-bit) | `memcmp(16)` — 2 x 64-bit CMP on amd64 |
| CIDR match | 1 AND + 1 CMP (32-bit) | `memcmp(N)` + 1 byte mask + 1 CMP |
| Cost per socket | ~1 ns | ~2-3 ns |
| Added latency for 100K sockets | ~0.1 ms (IPv4) | ~0.2-0.3 ms (IPv6) |

The CIDR matching cost is negligible compared to the ~5 ns per-socket
baseline of the SMR match callback. The IPv4 path uses a precomputed
32-bit netmask for single-instruction AND+CMP. The IPv6 path uses
byte-wise comparison which compiles to efficient `memcmp` on amd64.

---

## Cross-References

- **Kernel module design**: [05-kernel-module.md](05-kernel-module.md)
  - Section 12.3: `tcpstats_filter` struct definition (updated to match this document)
  - Section 12.4: `tcpstats_match()` implementation (updated for CIDR)
  - Section 12.5: Named filter profiles via sysctl
  - Section 12.7: Filter string syntax (references this document)
  - Section 12.10: Filter safety (references validation tables in this document)
- **Kernel module implementation plan**: [../../archive/kernel-module-impl-plan.md](../../archive/kernel-module-impl-plan.md)
