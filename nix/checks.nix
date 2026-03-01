{ pkgs, rustPlatform, src, advisory-db, constants }:

let
  commonArgs = {
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
  };
in
{
  clippy = rustPlatform.buildRustPackage (commonArgs // {
    pname = "${constants.pname}-clippy";

    buildPhase = ''
      cargo clippy --all-targets -- -D warnings
    '';
    installPhase = ''
      mkdir -p $out
    '';
    doCheck = false;
  });

  fmt = rustPlatform.buildRustPackage (commonArgs // {
    pname = "${constants.pname}-fmt";

    buildPhase = ''
      cargo fmt --check
    '';
    installPhase = ''
      mkdir -p $out
    '';
    doCheck = false;
  });

  test = rustPlatform.buildRustPackage (commonArgs // {
    pname = "${constants.pname}-test";
    doCheck = true;
  });
}
