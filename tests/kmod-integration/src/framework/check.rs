use anyhow::{Context, Result, bail};

use super::process::run_cmd;

/// Comparison operations for test assertions.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum CheckOp {
    Eq,
    Ge,
    Le,
    Range(u64, u64),
}

/// Read the count of matching records using read_tcpstats -c -f.
/// Returns 0 if the filter is invalid (e.g. ipv6_only with an IPv4 address).
pub fn read_count(read_tcpstats: &str, filter: &str) -> Result<u64> {
    let output = match run_cmd(read_tcpstats, &["-c", "-f", filter]) {
        Ok(out) => out,
        Err(_) => {
            // Filter conflicts (e.g. ipv6_only + IPv4 addr) cause read_tcpstats
            // to exit non-zero. Treat as 0 matching records.
            return Ok(0);
        }
    };

    // Output may contain multiple lines; the count is the last numeric value.
    // Typical output: just a number like "21"
    let count_str = output
        .lines()
        .last()
        .unwrap_or("")
        .trim();

    count_str
        .parse::<u64>()
        .with_context(|| format!("parse count from '{output}'"))
}

/// Assert a count matches the expected value using the given comparison op.
pub fn check_count(
    read_tcpstats: &str,
    filter: &str,
    op: CheckOp,
    expected: u64,
    test_id: &str,
) -> Result<()> {
    let actual = read_count(read_tcpstats, filter)?;

    let pass = match op {
        CheckOp::Eq => actual == expected,
        CheckOp::Ge => actual >= expected,
        CheckOp::Le => actual <= expected,
        CheckOp::Range(min, max) => actual >= min && actual <= max,
    };

    if !pass {
        bail!(
            "{test_id}: FAIL — filter={filter:?} expected {expected} ({op:?}), got {actual}"
        );
    }

    Ok(())
}
