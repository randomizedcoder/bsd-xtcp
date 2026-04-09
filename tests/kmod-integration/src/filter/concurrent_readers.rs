use std::path::Path;
use std::thread;

use crate::framework::check::read_count;
use crate::framework::process::ProcessGroup;

/// Category H: Concurrent Readers
pub fn tests(
    tcp_echo: &str,
    read_tcpstats: &str,
    output_dir: Option<&Path>,
) -> Vec<libtest_mimic::Trial> {
    let mut trials = Vec::new();
    let output_dir = output_dir.map(|p| p.to_path_buf());

    // H01: 4 concurrent readers, same filter
    {
        let tcp_echo = tcp_echo.to_string();
        let read_tcpstats = read_tcpstats.to_string();
        let output_dir = output_dir.clone();
        trials.push(libtest_mimic::Trial::test("H::h01", move || {
            let mut procs = ProcessGroup::new_in(output_dir.as_deref());
            procs
                .start_server(&tcp_echo, "127.0.0.18", "9071", 600)
                .map_err(|e| format!("{e}"))?;
            procs
                .start_clients(&tcp_echo, "127.0.0.18", "9071", 50, 600)
                .map_err(|e| format!("{e}"))?;

            let filter = "local_addr=127.0.0.18 local_port=9071";
            // At least 1 LISTEN + some established connections
            let min_expected = 11u64;

            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let rt = read_tcpstats.clone();
                    let f = filter.to_string();
                    thread::spawn(move || read_count(&rt, &f))
                })
                .collect();

            let mut counts = Vec::new();
            for (i, h) in handles.into_iter().enumerate() {
                let count = h.join().unwrap().map_err(|e| format!("reader {i}: {e}"))?;
                if count < min_expected {
                    return Err(format!(
                        "h01: reader {i}: expected >= {min_expected}, got {count}"
                    )
                    .into());
                }
                counts.push(count);
            }

            // All 4 readers should see the same count
            if counts.windows(2).any(|w| w[0] != w[1]) {
                return Err(format!("h01: readers saw different counts: {counts:?}").into());
            }

            procs.kill_all();
            Ok(())
        }));
    }

    // H02: 4 readers, 2 filters
    {
        let tcp_echo = tcp_echo.to_string();
        let read_tcpstats = read_tcpstats.to_string();
        let output_dir = output_dir.clone();
        trials.push(libtest_mimic::Trial::test("H::h02", move || {
            let mut procs = ProcessGroup::new_in(output_dir.as_deref());
            procs
                .start_server(&tcp_echo, "127.0.0.18", "9071,9072", 600)
                .map_err(|e| format!("{e}"))?;
            procs
                .start_clients(&tcp_echo, "127.0.0.18", "9071,9072", 40, 600)
                .map_err(|e| format!("{e}"))?;

            let filters = [
                "local_addr=127.0.0.18 local_port=9071",
                "local_addr=127.0.0.18 local_port=9072",
            ];
            let expected = 21u64;

            let handles: Vec<_> = (0..4)
                .map(|i| {
                    let rt = read_tcpstats.clone();
                    let f = filters[i % 2].to_string();
                    thread::spawn(move || read_count(&rt, &f))
                })
                .collect();

            for (i, h) in handles.into_iter().enumerate() {
                let count = h.join().unwrap().map_err(|e| format!("reader {i}: {e}"))?;
                if count != expected {
                    return Err(format!("h02: reader {i}: expected {expected}, got {count}").into());
                }
            }

            procs.kill_all();
            Ok(())
        }));
    }

    // H03: 8 concurrent readers (no EBUSY)
    {
        let tcp_echo = tcp_echo.to_string();
        let read_tcpstats = read_tcpstats.to_string();
        let output_dir = output_dir.clone();
        trials.push(libtest_mimic::Trial::test("H::h03", move || {
            let mut procs = ProcessGroup::new_in(output_dir.as_deref());
            procs
                .start_server(&tcp_echo, "127.0.0.18", "9073", 600)
                .map_err(|e| format!("{e}"))?;
            procs
                .start_clients(&tcp_echo, "127.0.0.18", "9073", 50, 600)
                .map_err(|e| format!("{e}"))?;

            let filter = "local_addr=127.0.0.18 local_port=9073";

            // 8 sequential reads — all must succeed (no EBUSY)
            for i in 0..8 {
                let count = read_count(&read_tcpstats, filter)
                    .map_err(|e| format!("h03: reader {i}: {e}"))?;
                if count == 0 {
                    return Err(format!("h03: reader {i}: unexpected 0 count").into());
                }
            }

            procs.kill_all();
            Ok(())
        }));
    }

    trials
}
