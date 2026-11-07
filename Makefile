.PHONY: build test clippy fmt check test-uinput list monitor serve

build:
	cargo build --features frp

test:
	cargo test --features frp,testing

clippy:
	cargo clippy --all-targets --features frp,testing -- -D warnings

fmt:
	cargo fmt --all

# Every feature combination must build, the config types for WebAssembly too,
# and the core must build off Linux.
check:
	cargo build --lib
	cargo build --lib --no-default-features --features config
	cargo build --lib --target wasm32-unknown-unknown --no-default-features --features config
	cargo build --lib --features driver
	cargo build --lib --features frp
	cargo build --lib --features testing
	cargo build --all-targets --features frp,testing
	cargo clippy --all-targets --target x86_64-pc-windows-gnu --features frp,testing -- -D warnings

# Needs a writable /dev/uinput and read access to new /dev/input/event* nodes.
test-uinput:
	cargo test --features testing --test uinput -- --ignored

list:
	cargo run --example list

monitor:
	cargo run --example monitor -- $(if $(device),--device $(device))

serve:
	cargo run --features frp --bin thumbswing-frp -- $(if $(device),--set device=$(device))
