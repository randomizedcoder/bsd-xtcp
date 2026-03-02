{ pkgs, rustPlatform, rustToolchainWithTargets, src, constants, rustTarget }:

pkgs.stdenv.mkDerivation {
  pname = "${constants.pname}-${rustTarget}";
  version = constants.version;
  inherit src;

  cargoDeps = rustPlatform.importCargoLock {
    lockFile = src + "/Cargo.lock";
  };

  nativeBuildInputs = [
    rustToolchainWithTargets
    pkgs.cargo-zigbuild
    pkgs.zig
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

    # cargo-zigbuild needs a writable HOME for its cache directory.
    export HOME=$(mktemp -d)

    cargo zigbuild --release --target ${rustTarget}
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
