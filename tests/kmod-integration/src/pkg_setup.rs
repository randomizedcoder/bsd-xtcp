use anyhow::Result;

use crate::framework::process::{run_cmd, run_cmd_ok};

/// Idempotent FreeBSD environment setup.
/// Installs kernel source and required packages.
pub fn run_pkg_setup() -> Result<()> {
    // 1. Bootstrap pkg
    println!("  bootstrapping pkg...");
    let _ = run_cmd_ok("env", &["ASSUME_ALWAYS_YES=yes", "pkg", "bootstrap"]);

    // 2. Install kernel source if not present
    if !std::path::Path::new("/usr/src/sys/kern").exists() {
        println!("  installing kernel source...");
        let release = run_cmd("sysctl", &["-n", "kern.osrelease"])?;
        let src_url = format!("https://download.freebsd.org/releases/amd64/{release}/src.txz");

        run_cmd("fetch", &["-o", "/tmp/src.txz", &src_url])?;
        run_cmd("tar", &["-xf", "/tmp/src.txz", "-C", "/"])?;
        let _ = std::fs::remove_file("/tmp/src.txz");
    } else {
        println!("  kernel source already installed");
    }

    // 3. Install packages
    println!("  installing packages...");
    run_cmd("pkg", &["install", "-y", "valgrind", "perl5"])?;

    println!("  pkg_setup complete");
    Ok(())
}
