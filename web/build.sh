#!/usr/bin/env sh
# Builds the pack builder's WebAssembly module into web/site (needs Rust with the
# wasm32-unknown-unknown target: rustup target add wasm32-unknown-unknown).
set -e
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown --manifest-path wasm/Cargo.toml
cp wasm/target/wasm32-unknown-unknown/release/seventv_pack_wasm.wasm site/pack.wasm
sed -n 's/^version = "\(.*\)"/\1/p' wasm/Cargo.toml | head -n 1 > site/version.txt
echo "Built web/site/pack.wasm ($(wc -c < site/pack.wasm) bytes)"
