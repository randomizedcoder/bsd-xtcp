#[macro_use]
pub mod macros;

pub mod address_filter;
pub mod combinatorial_coverage;
pub mod combined_filter;
pub mod concurrent_readers;
pub mod format_fields;
pub mod ipversion_filter;
pub mod named_profiles;
pub mod port_filter;
pub mod state_filter;

use std::path::Path;

/// Collect all filter tests for the given categories.
pub fn collect_tests(
    categories: &[&str],
    tcp_echo: &str,
    read_tcpstats: &str,
    output_dir: Option<&Path>,
) -> Vec<libtest_mimic::Trial> {
    let mut trials = Vec::new();
    let all = categories.contains(&"all");

    if all || categories.contains(&"A") {
        trials.extend(port_filter::tests(tcp_echo, read_tcpstats, output_dir));
    }
    if all || categories.contains(&"B") {
        trials.extend(state_filter::tests(tcp_echo, read_tcpstats, output_dir));
    }
    if all || categories.contains(&"C") {
        trials.extend(ipversion_filter::tests(tcp_echo, read_tcpstats, output_dir));
    }
    if all || categories.contains(&"D") {
        trials.extend(address_filter::tests(tcp_echo, read_tcpstats, output_dir));
    }
    if all || categories.contains(&"E") {
        trials.extend(combined_filter::tests(tcp_echo, read_tcpstats, output_dir));
    }
    if all || categories.contains(&"F") {
        trials.extend(format_fields::tests(tcp_echo, read_tcpstats, output_dir));
    }
    if all || categories.contains(&"G") {
        trials.extend(named_profiles::tests(tcp_echo, read_tcpstats, output_dir));
    }
    if all || categories.contains(&"H") {
        trials.extend(concurrent_readers::tests(tcp_echo, read_tcpstats, output_dir));
    }
    if all || categories.contains(&"I") {
        trials.extend(combinatorial_coverage::tests(tcp_echo, read_tcpstats, output_dir));
    }

    trials
}
