#!/bin/bash

set -euxo pipefail

# This is the whole kitchen sink to help ensure builds are ready to be published.

rustup run stable cargo check --no-default-features
rustup run stable cargo check --no-default-features --features=lock_api
rustup run stable cargo check --no-default-features --features=lock_api,counters

rustup run nightly cargo check

rustup run nightly cargo test --features=counters
rustup run nightly cargo test --tests --no-default-features
rustup run nightly cargo test --tests --no-default-features --features=lock_api,counters

rustup run nightly cargo miri test --tests
rustup run nightly cargo miri test --tests --target i686-unknown-linux-gnu

rustup run stable cargo check --no-default-features --target wasm32-unknown-unknown
rustup run stable cargo check --no-default-features --features=lock_api,counters --target wasm32-unknown-unknown

# check whether MSRV has been broken
rustup run 1.67.1 cargo check --no-default-features --features lock_api,counters


# check that the wasm benches haven't been broken

# check wasm size benches
./wasm-size.sh check
# check wasm size MSRV
cd wasm-size && rustup run 1.68 cargo check --target wasm32-unknown-unknown && cd -

# check wasm perf benches
./wasm-perf.sh check
# check wasm perf MSRV
cd wasm-perf && rustup run 1.67.1 wasm-pack --log-level warn build --dev --target web && cd -
