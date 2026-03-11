mod filter;
mod framework;
mod pkg_setup;
mod targets;

use std::path::{Path, PathBuf};
use std::process;
use std::time::Instant;

type NamedAction<'a> = (&'a str, Box<dyn Fn() -> Result<()> + 'a>);

use anyhow::{Result, bail};
use libtest_mimic::Arguments;

use framework::compile::CompileConfig;
use framework::exporter::ExporterHandle;
use framework::output::{RunOutput, TargetResult, TargetStatus};

/// Default paths — overridable via CLI flags.
const DEFAULT_TCP_ECHO: &str = "tcp-echo";
const DEFAULT_READ_TCPSTATS: &str = "read_tcpstats";
const DEFAULT_KMOD_SRC: &str = "kmod/tcp_stats_kld";
const DEFAULT_CC: &str = "cc";
const DEFAULT_EXPORTER: &str = "tcp-stats-kld-exporter";
const DEFAULT_BSD_XTCP: &str = "bsd-xtcp";

struct Config {
    tcp_echo: String,
    read_tcpstats: String,
    kmod_src: String,
    cc: String,
    exporter: String,
    bsd_xtcp: String,
    target: String,
    category: String,
    output_dir: Option<PathBuf>,
}

fn parse_args() -> Config {
    let args: Vec<String> = std::env::args().collect();

    let mut cfg = Config {
        tcp_echo: DEFAULT_TCP_ECHO.to_string(),
        read_tcpstats: DEFAULT_READ_TCPSTATS.to_string(),
        kmod_src: DEFAULT_KMOD_SRC.to_string(),
        cc: DEFAULT_CC.to_string(),
        exporter: DEFAULT_EXPORTER.to_string(),
        bsd_xtcp: DEFAULT_BSD_XTCP.to_string(),
        target: "all".to_string(),
        category: "all".to_string(),
        output_dir: None,
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--tcp-echo" => {
                i += 1;
                cfg.tcp_echo = args.get(i).cloned().unwrap_or_default();
            }
            "--read-tcpstats" => {
                i += 1;
                cfg.read_tcpstats = args.get(i).cloned().unwrap_or_default();
            }
            "--kmod-src" => {
                i += 1;
                cfg.kmod_src = args.get(i).cloned().unwrap_or_default();
            }
            "--cc" => {
                i += 1;
                cfg.cc = args.get(i).cloned().unwrap_or_default();
            }
            "--exporter" => {
                i += 1;
                cfg.exporter = args.get(i).cloned().unwrap_or_default();
            }
            "--bsd-xtcp" => {
                i += 1;
                cfg.bsd_xtcp = args.get(i).cloned().unwrap_or_default();
            }
            "--category" => {
                i += 1;
                cfg.category = args.get(i).cloned().unwrap_or_default();
            }
            "--output-dir" => {
                i += 1;
                cfg.output_dir = args.get(i).map(PathBuf::from);
            }
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            arg if !arg.starts_with('-') && cfg.target == "all" => {
                cfg.target = arg.to_string();
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_usage();
                process::exit(1);
            }
        }
        i += 1;
    }

    cfg
}

fn print_usage() {
    eprintln!(
        "\
Usage: kmod-integration [target] [options]

Compile-only targets:
  unit, memcheck, asan, ubsan, bench, callgrind, kmod, bench_read, gen_conn, all

Live targets (require root + kmod):
  live_smoke, live_bench, live_stats, live_dtrace, live_dos
  live_integration [--category A|B|...|I|all]
  live_all

Soak targets (long-running, require root + kmod):
  live_soak         -- configurable via SOAK_DURATION_HOURS (default 24) and SOAK_CONNECTIONS (default 1000)
  live_soak_24h     -- 24-hour soak test
  live_soak_48h     -- 48-hour soak test

Setup:
  pkg_setup       -- idempotent FreeBSD env setup

Options:
  --tcp-echo PATH         -- path to tcp-echo binary
  --read-tcpstats PATH    -- path to read_tcpstats binary
  --bsd-xtcp PATH         -- path to bsd-xtcp binary
  --kmod-src PATH         -- path to kmod source dir
  --cc PATH               -- C compiler (default: cc)
  --exporter PATH         -- path to tcp-stats-kld-exporter binary
  --category CAT          -- filter category for live_integration
  --output-dir PATH       -- override output directory (default: /tmp/kmod-integration/TIMESTAMP)

Soak environment variables:
  SOAK_DURATION_HOURS     -- soak duration in hours (default: 24, 0 = quick 2-cycle verify)
  SOAK_CONNECTIONS        -- number of TCP connections to maintain (default: 1000)"
    );
}

fn main() {
    let cfg = parse_args();

    // Create structured output directory (always-on)
    let mut run_output = match &cfg.output_dir {
        Some(dir) => {
            let ts = chrono::Local::now().format("%Y-%m-%d-%H-%M-%S").to_string();
            RunOutput::new_at(dir.clone(), ts)
        }
        None => RunOutput::new(),
    };

    match &mut run_output {
        Ok(ro) => {
            println!("output: {}", ro.root().display());
        }
        Err(e) => {
            eprintln!("warn: could not create output dir: {e}");
        }
    }

    let output_dir = run_output.as_ref().ok().map(|ro| ro.root().to_path_buf());
    let output_path = output_dir.as_deref();

    let result = match cfg.target.as_str() {
        // Compile-only targets
        "unit" | "memcheck" | "asan" | "ubsan" | "bench" | "callgrind" | "kmod"
        | "bench_read" | "gen_conn" | "all" => {
            run_compile_target(&cfg, run_output.as_mut().ok())
        }

        // Setup
        "pkg_setup" => pkg_setup::run_pkg_setup(),

        // Live targets
        "live_smoke" => run_live_smoke(&cfg, output_path),
        "live_bench" => run_live_bench(&cfg, None, output_path),
        "live_stats" => run_live_stats(&cfg, None, output_path),
        "live_dtrace" => run_live_dtrace(&cfg, None, output_path),
        "live_dos" => run_live_dos(&cfg, None, output_path),
        "live_integration" => run_live_integration(&cfg, output_path),
        "live_all" => run_live_all(&cfg, run_output.as_mut().ok()),

        // Soak targets
        "live_soak" => run_live_soak(&cfg, output_path),
        "live_soak_24h" => run_live_soak_hours(&cfg, 24, output_path),
        "live_soak_48h" => run_live_soak_hours(&cfg, 48, output_path),

        other => {
            eprintln!("unknown target: {other}");
            print_usage();
            process::exit(1);
        }
    };

    // Write summary if output dir was created
    if let Ok(ref ro) = run_output {
        if let Err(e) = ro.write_summary() {
            eprintln!("warn: could not write summary: {e}");
        }
    }

    if let Err(e) = result {
        eprintln!("FAILED: {e:#}");
        process::exit(1);
    }
}

fn compile_config(cfg: &Config) -> CompileConfig<'_> {
    CompileConfig {
        cc: &cfg.cc,
        kmod_src: &cfg.kmod_src,
    }
}

fn run_compile_target(cfg: &Config, mut run_output: Option<&mut RunOutput>) -> Result<()> {
    let cc = compile_config(cfg);

    let run_all = cfg.target == "all";

    let targets: Vec<NamedAction<'_>> = vec![
        ("unit", Box::new(|| targets::compile_tests::run_unit(&cc))),
        ("memcheck", Box::new(|| targets::compile_tests::run_memcheck(&cc))),
        ("asan", Box::new(|| targets::compile_tests::run_asan(&cc))),
        ("ubsan", Box::new(|| targets::compile_tests::run_ubsan(&cc))),
        ("bench", Box::new(|| targets::compile_tests::run_bench(&cc))),
        ("callgrind", Box::new(|| targets::compile_tests::run_callgrind(&cc))),
        ("kmod", Box::new(|| {
            targets::compile_tests::build_kmod(&cc)?;
            targets::compile_tests::build_kmod_stats(&cc)?;
            targets::compile_tests::build_kmod_dtrace(&cc)?;
            Ok(())
        })),
        ("bench_read", Box::new(|| targets::compile_tests::build_bench_read(&cc))),
        ("gen_conn", Box::new(|| targets::compile_tests::build_gen_conn(&cc))),
    ];

    let mut passed = 0u32;
    let mut failed = 0u32;

    for (name, func) in &targets {
        if run_all || cfg.target == *name {
            print!("  {name}... ");
            let start = Instant::now();
            let res = func();
            let duration_secs = start.elapsed().as_secs_f64();

            match &res {
                Ok(()) => {
                    println!("PASS");
                    passed += 1;
                }
                Err(e) => {
                    println!("FAIL: {e:#}");
                    failed += 1;
                }
            }

            if let Some(ref mut ro) = run_output {
                ro.record(TargetResult {
                    name: name.to_string(),
                    status: if res.is_ok() { TargetStatus::Pass } else { TargetStatus::Fail },
                    duration_secs,
                    sub_tests: Vec::new(),
                });
            }
        }
    }

    println!("\ncompile targets: {passed} passed, {failed} failed");
    if failed > 0 {
        bail!("{failed} compile target(s) failed");
    }
    Ok(())
}

fn run_live_smoke(cfg: &Config, output_dir: Option<&Path>) -> Result<()> {
    println!("=== live_smoke ===");
    let read_tcpstats = ensure_read_tcpstats(cfg)?;
    targets::kmod_lifecycle::run_smoke(&cfg.kmod_src, &read_tcpstats, output_dir)
}

fn run_live_bench(
    cfg: &Config,
    exporter: Option<&ExporterHandle>,
    output_dir: Option<&Path>,
) -> Result<()> {
    println!("=== live_bench ===");
    let read_tcpstats = ensure_read_tcpstats(cfg)?;
    targets::read_bench::run_bench(&cfg.tcp_echo, &read_tcpstats, exporter, output_dir)
}

fn run_live_stats(
    cfg: &Config,
    exporter: Option<&ExporterHandle>,
    output_dir: Option<&Path>,
) -> Result<()> {
    println!("=== live_stats ===");
    let read_tcpstats = ensure_read_tcpstats(cfg)?;
    targets::sysctl_counters::run_stats_validation(
        &cfg.tcp_echo,
        &read_tcpstats,
        exporter,
        output_dir,
    )
}

fn run_live_dtrace(
    cfg: &Config,
    exporter: Option<&ExporterHandle>,
    output_dir: Option<&Path>,
) -> Result<()> {
    println!("=== live_dtrace ===");
    let read_tcpstats = ensure_read_tcpstats(cfg)?;
    targets::dtrace_probes::run_dtrace_validation(
        &cfg.tcp_echo,
        &read_tcpstats,
        exporter,
        output_dir,
    )
}

fn run_live_dos(
    cfg: &Config,
    exporter: Option<&ExporterHandle>,
    output_dir: Option<&Path>,
) -> Result<()> {
    println!("=== live_dos ===");
    let cc = compile_config(cfg);
    let dos_bin = cc.build_dos_limits()?;
    targets::dos_protection::run_dos_tests(
        &cfg.tcp_echo,
        &cfg.kmod_src,
        &dos_bin,
        exporter,
        output_dir,
    )
}

/// Ensure read_tcpstats binary exists. If the configured path doesn't exist,
/// compile it from the kmod source and return the new path.
fn ensure_read_tcpstats(cfg: &Config) -> Result<String> {
    if std::path::Path::new(&cfg.read_tcpstats).exists() {
        return Ok(cfg.read_tcpstats.clone());
    }

    println!("  read_tcpstats not found at '{}', compiling...", cfg.read_tcpstats);
    let cc = compile_config(cfg);
    let path = cc.build_read_tcpstats()?;
    println!("  compiled read_tcpstats at {path}");
    Ok(path)
}

fn run_live_integration(cfg: &Config, output_dir: Option<&Path>) -> Result<()> {
    println!("=== live_integration (category={}) ===", cfg.category);

    // Ensure read_tcpstats binary is available
    let read_tcpstats = ensure_read_tcpstats(cfg)?;

    // Parse categories
    let categories: Vec<&str> = if cfg.category == "all" {
        vec!["all"]
    } else {
        cfg.category.split(',').collect()
    };

    // Setup loopback aliases
    let mut aliases = framework::loopback::LoopbackAliases::new();

    // IPv4 aliases
    for addr in &[
        "127.0.0.10",
        "127.0.0.11",
        "127.0.0.12",
        "127.0.0.13",
        "127.0.0.14",
        "127.0.0.15",
        "127.0.0.16",
        "127.0.0.17",
        "127.0.0.18",
        "127.0.0.19",
    ] {
        aliases.add_v4(addr)?;
    }

    // IPv6 aliases
    for addr in &["fd00::10", "fd00::13", "fd00::14", "fd00::19"] {
        aliases.add_v6(addr)?;
    }

    // Check for active firewalls that might block loopback traffic
    framework::system::check_firewall()?;

    // Shorten TCP timers so TIME_WAIT sockets recycle quickly between tests
    framework::system::tune_tcp_timers()?;

    // Create output subdir for live_integration
    let integration_dir = output_dir.map(|d| d.join("live_integration"));
    if let Some(ref dir) = integration_dir {
        let _ = std::fs::create_dir_all(dir);
    }

    // Collect and run tests via libtest-mimic
    let trials = filter::collect_tests(
        &categories,
        &cfg.tcp_echo,
        &read_tcpstats,
        integration_dir.as_deref(),
    );

    if trials.is_empty() {
        println!("  no tests matched category={}", cfg.category);
        return Ok(());
    }

    println!("  {} tests collected", trials.len());

    // Use libtest-mimic runner with default args (don't parse our CLI flags)
    let args = Arguments {
        test_threads: Some(1), // sequential — tests share loopback and ports
        ..Arguments::default()
    };

    let conclusion = libtest_mimic::run(&args, trials);

    // Teardown aliases
    aliases.teardown();

    conclusion.exit();
}

fn run_live_all(cfg: &Config, run_output: Option<&mut RunOutput>) -> Result<()> {
    println!("========================================");
    println!("  live_all: running all live targets");
    println!("========================================");

    let output_dir = run_output
        .as_ref()
        .map(|ro| ro.root().to_path_buf());

    let mut passed = 0u32;
    let mut failed = 0u32;

    let mut run = |name: &str,
                   f: &dyn Fn() -> Result<()>,
                   run_output: &mut Option<&mut RunOutput>| {
        let start = Instant::now();
        let res = f();
        let duration_secs = start.elapsed().as_secs_f64();

        match &res {
            Ok(()) => {
                println!("  {name}: PASS");
                passed += 1;
            }
            Err(e) => {
                println!("  {name}: FAIL -- {e:#}");
                failed += 1;
            }
        }

        if let Some(ref mut ro) = run_output {
            ro.record(TargetResult {
                name: name.to_string(),
                status: if res.is_ok() { TargetStatus::Pass } else { TargetStatus::Fail },
                duration_secs,
                sub_tests: Vec::new(),
            });
        }
    };

    let od = output_dir.as_deref();
    let mut ro = run_output;

    // live_smoke does its own build/load/unload cycle
    run("live_smoke", &|| run_live_smoke(cfg, od), &mut ro);

    // Rebuild with full observability flags and reload.
    // Set TCPSTATS_DEBUG=1 env var to add verbose filter logging to dmesg.
    run("_reload_kmod", &|| {
        let debug_flag = if std::env::var("TCPSTATS_DEBUG").as_deref() == Ok("1") {
            println!("  TCPSTATS_DEBUG enabled -- filter debug logging to dmesg");
            " -DTCPSTATS_DEBUG"
        } else {
            ""
        };
        let flags = format!("-DTCPSTATS_STATS -DTCPSTATS_DTRACE{debug_flag}");
        println!("  reloading kmod with {flags}...");
        framework::system::kmod_build(&cfg.kmod_src, Some(&flags))?;
        framework::system::kmod_load(&cfg.kmod_src)?;
        Ok(())
    }, &mut ro);

    // Attempt to start the exporter; if it fails, continue with None
    let exporter = match ExporterHandle::start(&cfg.exporter, "127.0.0.1:9814") {
        Ok(handle) => {
            println!("  exporter started on 127.0.0.1:9814");
            Some(handle)
        }
        Err(e) => {
            println!("  warn: exporter not available: {e}");
            None
        }
    };
    let exp = exporter.as_ref();

    run("live_bench", &|| run_live_bench(cfg, exp, od), &mut ro);
    run("live_stats", &|| run_live_stats(cfg, exp, od), &mut ro);
    run("live_dtrace", &|| run_live_dtrace(cfg, exp, od), &mut ro);
    run("live_dos", &|| run_live_dos(cfg, exp, od), &mut ro);
    run("live_integration", &|| run_live_integration(cfg, od), &mut ro);

    // exporter killed on drop

    println!("========================================");
    println!("  live_all: {passed} passed, {failed} failed");
    println!("========================================");

    if failed > 0 {
        bail!("{failed} live target(s) failed");
    }
    Ok(())
}

fn run_live_soak(cfg: &Config, output_dir: Option<&Path>) -> Result<()> {
    let duration_hours: u64 = std::env::var("SOAK_DURATION_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let connections: u32 = std::env::var("SOAK_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    let read_tcpstats = ensure_read_tcpstats(cfg)?;

    let soak_config = targets::soak::SoakConfig {
        tcp_echo: cfg.tcp_echo.clone(),
        read_tcpstats,
        bsd_xtcp: cfg.bsd_xtcp.clone(),
        kmod_src: cfg.kmod_src.clone(),
        duration_hours,
        connections,
    };

    targets::soak::run_soak(&soak_config, output_dir)
}

fn run_live_soak_hours(cfg: &Config, hours: u64, output_dir: Option<&Path>) -> Result<()> {
    let connections: u32 = std::env::var("SOAK_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    let read_tcpstats = ensure_read_tcpstats(cfg)?;

    let soak_config = targets::soak::SoakConfig {
        tcp_echo: cfg.tcp_echo.clone(),
        read_tcpstats,
        bsd_xtcp: cfg.bsd_xtcp.clone(),
        kmod_src: cfg.kmod_src.clone(),
        duration_hours: hours,
        connections,
    };

    targets::soak::run_soak(&soak_config, output_dir)
}
