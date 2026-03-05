{ pkgs, rustPlatform, src, constants }:

rustPlatform.buildRustPackage {
  pname = "tcp-stats-kld-exporter";
  version = constants.version;
  inherit src;

  cargoLock.lockFile = src + "/Cargo.lock";

  # Only build the exporter binary from the workspace.
  cargoBuildFlags = [ "-p" "tcp-stats-kld-exporter" ];
  cargoTestFlags = [ "-p" "tcp-stats-kld-exporter" ];

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
    description = "Prometheus exporter for tcp_stats_kld kernel module stats";
    license = licenses.mit;
  };
}
