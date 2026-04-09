use std::path::Path;

use anyhow::{bail, Result};

use crate::framework::process::run_cmd;
use crate::framework::system::{kmod_build, kmod_is_loaded, kmod_load, kmod_unload, verify_device};

/// Smoke test: build, load, verify device, read, unload.
pub fn run_smoke(kmod_src: &str, read_tcpstats: &str, _output_dir: Option<&Path>) -> Result<()> {
    // Build
    println!("  building kmod...");
    kmod_build(kmod_src, None)?;

    // Load
    println!("  loading kmod...");
    kmod_load(kmod_src)?;

    // Verify device exists
    verify_device()?;

    // Verify module is loaded
    if !kmod_is_loaded()? {
        bail!("kmod not loaded after kldload");
    }

    // Do a test read
    println!("  test read...");
    let output = run_cmd(read_tcpstats, &["-c"])?;
    println!("  read returned: {output}");

    // Unload
    println!("  unloading kmod...");
    kmod_unload()?;

    if kmod_is_loaded()? {
        bail!("kmod still loaded after kldunload");
    }

    Ok(())
}
