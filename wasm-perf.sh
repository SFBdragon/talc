#!/bin/bash

# This script runs a benchmark on global alloctors for WASM.
# requires wasm-pack and deno

cd wasm-perf

ALLOCATORS="talc talc_arena dlmalloc lol_alloc"
for ALLOCATOR in ${ALLOCATORS}; do
    echo "${ALLOCATOR}"
    wasm-pack --log-level warn build --release --quiet --target web --features ${ALLOCATOR}

    if [[ $1 != "check" ]]; then
        deno run --allow-read bench.js
    fi
done
