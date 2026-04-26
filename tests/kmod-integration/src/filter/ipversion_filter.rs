use std::path::Path;

use crate::framework::check::{check_count, CheckOp};
use crate::framework::process::ProcessGroup;

/// Category C: IP Version Filtering
/// Tests C01-C06 use simple_tests pattern, C07-C08 need dual-stack setup.
pub fn tests(
    tcp_echo: &str,
    read_tcpstats: &str,
    output_dir: Option<&Path>,
) -> Vec<libtest_mimic::Trial> {
    let mut trials = Vec::new();
    let output_dir = output_dir.map(|p| p.to_path_buf());

    // C01-C06: simple tests (each with own fixture)
    struct SimpleCase {
        id: &'static str,
        bind: &'static str,
        ports: &'static str,
        conns: u32,
        filter: &'static str,
        op: CheckOp,
        expected: u64,
    }

    let simple_cases = [
        SimpleCase {
            id: "c01",
            bind: "127.0.0.12",
            ports: "9021",
            conns: 20,
            filter: "local_addr=127.0.0.12 ipv4_only",
            op: CheckOp::Eq,
            expected: 41,
        },
        SimpleCase {
            id: "c02",
            bind: "127.0.0.12",
            ports: "9021",
            conns: 20,
            filter: "ipv6_only local_port=9021",
            op: CheckOp::Eq,
            expected: 0,
        },
        SimpleCase {
            id: "c03",
            bind: "fd00::10",
            ports: "9022",
            conns: 10,
            filter: "ipv6_only local_port=9022",
            op: CheckOp::Eq,
            expected: 11,
        },
        SimpleCase {
            id: "c04",
            bind: "fd00::10",
            ports: "9022",
            conns: 10,
            filter: "ipv4_only local_port=9022",
            op: CheckOp::Eq,
            expected: 0,
        },
        SimpleCase {
            id: "c05",
            bind: "fd00::10",
            ports: "9023",
            conns: 10,
            filter: "local_addr=fd00::10",
            op: CheckOp::Ge,
            expected: 21,
        },
        SimpleCase {
            id: "c06",
            bind: "fd00::10",
            ports: "9024",
            conns: 10,
            filter: "local_addr=fd00::/16",
            op: CheckOp::Ge,
            expected: 21,
        },
    ];

    for case in simple_cases {
        let tcp_echo = tcp_echo.to_string();
        let read_tcpstats = read_tcpstats.to_string();
        let output_dir = output_dir.clone();

        trials.push(libtest_mimic::Trial::test(
            format!("C::{}", case.id),
            move || {
                let mut procs = ProcessGroup::new_in(output_dir.as_deref());
                procs
                    .start_server(&tcp_echo, case.bind, case.ports, 600)
                    .map_err(|e| format!("{e}"))?;
                procs
                    .start_clients(&tcp_echo, case.bind, case.ports, case.conns, 600)
                    .map_err(|e| format!("{e}"))?;

                check_count(&read_tcpstats, case.filter, case.op, case.expected, case.id)
                    .map_err(|e| format!("{e}"))?;

                procs.kill_all();
                Ok(())
            },
        ));
    }

    // C07-C08: dual-stack tests (shared fixture: IPv4 + IPv6 servers on same port)
    let fixture: std::sync::Arc<std::sync::Mutex<Option<ProcessGroup>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));

    struct DualCase {
        id: &'static str,
        filter: &'static str,
        op: CheckOp,
        expected: u64,
    }

    let dual_cases = [
        DualCase {
            id: "c07",
            filter: "ipv4_only local_port=9025",
            op: CheckOp::Ge,
            expected: 11,
        },
        DualCase {
            id: "c08",
            filter: "ipv6_only local_port=9025",
            op: CheckOp::Ge,
            expected: 11,
        },
    ];

    for case in dual_cases {
        let tcp_echo = tcp_echo.to_string();
        let read_tcpstats = read_tcpstats.to_string();
        let fixture = fixture.clone();
        let output_dir = output_dir.clone();

        trials.push(libtest_mimic::Trial::test(
            format!("C::{}", case.id),
            move || {
                // Lazy-init dual-stack fixture
                {
                    let mut guard = fixture.lock().unwrap();
                    if guard.is_none() {
                        let mut procs = ProcessGroup::new_in(output_dir.as_deref());
                        // IPv4 server + clients
                        procs
                            .start_server(&tcp_echo, "127.0.0.12", "9025", 600)
                            .map_err(|e| format!("{e}"))?;
                        procs
                            .start_clients(&tcp_echo, "127.0.0.12", "9025", 10, 600)
                            .map_err(|e| format!("{e}"))?;
                        // IPv6 server + clients
                        procs
                            .start_server(&tcp_echo, "fd00::10", "9025", 600)
                            .map_err(|e| format!("{e}"))?;
                        procs
                            .start_clients(&tcp_echo, "fd00::10", "9025", 10, 600)
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
