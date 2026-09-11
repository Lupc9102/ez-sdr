.PHONY: all check test clippy fmt clean fix audit deny

all: check test clippy fmt

check:
	cargo check --workspace --all-targets

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --check

fix:
	cargo fmt

clean:
	cargo clean

audit:
	cargo audit

deny:
	cargo deny check
