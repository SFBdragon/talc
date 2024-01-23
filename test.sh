#!/bin/bash

set -euxo pipefail

# This is the whole kitchen sync to help ensure builds are ready to be published.

rustup default nightly

cargo check
cargo test --features=counters
cargo test --tests --no-default-features
cargo test --tests --no-default-features --features=lock_api

cargo miri test --tests
cargo miri test --tests --target i686-unknown-linux-gnu


rustup default stable
cargo check --no-default-features
cargo check --no-default-features --target wasm32-unknown-unknown
cargo check --no-default-features --features=lock_api --target wasm32-unknown-unknown

rustup default nightly
cargo check
cargo check --no-default-features

# both examples and docs contain things that miri isn't a fan of
# cargo miri test --doc


# check that the wasm projects haven't been broken

cd wasm-size

cargo check --target wasm32-unknown-unknown
cargo check --target wasm32-unknown-unknown --features lol_alloc
cargo check --target wasm32-unknown-unknown --features dlmalloc

cd -

cd wasm-bench

wasm-pack --log-level warn build --dev --target web
wasm-pack --log-level warn build --dev --target web --features talc
wasm-pack --log-level warn build --dev --target web --features lol_alloc

cd -