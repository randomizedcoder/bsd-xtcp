{ pkgs, rustPlatform, src, constants }:

rustPlatform.buildRustPackage {
  pname = constants.pname;
  version = constants.version;
  inherit src;

  cargoLock.lockFile = src + "/Cargo.lock";

  nativeBuildInputs = with pkgs; [
    protobuf
    pkg-config
  ];

  buildInputs = pkgs.lib.optionals pkgs.stdenv.isDarwin (
    (with pkgs.darwin.apple_sdk.frameworks; [ SystemConfiguration ])
    ++ [ pkgs.libiconv ]
  );

  env.PROTOC = "${pkgs.protobuf}/bin/protoc";

  doCheck = true;

  meta = with pkgs.lib; {
    description = "BSD/macOS TCP socket statistics extraction tool";
    license = licenses.mit;
  };
}
