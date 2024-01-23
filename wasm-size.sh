#!/bin/bash

# This script calculates a weight heurisitic for WASM allocators.

cd wasm-size

echo "talc"
cargo build --quiet --release --target wasm32-unknown-unknown
wc -c ./target/wasm32-unknown-unknown/release/wasm_size.wasm

echo ""
echo "dlmalloc (default)"
cargo build --quiet --release --target wasm32-unknown-unknown --features dlmalloc
wc -c ./target/wasm32-unknown-unknown/release/wasm_size.wasm

echo ""
echo "lol_alloc"
cargo build --quiet --release --target wasm32-unknown-unknown --features lol_alloc
wc -c ./target/wasm32-unknown-unknown/release/wasm_size.wasm
