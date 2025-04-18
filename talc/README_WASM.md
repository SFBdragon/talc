# Talc for WebAssembly

Run `cargo add talc` and add the following lines somewhere in your code:

```rust,no_run
// SAFETY: The runtime environment must be single-threaded WASM.
#[global_allocator]
static TALC: talc::wasm::WasmDynamicTalc = unsafe { talc::wasm::new_wasm_dynamic_allocator() };
```

---

Talc is also a drop-in replacement for the default Rust WebAssembly allocator, DLmalloc.

Talc is much faster than DLmalloc and much smaller. See the [WebAssembly Allocator Benchmarks](https://github.com/SFBdragon/talc/blob/master/talc/BENCHMARK_RESULTS_WASM.md).

## Configuration features for WebAssembly

Reducing WebAssembly module size:
* `"disable-grow-in-place"` - disables grow-in-place, saving WebAssembly module bytes, but sacrifices some runtime speed
* `"disable-realloc-in-place"` - disables realloc-in-place (grow and shrink), saving WebAssembly module bytes, but sacrifices some runtime speed
    * `"disable-grow-in-place"` has no effect if `"disable-realloc-in-place"` is enabled

See the [WebAssembly Allocator Benchmarks](https://github.com/SFBdragon/talc/blob/master/talc/BENCHMARK_RESULTS_WASM.md) to get a sense for the tradeoffs between performance and size, as a bunch of possible configuration are tested.

Not WebAssembly-specific:
- `"counters"`: `Talc` will track heap and allocation metrics. Use the `counters` associated function to access them.
- `"nightly"`: Enable nightly-only APIs. Currently allows `TalcLock` and `TalcCell` to implement `core::alloc::Allocator`.
- `"cache-aligned-allocation"`: `Talc` will align all of its chunks according to `crossbeam_utils::CachePadded`.
    - This is intended to mitigate [false sharing](https://en.wikipedia.org/wiki/False_sharing) between different
        allocations that will be used from different threads.
    - Using this is strongly recommended as opposed to always demanding very-high alignments from Talc

## Global Allocator for single-threaded WebAssembly

Run `cargo add talc`

Then add these lines into your source code somewhere...

```rust,no_run
use talc::wasm::*;

// `WasmDynamicTalc` dynamically obtains memory from the WebAssembly
// memory subsystem on-demand.
// SAFETY: The runtime environment must be single-threaded WASM.
#[global_allocator]
static TALC: WasmDynamicTalc = unsafe { new_wasm_dynamic_allocator() };
```

Or if arena allocation is desired...

```rust
use talc::wasm::*;

// `WasmArenaTalc` reserves a fixed-width arena for allocation.
// SAFETY: The runtime environment must be single-threaded WASM.
#[global_allocator]
static TALC: WasmArenaTalc = {
    use core::mem::MaybeUninit;
    static mut MEMORY: [MaybeUninit<u8>; 0x8000000] = [MaybeUninit::uninit(); 0x8000000];
    unsafe { new_wasm_arena_allocator(&raw mut MEMORY) }
};
```

## Global Allocator for threaded WebAssembly

Using a dynamically-sized heap with WebAssembly memory integration...

```rust,no_run
use talc::{wasm::*, sync::TalcLock};

#[global_allocator]
static TALC: TalcLock<spin::Mutex<()>, ClaimWasmMemOnOom, WasmBinning> = TalcLock::new(ClaimWasmMemOnOom);
```

Simple arena allocation...

```rust
use talc::{wasm::WasmBinning, sync::TalcLock, Claim};

#[global_allocator]
static TALC: TalcLock<spin::Mutex<()>, Claim, WasmBinning> = {
    use core::mem::MaybeUninit;
    static mut MEMORY: [MaybeUninit<u8>; 0x8000000] = [MaybeUninit::uninit(); 0x8000000];
    TalcLock::new(unsafe { Claim::array(&raw mut MEMORY) })
};
```
