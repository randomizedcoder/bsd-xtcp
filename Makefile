.PHONY: build build-tcp-echo test clippy fmt check clean \
       cross-x86_64-darwin cross-aarch64-darwin cross-all \
       tcp-echo-cross-x86_64-darwin tcp-echo-cross-aarch64-darwin \
       kmod-analysis-gcc-warnings kmod-analysis-gcc-fanalyzer \
       kmod-analysis-scan-build kmod-analysis-clang-tidy \
       kmod-analysis-cppcheck kmod-analysis-semgrep \
       kmod-analysis-flawfinder kmod-analysis-all analyze

# Native builds
build:
	nix build .

build-tcp-echo:
	nix build .#tcp-echo

# Workspace-wide checks
test:
	nix develop -c cargo test --workspace

clippy:
	nix develop -c cargo clippy --workspace --all-targets -- -D warnings

fmt:
	nix develop -c cargo fmt --all --check

check: clippy fmt test

# Cross-compilation: bsd-xtcp (Linux host only)
cross-x86_64-darwin:
	nix build .#cross-x86_64-darwin --out-link result-cross-x86_64-darwin
	@echo "Output: result-cross-x86_64-darwin/bin/bsd-xtcp"
	@file result-cross-x86_64-darwin/bin/bsd-xtcp

cross-aarch64-darwin:
	nix build .#cross-aarch64-darwin --out-link result-cross-aarch64-darwin
	@echo "Output: result-cross-aarch64-darwin/bin/bsd-xtcp"
	@file result-cross-aarch64-darwin/bin/bsd-xtcp

cross-all: cross-x86_64-darwin cross-aarch64-darwin
	@echo "All cross targets built."

# Cross-compilation: tcp-echo (Linux host only)
tcp-echo-cross-x86_64-darwin:
	nix build .#tcp-echo-cross-x86_64-darwin --out-link result-tcp-echo-cross-x86_64-darwin
	@echo "Output: result-tcp-echo-cross-x86_64-darwin/bin/tcp-echo"
	@file result-tcp-echo-cross-x86_64-darwin/bin/tcp-echo

tcp-echo-cross-aarch64-darwin:
	nix build .#tcp-echo-cross-aarch64-darwin --out-link result-tcp-echo-cross-aarch64-darwin
	@echo "Output: result-tcp-echo-cross-aarch64-darwin/bin/tcp-echo"
	@file result-tcp-echo-cross-aarch64-darwin/bin/tcp-echo

clean:
	rm -f result result-*

# C static analysis (kmod)
kmod-analysis-gcc-warnings:
	nix run .#kmod-analysis-gcc-warnings

kmod-analysis-gcc-fanalyzer:
	nix run .#kmod-analysis-gcc-fanalyzer

kmod-analysis-scan-build:
	nix run .#kmod-analysis-scan-build

kmod-analysis-clang-tidy:
	nix run .#kmod-analysis-clang-tidy

kmod-analysis-cppcheck:
	nix run .#kmod-analysis-cppcheck

kmod-analysis-semgrep:
	nix run .#kmod-analysis-semgrep

kmod-analysis-flawfinder:
	nix run .#kmod-analysis-flawfinder

kmod-analysis-all:
	nix run .#kmod-analysis-all

analyze: kmod-analysis-all

freebsd150:
	ssh root@192.168.122.41

freebsd143:
	ssh root@192.168.122.27
