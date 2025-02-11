# Talc Dynamic Memory Allocator

[![Crates.io](https://img.shields.io/crates/v/talc?style=flat-square&color=orange)](https://crates.io/crates/talc) [![Downloads](https://img.shields.io/crates/d/talc?style=flat-square)](https://crates.io/crates/talc) [![docs.rs](https://img.shields.io/docsrs/talc?style=flat-square)](https://docs.rs/talc/latest/talc/)

<sub><i>If you find Talc useful, please consider leaving tip via [Paypal](https://www.paypal.com/donate/?hosted_button_id=8CSQ92VV58VPQ)</i></sub>

<sep>

## What is this for?
- Embedded systems, OS kernels, and other `no_std` environments
- WebAssembly modules, as one of the best drop-in replacements for DLmalloc
- Normal programs that need quick single-threaded allocation

## Why Talc?
Performance:
- Fast & Efficient: [Benchmarks (Linux x86_64)](https://github.com/SFBdragon/talc/blob/master/talc/BENCHMARK_RESULTS.md)
- Fast & Small: [WebAssembly Benchmarks](https://github.com/SFBdragon/talc/blob/master/talc/BENCHMARK_RESULTS_WASM.md)

Features:
- Safe, zero-runtime-overhead\* `TalcCell` for single-threaded allocation with `GlobalAlloc` and `Allocator`
- Flexible locking using `lock_api` for multi-threaded allocation using `Talck`
- Supports `allocator-api2` for using the `Allocator` API in safe Rust
- `"counters"`: Provides allocation statistics for debugging and performance insights
- `"cache-aligned-allocations"`: Mitigates false sharing between allocations
- Custom Out-Of-Memory handlers for just-in-time heap management, fallback, and recovery
- Supports creating and resizing arbitrarily many heaps
- Correctness verified with tests, MIRI, and fuzzing

\* `TalcCell` doesn't require any locking or runtime borrow-checking to safely allocate through shared references.

## Why not Talc?
- Doesn't scale well to allocation-heavy concurrent processing
- 16-bit architectures are unsupported

## Getting started

- [The `talc` README](https://github.com/SFBdragon/talc/blob/master/talc/README.md)
- [The `talc` README for WebAssembly](https://github.com/SFBdragon/talc/blob/master/talc/README_WASM.md)
- [The API reference](https://docs.rs/talc/latest/talc/)
