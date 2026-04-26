{ pkgs, src }:

pkgs.stdenvNoCC.mkDerivation {
  pname = "tcpstats-reader-proto";
  version = "0.1.0";
  inherit src;

  nativeBuildInputs = [ pkgs.protobuf ];

  buildPhase = ''
    protoc \
      --descriptor_set_out=descriptor.bin \
      --proto_path=proto \
      proto/tcp_stats.proto
  '';

  installPhase = ''
    mkdir -p $out
    cp descriptor.bin $out/
    cp proto/tcp_stats.proto $out/
  '';
}
