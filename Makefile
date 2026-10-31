.PHONY: check bench dump
check:
	cargo build --release
	cargo clippy --release -- -D warnings
	cargo test --release

bench:
	cargo run --release -q -- --bench

dump:
	cargo run --release -q -- --dump 3 --out out --png
