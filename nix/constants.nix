rec {
  pname = "bsd-xtcp";
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
  ];

  # Analysis tools for the dev shell.
  analysisTools = [
    "rust-analyzer"
    "cargo-expand"
  ];

  # Cross-compilation targets (Linux host → macOS).
  crossTargets = {
    "cross-x86_64-darwin" = {
      rustTarget = "x86_64-apple-darwin";
    };
    "cross-aarch64-darwin" = {
      rustTarget = "aarch64-apple-darwin";
    };
  };
}
