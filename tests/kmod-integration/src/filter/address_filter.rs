use std::path::Path;

use crate::framework::check::{CheckOp, check_count};
use crate::framework::process::ProcessGroup;

/// Category D: Address Filtering
/// D01-D12: shared IPv4 fixture (3 servers on .13/.14/.15)
/// D13-D18: shared IPv6 fixture (2 servers on fd00::13/fd00::14)
pub fn tests(tcp_echo: &str, read_tcpstats: &str, output_dir: Option<&Path>) -> Vec<libtest_mimic::Trial> {
    let mut trials = Vec::new();
    let output_dir = output_dir.map(|p| p.to_path_buf());

    // --- IPv4 fixture (D01-D12) ---
    let ipv4_fixture: std::sync::Arc<std::sync::Mutex<Option<ProcessGroup>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));

    struct Case {
        id: &'static str,
        filter: &'static str,
        op: CheckOp,
        expected: u64,
    }

    let ipv4_cases = [
        Case { id: "d01", filter: "local_addr=127.0.0.13",                                    op: CheckOp::Ge, expected: 21 },
        Case { id: "d02", filter: "local_addr=127.0.0.14",                                    op: CheckOp::Ge, expected: 21 },
        Case { id: "d03", filter: "local_addr=127.0.0.15",                                    op: CheckOp::Ge, expected: 21 },
        Case { id: "d04", filter: "local_addr=127.0.0.0/8",                                   op: CheckOp::Ge, expected: 63 },
        Case { id: "d05", filter: "local_addr=10.0.0.0/8",                                    op: CheckOp::Ge, expected:  0 },
        Case { id: "d06", filter: "remote_addr=127.0.0.13",                                   op: CheckOp::Ge, expected: 20 },
        Case { id: "d07", filter: "remote_addr=127.0.0.14",                                   op: CheckOp::Ge, expected: 20 },
        Case { id: "d08", filter: "local_addr=127.0.0.13 remote_addr=127.0.0.13",             op: CheckOp::Ge, expected: 20 },
        Case { id: "d09", filter: "local_addr=127.0.0.13 remote_addr=127.0.0.14",             op: CheckOp::Ge, expected:  0 },
        Case { id: "d10", filter: "local_addr=127.0.0.13 local_port=9031",                    op: CheckOp::Eq, expected: 11 },
        Case { id: "d11", filter: "local_addr=127.0.0.13 exclude=listen",                     op: CheckOp::Ge, expected: 20 },
        Case { id: "d12", filter: "remote_addr=192.168.0.0/16",                               op: CheckOp::Ge, expected:  0 },
    ];

    for case in ipv4_cases {
        let tcp_echo = tcp_echo.to_string();
        let read_tcpstats = read_tcpstats.to_string();
        let fixture = ipv4_fixture.clone();
        let output_dir = output_dir.clone();

        trials.push(libtest_mimic::Trial::test(
            format!("D::{}", case.id),
            move || {
                // Lazy-init IPv4 fixture
                {
                    let mut guard = fixture.lock().unwrap();
                    if guard.is_none() {
                        let mut procs = ProcessGroup::new_in(output_dir.as_deref());
                        // Server 1: 127.0.0.13:9031
                        procs.start_server(&tcp_echo, "127.0.0.13", "9031", 600)
                            .map_err(|e| format!("{e}"))?;
                        procs.start_clients(&tcp_echo, "127.0.0.13", "9031", 10, 600)
                            .map_err(|e| format!("{e}"))?;
                        // Server 2: 127.0.0.14:9032
                        procs.start_server(&tcp_echo, "127.0.0.14", "9032", 600)
                            .map_err(|e| format!("{e}"))?;
                        procs.start_clients(&tcp_echo, "127.0.0.14", "9032", 10, 600)
                            .map_err(|e| format!("{e}"))?;
                        // Server 3: 127.0.0.15:9033
                        procs.start_server(&tcp_echo, "127.0.0.15", "9033", 600)
                            .map_err(|e| format!("{e}"))?;
                        procs.start_clients(&tcp_echo, "127.0.0.15", "9033", 10, 600)
                            .map_err(|e| format!("{e}"))?;
                        *guard = Some(procs);
                    }
                }

                check_count(&read_tcpstats, case.filter, case.op, case.expected, case.id)
                    .map_err(|e| format!("{e}"))?;
                Ok(())
            },
        ));
    }

    // --- IPv6 fixture (D13-D18) ---
    let ipv6_fixture: std::sync::Arc<std::sync::Mutex<Option<ProcessGroup>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));

    let ipv6_cases = [
        Case { id: "d13", filter: "local_addr=fd00::13",                                      op: CheckOp::Ge, expected: 21 },
        Case { id: "d14", filter: "local_addr=fd00::14",                                      op: CheckOp::Ge, expected: 21 },
        Case { id: "d15", filter: "local_addr=fd00::/16",                                     op: CheckOp::Ge, expected: 42 },
        Case { id: "d16", filter: "local_addr=fd00::13 remote_addr=fd00::14",                  op: CheckOp::Ge, expected:  0 },
        Case { id: "d17", filter: "local_addr=fd00::13 exclude=listen",                        op: CheckOp::Ge, expected: 20 },
        Case { id: "d18", filter: "remote_addr=fd00::13 local_port=9037",                      op: CheckOp::Eq, expected: 10 },
    ];

    for case in ipv6_cases {
        let tcp_echo = tcp_echo.to_string();
        let read_tcpstats = read_tcpstats.to_string();
        let fixture = ipv6_fixture.clone();
        let output_dir = output_dir.clone();

        trials.push(libtest_mimic::Trial::test(
            format!("D::{}", case.id),
            move || {
                // Lazy-init IPv6 fixture
                {
                    let mut guard = fixture.lock().unwrap();
                    if guard.is_none() {
                        let mut procs = ProcessGroup::new_in(output_dir.as_deref());
                        // Server 1: fd00::13:9037
                        procs.start_server(&tcp_echo, "fd00::13", "9037", 600)
                            .map_err(|e| format!("{e}"))?;
                        procs.start_clients(&tcp_echo, "fd00::13", "9037", 10, 600)
                            .map_err(|e| format!("{e}"))?;
                        // Server 2: fd00::14:9038
                        procs.start_server(&tcp_echo, "fd00::14", "9038", 600)
                            .map_err(|e| format!("{e}"))?;
                        procs.start_clients(&tcp_echo, "fd00::14", "9038", 10, 600)
                            .map_err(|e| format!("{e}"))?;
                        *guard = Some(procs);
                    }
                }

                check_count(&read_tcpstats, case.filter, case.op, case.expected, case.id)
                    .map_err(|e| format!("{e}"))?;
                Ok(())
            },
        ));
    }

    trials
}
