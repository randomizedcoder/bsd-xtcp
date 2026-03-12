[← Back to README](../../README.md)

# Nix Build System

## Table of Contents

- [13. Overview](#13-overview)
- [14. Flake Inputs and Toolchain](#14-flake-inputs-and-toolchain)
  - [14.1 Flake Inputs](#141-flake-inputs)
  - [14.2 Rust Toolchain Pinning](#142-rust-toolchain-pinning)
  - [14.3 Target Systems](#143-target-systems)
- [15. Modular File Structure](#15-modular-file-structure)
  - [15.1 Layout](#151-layout)
  - [15.2 `nix/constants.nix` — Centralized Configuration](#152-nixconstantsnix--centralized-configuration)
  - [15.3 `nix/package.nix` — Rust Binary Build](#153-nixpackagenix--rust-binary-build)
  - [15.4 `nix/proto.nix` — Protobuf Compilation](#154-nixprotonix--protobuf-compilation)
  - [15.5 `nix/checks.nix` — Automated Verification](#155-nixchecksnix--automated-verification)
  - [15.6 `nix/shell.nix` — Development Shell](#156-nixshellnix--development-shell)
  - [15.7 `flake.nix` — Top-Level Orchestration](#157-flakelock--top-level-orchestration)
- [16. Security Tooling](#16-security-tooling)
  - [16.1 Static Analysis](#161-static-analysis)
  - [16.2 Dependency Auditing](#162-dependency-auditing)
  - [16.3 Unsafe Code Detection](#163-unsafe-code-detection)
  - [16.4 Fuzzing](#164-fuzzing)
  - [16.5 Coverage](#165-coverage)
- [17. Build Targets](#17-build-targets)

---

## 13. Overview

The build system uses a Nix flake with modular `.nix` files under a `nix/` directory, following the same organizational pattern as the PCP project. The Rust binary is built using `rustPlatform.buildRustPackage` from nixpkgs, which runs `cargo build --release` under the hood. No third-party Rust build frameworks (crane, naersk) are used — standard cargo is sufficient for a project of this scope and avoids unnecessary flake input complexity.

Protobuf compilation is handled by `prost-build` in `build.rs` during the normal cargo build, with the Nix expressions providing `protoc` in the build environment. This is the standard pattern used by protobuf+Rust packages in nixpkgs (garage, chirpstack, sozu, grpc-health-check).

The Rust toolchain is pinned to an exact version via `rust-overlay` to ensure reproducible builds and to provide a nightly toolchain for fuzzing tools (`cargo-fuzz`, `cargo-udeps`).

---

## 14. Flake Inputs and Toolchain

### 14.1 Flake Inputs

| Input | Source | Purpose |
|-------|--------|---------|
| `nixpkgs` | `github:NixOS/nixpkgs/nixpkgs-unstable` | Base packages, `rustPlatform`, protobuf, security tools |
| `rust-overlay` | `github:oxalica/rust-overlay` | Pin Rust 1.93.1 exactly; provide nightly for cargo-fuzz |
| `flake-utils` | `github:numtide/flake-utils` | `eachSystem` helper for multi-platform outputs |
| `advisory-db` | `github:rustsec/advisory-db` | RustSec vulnerability database for `cargo-audit` checks |

`rust-overlay` follows nixpkgs to avoid duplicate nixpkgs evaluations:

```nix
rust-overlay = {
  url = "github:oxalica/rust-overlay";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

`advisory-db` is a non-flake input (raw git repo) consumed by the audit check:

```nix
advisory-db = {
  url = "github:rustsec/advisory-db";
  flake = false;
};
```

### 14.2 Rust Toolchain Pinning

The nixpkgs-unstable channel ships whatever Rust version was current at the time of the nixpkgs commit, which drifts unpredictably. `rust-overlay` provides exact version pinning:

```nix
rustToolchain = pkgs.rust-bin.stable."1.93.1".default.override {
  extensions = [ "rust-src" "clippy" "rustfmt" ];
};
```

This toolchain is used to construct a custom `rustPlatform`:

```nix
rustPlatform = pkgs.makeRustPlatform {
  rustc = rustToolchain;
  cargo = rustToolchain;
};
```

A nightly toolchain is provided separately in the dev shell for tools that require it (`cargo-fuzz` needs nightly for `-Z` flags, `cargo-udeps` needs nightly for `--output-format`).

### 14.3 Target Systems

The tool runs on macOS. Linux systems are supported for development and CI:

| System | Purpose |
|--------|---------|
| `aarch64-darwin` | Primary target: Apple Silicon Macs |
| `x86_64-darwin` | Intel Macs |
| `aarch64-linux` | CI, development (ARM Linux) |
| `x86_64-linux` | CI, development (x86 Linux) |

The Rust binary itself targets macOS APIs (`sysctl`, `libproc`, `getsockopt(TCP_CONNECTION_INFO)`), so `nix build` on Linux produces a binary that links against Darwin frameworks and is only useful for macOS. The Linux dev shell provides all the security tooling and analysis tools for development workflows.

---

## 15. Modular File Structure

### 15.1 Layout

```
bsd-xtcp/
├── flake.nix                  # Top-level orchestration, imports nix/ modules
├── flake.lock                 # Pinned input revisions
├── nix/
│   ├── constants.nix          # Single source of truth: versions, tool lists
│   ├── package.nix            # Rust binary (rustPlatform.buildRustPackage)
│   ├── proto.nix              # Protobuf validation target
│   ├── checks.nix             # Flake checks: clippy, fmt, audit, deny, tests
│   └── shell.nix              # Dev shell with security/analysis tools
├── src/                       # Rust source (see Section 7.2)
├── proto/                     # .proto definitions
├── Cargo.toml
├── Cargo.lock
└── deny.toml                  # cargo-deny configuration
```

Each `.nix` file under `nix/` is a function that takes explicit dependencies as arguments. No file reads global state. This makes dependencies between modules visible and testable.

### 15.2 `nix/constants.nix` — Centralized Configuration

Single source of truth for all shared values, following the PCP project's `constants.nix` pattern:

```nix
rec {
  pname = "bsd-xtcp";
  version = "0.1.0";

  rustVersion = "1.93.1";

  systems = [
    "aarch64-darwin"
    "x86_64-darwin"
    "aarch64-linux"
    "x86_64-linux"
  ];

  protoFiles = [
    "proto/tcp_record.proto"
    "proto/system_record.proto"
  ];

  securityTools = [
    "cargo-audit"
    "cargo-deny"
    "cargo-fuzz"
    "cargo-geiger"
    "cargo-vet"
    "cargo-nextest"
    "cargo-tarpaulin"
    "cargo-machete"
    "cargo-udeps"
  ];

  analysisTools = [
    "rust-analyzer"
    "cargo-expand"
    "cargo-bloat"
  ];
}
```

### 15.3 `nix/package.nix` — Rust Binary Build

```nix
{ pkgs, rustPlatform, src, constants }:
```

Uses `rustPlatform.buildRustPackage`, which:

1. Vendors all crate dependencies (from `Cargo.lock` + `cargoHash`)
2. Runs `cargo build --release`
3. Installs binaries from `target/release/` to `$out/bin/`

Key attributes:

| Attribute | Value | Rationale |
|-----------|-------|-----------|
| `pname` | from `constants.pname` | Centralized naming |
| `version` | from `constants.version` | Centralized versioning |
| `cargoHash` | SRI hash | Reproducible dependency vendoring |
| `nativeBuildInputs` | `[ protobuf pkg-config ]` | `protoc` for prost-build, pkg-config for native deps |
| `buildInputs` (darwin) | `[ SystemConfiguration libiconv ]` | macOS network APIs, string encoding |
| `env.PROTOC` | `"${pkgs.protobuf}/bin/protoc"` | Tell prost-build where protoc is |
| `doCheck` | `true` | Run `cargo test` during build |
| `meta.platforms` | `lib.platforms.darwin` | Binary only meaningful on macOS |

### 15.4 `nix/proto.nix` — Protobuf Compilation

```nix
{ pkgs, src }:
```

Provides an independent protobuf validation target (`nix build .#proto`). This does NOT feed into the main package build — `prost-build` in `build.rs` handles codegen during `cargo build` with the `PROTOC` environment variable pointing to nixpkgs' `protobuf`.

The proto target exists for:

- **CI validation:** confirm `.proto` files parse and are well-formed independently of Rust compilation
- **Proto linting:** run `buf lint` or `protoc --lint_out` against the proto definitions
- **Cross-language consumers:** generate code for languages other than Rust if needed later

### 15.5 `nix/checks.nix` — Automated Verification

```nix
{ pkgs, rustPlatform, src, advisory-db, constants }:
```

Returns an attribute set of check derivations, each running independently and caching separately:

| Check | Command | Fails On |
|-------|---------|----------|
| `clippy` | `cargo clippy -- -D warnings` | Any clippy warning |
| `fmt` | `cargo fmt --check` | Unformatted code |
| `test` | `cargo nextest run` | Test failures |
| `audit` | `cargo audit -d ${advisory-db}` | Known CVEs in dependencies |
| `deny` | `cargo deny check` | License violations, duplicate deps, advisories |
| `doc` | `cargo doc --no-deps` | Documentation build errors |

All checks run via `nix flake check`. Each is a separate derivation, so they execute in parallel and cache independently. A passing `nix flake check` means the code is formatted, lint-clean, tested, and free of known dependency vulnerabilities.

### 15.7 `flake.nix` — Top-Level Orchestration

The flake is kept lean — it imports `nix/constants.nix`, constructs the toolchain and `rustPlatform`, then delegates to each module:

```nix
{
  description = "BSD/macOS TCP socket statistics extraction tool";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
    advisory-db = { url = "github:rustsec/advisory-db"; flake = false; };
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

        package = import ./nix/package.nix { inherit pkgs rustPlatform src constants; };
        proto = import ./nix/proto.nix { inherit pkgs src; };
        checks = import ./nix/checks.nix { inherit pkgs rustPlatform src advisory-db constants; };
        shell = import ./nix/shell.nix { inherit pkgs rustToolchain package constants; };
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
```

### 15.6 `nix/shell.nix` — Development Shell

```nix
{ pkgs, rustToolchain, package, constants }:
```

The dev shell inherits all build dependencies from the package and adds security/analysis tooling:

```nix
pkgs.mkShell {
  inputsFrom = [ package ];

  packages = [
    rustToolchain
    pkgs.protobuf
    pkgs.buf                       # protobuf linter
  ]
  ++ (map (t: pkgs.${t}) constants.securityTools)
  ++ (map (t: pkgs.${t}) constants.analysisTools)
  ++ lib.optionals pkgs.stdenv.isDarwin [ pkgs.lldb ]
  ++ lib.optionals pkgs.stdenv.isLinux [ pkgs.gdb ];

  env = {
    PROTOC = "${pkgs.protobuf}/bin/protoc";
    RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
    CARGO_TERM_COLOR = "always";
  };

  shellHook = ''
    echo "bsd-xtcp dev shell — Rust ${constants.rustVersion}"
    echo "  cargo build        build the binary"
    echo "  cargo clippy       lint"
    echo "  cargo audit        CVE scan"
    echo "  cargo deny check   dependency policy"
    echo "  cargo geiger       unsafe code report"
    echo "  cargo nextest run  tests"
  '';
}
```

`inputsFrom = [ package ]` pulls in all `nativeBuildInputs` and `buildInputs` from the package (protobuf, pkg-config, Darwin frameworks) so the shell can compile the project without duplicating dependencies.

---

## 16. Security Tooling

The tool handles raw kernel data structures parsed from untrusted binary sysctl output. The Rust implementation must be resilient to malformed or truncated data. The security tooling enforced via `nix flake check` and available in the dev shell targets multiple layers:

### 16.1 Static Analysis

| Tool | Purpose | Integration |
|------|---------|-------------|
| `cargo clippy` | Lint for common mistakes, UB patterns, performance anti-patterns | Flake check (`-D warnings` = zero tolerance) |
| `cargo fmt` | Consistent formatting, reduces review noise | Flake check |
| `cargo doc` | Ensures public API documentation compiles | Flake check |

### 16.2 Dependency Auditing

| Tool | Purpose | Integration |
|------|---------|-------------|
| `cargo audit` | Cross-references `Cargo.lock` against RustSec advisory database | Flake check (uses pinned `advisory-db` input) |
| `cargo deny` | Enforces license policy, flags duplicate deps, checks advisories | Flake check (configured via `deny.toml`) |
| `cargo vet` | Mozilla's supply chain trust model — track which crate versions have been reviewed | Dev shell (manual, requires persistent state) |
| `cargo machete` | Detects unused dependencies in `Cargo.toml` | Dev shell |
| `cargo udeps` | Detects unused dependencies via nightly compiler analysis | Dev shell (requires nightly) |

### 16.3 Unsafe Code Detection

| Tool | Purpose | Integration |
|------|---------|-------------|
| `cargo geiger` | Reports all `unsafe` usage in the dependency tree | Dev shell |

This tool is critical for a project parsing raw kernel binary data. The sysctl parsing code (`pcblist.rs`, `procmap.rs`) will necessarily contain `unsafe` blocks for `transmute` / pointer casts of kernel structs. `cargo geiger` ensures unsafe usage is tracked and does not grow unintentionally in the rest of the codebase.

### 16.4 Fuzzing

| Tool | Purpose | Integration |
|------|---------|-------------|
| `cargo fuzz` | Libfuzzer-based fuzzing harnesses | Dev shell (requires nightly) |

Fuzz targets should cover the binary parsing functions that consume raw sysctl output:

- `pcblist::parse_pcblist` — parse `xtcpcb` array from raw bytes
- `procmap::parse_filetable` — parse `kern.file` output
- `platform::macos::parse_pcblist_n` — parse tagged `pcblist_n` stream

These are the highest-risk code paths — they interpret untrusted kernel binary data and contain `unsafe` pointer casts. Fuzzing them with arbitrary byte sequences tests resilience to truncated, corrupted, or version-mismatched kernel output.

### 16.5 Coverage

| Tool | Purpose | Integration |
|------|---------|-------------|
| `cargo nextest` | Fast parallel test runner with per-test isolation | Flake check |
| `cargo tarpaulin` | Line and branch coverage reporting | Dev shell |

---

## 17. Build Targets

Summary of all Nix build targets and their purposes:

| Command | What It Does |
|---------|--------------|
| `nix build` | Build the bsd-xtcp Rust binary (`cargo build --release`) |
| `nix build .#proto` | Validate protobuf definitions compile cleanly |
| `nix flake check` | Run all checks: clippy, fmt, tests, audit, deny, doc |
| `nix develop` | Enter dev shell with Rust toolchain + all security/analysis tools |
| `nix flake show` | List all outputs (packages, checks, devShells) |

Developer workflow:

```sh
# Enter the dev environment (all tools available)
nix develop

# Build and test
cargo build
cargo nextest run

# Pre-commit security checks (same as CI)
cargo clippy -- -D warnings
cargo fmt --check
cargo audit
cargo deny check

# Deeper analysis
cargo geiger                       # unsafe usage report
cargo tarpaulin                    # coverage
cargo +nightly fuzz run parse_pcblist   # fuzz the parser

# Full CI-equivalent check
nix flake check
```
