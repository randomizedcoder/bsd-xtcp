# BSD/macOS TCP socket statistics extraction tool.
#
# Nix outputs:
#
#   Packages:
#     default / bsd-xtcp          Native build for current platform
#     tcp-echo                    TCP echo utility for stats verification
#     proto                       Standalone protobuf schema validation
#     cross-x86_64-darwin         Cross-compile bsd-xtcp for Intel Mac (Linux host only)
#     cross-aarch64-darwin        Cross-compile bsd-xtcp for Apple Silicon (Linux host only)
#     cross-x86_64-freebsd       Cross-compile bsd-xtcp for FreeBSD amd64 (Linux host, Docker)
#     cross-aarch64-freebsd      Cross-compile bsd-xtcp for FreeBSD aarch64 (Linux host, Docker)
#     cross-all                   All cross targets, binaries named by triple
#     tcp-echo-cross-x86_64-darwin    Cross-compile tcp-echo for Intel Mac (Linux host only)
#     tcp-echo-cross-aarch64-darwin   Cross-compile tcp-echo for Apple Silicon (Linux host only)
#     kmod-test-unit              Filter parser unit tests
#     kmod-test-memcheck          Valgrind memcheck (Linux only)
#     kmod-test-asan              AddressSanitizer + UBSan
#     kmod-test-ubsan             UndefinedBehaviorSanitizer standalone
#     kmod-test-bench             Filter parser benchmark
#     kmod-test-callgrind         Callgrind CPU profiling (Linux only)
#     kmod-test-all               All kmod tests sequentially
#     kmod-analysis-gcc-warnings  GCC max warnings + -Werror
#     kmod-analysis-gcc-fanalyzer GCC -fanalyzer interprocedural analysis
#     kmod-analysis-scan-build    Clang Static Analyzer
#     kmod-analysis-clang-tidy    clang-tidy (bugprone, cert, security)
#     kmod-analysis-cppcheck      Cppcheck --enable=all --force
#     kmod-analysis-infer         Meta Infer (conditional availability)
#     kmod-analysis-semgrep       Semgrep with custom kernel rules
#     kmod-analysis-flawfinder    CWE-oriented source scanner
#     kmod-analysis-format-check  clang-format style check
#     kmod-analysis-all           All C static analyzers
#     bsd-xtcp-freebsd            Deploy + build + test bsd-xtcp on ALL FreeBSD VMs
#     bsd-xtcp-freebsd150         Deploy + build + test on FreeBSD 15.0 only
#     bsd-xtcp-freebsd143         Deploy + build + test on FreeBSD 14.3 only
#     integration-test-freebsd    Deploy + integration test on ALL FreeBSD VMs
#     integration-test-freebsd150 Deploy + integration test on FreeBSD 15.0 only
#     integration-test-freebsd143 Deploy + integration test on FreeBSD 14.3 only
#                                 Set INTEGRATION_TARGET env var to select target
#                                 (default: live_integration; options: all, live_all, live_smoke, pkg_setup, ...)
#
#   Apps (build with auto-named output dirs):
#     cross-x86_64-darwin         nix run .#cross-x86_64-darwin  -> result-cross-x86_64-darwin/
#     cross-aarch64-darwin        nix run .#cross-aarch64-darwin -> result-cross-aarch64-darwin/
#     cross-x86_64-freebsd       nix run .#cross-x86_64-freebsd -> result-cross-x86_64-freebsd/
#     cross-aarch64-freebsd      nix run .#cross-aarch64-freebsd -> result-cross-aarch64-freebsd/
#     build-cross-all             nix run .#build-cross-all      -> builds all with separate dirs
#     bsd-xtcp-freebsd            nix run .#bsd-xtcp-freebsd     -> deploy + test on all VMs
#     bsd-xtcp-freebsd150         nix run .#bsd-xtcp-freebsd150  -> deploy + test on FreeBSD 15.0
#     bsd-xtcp-freebsd143         nix run .#bsd-xtcp-freebsd143  -> deploy + test on FreeBSD 14.3
#
#   Checks:
#     clippy, fmt, test           Parallel CI checks (covers full workspace)
#
#   Dev shell:
#     nix develop                 Rust toolchain + protobuf + security/analysis tools
#                                 + cross-rs + cargo-zigbuild + zig (Linux)
#
# Cross-compilation:
#   macOS targets use cargo-zigbuild + zig (bundles macOS SDK stubs).
#   FreeBSD targets use cross-rs + Docker (FreeBSD cross-compilation sysroot).
#   No Xcode or macOS SDK installation required. See also: Makefile.
#
{
  description = "BSD/macOS TCP socket statistics extraction tool";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, advisory-db }:
    let
      constants = import ./nix/constants.nix;
    in
    flake-utils.lib.eachSystem constants.systems (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustToolchain = pkgs.rust-bin.stable.${constants.rustVersion}.default.override {
          extensions = [ "rust-src" "clippy" "rustfmt" ];
        };

        # Toolchain with cross-compilation targets added (zigbuild targets only).
        rustToolchainWithTargets = pkgs.rust-bin.stable.${constants.rustVersion}.default.override {
          extensions = [ "rust-src" ];
          targets = builtins.map (t: t.rustTarget) constants.zigbuildTargets;
        };

        # Toolchain with FreeBSD cross-compilation targets.
        rustToolchainWithFreebsdTargets = pkgs.rust-bin.stable.${constants.rustVersion}.default.override {
          extensions = [ "rust-src" ];
          targets = builtins.map (t: t.rustTarget) constants.crossRsTargets;
        };

        rustPlatform = pkgs.makeRustPlatform {
          rustc = rustToolchain;
          cargo = rustToolchain;
        };

        src = pkgs.lib.cleanSource self;

        package = import ./nix/package.nix {
          inherit pkgs rustPlatform src constants;
        };

        tcpEcho = import ./nix/tcp-echo-package.nix {
          inherit pkgs rustPlatform src constants;
        };

        proto = import ./nix/proto.nix {
          inherit pkgs src;
        };

        kmodTests = import ./nix/kmod-tests.nix {
          inherit pkgs src;
        };

        kmodAnalysis = import ./nix/kmod-analysis.nix {
          inherit pkgs src;
        };

        freebsdDeploy = import ./nix/freebsd-deploy.nix {
          inherit pkgs src;
        };

        freebsdIntegration = import ./nix/freebsd-integration.nix {
          inherit pkgs src;
        };

        tcpStatsKldExporter = import ./nix/tcp-stats-kld-exporter-package.nix {
          inherit pkgs rustPlatform src constants;
        };

        exporterDeploy = import ./nix/exporter-deploy.nix {
          inherit pkgs src;
        };

        checks = import ./nix/checks.nix {
          inherit pkgs rustPlatform src advisory-db constants;
        };

        shell = import ./nix/shell.nix {
          inherit pkgs rustToolchain constants;
        };

        # Filter cross targets by method.
        zigbuildCrossTargets = pkgs.lib.filterAttrs
          (_: cfg: cfg.method == "zigbuild")
          constants.crossTargets;

        crossRsCrossTargets = pkgs.lib.filterAttrs
          (_: cfg: cfg.method == "cross-rs")
          constants.crossTargets;

        # Cross-compiled packages using zigbuild (Linux host only).
        zigbuildPackages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
          builtins.mapAttrs (name: targetCfg:
            import ./nix/cross.nix {
              inherit pkgs rustPlatform rustToolchainWithTargets src constants;
              inherit (targetCfg) rustTarget;
            }
          ) zigbuildCrossTargets
        );

        # Cross-compiled packages using cross-rs (Linux host only, requires Docker).
        crossRsPackages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
          builtins.mapAttrs (name: targetCfg:
            import ./nix/cross-freebsd.nix {
              inherit pkgs rustPlatform src constants;
              rustToolchainWithTargets = rustToolchainWithFreebsdTargets;
              inherit (targetCfg) rustTarget;
            }
          ) crossRsCrossTargets
        );

        crossPackages = zigbuildPackages // crossRsPackages;

        # Cross-compiled tcp-echo packages (Linux host only).
        tcpEchoCrossPackages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
          builtins.mapAttrs (name: targetCfg:
            import ./nix/tcp-echo-cross.nix {
              inherit pkgs rustPlatform rustToolchainWithTargets src constants;
              inherit (targetCfg) rustTarget;
            }
          ) (builtins.mapAttrs (name: cfg: cfg) {
            "tcp-echo-cross-x86_64-darwin" = { rustTarget = "x86_64-apple-darwin"; };
            "tcp-echo-cross-aarch64-darwin" = { rustTarget = "aarch64-apple-darwin"; };
          })
        );

        # Combined cross-compilation output with all targets (Linux host only).
        crossAll = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          cross-all = pkgs.runCommand "bsd-xtcp-cross-all" {} (
            builtins.concatStringsSep "\n" (
              [ "mkdir -p $out/bin" ] ++
              pkgs.lib.mapAttrsToList (name: targetCfg:
                let pkg = crossPackages.${name}; in
                "cp ${pkg}/bin/${constants.pname} $out/bin/${constants.pname}-${targetCfg.rustTarget}"
              ) constants.crossTargets
            )
          );
        };

        # Apps that build cross targets with per-target output symlinks.
        crossApps = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
          builtins.mapAttrs (name: _: {
            type = "app";
            program = toString (pkgs.writeShellScript "build-${name}" ''
              set -euo pipefail
              echo "Building ${name}..."
              nix build .#${name} --out-link result-${name} "$@"
              echo "Output: result-${name}/bin/${constants.pname}"
            '');
          }) constants.crossTargets
          // {
            build-cross-all = {
              type = "app";
              program = toString (pkgs.writeShellScript "build-cross-all" ''
                set -euo pipefail
                ${builtins.concatStringsSep "\n" (
                  pkgs.lib.mapAttrsToList (name: _: ''
                    echo "Building ${name}..."
                    nix build .#${name} --out-link result-${name}
                    echo "Output: result-${name}/bin/${constants.pname}"
                  '') constants.crossTargets
                )}
                echo "All cross targets built."
              '');
            };
          }
        );

        # FreeBSD deploy apps.
        freebsdApps = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
          pkgs.lib.mapAttrs' (name: pkg: {
            name = name;
            value = {
              type = "app";
              program = "${pkg}/bin/${pkg.name}";
            };
          }) freebsdDeploy
        );

        # FreeBSD integration test apps.
        integrationApps = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
          pkgs.lib.mapAttrs' (name: pkg: {
            name = name;
            value = {
              type = "app";
              program = "${pkg}/bin/${pkg.name}";
            };
          }) freebsdIntegration
        );
        # Exporter deploy apps.
        exporterApps = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
          pkgs.lib.mapAttrs' (name: pkg: {
            name = name;
            value = {
              type = "app";
              program = "${pkg}/bin/${pkg.name}";
            };
          }) exporterDeploy
        );
      in
      {
        packages = {
          default = package;
          bsd-xtcp = package;
          tcp-echo = tcpEcho;
          tcp-stats-kld-exporter = tcpStatsKldExporter;
          proto = proto;
        } // kmodTests // kmodAnalysis // freebsdDeploy // freebsdIntegration // exporterDeploy // crossPackages // tcpEchoCrossPackages // crossAll;

        apps = crossApps // freebsdApps // integrationApps // exporterApps;

        checks = checks;

        devShells.default = shell;
      }
    );
}
