simple_tests! {
    category: "F",
    label: "Format and Fields",
    bind: "127.0.0.17",
    f01, "fields=identity,state",  "9051", 20, "local_addr=127.0.0.17 local_port=9051 fields=identity,state", Eq, 21;
    f02, "fields=all",             "9052", 20, "local_addr=127.0.0.17 local_port=9052 fields=all",             Eq, 21;
    f03, "format=compact",         "9053", 20, "local_addr=127.0.0.17 local_port=9053 format=compact",         Eq, 21
}
