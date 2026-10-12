.PHONY: all check test clippy fmt clean fix audit deny

all: check test clippy fmt

check:
	cargo check --workspace --locked --all-targets

test:
	cargo test --workspace --locked

clippy:
	cargo clippy --workspace --locked --all-targets -- -D warnings

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
