{ pkgs, rustToolchain, constants }:

let
  # Gracefully handle packages that might not exist on all platforms.
  tryPkg = name:
    if builtins.hasAttr name pkgs then pkgs.${name} else null;

  securityPkgs = builtins.filter (p: p != null)
    (map tryPkg constants.securityTools);

  analysisPkgs = builtins.filter (p: p != null)
    (map tryPkg constants.analysisTools);

  cAnalysisPkgs = builtins.filter (p: p != null)
    (map tryPkg constants.cAnalysisTools);

  linuxProfilingPkgs = pkgs.lib.optionals pkgs.stdenv.isLinux [
    pkgs.perf
    pkgs.heaptrack
  ];

  # FreeBSD cross-compilation tools (Linux only, requires Docker for cross-rs).
  freebsdCrossPkgs = pkgs.lib.optionals pkgs.stdenv.isLinux [
    pkgs.cargo-cross
    pkgs.cargo-zigbuild
    pkgs.zig
  ];
in
pkgs.mkShell {
  # List build deps explicitly rather than using inputsFrom with
  # buildRustPackage, which brings in cargo hooks (cargo-auditable,
  # cargo-setup-hook) that conflict with interactive toolchain use.
  nativeBuildInputs = [
    rustToolchain
    pkgs.protobuf
    pkgs.pkg-config
  ]
  ++ securityPkgs
  ++ analysisPkgs
  ++ cAnalysisPkgs
  ++ linuxProfilingPkgs
  ++ freebsdCrossPkgs
  ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [ pkgs.lldb ]
  ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.gdb ];

  buildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin (
    (with pkgs.darwin.apple_sdk.frameworks; [ SystemConfiguration ])
    ++ [ pkgs.libiconv ]
  );

  env = {
    PROTOC = "${pkgs.protobuf}/bin/protoc";
    RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
    CARGO_TERM_COLOR = "always";
    # Prevent cargo from finding rustup proxies (cargo-clippy, etc.)
    # in ~/.cargo/bin, which would shadow the nix-provided toolchain.
    CARGO_HOME = ".cargo";
  };

  shellHook = ''
    echo "tcpstats-reader dev shell — Rust $(rustc --version)"
    echo "  cargo build        build the binary"
    echo "  cargo clippy       lint"
    echo "  cargo run          run the demo"
  '';
}
