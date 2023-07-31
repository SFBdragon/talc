#!/bin/bash

set -euxo pipefail

cd wasm-example

cargo build --release --target wasm32-unknown-unknown

wc -c ./target/wasm32-unknown-unknown/release/talc_wasm_example.wasm