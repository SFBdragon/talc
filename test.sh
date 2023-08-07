#!/bin/bash

set -euxo pipefail

cargo test
cargo test --tests --no-default-features
cargo test --tests --no-default-features --features=lock_api

cargo miri test --tests
cargo miri test --tests --target i686-unknown-linux-gnu

# both examples and docs contain things that miri isn't a fan of
# cargo miri test --doc


# check that the wasm project hasn't been broken

cd wasm-size

cargo check --target wasm32-unknown-unknown
cargo check --target wasm32-unknown-unknown --features lol_alloc
cargo check --target wasm32-unknown-unknown --features dlmalloc

cd -