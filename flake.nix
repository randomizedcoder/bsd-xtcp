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
      in
      {
        packages.default = package;
        packages.bsd-xtcp = package;
        packages.proto = proto;

        checks = checks;

        devShells.default = shell;
      }
    );
}
