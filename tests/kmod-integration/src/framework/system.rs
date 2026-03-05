use std::process::Command;

use anyhow::{Context, Result, bail};

use super::process::{run_cmd, run_cmd_ok};

/// Read a sysctl value as a string.
pub fn sysctl_get(name: &str) -> Result<String> {
    run_cmd("sysctl", &["-n", name])
        .with_context(|| format!("sysctl get {name}"))
}

/// Read a sysctl value as u64.
pub fn sysctl_get_u64(name: &str) -> Result<u64> {
    let val = sysctl_get(name)?;
    val.parse::<u64>()
        .with_context(|| format!("parse sysctl {name}={val}"))
}

/// Set a sysctl value.
pub fn sysctl_set(name: &str, value: &str) -> Result<()> {
    let kv = format!("{name}={value}");
    run_cmd("sysctl", &[&kv])?;
    Ok(())
}

/// Build the kernel module.
pub fn kmod_build(kmod_src: &str, extra_cflags: Option<&str>) -> Result<()> {
    let mut args = vec!["-C", kmod_src, "clean", "all"];
    let flag_str;
    if let Some(flags) = extra_cflags {
        flag_str = format!("EXTRA_CFLAGS={flags}");
        args.push(&flag_str);
    }
    run_cmd("make", &args)?;
    Ok(())
}

/// Load the kernel module.
pub fn kmod_load(kmod_src: &str) -> Result<()> {
    // Unload first if already loaded
    let _ = kmod_unload();
    let ko = format!("{kmod_src}/tcp_stats_kld.ko");
    run_cmd("kldload", &[&ko])?;
    Ok(())
}

/// Unload the kernel module.
pub fn kmod_unload() -> Result<()> {
    let _ = run_cmd("kldunload", &["tcp_stats_kld"]);
    Ok(())
}

/// Check if the kernel module is loaded.
pub fn kmod_is_loaded() -> Result<bool> {
    run_cmd_ok("kldstat", &["-q", "-n", "tcp_stats_kld"])
}

/// Verify /dev/tcpstats device exists.
pub fn verify_device() -> Result<()> {
    if !std::path::Path::new("/dev/tcpstats").exists() {
        bail!("/dev/tcpstats does not exist — kmod not loaded?");
    }
    Ok(())
}

/// Tune system parameters for high connection counts.
pub fn tune_system() -> Result<()> {
    sysctl_set("kern.maxfiles", "500000")?;
    sysctl_set("kern.maxfilesperproc", "250000")?;
    sysctl_set("net.inet.ip.portrange.first", "1024")?;
    sysctl_set("net.inet.ip.portrange.last", "65535")?;
    Ok(())
}

/// Shorten TCP TIME_WAIT and FIN_WAIT_2 timers for faster socket recycling.
/// Safe on an isolated test VM; prevents port conflicts between sequential tests.
pub fn tune_tcp_timers() -> Result<()> {
    sysctl_set("net.inet.tcp.msl", "100")?; // TIME_WAIT = 2×MSL = 200ms (default 30s)
    sysctl_set("net.inet.tcp.finwait2_timeout", "1000")?; // FIN_WAIT_2 = 1s (default 60s)
    Ok(())
}

/// Check whether ipfw or pf firewalls are active and warn if so.
/// These are warnings only — some firewalls may still allow loopback traffic.
pub fn check_firewall() -> Result<()> {
    // Check ipfw: sysctl net.inet.ip.fw.enable (1 = enabled)
    if let Ok(val) = sysctl_get("net.inet.ip.fw.enable") {
        if val.trim() == "1" {
            eprintln!("  warn: ipfw firewall is enabled (net.inet.ip.fw.enable=1)");
        }
    }

    // Check pf: pfctl -s info
    if let Ok(output) = Command::new("pfctl")
        .args(["-s", "info"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if output.status.success() && stdout.contains("Status: Enabled") {
            eprintln!("  warn: pf firewall is enabled");
        }
    }

    Ok(())
}
