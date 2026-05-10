#!/usr/bin/env bash
# Rebuild the init-smoke fixture component.
#
# Prerequisites:
#   rustup target add wasm32-wasip2
#   cargo install cargo-component --locked
#
# Usage: ./build.sh
#
# The generated init_smoke.wasm is checked into the repo so torvyn-engine
# integration tests do not require a Wasm toolchain at test time.

set -euo pipefail

cd "$(dirname "$0")/init-smoke"

cargo component build --release --target wasm32-wasip2

cp target/wasm32-wasip2/release/torvyn_engine_init_smoke.wasm \
   ../init_smoke.wasm

echo "rebuilt $(realpath ../init_smoke.wasm)"
