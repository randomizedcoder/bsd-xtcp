# BSD/macOS TCP socket statistics extraction tool.
#
# Nix outputs:
#
#   Packages:
#     default / bsd-xtcp          Native build for current platform
#     proto                       Standalone protobuf schema validation
#     cross-x86_64-darwin         Cross-compile for Intel Mac (Linux host only)
#     cross-aarch64-darwin        Cross-compile for Apple Silicon M1/M2/M3/M4 (Linux host only)
#     cross-all                   All cross targets, binaries named by triple
#
#   Apps (build with auto-named output dirs):
#     cross-x86_64-darwin         nix run .#cross-x86_64-darwin  -> result-cross-x86_64-darwin/
#     cross-aarch64-darwin        nix run .#cross-aarch64-darwin -> result-cross-aarch64-darwin/
#     build-cross-all             nix run .#build-cross-all      -> builds all with separate dirs
#
#   Checks:
#     clippy, fmt, test           Parallel CI checks
#
#   Dev shell:
#     nix develop                 Rust toolchain + protobuf + security/analysis tools
#
# Cross-compilation uses cargo-zigbuild + zig (bundles macOS SDK stubs).
# No Xcode or macOS SDK installation required. See also: Makefile.
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

        # Toolchain with darwin cross-compilation targets added.
        rustToolchainWithTargets = pkgs.rust-bin.stable.${constants.rustVersion}.default.override {
          extensions = [ "rust-src" ];
          targets = builtins.map (t: t.rustTarget) (builtins.attrValues constants.crossTargets);
        };

        rustPlatform = pkgs.makeRustPlatform {
          rustc = rustToolchain;
          cargo = rustToolchain;
        };

        src = pkgs.lib.cleanSource self;

        package = import ./nix/package.nix {
          inherit pkgs rustPlatform src constants;
        };

        proto = import ./nix/proto.nix {
          inherit pkgs src;
        };

        checks = import ./nix/checks.nix {
          inherit pkgs rustPlatform src advisory-db constants;
        };

        shell = import ./nix/shell.nix {
          inherit pkgs rustToolchain constants;
        };

        # Cross-compiled packages (Linux host only).
        crossPackages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
          builtins.mapAttrs (name: targetCfg:
            import ./nix/cross.nix {
              inherit pkgs rustPlatform rustToolchainWithTargets src constants;
              inherit (targetCfg) rustTarget;
            }
          ) constants.crossTargets
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
      in
      {
        packages = {
          default = package;
          bsd-xtcp = package;
          proto = proto;
        } // crossPackages // crossAll;

        apps = crossApps;

        checks = checks;

        devShells.default = shell;
      }
    );
}
