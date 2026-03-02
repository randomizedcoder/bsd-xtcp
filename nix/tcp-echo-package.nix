{ pkgs, rustPlatform, src, constants }:

rustPlatform.buildRustPackage {
  pname = "tcp-echo";
  version = constants.version;
  inherit src;

  cargoLock.lockFile = src + "/Cargo.lock";

  # Only build the tcp-echo binary from the workspace.
  cargoBuildFlags = [ "-p" "tcp-echo" ];
  cargoTestFlags = [ "-p" "tcp-echo" ];

  nativeBuildInputs = with pkgs; [
    # Protobuf is needed because the workspace Cargo.lock includes
    # prost-build from the root package, and cargo resolves the full
    # workspace even when building a single member.
    protobuf
    pkg-config
  ];

  buildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin [
    pkgs.libiconv
  ];

  env.PROTOC = "${pkgs.protobuf}/bin/protoc";

  doCheck = true;

  meta = with pkgs.lib; {
    description = "TCP echo server+client for testing bsd-xtcp socket stats";
    license = licenses.mit;
  };
}
