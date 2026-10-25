#!/bin/sh
# Builds the web version into web/dist, ready to host as static files.
# Needs: rustup target add wasm32-unknown-unknown, and wasm-bindgen-cli at
# the same version as the wasm-bindgen crate in Cargo.lock.
set -e
cd "$(dirname "$0")/.."
cargo build --release --target wasm32-unknown-unknown
rm -rf web/dist
wasm-bindgen --target web --no-typescript --out-dir web/dist \
  target/wasm32-unknown-unknown/release/daw.wasm
cp web/index.html web/dist/
cp assets/icon-512.png web/dist/icon.png
