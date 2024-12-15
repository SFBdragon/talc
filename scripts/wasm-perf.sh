#!/bin/bash

# This script runs a benchmark on global alloctors for WASM.
# requires wasm-pack and deno

ALLOCATORS="talc talc_arena rlsf dlmalloc lol_alloc"
for ALLOCATOR in ${ALLOCATORS}; do
    echo "${ALLOCATOR}"
    wasm-pack --log-level warn build wasm-perf --release --target web --features ${ALLOCATOR}

    if [[ $1 != "check" ]]; then
        cd wasm-perf
        deno run --allow-read bench.js
        cd -
    fi
done
