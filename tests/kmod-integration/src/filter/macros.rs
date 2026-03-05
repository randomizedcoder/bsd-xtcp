/// Generate libtest-mimic test entries for tests where each test gets its own
/// server+client fixture (start, test, teardown).
///
/// Usage:
/// ```
/// simple_tests! {
///     category: "A",
///     label: "Port Filtering",
///     bind: "127.0.0.10",
///     // id, name, ports, conns, filter, op, expected
///     a01, "local_port match", "9001", 20, "local_addr=127.0.0.10 local_port=9001", Eq, 21;
///     a02, "local_port no match", "9001", 20, "local_addr=127.0.0.10 local_port=9999", Eq, 0;
/// }
/// ```
macro_rules! simple_tests {
    (
        category: $cat:expr,
        label: $label:expr,
        bind: $bind:expr,
        $( $id:ident, $name:expr, $ports:expr, $conns:expr, $filter:expr, $op:ident, $expected:expr );+ $(;)?
    ) => {
        pub fn tests(
            tcp_echo: &str,
            read_tcpstats: &str,
            output_dir: Option<&std::path::Path>,
        ) -> Vec<libtest_mimic::Trial> {
            let tcp_echo = tcp_echo.to_string();
            let read_tcpstats = read_tcpstats.to_string();
            let output_dir = output_dir.map(|p| p.to_path_buf());

            vec![
                $(
                    {
                        let tcp_echo = tcp_echo.clone();
                        let read_tcpstats = read_tcpstats.clone();
                        let output_dir = output_dir.clone();
                        libtest_mimic::Trial::test(
                            format!("{}::{}", $cat, stringify!($id)),
                            move || {
                                use std::io::Write;
                                use crate::framework::check::{CheckOp, check_count};
                                use crate::framework::process::ProcessGroup;

                                let log_file = output_dir.as_ref().map(|dir| {
                                    let cat_dir = dir.join(
                                        format!("{}_{}", $cat, $label.to_lowercase().replace(' ', "_"))
                                    );
                                    let _ = std::fs::create_dir_all(&cat_dir);
                                    cat_dir.join(format!("{}.log", stringify!($id)))
                                });

                                let mut procs = ProcessGroup::new_in(output_dir.as_deref());
                                procs.start_server(&tcp_echo, $bind, $ports, 600)
                                    .map_err(|e| format!("{e}"))?;
                                procs.start_clients(&tcp_echo, $bind, $ports, $conns, 600)
                                    .map_err(|e| format!("{e}"))?;

                                let op = CheckOp::$op;
                                let result = check_count(
                                    &read_tcpstats,
                                    $filter,
                                    op,
                                    $expected,
                                    concat!(stringify!($id), ": ", $name),
                                );

                                if let Some(ref path) = log_file {
                                    if let Ok(mut f) = std::fs::File::create(path) {
                                        let ts = chrono::Local::now()
                                            .format("%Y-%m-%dT%H:%M:%S%.3f");
                                        let _ = writeln!(f, "[{ts}] test: {}::{}", $cat, stringify!($id));
                                        let _ = writeln!(f, "[{ts}] filter: {}", $filter);
                                        let _ = writeln!(f, "[{ts}] expected: {} ({:?})", $expected, op);
                                        let status = if result.is_ok() { "PASS" } else { "FAIL" };
                                        let _ = writeln!(f, "[{ts}] result: {status}");
                                        if let Err(ref e) = result {
                                            let _ = writeln!(f, "[{ts}] error: {e:#}");
                                        }
                                    }
                                }

                                result.map_err(|e| format!("{e}"))?;

                                procs.kill_all();
                                Ok(())
                            },
                        )
                    },
                )+
            ]
        }
    };
}

/// Generate libtest-mimic test entries that share a common fixture setup.
/// The fixture function is called once, and all tests run against that state.
///
/// Usage:
/// ```
/// shared_fixture_tests! {
///     category: "I",
///     label: "Combinatorial Coverage",
///     fixture: setup_combinatorial,
///     // id, name, filter, op, expected
///     i01, "RP+IV match", "remote_port=9081 ipv4_only local_addr=127.0.0.19", Eq, 10;
/// }
/// ```
macro_rules! shared_fixture_tests {
    (
        category: $cat:expr,
        label: $label:expr,
        fixture: $fixture:ident,
        tcp_echo: $tcp_echo_arg:ident,
        read_tcpstats: $read_arg:ident,
        $( $id:ident, $name:expr, $filter:expr, $op:ident, $expected:expr );+ $(;)?
    ) => {
        pub fn tests(
            tcp_echo: &str,
            read_tcpstats: &str,
            output_dir: Option<&std::path::Path>,
        ) -> Vec<libtest_mimic::Trial> {
            let tcp_echo = tcp_echo.to_string();
            let read_tcpstats = read_tcpstats.to_string();
            let output_dir = output_dir.map(|p| p.to_path_buf());

            // Wrap the fixture in Arc<Mutex<Option<...>>> for lazy init + shared teardown
            let fixture: std::sync::Arc<std::sync::Mutex<Option<FixtureState>>> =
                std::sync::Arc::new(std::sync::Mutex::new(None));

            vec![
                $(
                    {
                        let tcp_echo = tcp_echo.clone();
                        let read_tcpstats = read_tcpstats.clone();
                        let fixture = fixture.clone();
                        let output_dir = output_dir.clone();
                        libtest_mimic::Trial::test(
                            format!("{}::{}", $cat, stringify!($id)),
                            move || {
                                use std::io::Write;
                                use crate::framework::check::{CheckOp, check_count};

                                let log_file = output_dir.as_ref().map(|dir| {
                                    let cat_dir = dir.join(
                                        format!("{}_{}", $cat, $label.to_lowercase().replace(' ', "_"))
                                    );
                                    let _ = std::fs::create_dir_all(&cat_dir);
                                    cat_dir.join(format!("{}.log", stringify!($id)))
                                });

                                // Lazy-init fixture on first test
                                {
                                    let mut guard = fixture.lock().unwrap();
                                    if guard.is_none() {
                                        let state = $fixture(&tcp_echo)
                                            .map_err(|e| format!("fixture setup: {e}"))?;
                                        *guard = Some(state);
                                    }
                                }

                                let op = CheckOp::$op;
                                let result = check_count(
                                    &read_tcpstats,
                                    $filter,
                                    op,
                                    $expected,
                                    concat!(stringify!($id), ": ", $name),
                                );

                                if let Some(ref path) = log_file {
                                    if let Ok(mut f) = std::fs::File::create(path) {
                                        let ts = chrono::Local::now()
                                            .format("%Y-%m-%dT%H:%M:%S%.3f");
                                        let _ = writeln!(f, "[{ts}] test: {}::{}", $cat, stringify!($id));
                                        let _ = writeln!(f, "[{ts}] filter: {}", $filter);
                                        let _ = writeln!(f, "[{ts}] expected: {} ({:?})", $expected, op);
                                        let status = if result.is_ok() { "PASS" } else { "FAIL" };
                                        let _ = writeln!(f, "[{ts}] result: {status}");
                                        if let Err(ref e) = result {
                                            let _ = writeln!(f, "[{ts}] error: {e:#}");
                                        }
                                    }
                                }

                                result.map_err(|e| format!("{e}"))?;

                                Ok(())
                            },
                        )
                    },
                )+
            ]
        }
    };
}

pub(crate) use shared_fixture_tests;
