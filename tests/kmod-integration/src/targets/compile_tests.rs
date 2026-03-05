use anyhow::Result;

use crate::framework::compile::CompileConfig;
use crate::framework::process::run_cmd;

/// Run the unit test binary after compilation.
pub fn run_unit(cfg: &CompileConfig) -> Result<()> {
    let bin = cfg.build_unit()?;
    run_cmd(&bin, &[])?;
    Ok(())
}

/// Run valgrind memcheck on the test binary.
pub fn run_memcheck(cfg: &CompileConfig) -> Result<()> {
    let bin = cfg.build_memcheck()?;
    run_cmd(
        "valgrind",
        &[
            "--tool=memcheck",
            "--leak-check=full",
            "--track-origins=yes",
            "--error-exitcode=1",
            "--show-error-list=yes",
            &bin,
        ],
    )?;
    Ok(())
}

/// Run AddressSanitizer + UBSan test.
pub fn run_asan(cfg: &CompileConfig) -> Result<()> {
    let bin = cfg.build_asan()?;
    run_cmd(&bin, &[])?;
    Ok(())
}

/// Run UBSan-only test.
pub fn run_ubsan(cfg: &CompileConfig) -> Result<()> {
    let bin = cfg.build_ubsan()?;
    run_cmd(&bin, &[])?;
    Ok(())
}

/// Run benchmark with default iterations.
pub fn run_bench(cfg: &CompileConfig) -> Result<()> {
    let bin = cfg.build_bench()?;
    run_cmd(&bin, &["1000000"])?;
    Ok(())
}

/// Run callgrind profiling.
pub fn run_callgrind(cfg: &CompileConfig) -> Result<()> {
    let bin = cfg.build_callgrind()?;
    let test_dir = format!("{}/test", cfg.kmod_src);
    let out_file = format!("{test_dir}/callgrind.out");

    run_cmd(
        "valgrind",
        &[
            "--tool=callgrind",
            &format!("--callgrind-out-file={out_file}"),
            "--collect-jumps=yes",
            &bin,
            "100000",
        ],
    )?;

    run_cmd("callgrind_annotate", &["--auto=yes", &out_file])?;
    Ok(())
}

/// Build kernel module (standard).
pub fn build_kmod(cfg: &CompileConfig) -> Result<()> {
    cfg.build_kmod(None)
}

/// Build kernel module with stats.
pub fn build_kmod_stats(cfg: &CompileConfig) -> Result<()> {
    cfg.build_kmod(Some("-DTCPSTATS_STATS"))
}

/// Build kernel module with dtrace.
pub fn build_kmod_dtrace(cfg: &CompileConfig) -> Result<()> {
    cfg.build_kmod(Some("-DTCPSTATS_DTRACE"))
}

/// Build read_tcpstats binary.
#[allow(dead_code)]
pub fn build_read_tcpstats(cfg: &CompileConfig) -> Result<()> {
    cfg.build_read_tcpstats()?;
    Ok(())
}

/// Build bench_read_tcpstats binary.
pub fn build_bench_read(cfg: &CompileConfig) -> Result<()> {
    cfg.build_bench_read()?;
    Ok(())
}

/// Build gen_connections binary.
pub fn build_gen_conn(cfg: &CompileConfig) -> Result<()> {
    cfg.build_gen_conn()?;
    Ok(())
}

/// Build test_dos_limits binary.
#[allow(dead_code)]
pub fn build_dos_limits(cfg: &CompileConfig) -> Result<()> {
    cfg.build_dos_limits()?;
    Ok(())
}
