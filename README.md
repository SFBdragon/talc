# Talc Dynamic Memory Allocator

[![Crates.io](https://img.shields.io/crates/v/talc?style=flat-square&color=orange)](https://crates.io/crates/talc) [![Downloads](https://img.shields.io/crates/d/talc?style=flat-square)](https://crates.io/crates/talc) [![docs.rs](https://img.shields.io/docsrs/talc?style=flat-square)](https://docs.rs/talc/latest/talc/)

<sub><i>If you find Talc useful, please consider leaving tip via [Paypal](https://www.paypal.com/donate/?hosted_button_id=8CSQ92VV58VPQ)</i></sub>

<sep>

## What is this for?
- Embedded systems, OS kernels, and other `no_std` environments
- WebAssembly modules, as a drop-in replacement for DLmalloc

## Why Talc?
Performance:
- Fast & Efficient: [Benchmarks (Linux x86_64)](./BENCHMARKS.md)
- Fast & Small: [WebAssembly Benchmarks](./BENCHMARKS_WASM.md)

Features:
- Safe, lockless `TalcCell` for single-threaded allocation with `GlobalAlloc` and `Allocator`
- Flexible locking using `lock_api` for multi-threaded allocation using `TalcLock`
- Supports `allocator-api2` for using the `Allocator` API in safe Rust
- `"counters"`: Provides allocation statistics for debugging and performance insights
- `"cache-aligned-allocations"`: Mitigates false sharing between allocations
- Supports creating and resizing arbitrarily many heaps, manually or automatically
- Supports automatic reclaim of unused memory
- Correctness verified with tests, MIRI, and fuzzing

## Why not Talc?

If you're on a mature hosted system, especially one that `jemalloc` or `mimalloc` supports, consider those instead.
Those provide allocation concurrency, well-tested virtual memory API integration,
and are all-round more mature and sophisticated and more cleverly implemented than Talc.
Even the default allocator for Rust on Linux, for example, has impressive performance characteristics.

## Getting started

- [The `talc` README](./talc/README.md)
- [The `talc` README for WebAssembly](./talc/README_WASM.md)
- [The API reference](https://docs.rs/talc/latest/talc/)
