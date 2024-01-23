#!/bin/bash

# This script runs a benchmark on global alloctors for WASM.
# requires wasm-pack and deno

cd wasm-bench

echo "talc"
wasm-pack --log-level warn build --release --quiet --target web --features talc
deno run --allow-read bench.js

echo ""
echo "dlmalloc (default)"
wasm-pack --log-level warn build --release --quiet --target web
deno run --allow-read bench.js

echo ""
echo "lol_alloc"
wasm-pack --log-level warn build --release --quiet --target web --features lol_alloc
deno run --allow-read bench.js
