#!/bin/bash

# This script calculates a weight heurisitic for WASM allocators.


COMMAND=""
if [[ $1 == "check" ]]; then
    COMMAND="check"
else
    COMMAND="build"
fi

cd wasm-size

ALLOCATORS="talc talc_arena dlmalloc lol_alloc"
for ALLOCATOR in ${ALLOCATORS}; do
    echo "${ALLOCATOR}"
    cargo $COMMAND --quiet --release --target wasm32-unknown-unknown --features ${ALLOCATOR}

    if [[ $1 != "check" ]]; then
        wasm-opt -Oz -o target/wasm32-unknown-unknown/release/wasm_size_opt.wasm target/wasm32-unknown-unknown/release/wasm_size.wasm
        echo -n "  "
        wc -c ./target/wasm32-unknown-unknown/release/wasm_size_opt.wasm
    fi
done
