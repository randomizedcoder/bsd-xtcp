use std::path::Path;

use crate::framework::check::read_count;
use crate::framework::process::{run_cmd, ProcessGroup};
use crate::framework::system::sysctl_set;

/// Check if named profile support is available in the kmod.
fn profiles_available() -> bool {
    // Try to read the profile_delete sysctl — if it doesn't exist, profiles aren't supported
    crate::framework::system::sysctl_get("dev.tcpstats.profile_delete").is_ok()
}

/// Category G: Named Profile Cross-Validation
pub fn tests(
    tcp_echo: &str,
    read_tcpstats: &str,
    output_dir: Option<&Path>,
) -> Vec<libtest_mimic::Trial> {
    let mut trials = Vec::new();
    let output_dir = output_dir.map(|p| p.to_path_buf());
    let profiles_ok = profiles_available();

    // G01: profile vs ioctl cross-validation
    {
        let tcp_echo = tcp_echo.to_string();
        let read_tcpstats = read_tcpstats.to_string();
        let output_dir = output_dir.clone();
        let trial = libtest_mimic::Trial::test("G::g01", move || {
            let mut procs = ProcessGroup::new_in(output_dir.as_deref());
            procs
                .start_server(&tcp_echo, "127.0.0.17", "9061", 600)
                .map_err(|e| format!("{e}"))?;
            procs
                .start_clients(&tcp_echo, "127.0.0.17", "9061", 20, 600)
                .map_err(|e| format!("{e}"))?;

            let filter = "local_addr=127.0.0.17 local_port=9061 exclude=listen";

            // Create profile
            let profile_def = format!("test_g01 {filter}");
            sysctl_set("dev.tcpstats.profile_set", &profile_def)
                .map_err(|e| format!("create profile: {e}"))?;

            // Read via profile device
            let profile_count = run_cmd(&read_tcpstats, &["-c", "-d", "/dev/tcpstats/test_g01"])
                .map_err(|e| format!("read profile: {e}"))?
                .trim()
                .parse::<u64>()
                .map_err(|e| format!("parse profile count: {e}"))?;

            // Read via ioctl filter
            let ioctl_count = read_count(&read_tcpstats, filter).map_err(|e| format!("{e}"))?;

            // Delete profile
            let _ = sysctl_set("dev.tcpstats.profile_delete", "test_g01");

            if profile_count != ioctl_count {
                return Err(
                    format!("g01: profile({profile_count}) != ioctl({ioctl_count})").into(),
                );
            }

            procs.kill_all();
            Ok(())
        })
        .with_ignored_flag(!profiles_ok);
        trials.push(trial);
    }

    // G02: profile filter update
    {
        let tcp_echo = tcp_echo.to_string();
        let read_tcpstats = read_tcpstats.to_string();
        let output_dir = output_dir.clone();
        let trial = libtest_mimic::Trial::test("G::g02", move || {
            let mut procs = ProcessGroup::new_in(output_dir.as_deref());
            procs
                .start_server(&tcp_echo, "127.0.0.17", "9062", 600)
                .map_err(|e| format!("{e}"))?;
            procs
                .start_clients(&tcp_echo, "127.0.0.17", "9062", 20, 600)
                .map_err(|e| format!("{e}"))?;

            // Create profile matching traffic
            let profile_def = "test_g02 local_addr=127.0.0.17 local_port=9062 exclude=listen";
            sysctl_set("dev.tcpstats.profile_set", profile_def)
                .map_err(|e| format!("create profile: {e}"))?;

            // First read should find connections
            let count1 = run_cmd(&read_tcpstats, &["-c", "-d", "/dev/tcpstats/test_g02"])
                .map_err(|e| format!("read profile: {e}"))?
                .trim()
                .parse::<u64>()
                .map_err(|e| format!("parse count1: {e}"))?;

            if count1 == 0 {
                let _ = sysctl_set("dev.tcpstats.profile_delete", "test_g02");
                return Err("g02: initial profile read should find connections".into());
            }

            // Update profile to match nothing
            let update_def = "test_g02 local_addr=127.0.0.17 local_port=9999 exclude=listen";
            sysctl_set("dev.tcpstats.profile_set", update_def)
                .map_err(|e| format!("update profile: {e}"))?;

            // Second read should find 0
            let count2 = run_cmd(&read_tcpstats, &["-c", "-d", "/dev/tcpstats/test_g02"])
                .map_err(|e| format!("read updated profile: {e}"))?
                .trim()
                .parse::<u64>()
                .map_err(|e| format!("parse count2: {e}"))?;

            // Delete profile
            let _ = sysctl_set("dev.tcpstats.profile_delete", "test_g02");

            if count2 != 0 {
                return Err(format!("g02: updated profile should return 0, got {count2}").into());
            }

            procs.kill_all();
            Ok(())
        })
        .with_ignored_flag(!profiles_ok);
        trials.push(trial);
    }

    trials
}
