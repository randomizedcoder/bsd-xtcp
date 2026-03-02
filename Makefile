.PHONY: build test clippy fmt check clean \
       cross-x86_64-darwin cross-aarch64-darwin cross-all

# Native build
build:
	nix build .

test:
	nix develop -c cargo test

clippy:
	nix develop -c cargo clippy --all-targets -- -D warnings

fmt:
	nix develop -c cargo fmt --check

check: clippy fmt test

# Cross-compilation (Linux host only)
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

clean:
	rm -f result result-*
