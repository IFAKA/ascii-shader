.PHONY: check bench dump pvshort
check:
	cargo build --release
	cargo clippy --release -- -D warnings
	cargo test --release

bench:
	cargo run --release -q -- --bench

dump:
	cargo run --release -q -- --dump 3 --out out --png

# 10 s ONE CUT social short: events -> sound -> sync check -> out/pv-short.mp4 + .gif
pvshort:
	tools/pv_short.sh out
