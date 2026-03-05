simple_tests! {
    category: "A",
    label: "Port Filtering",
    bind: "127.0.0.10",
    a01, "local_port match",               "9101",                          20, "local_addr=127.0.0.10 local_port=9101",                                                                          Eq, 21;
    a02, "local_port no match",            "9102",                          20, "local_addr=127.0.0.10 local_port=9999",                                                                          Eq,  0;
    a03, "multi local_port",               "9103,9104",                     40, "local_addr=127.0.0.10 local_port=9103,9104",                                                                     Eq, 42;
    a04, "remote_port match",              "9105",                          20, "local_addr=127.0.0.10 remote_port=9105",                                                                         Eq, 20;
    a05, "remote_port no match",           "9106",                          20, "local_addr=127.0.0.10 remote_port=9999",                                                                         Eq,  0;
    a06, "local AND remote port no overlap","9107",                         20, "local_addr=127.0.0.10 local_port=9107 remote_port=9107",                                                         Eq,  0;
    a07, "3-port multi local_port",        "9108,9109,9110",                30, "local_addr=127.0.0.10 local_port=9108,9109,9110",                                                                Eq, 33;
    a08, "single of multi-port server",    "9111,9112,9113",                30, "local_addr=127.0.0.10 local_port=9111",                                                                          Eq, 11;
    a09, "8-port full",                    "9114,9115,9116,9117,9118,9119,9120,9121", 80, "local_addr=127.0.0.10 local_port=9114,9115,9116,9117,9118,9119,9120,9121",                              Eq, 88;
    a10, "multi remote_port",              "9122,9123",                     40, "local_addr=127.0.0.10 remote_port=9122,9123",                                                                    Eq, 40;
    a11, "local_port + exclude listen",    "9124",                          20, "local_addr=127.0.0.10 local_port=9124 exclude=listen",                                                           Eq, 20;
    a12, "high port edge case",            "60000",                         10, "local_addr=127.0.0.10 local_port=60000",                                                                         Eq, 11
}
