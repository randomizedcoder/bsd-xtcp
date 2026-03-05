use anyhow::Result;

use crate::framework::process::ProcessGroup;

/// Fixture state: 3 servers (2 IPv4, 1 IPv6) with clients.
struct FixtureState {
    _procs: ProcessGroup,
}

fn setup_combinatorial(tcp_echo: &str) -> Result<FixtureState> {
    let mut procs = ProcessGroup::new();

    // S1: 127.0.0.19:9081, 10 IPv4 conns
    procs.start_server(tcp_echo, "127.0.0.19", "9081", 600)?;
    procs.start_clients(tcp_echo, "127.0.0.19", "9081", 10, 600)?;

    // S2: 127.0.0.19:9082, 10 IPv4 conns
    procs.start_server(tcp_echo, "127.0.0.19", "9082", 600)?;
    procs.start_clients(tcp_echo, "127.0.0.19", "9082", 10, 600)?;

    // S3: fd00::19:9083, 10 IPv6 conns
    procs.start_server(tcp_echo, "fd00::19", "9083", 600)?;
    procs.start_clients(tcp_echo, "fd00::19", "9083", 10, 600)?;

    Ok(FixtureState { _procs: procs })
}

crate::filter::macros::shared_fixture_tests! {
    category: "I",
    label: "Combinatorial Coverage",
    fixture: setup_combinatorial,
    tcp_echo: tcp_echo,
    read_tcpstats: read_tcpstats,

    // === 2-Way Pairs ===
    i01, "{RP,IV} match: RP=9081 ipv4",            "remote_port=9081 ipv4_only local_addr=127.0.0.19",                                           Eq, 10;
    i02, "{RP,IV} match: RP=9083 ipv6",            "remote_port=9083 ipv6_only local_addr=fd00::19",                                              Eq, 10;
    i03, "{RP,IV} conflict: RP=9081 ipv6",         "remote_port=9081 ipv6_only",                                                                  Eq,  0;
    i04, "{RA,ST} match: estab",                   "remote_addr=127.0.0.19 include_state=established local_addr=127.0.0.19",                      Ge, 40;
    i05, "{RA,ST} conflict: listen",               "remote_addr=127.0.0.19 include_state=listen local_addr=127.0.0.19",                           Ge,  0;
    i06, "{RA,ST} conflict: excl estab",           "remote_addr=127.0.0.19 exclude=established local_addr=127.0.0.19",                            Ge,  0;
    i07, "{RA,IV} match: v4",                      "remote_addr=127.0.0.19 ipv4_only local_addr=127.0.0.19",                                      Eq, 40;
    i08, "{RA,IV} match: v6",                      "remote_addr=fd00::19 ipv6_only local_addr=fd00::19",                                           Eq, 20;
    i09, "{RA,IV} conflict: v4 addr v6 flag",      "remote_addr=127.0.0.19 ipv6_only",                                                            Eq,  0;
    i10, "{ST,IV} match: estab v4",                "include_state=established ipv4_only local_addr=127.0.0.19",                                    Eq, 40;
    i11, "{ST,IV} match: listen v6",               "include_state=listen ipv6_only local_addr=fd00::19",                                           Eq,  1;
    i12, "{ST,IV} match: excl listen v6",          "exclude=listen ipv6_only local_addr=fd00::19",                                                 Eq, 20;

    // === 3-Way Triples ===
    i13, "{LP,RP,ST} conflict",                    "local_port=9081 remote_port=9081 include_state=established local_addr=127.0.0.19",             Eq,  0;
    i14, "{LP,RP,IV} conflict",                    "local_port=9081 remote_port=9081 ipv4_only local_addr=127.0.0.19",                             Eq,  0;
    i15, "{LP,RP,RA} conflict",                    "local_port=9081 remote_port=9082 remote_addr=127.0.0.19 local_addr=127.0.0.19",               Eq,  0;
    i16, "{LP,LA,RA} match",                       "local_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19",                                 Eq, 10;
    i17, "{LP,LA,RA} conflict: bad RA",            "local_port=9081 local_addr=127.0.0.19 remote_addr=10.0.0.1",                                   Eq,  0;
    i18, "{LP,IV,RA} match: v4",                   "local_port=9081 ipv4_only remote_addr=127.0.0.19 local_addr=127.0.0.19",                       Eq, 10;
    i19, "{LP,IV,RA} match: v6",                   "local_port=9083 ipv6_only remote_addr=fd00::19 local_addr=fd00::19",                           Eq, 10;
    i20, "{LP,IV,RA} conflict",                    "local_port=9081 ipv6_only remote_addr=127.0.0.19",                                             Eq,  0;
    i21, "{RP,LA,IV} match: v4",                   "remote_port=9081 local_addr=127.0.0.19 ipv4_only",                                             Eq, 10;
    i22, "{RP,LA,IV} match: v6",                   "remote_port=9083 local_addr=fd00::19 ipv6_only",                                               Eq, 10;
    i23, "{RP,LA,IV} conflict",                    "remote_port=9081 ipv6_only local_addr=fd00::19",                                               Eq,  0;
    i24, "{RP,LA,RA} match",                       "remote_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19",                                Eq, 10;
    i25, "{RP,LA,RA} conflict: bad RA",            "remote_port=9081 local_addr=127.0.0.19 remote_addr=10.0.0.1",                                  Eq,  0;
    i26, "{RP,ST,IV} match",                       "remote_port=9081 include_state=established ipv4_only local_addr=127.0.0.19",                   Eq, 10;
    i27, "{RP,ST,IV} conflict: listen",            "remote_port=9081 include_state=listen ipv4_only local_addr=127.0.0.19",                        Eq,  0;
    i28, "{RP,ST,RA} match",                       "remote_port=9081 include_state=established remote_addr=127.0.0.19 local_addr=127.0.0.19",     Eq, 10;
    i29, "{RP,ST,RA} conflict: excl estab",        "remote_port=9081 exclude=established remote_addr=127.0.0.19 local_addr=127.0.0.19",           Eq,  0;
    i30, "{RP,IV,RA} match",                       "remote_port=9081 ipv4_only remote_addr=127.0.0.19 local_addr=127.0.0.19",                     Eq, 10;
    i31, "{RP,IV,RA} conflict",                    "remote_port=9081 ipv6_only remote_addr=127.0.0.19",                                            Eq,  0;
    i32, "{LA,ST,IV} match: v4 estab",             "local_addr=127.0.0.19 include_state=established ipv4_only",                                    Eq, 40;
    i33, "{LA,ST,IV} match: v6 excl listen",       "local_addr=fd00::19 exclude=listen ipv6_only",                                                 Eq, 20;
    i34, "{LA,ST,IV} conflict: v4 listen v6",      "local_addr=127.0.0.19 include_state=listen ipv6_only",                                         Eq,  0;
    i35, "{LA,ST,RA} match",                       "local_addr=127.0.0.19 include_state=established remote_addr=127.0.0.19",                       Ge, 40;
    i36, "{LA,ST,RA} conflict",                    "local_addr=127.0.0.19 exclude=established remote_addr=127.0.0.19",                             Ge,  0;
    i37, "{LA,IV,RA} match: v4",                   "local_addr=127.0.0.19 ipv4_only remote_addr=127.0.0.19",                                       Eq, 40;
    i38, "{LA,IV,RA} match: v6",                   "local_addr=fd00::19 ipv6_only remote_addr=fd00::19",                                           Eq, 20;
    i39, "{LA,IV,RA} conflict",                    "local_addr=127.0.0.19 ipv6_only remote_addr=127.0.0.19",                                       Eq,  0;
    i40, "{ST,IV,RA} match: v4 estab",             "include_state=established ipv4_only remote_addr=127.0.0.19 local_addr=127.0.0.19",             Eq, 40;
    i41, "{ST,IV,RA} match: v6 estab",             "include_state=established ipv6_only remote_addr=fd00::19 local_addr=fd00::19",                 Eq, 20;
    i42, "{ST,IV,RA} conflict: listen v4 RA",      "include_state=listen ipv4_only remote_addr=127.0.0.19 local_addr=127.0.0.19",                 Eq,  0;

    // === 4-Way Combinations ===
    i43, "{LP,RA,ST,IV} match",                    "local_port=9081 remote_addr=127.0.0.19 include_state=established ipv4_only local_addr=127.0.0.19", Eq, 10;
    i44, "{LP,RA,ST,IV} conflict: listen",         "local_port=9081 remote_addr=127.0.0.19 include_state=listen ipv4_only local_addr=127.0.0.19",      Eq,  0;
    i45, "{RP,LA,ST,IV} match: v4",                "remote_port=9081 local_addr=127.0.0.19 include_state=established ipv4_only",                       Eq, 10;
    i46, "{RP,LA,ST,IV} match: v6",                "remote_port=9083 local_addr=fd00::19 include_state=established ipv6_only",                         Eq, 10;
    i47, "{LA,RA,ST,IV} match: v4",                "local_addr=127.0.0.19 remote_addr=127.0.0.19 include_state=established ipv4_only",                 Eq, 40;
    i48, "{LA,RA,ST,IV} match: v6",                "local_addr=fd00::19 remote_addr=fd00::19 include_state=established ipv6_only",                     Eq, 20;
    i49, "{LA,RA,ST,IV} conflict: listen",         "local_addr=127.0.0.19 remote_addr=127.0.0.19 include_state=listen ipv4_only",                     Eq,  0;
    i50, "{LP,LA,RA,ST} match",                    "local_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19 include_state=established",           Eq, 10;
    i51, "{LP,LA,RA,ST} conflict: listen",         "local_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19 include_state=listen",                Eq,  0;
    i52, "{LP,LA,RA,IV} match: v4",                "local_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19 ipv4_only",                           Eq, 10;
    i53, "{LP,LA,RA,IV} match: v6",                "local_port=9083 local_addr=fd00::19 remote_addr=fd00::19 ipv6_only",                               Eq, 10;
    i54, "{LP,LA,ST,IV} match",                    "local_port=9082 local_addr=127.0.0.19 include_state=established ipv4_only",                        Eq, 10;
    i55, "{RP,LA,RA,ST} match",                    "remote_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19 include_state=established",          Eq, 10;
    i56, "{RP,LA,RA,ST} conflict: listen",         "remote_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19 include_state=listen",               Eq,  0;
    i57, "{RP,LA,RA,IV} match: v4",                "remote_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19 ipv4_only",                          Eq, 10;
    i58, "{RP,LA,RA,IV} conflict: bad RA",         "remote_port=9081 local_addr=127.0.0.19 remote_addr=10.0.0.1 ipv4_only",                            Eq,  0;
    i59, "{RP,RA,ST,IV} match",                    "remote_port=9081 remote_addr=127.0.0.19 include_state=established ipv4_only local_addr=127.0.0.19", Eq, 10;
    i60, "{RP,RA,ST,IV} conflict: listen v6",      "remote_port=9081 remote_addr=127.0.0.19 include_state=listen ipv6_only",                           Eq,  0;

    // === 5-Way Combinations ===
    i61, "{LP,LA,RA,ST,IV} match: v4",             "local_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19 include_state=established ipv4_only", Eq, 10;
    i62, "{LP,LA,RA,ST,IV} match: v6",             "local_port=9083 local_addr=fd00::19 remote_addr=fd00::19 include_state=established ipv6_only",     Eq, 10;
    i63, "{RP,LA,RA,ST,IV} match: v4",             "remote_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19 include_state=established ipv4_only", Eq, 10;
    i64, "{RP,LA,RA,ST,IV} match: v6",             "remote_port=9083 local_addr=fd00::19 remote_addr=fd00::19 include_state=established ipv6_only",    Eq, 10;
    i65, "{LP,RP,LA,RA,ST} conflict: LP=RP",       "local_port=9081 remote_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19 include_state=established", Eq, 0;
    i66, "{LP,RP,LA,ST,IV} conflict: LP=RP",       "local_port=9081 remote_port=9081 local_addr=127.0.0.19 include_state=established ipv4_only",      Eq,  0;

    // === 6-Way (All Dimensions) ===
    i67, "{all} LP=RP conflict",                   "local_port=9081 remote_port=9081 local_addr=127.0.0.19 remote_addr=127.0.0.19 include_state=established ipv4_only", Eq, 0;
    i68, "{all} LP!=RP cross-port",                "local_port=9081 remote_port=9082 local_addr=127.0.0.19 remote_addr=127.0.0.19 include_state=established ipv4_only", Eq, 0;
    i69, "{all} LP!=RP excl estab",                "local_port=9081 remote_port=9082 local_addr=127.0.0.19 remote_addr=127.0.0.19 exclude=established ipv4_only",       Eq, 0
}
