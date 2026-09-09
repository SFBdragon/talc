# Talc Dynamic Memory Allocator

[![Crates.io](https://img.shields.io/crates/v/talc?style=flat-square&color=orange)](https://crates.io/crates/talc) [![Downloads](https://img.shields.io/crates/d/talc?style=flat-square)](https://crates.io/crates/talc) [![docs.rs](https://img.shields.io/docsrs/talc?style=flat-square)](https://docs.rs/talc/latest/talc/)

<sub><i>If you find Talc useful, please consider leaving tip via [Paypal](https://www.paypal.com/donate/?hosted_button_id=8CSQ92VV58VPQ) or [Ko-Fi](https://ko-fi.com/shaunbeautement)</i></sub>

---

Note that this README acts as a guide to using Talc. For a brief explanation of what Talc is and why you should or shouldn't use it, see the [repository README.md](https://github.com/SFBdragon/talc/blob/master/README.md).

## Table of Contents

Targeting WebAssembly? Check out [the WebAssembly README](https://github.com/SFBdragon/talc/blob/master/talc/README_WASM.md).

- [Optional Features](#optional-features)
- [Setup](#setup)
- [General Usage](#general-usage)
- [Advanced Usage](#advanced-usage)
- [Algorithm](#algorithm)
- [Changelog](#changelog)


## Optional Features
- `"counters"`: `Talc` will track heap and allocation metrics. Use the `counters` associated function to access them.
- `"nightly"`: Enable nightly-only APIs. Currently allows `TalcLock` and `TalcCell` to implement `core::alloc::Allocator`.
- `"disable-grow-in-place"`: Never uses the grow-in-place routine to implement `GlobalAlloc` or `Allocator`. Intended to reduce size for WebAssembly.
- `"disable-realloc-in-place"`: Never uses grow- or shrink-in-place routines to implement `GlobalAlloc` or `Allocator`. Intended to reduce size for WebAssembly.

## Setup

There are two choices to make.

```text
 ----- Wrapper ----- | -- Allocator -- | ----- Source -----
                     |                 |
  Provides interior  |                 |       Manual
   mutability, for   |                 |
   GlobalAlloc and   |                 |       Claim
    Allocator APIs   |                 |
                     |                 |  GlobalAllocSource
  --- TalcLock  ---  |                 |   AllocatorSource
                     |      Talc       |
  Synchronized via   |                 |          
      lock_api       |                 |   WasmGrowAndClaim
                     |                 |   WasmGrowAndExtend
  --- TalcCell  ---  |                 |
                     |                 |
 Exposes an API with |                 |
 Cell's constraints, |                 |     /Your Own!
  free !Sync access  |                 |
                     |                 |
```

`Talc` is the core of the allocator, but usually not useful alone.
- Use `TalcCell` for single-threaded allocation, e.g. using the `Allocator` interface.
- Use `TalcLock` for multi-threaded allocation, e.g. as a `#[global_allocator]`
    - TalcLock requires a locking mechanism implementing `lock_api::RawMutex`, e.g. `spinning_top::Raw`

Now you need to decide how you're going to establish heaps for `Talc` to allocate from.
- You can manually do this using `claim`
- You can have an `Source` do this for you
    - `Claim` tries to `claim` a region of memory you specify, once needed.
    - Platform-specific sources like `WasmGrowAndExtend` retrieve memory from the system as needed.
    - Some, like `GlobalAllocSource`, and `AllocatorSource` reserve and release memory dynamically.

See the following two examples of how this looks in practice.

#### As a global allocator

```rust
use talc::{*, source::Claim};

#[global_allocator]
static TALC: TalcLock<spinning_top::RawSpinlock, Claim> = TalcLock::new(unsafe {
    static mut INITIAL_HEAP: [u8; min_first_heap_size::<DefaultBinning>() + 100000] =
        [0; min_first_heap_size::<DefaultBinning>() + 100000];

    Claim::array(&raw mut INITIAL_HEAP)
});

fn main() {
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
}
```

See [examples/global_allocator.rs](https://github.com/SFBdragon/talc/blob/master/talc/examples/global_allocator.rs) for a more detailed example.

#### Using the `Allocator` API

```rust
// if "nightly" is enabled, core::alloc::Allocator can be used instead of allocator-api2
// #![feature(allocator_api)]

use allocator_api2::alloc::{Allocator, Layout};
use talc::{TalcCell, source::Claim};

fn main() {
    let mut heap = [0u8; 10000];
    let talc = TalcCell::new(unsafe { Claim::array(&raw mut heap) });

    let my_vec = allocator_api2::vec::Vec::<u8, _>::with_capacity_in(234, &talc);
    let my_allocation = talc.allocate(Layout::new::<[u32; 16]>()).unwrap();
}
```

See [examples/allocator_api.rs](https://github.com/SFBdragon/talc/blob/master/talc/examples/allocator_api.rs) for a more detailed example.

## API Overview

Whether you're using `Talc`, `TalcLock` (call `lock` to get the `Talc`), or `TalcCell` (re-exposes the API directly): usage is similar.

#### Allocation

`TalcLock` and `TalcCell` implement the `GlobalAlloc` and `Allocator` traits.

`Talc` exposes the allocation primitives
- `allocate`
- `try_allocate`
- `deallocate`
- `try_grow_in_place`
- `shrink`
- `try_realloc_in_place`

#### Heap Management
* `claim` - establish a heap
* `reserved` - query for the region of bytes reserved due to allocations
* `extend`/`truncate`/`resize` - change the size of an existing heap

#### Statistics - requires `"counters"` feature
* `counters` - obtains the `Counters` struct which contains heap and allocation statistics

Read their [documentation](https://docs.rs/talc/latest/talc/base/struct.Talc.html) for more info.

## Sources

Implementations of `Source` inform how the allocator establishes and manages the heaps of memory
is allocates from.

Note that you can always use `claim`/`extend`/`truncate`/`resize` to manage heaps
with some sources, but not others.

Provided `Source` implementations include:
- Manual heap management
    - `Manual`: allocations fail on OOM, manual heap management allowed
    - `Claim`: claims a heap upon first OOM, useful for initialization
- Automatic heap management
    - `GlobalAllocSource` and `AllocatorSource`: obtains and frees memory back to another allocator
    - `WasmGrow*`: use platform APIs to manage memory

Custom ones can be implemented too.

## Algorithm & Internals

This is a dlmalloc-style linked list allocator with boundary tagging and binning, aimed at general-purpose use cases. Allocation O(n) worst case (but in practice its near-constant time, see microbenchmarks), while in-place reallocations and deallocations are O(1).

The implementation shares a lot of similarities with the TLSF algorithm, but is nowhere near as pure as `rlsf`.

## Migrating from v4 to v5

If you're using WebAssembly, check out the [guide](https://github.com/SFBdragon/talc/blob/master/talc/README_WASM.md).

The allocator is now stable-by-default. Enable the `"nightly"` feature if necessary.

The configurable features have changes significantly as well. See the [Features](#features) section.

You typically won't use `Talc::new` anymore. Use `TalcLock::new` or `TalcCell::new`.

The heap management APIs: `Talc::claim`, `Talc::extend`, `Talc::reserved` (previously `get_allocated_span`), `Talc::truncate`, `Talc::resize` (new!) changed in various ways. Please check their docs for more info. `Span` has been removed.

Feel free to reach out or open a PR if you have any unaddressed questions.

## Changelog

The full changelog can be [found here](./CHANGELOG.md). The most recent changes are:

#### v5.0.4

- Bug fix: chunk tagging was not robust to chunks over `pow(2, 8 * (size_of::<usize>() - 1))`.
  Thanks [João Lucas](https://github.com/jlucaso1)!
- Bug fix: `WasmGrowAndClaim` was broken due to undersizing the claimed memory in certain circumstances.
  This could cause infinite recusion, growing the WASM linear memory until that failed.
  Thanks [max-dau](https://github.com/max-dau) and [João Lucas](https://github.com/jlucaso1)!

This incurred a regression: allocation metadata overhead is a `usize`, not a `u8`.
(This has no impact on allocations of 24 bytes or less.)

The internal details may change in a future update to optimize chunk overhead.

#### v5.1.0

- Bug fix for [#54](https://github.com/SFBdragon/talc/issues/54): GlobalAllocSource and AllocatorSource did not implement `Send`.
  Thanks [funsafemath](https://github.com/funsafemath) for the issue!

- (Possible API Break) Switched `talc::wasm::new_wasm_dynamic_allocator()` and `talc::wasm::WasmDynamicTalc`
  to use `WasmGrowAndExtend` instead of `WasmGrowAndClaim`.

  The rationale for this change is given [in this issue](https://github.com/SFBdragon/talc/issues/51):
    - Advantage: `WasmClaimAndExtend` is significantly more memory-efficient than `WasmGrowAndClaim`
      which certain pathological cases, including growing a vector repeatedly consuming as much as 10x more memory.
    - Disadvantage: `WasmClaimAndExtend` costs 97B of additional binary size (8~9% regression).
    - Deciding factor: binary size is a much more visible artifact to WASM developers than memory efficiency.
      Therefore it's better for the default to compromise on the more visible downside (and alternative tradeoffs).
      For those who wish to minimize their binary size as much as possible at the cost of other metrics should
      check out [the WASM README for options and alternatives for reducing binary size](https://github.com/SFBdragon/talc/README_WASM.md).
    
  Because `WasmDynamicTalc` is a type alias, not a type itself, this could potentially
  lead to breaking changes, but they should be easy to rectify:
  - to revert back to `WasmGrowAndClaim` use e.g.
  `static TALC: TalcSyncCell<WasmGrowAndClaim, WasmBinning> = TalcSyncCell::new_wasm(WasmGrowAndClaim);`
  - to use `WasmGrowAndExtend` use e.g.
  `static TALC: talc::wasm::WasmDynamicTalc = talc::wasm::new_wasm_dynamic_allocator();`

#### v5.1.1

- Re-export `lock_api` as `talc::lock_api`. This is mostly for convenience, and potentially reducing
  the number of dependencies to manage by library consumers. This addresses [Issue #55](https://github.com/SFBdragon/talc/issues/55),
  thanks [gzp79](https://github.com/gzp79)!
