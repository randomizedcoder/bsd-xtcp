rec {
  pname = "tcpstats-reader";
  version = "0.1.0";

  # Pinned Rust version — use latest stable available in rust-overlay.
  # Update this when a newer stable release is needed.
  rustVersion = "1.93.0";

  systems = [
    "aarch64-darwin"
    "x86_64-darwin"
    "aarch64-linux"
    "x86_64-linux"
  ];

  # Single proto file per Section 19.1 design decision.
  protoFiles = [
    "proto/tcp_stats.proto"
  ];

  # Security tools for the dev shell (expand as needed).
  securityTools = [
    "cargo-audit"
    "cargo-deny"
    "cargo-nextest"
    "aflplusplus"
  ];

  # Analysis tools for the dev shell.
  analysisTools = [
    "rust-analyzer"
    "cargo-expand"
    "valgrind"
    "kcachegrind"
    "flamegraph"
    "cppcheck"
    "flawfinder"
    "semgrep"
  ];

  # Clang-based C analysis tools (separate list — needs clang-tools).
  cAnalysisTools = [ "clang-tools" ];

  # FreeBSD test VMs — rsync + SSH targets for kmod testing.
  freebsdVMs = {
    freebsd150 = {
      host = "root@192.168.122.41";
      label = "FreeBSD 15.0";
    };
    freebsd144 = {
      host = "root@192.168.122.85";
      label = "FreeBSD 14.4";
    };
    freebsd143 = {
      host = "root@192.168.122.27";
      label = "FreeBSD 14.3";
    };
  };

  # Cross-compilation targets (Linux host only).
  # method = "zigbuild" uses cargo-zigbuild (macOS targets).
  # method = "cross-rs" uses cross-rs with Docker (FreeBSD targets).
  crossTargets = {
    "cross-x86_64-darwin" = {
      rustTarget = "x86_64-apple-darwin";
      method = "zigbuild";
    };
    "cross-aarch64-darwin" = {
      rustTarget = "aarch64-apple-darwin";
      method = "zigbuild";
    };
    "cross-x86_64-freebsd" = {
      rustTarget = "x86_64-unknown-freebsd";
      method = "cross-rs";
    };
    "cross-aarch64-freebsd" = {
      rustTarget = "aarch64-unknown-freebsd";
      method = "cross-rs";
    };
  };

  # Subset of cross targets that use zigbuild (for toolchain target setup).
  zigbuildTargets = builtins.filter
    (t: t.method == "zigbuild")
    (builtins.attrValues crossTargets);

  # Subset of cross targets that use cross-rs.
  crossRsTargets = builtins.filter
    (t: t.method == "cross-rs")
    (builtins.attrValues crossTargets);
}
