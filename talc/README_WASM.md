# Talc for WebAssembly

Talc is also a drop-in replacement for the default Rust WebAssembly allocator, DLmalloc.

Talc is much faster than DLmalloc and much smaller. See the [WebAssembly Allocator Benchmarks](https://github.com/SFBdragon/talc/blob/master/BENCHMARKS_WASM.md).

---

Run `cargo add talc` and add the following lines somewhere in your code:

```rust
#[cfg(all(not(target_feature = "atomics"), target_family = "wasm"))]
#[global_allocator]
static TALC: talc::wasm::WasmDynamicTalc = talc::wasm::new_wasm_dynamic_allocator();
```

---

## Configuration features for WebAssembly

Reducing WebAssembly module size:
* `"disable-grow-in-place"` - disables grow-in-place, saving WebAssembly module bytes, but sacrifices some runtime speed
* `"disable-realloc-in-place"` - disables realloc-in-place (grow and shrink), saving WebAssembly module bytes, but sacrifices some runtime speed
    * `"disable-grow-in-place"` has no effect if `"disable-realloc-in-place"` is enabled

See the [WebAssembly Allocator Benchmarks](https://github.com/SFBdragon/talc/blob/master/BENCHMARKS_WASM.md) to get a sense for the tradeoffs between performance and size, as a bunch of possible configuration are tested.

Not WebAssembly-specific:
- `"counters"`: `Talc` will track heap and allocation metrics. Use the `counters` associated function to access them.
- `"nightly"`: Enable nightly-only APIs. Currently allows `TalcLock` and `TalcCell` to implement `core::alloc::Allocator`.

## Global Allocator for single-threaded WebAssembly

Run `cargo add talc`

Then add these lines into your source code somewhere...

```rust
#[cfg(all(not(target_feature = "atomics"), target_family = "wasm"))]
#[global_allocator]
static TALC: talc::wasm::WasmDynamicTalc = talc::wasm::new_wasm_dynamic_allocator();
```

Or if arena allocation is desired...

```rust
#[cfg(all(not(target_feature = "atomics"), target_family = "wasm"))]
#[global_allocator]
static TALC: talc::wasm::WasmArenaTalc = {
    use core::mem::MaybeUninit;
    static mut MEMORY: [MaybeUninit<u8>; 0x8000000] = [MaybeUninit::uninit(); 0x8000000];
    // SAFETY: the memory for MEMORY is never modified externally. It's the allocator's.
    unsafe { talc::wasm::new_wasm_arena_allocator(&raw mut MEMORY) }
};
```

## Global Allocator for threaded WebAssembly

Using a dynamically-sized heap with WebAssembly memory integration...

```rust
use talc::{wasm::*, sync::TalcLock};

#[cfg(target_family = "wasm")]
#[global_allocator]
static TALC: TalcLock<spinning_top::RawSpinlock, WasmGrowAndClaim, WasmBinning> = TalcLock::new(WasmGrowAndClaim);
```

Simple arena allocation...

```rust
use talc::{wasm::WasmBinning, sync::TalcLock, source::Claim};

#[global_allocator]
static TALC: TalcLock<spinning_top::RawSpinlock, Claim, WasmBinning> = {
    use core::mem::MaybeUninit;
    static mut MEMORY: [MaybeUninit<u8>; 0x8000000] = [MaybeUninit::uninit(); 0x8000000];
    TalcLock::new(unsafe { Claim::array(&raw mut MEMORY) })
};
```
