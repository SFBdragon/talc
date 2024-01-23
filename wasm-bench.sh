#!/bin/bash

# This script runs a benchmark on global alloctors for WASM.
# requires wasm-pack and deno

cd wasm-bench

echo "talc"
rustup run nightly wasm-pack --log-level warn build --release --quiet --target web --features talc
deno run --allow-read bench.js

echo ""
echo "talc (static)"
rustup run nightly wasm-pack --log-level warn build --release --quiet --target web --features talc_static
deno run --allow-read bench.js

echo ""
echo "dlmalloc (default)"
rustup run nightly wasm-pack --log-level warn build --release --quiet --target web
deno run --allow-read bench.js

echo ""
echo "lol_alloc"
rustup run nightly wasm-pack --log-level warn build --release --quiet --target web --features lol_alloc
deno run --allow-read bench.js
