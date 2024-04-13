#!/bin/bash

set -euxo pipefail

# This is the whole kitchen sink to help ensure builds are ready to be published.

# STABLE CONFIGURATIONS

rustup run stable cargo check --no-default-features -p talc
rustup run stable cargo check --no-default-features -p talc --features=lock_api
rustup run stable cargo check --no-default-features -p talc --features=lock_api,allocator-api2,counters

rustup run stable cargo check -p talc --no-default-features --target wasm32-unknown-unknown
rustup run stable cargo check -p talc --no-default-features --features=lock_api,counters --target wasm32-unknown-unknown

# check that the examples work
rustup run stable cargo check -p stable_examples --example stable_allocator_api
rustup run stable cargo check -p stable_examples --example std_global_allocator

# check whether MSRV has been broken
rustup run 1.70.0 cargo check -p talc --no-default-features --features lock_api,allocator-api2,counters


# NIGHTLY CONFIGURATIONS

rustup run nightly cargo check -p talc

rustup run nightly cargo test -p talc --features=counters
rustup run nightly cargo test -p talc --tests --no-default-features
rustup run nightly cargo test -p talc --tests --no-default-features --features=lock_api,allocator-api2,counters

rustup run nightly cargo miri test -p talc --tests
rustup run nightly cargo miri test -p talc --tests --target i686-unknown-linux-gnu

# check the benchmarks
rustup run nightly cargo check -p benchmarks --bin microbench
rustup run nightly cargo check -p benchmarks --bin random_actions


# WASM BENCHMARKS CHECK

# check wasm size benches
./wasm-size.sh check
# check wasm size MSRV
rustup run 1.70.0 cargo check -p wasm-size --target wasm32-unknown-unknown

# check wasm perf benches
./wasm-perf.sh check
# check wasm perf MSRV
rustup run 1.73 wasm-pack --log-level warn build wasm-perf --dev --target web
