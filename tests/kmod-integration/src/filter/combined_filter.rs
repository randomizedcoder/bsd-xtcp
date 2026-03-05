simple_tests! {
    category: "E",
    label: "Combined Filters",
    bind: "127.0.0.16",
    e01, "port + exclude listen",             "9041",      20, "local_addr=127.0.0.16 local_port=9041 exclude=listen",                    Eq, 20;
    e02, "port + include_state established",  "9042",      20, "local_addr=127.0.0.16 local_port=9042 include_state=established",          Eq, 20;
    e03, "port + ipv4_only + exclude listen", "9043",      20, "local_addr=127.0.0.16 local_port=9043 ipv4_only exclude=listen",           Eq, 20;
    e04, "addr + port + include_state",       "9044",      20, "local_addr=127.0.0.16 local_port=9044 include_state=established",          Eq, 20;
    e05, "no match combined",                 "9045",      20, "local_addr=127.0.0.16 local_port=9999 exclude=listen",                     Eq,  0;
    e06, "format=full doesn't affect count",  "9046",      20, "local_addr=127.0.0.16 local_port=9046 format=full",                        Eq, 21;
    e07, "multi-port + exclude listen",       "9047,9048", 40, "local_addr=127.0.0.16 local_port=9047,9048 exclude=listen",                Eq, 40;
    e08, "remote_port + include_state",       "9041,9042", 40, "local_addr=127.0.0.16 remote_port=9041 include_state=established",         Eq, 20
}
