{ pkgs, rustPlatform, rustToolchainWithTargets, src, constants, rustTarget }:

# Cross-compilation for FreeBSD targets using cross-rs (Docker-based).
#
# Requires Docker daemon running on the build host. cross-rs spawns
# Docker containers with FreeBSD cross-compilation sysroot.
#
# Usage:
#   nix build .#cross-x86_64-freebsd
#   nix build .#cross-aarch64-freebsd

pkgs.stdenv.mkDerivation {
  pname = "${constants.pname}-${rustTarget}";
  version = constants.version;
  inherit src;

  cargoDeps = rustPlatform.importCargoLock {
    lockFile = src + "/Cargo.lock";
  };

  nativeBuildInputs = [
    rustToolchainWithTargets
    pkgs.cargo-cross
    pkgs.protobuf
    pkgs.pkg-config
    rustPlatform.cargoSetupHook
  ];

  env = {
    PROTOC = "${pkgs.protobuf}/bin/protoc";
    CARGO_BUILD_TARGET = rustTarget;
  };

  buildPhase = ''
    runHook preBuild

    # cross-rs needs a writable HOME for its cache/config.
    export HOME=$(mktemp -d)

    cross build --release --target ${rustTarget}
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    cp target/${rustTarget}/release/${constants.pname} $out/bin/
    runHook postInstall
  '';

  doCheck = false;

  meta = with pkgs.lib; {
    description = "BSD/macOS TCP statistics tool (cross-compiled for ${rustTarget})";
    license = licenses.mit;
    platforms = platforms.linux;
  };
}
