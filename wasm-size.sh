#!/bin/bash

# This script calculates a weight heurisitic for WASM allocators.

# run `./wasm_size.sh` to measure talc's size
# run `./wasm_size.sh xyz`, where xyz is lol_alloc or dlmalloc 
#  to measure their size contribution respectively.

cd wasm-size

if [ $# = 1 ]; then
    cargo build --release --target wasm32-unknown-unknown --features $1
else
    cargo build --release --target wasm32-unknown-unknown
fi

wc -c ./target/wasm32-unknown-unknown/release/wasm_size.wasm
