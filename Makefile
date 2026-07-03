.PHONY: all check test clippy fmt clean

all: check test clippy fmt

check:
	cargo check --workspace

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace -- -D warnings

fmt:
	cargo fmt --check

fix:
	cargo fmt

clean:
	cargo clean

audit:
	cargo audit
