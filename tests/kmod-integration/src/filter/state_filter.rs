simple_tests! {
    category: "B",
    label: "State Filtering",
    bind: "127.0.0.11",
    b01, "exclude listen",                "9011", 20, "local_addr=127.0.0.11 local_port=9011 exclude=listen",                                                                                              Eq, 20;
    b02, "exclude established",           "9017", 20, "local_addr=127.0.0.11 local_port=9017 exclude=established",                                                                                         Eq,  1;
    b03, "exclude listen,time_wait",      "9018", 20, "local_addr=127.0.0.11 local_port=9018 exclude=listen,time_wait",                                                                                    Eq, 20;
    b04, "include_state established",     "9012", 20, "local_addr=127.0.0.11 local_port=9012 include_state=established",                                                                                   Eq, 20;
    b05, "include_state listen",          "9013", 20, "local_addr=127.0.0.11 local_port=9013 include_state=listen",                                                                                        Eq,  1;
    b06, "include_state established,listen","9014",20,"local_addr=127.0.0.11 local_port=9014 include_state=established,listen",                                                                            Eq, 21;
    b07, "exclude all states",            "9015", 20, "local_addr=127.0.0.11 local_port=9015 exclude=closed,listen,syn_sent,syn_received,established,close_wait,fin_wait_1,closing,last_ack,fin_wait_2,time_wait", Eq, 0;
    b08, "include_state syn_sent",        "9016", 20, "local_addr=127.0.0.11 local_port=9016 include_state=syn_sent",                                                                                     Eq,  0
}
