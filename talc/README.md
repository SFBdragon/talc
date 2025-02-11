# Talc Dynamic Memory Allocator 

[![Crates.io](https://img.shields.io/crates/v/talc?style=flat-square&color=orange)](https://crates.io/crates/talc) [![Downloads](https://img.shields.io/crates/d/talc?style=flat-square)](https://crates.io/crates/talc) [![docs.rs](https://img.shields.io/docsrs/talc?style=flat-square)](https://docs.rs/talc/latest/talc/)

## Table of Contents

Targeting WebAssembly? Check out [the WebAssembly README](https://github.com/SFBdragon/talc/blob/master/talc/README_WASM.md).

- [Features](#features)
- [Setup](#setup)
- [General Usage](#general-usage)
- [Advanced Usage](#advanced-usage)
- [Algorithm](#algorithm)
- [Changelog](#changelog)


## Features
- `"counters"`: `Talc` will track arena and allocation metrics. Use the `counters` associated function to access them.
- `"nightly"`: Enable nightly-only APIs. Currently allows `Talck` and `TalcCell` to implement `core::alloc::Allocator`.
- `"cache-aligned-allocation"`: `Talc` will align all of its chunks according to `crossbeam_utils::CachePadded`.
    - This is intended to mitigate [false sharing](https://en.wikipedia.org/wiki/False_sharing) between different
        allocations that will be used from different threads.
    - Using this is strongly recommended as opposed to always demanding very-high alignments from Talc

- `"disable-grow-in-place"`: Never uses the grow-in-place routine to implement `GlobalAlloc` or `Allocator`. Intended to reduce size for WebAssembly.
- `"disable-realloc-in-place"`: Never uses grow- or shrink-in-place routines to implement `GlobalAlloc` or `Allocator`. Intended to reduce size for WebAssembly.

## Setup

`Talc` is the core, but usually not useful alone.
- Use `TalcCell` for single-threaded allocation, e.g. using the `Allocator` interface.
- Use `Talck` for multi-threaded allocation, e.g. as a `#[global_allocator]`
    - Talck requires a locking mechanism implementing `lock_api::RawMutex`, e.g. `spin::Mutex<()>`

Now you need to decide how you're going to establish arenas for `Talc` to allocate from.
- You can manually do this using `claim` 
- You can have an OOM (Out Of Memory) handler do this for you
    - `ClaimOnOom` tries to `claim` a region of memory you specify.
    - Platform-specific OOM handlers like `ClaimWasmMemOnOom` retrieve memory from the system as needed.
    - TODO

See the following two examples of how this looks in practice.

#### As a global allocator

```rust
use talc::*;

static mut ARENA: [u8; 10000] = [0; 10000];

#[global_allocator]
static TALCK: Talck<spin::Mutex<()>, ClaimOnOom> = Talck::new(unsafe {
    // If we're in a hosted environment, the Rust runtime may allocate before
    // main() is called, so we need to initialize the arena automatically.
    // We use `ClaimOnOom` to claim `ARENA` when 
    ClaimOnOom::new(Span::slice(&raw mut ARENA))
});

fn main() {
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
}
```

See [examples/global_allocator.rs]((https://github.com/SFBdragon/talc/blob/master/talc/examples/global_allocator.rs) for a more detailed example.

#### Using the `Allocator` API

```rust
// if "nightly" is enabled, core::alloc::Allocator can be used instead
// #![feature(allocator_api)]

use allocator_api2::alloc::{Allocator, Layout}
use talc::*;

fn main() {
    let mut arena = [0u8; 10000];

    let talc = TalcCell::new(ErrOnOom);
    unsafe { talc.claim(arena.as_mut().into()); }
    
    let my_vec = allocator_api2::vec::Vec::new_in(&talc);
    let my_allocation = talc.allocate(Layout::new::<[u32; 16]>()).unwrap();
}
```

See [examples/allocator_api.rs]((https://github.com/SFBdragon/talc/blob/master/talc/examples/allocator_api.rs) for a more detailed example.

## API Overview

Whether you're using `Talc`, `Talck` (call `lock` to get the `Talc`), or `TalcCell` (re-exposes the API directly), usage is similar.

#### Allocation

`Talck` and `TalcCell` implement the `GlobalAlloc` and `Allocator` traits.

`Talc` exposes the allocation primitives
- `allocate`
- `deallocate`
- `try_grow_in_place`
- `shrink`
- `try_resize_in_place`

#### Arena Management
* `claim` - establish an `Arena`
* `reserved` - query for the region of bytes reserved due to allocations
* `extend`/`truncate`/`resize` - change the size of an existing `Arena`

#### Statistics - requires "counters" feature
* `counter` - obtains the `Counters` struct which contains arena and allocation statistics

Read their [documentation](https://docs.rs/talc/latest/talc/struct.Talc.html) for more info.

## Advanced Usage

The most powerful feature of the allocator is that it has a modular OOM handling system, allowing you to fail out of or recover from allocation failure easily. 

Provided `OomHandler` implementations include:
- `ErrOnOom`: allocations fail on OOM
- `ClaimOnOom`: claims a heap upon first OOM, useful for initialization
- `WasmHandler`: itegrate with WebAssembly's `memory` module for automatic memory heap management

As an example of a custom implementation, recovering by extending the heap is implemented below.

```rust
use talc::*;

struct MyOomHandler {
    heap: Span,
}

impl OomHandler for MyOomHandler {
    fn handle_oom(talc: &mut Talc<Self>, layout: core::alloc::Layout) -> Result<(), ()> {
        // Talc doesn't have enough memory, and we just got called!
        // We'll go through an example of how to handle this situation.
    
        // We can inspect `layout` to estimate how much we should free up for this allocation
        // or we can extend by any amount (increasing powers of two has good time complexity).
        // (Creating another heap with `claim` will also work.)
    
        // This function will be repeatedly called until we free up enough memory or 
        // we return Err(()) causing allocation failure. Be careful to avoid conditions where 
        // the heap isn't sufficiently extended indefinitely, causing an infinite loop.
    
        // an arbitrary address limit for the sake of example
        const HEAP_TOP_LIMIT: *mut u8 = 0x80000000 as *mut u8;
    
        let old_heap: Span = talc.oom_handler.heap;
    
        // we're going to extend the heap upward, doubling its size
        // but we'll be sure not to extend past the limit
        let new_heap: Span = old_heap.extend(0, old_heap.size()).below(HEAP_TOP_LIMIT);
    
        if new_heap == old_heap {
            // we won't be extending the heap, so we should return Err
            return Err(());
        }
    
        unsafe {
            // we're assuming the new memory up to HEAP_TOP_LIMIT is unused and allocatable
            talc.oom_handler.heap = talc.extend(old_heap, new_heap);
        }
    
        Ok(())
    }
}
```

## Algorithm
This is a dlmalloc-style linked list allocator with boundary tagging and binning, aimed at general-purpose use cases. Allocation is O(n) worst case (but in practice its near-constant time, see microbenchmarks), while in-place reallocations and deallocations are O(1).

Additionally, the layout of chunk metadata is rearranged to allow for smaller minimum-size chunks to reduce memory overhead of small allocations. The minimum chunk size is `3 * usize`, with a single `usize` being reserved per allocation. This is more efficient than `dlmalloc` and `galloc`, despite using a similar algorithm.

## Migrating from v4 to v5

If you're using WebAssembly, check out the [guide](https://github.com/SFBdragon/talc/blob/master/talc/README_WASM.md).

The allocator is now stable-by-default. The configurable features have changes significantly as well. See the [Features](#features) section.

Don't use `Talc::new` anymore. Use `Talck::new` (global allocators, multi-threaded allocation) or `TalcCell::new` (single-threaded allocation).

The arena management APIs (`Talc::claim`, `Talc::extend`, `Talc::reserved` (previously `get_allocated_span`), `Talc::truncate`, `Talc::resize` (new!)) changed in various ways. `Span` has been removed, but the functions should be easier to use. Please check their docs for more info.

## Changelog

#### v5.0.0-beta

Check out the [migration guide](#migrating-from-v4-to-v5) 

In general, the allocator got a lot better at doing its job. Also took the opportunity to clean up the APIs, setup, and configuration.

Here are some highlights:

- The crate is now stable-by-default, and the MSRV has _dropped_ to Rust 1.63
- The available features have changed, see [Features](#conditional-features)
- `TalcCell` introduced: safe, `!Sync`, zero-runtime-overhead implementor of `GlobalAlloc` and `Allocator`
- `Span` is gone, rest in peace. It's been sort-of-replaced by `Arena` but `Arena` has a more narrow focus.
    - `Talc`'s arena-management APIs have changed in general. Notably the base of arenas are now fixed.
- `AssumeUnlockable` is gone, good riddance. Use `TalcCellAssumeSingleThreaded` if you need something similar.
- WebAssembly-specific things are all in `talc::wasm` now. `WasmHandler` became `ExtendWasmMemOnOom`. `ClaimWasmMemOnOom` is the default though.
- Binning configuration for Talc has been added. This primarily benefitted Talc for WebAssembly.

A bunch of other things have changed.

#### v4.4.2

- [polarathene](https://github.com/polarathene): Replace README relative links with fully-qualified links.
- [polarathene](https://github.com/polarathene): Improve docs for `stable_examples/examples/std_global_allocator.rs`.

- Improved docs for `stable_examples/examples/stable_allocator_api.rs` and `stable_examples/examples/std_global_allocator.rs`.
- Deprecated the `Span::from*` function for converting from shared references and const pointers, as they make committing UB easy. These will be removed in v5.
- Fixed up a bunch of warnings all over the project.

#### v4.4.1

- Added utility function `except` to `Span`, which takes the set difference, potentially splitting the `Span`. Thanks [bjorn3](https://github.com/bjorn3) for the suggestion!

#### v4.4.0

- Added feature `allocator-api2` which allows using the `Allocator` trait on stable via the [`allocator-api2`](https://github.com/zakarumych/allocator-api2) crate. Thanks [jess-sol](https://github.com/jess-sol)!

#### v4.3.1

- Updated the README a little

#### v4.3.0

- Added an implementation for `Display` for the counters. Hopefully this makes your logs a bit prettier. 
    - Bug me if you have opinions about the current layout, I'm open to changing it.

- Added Frusa and RLSF to the benchmarks. 
    - Good showing by RLSF all around, and Frusa has particular workloads it excels at.
- Changed random actions benchmark to measure over various allocation sizes.

#### v4.2.0

- Optimized reallocation to allows other allocation operations to occur while memcopy-ing if an in-place reallocation failed.
    - As a side effect Talc now has a `grow_in_place` function that returns `Err` if growing the memory in-place isn't possible.
    - A graph of the random actions benchmark with a workload that benefits from this has been included in the [benchmarks](#benchmarks) section.

- Added `Span::from_*` and `From<>` functions for const pointers and shared references.
    - This makes creating a span in static contexts on stable much easier: `Span::from_const_array(addr_of!(MEMORY))`
- Fix: Made `Talck` derive `Debug` again.

- Contribution by [Ken Hoover](https://github.com/khoover): add Talc arena-style allocation size and perf WASM benchmarks
    - This might be a great option if you have a known dynamic memory requirement and would like to reduce your WASM size a little more.

- `wasm-size` now uses _wasm-opt_, giving more realistic size differences for users of _wasm-pack_
- Improved shell scripts
- Overhauled microbenchmarks
    - No longer simulates high-heap pressure as tolerating allocation failure is rare
    - Data is now displayed using box-and-whisker plots

#### v4.1.1

- Fix: Reset MSRV to 1.67.1 and added a check to `test.sh` for it

#### v4.1.0 (yanked, use 4.1.1)

- Added optional tracking of allocation metrics. Thanks [Ken Hoover](https://github.com/khoover) for the suggestion!
    - Enable the `"counters"` feature. Access the data via `talc.get_counters()`
    - Metrics include allocation count, bytes available, fragmentation, overhead, and more.
- Improvements to documentation
- Improved and updated benchmarks
- Integrated the WASM performance benchmark into the project. Use `wasm-bench.sh` to run (requires _wasm-pack_ and _deno_)
- Improved `wasm-size` and `wasm-size.sh`

#### v4.0.0
- Changed `Talck`'s API to be more inline with Rust norms. 
    - `Talck` now hides its internal structure (no more `.0`).
    - `Talck::talc()` has been replaced by `Talck::lock()`. 
    - `Talck::new()` and `Talck::into_inner(self)` have been added.
    - Removed `TalckRef` and implemented the `Allocator` trait on `Talck` directly. No need to call `talck.allocator()` anymore.
- Changed API for provided locking mechanism
    - Moved `AssumeUnlockable` into `talc::locking::AssumeUnlockable`
    - Removed `Talc::lock_assume_single_threaded`, use `.lock::<talc::locking::AssumeUnlockable>()` if necessary.
- Improvements to documentation here and there. Thanks [polarathene](https://github.com/polarathene) for the contribution!

#### v3.1.2
- Some improvements to documentation.

#### v3.1.1
- Changed the WASM OOM handler's behavior to be more robust if other code calls `memory.grow` during the allocator's use.

#### v3.1.0
- Reduced use of nightly-only features, and feature-gated the remainder (`Span::from(*mut [T])` and `Span::from_slice`) behind `nightly_api`.
- `nightly_api` feature is default-enabled
    - *WARNING:* use of `default-features = false` may cause unexpected errors if the gated functions are used. Consider adding `nightly_api` or using another function.

#### v3.0.1
- Improved documentation
- Improved and updated benchmarks
    - Increased the range of allocation sizes on Random Actions. (sorry Buddy Allocator!)
    - Increased the number of iterations the Heap Efficiency benchmark does to produce more accurate and stable values.

#### v3.0.0
- Added support for multiple discontinuous heaps! This required some major API changes
    - `new_arena` no longer exists (use `new` and then `claim`)
    - `init` has been replaced with `claim`
    - `claim`, `extend` and `truncate` now return the new heap extent 
    - `InitOnOom` is now `ClaimOnOom`. 
    - All of the above now have different behavior and documentation.
- Each heap now has a fixed overhead of one `usize` at the bottom.

To migrate from v2 to v3, keep in mind that you must keep track of the heaps if you want to resize them, by storing the returned `Span`s. Read [`claim`](https://docs.rs/talc/latest/talc/struct.Talc.html#method.claim), [`extend`](https://docs.rs/talc/latest/talc/struct.Talc.html#method.extend) and [`truncate`](https://docs.rs/talc/latest/talc/struct.Talc.html#method.truncate)'s documentation for all the details.

#### v2.2.1
- Rewrote the allocator internals to place allocation metadata above the allocation.
    - This will have the largest impact on avoiding false sharing, where previously, the allocation metadata for one allocation would infringe on the cache-line of the allocation before it, even if a sufficiently high alignment was demanded. Single-threaded performance marginally increased, too.
- Removed heap_exhaustion and replaced heap_efficiency benchmarks.
- Improved documentation and other resources.
- Changed the WASM size measurement to include slightly less overhead.

#### v2.2.0
- Added `dlmalloc` to the benchmarks.
- WASM should now be fully supported via `TalckWasm`. Let me know what breaks ;)
    - Find more details [here](https://github.com/SFBdragon/talc/README_WASM.md).


#### v2.1.0
- Tests are now passing on 32 bit targets.
- Documentation fixes and improvements for various items.
- Fixed using `lock_api` without `allocator`.
- Experimental WASM support has been added via `TalckWasm` on WASM targets.


#### v2.0.0
- Removed dependency on `spin` and switched to using `lock_api` (thanks [Stefan Lankes](https://github.com/stlankes))
    - You can specify the lock you want to use with `talc.lock::<spin::Mutex<()>>()` for example.
- Removed the requirement that the `Talc` struct must not be moved, and removed the `mov` function.
    - The arena is now used to store metadata, so extremely small arenas will result in allocation failure.
- Made the OOM handling system use generics and traits instead of a function pointer.
    - Use `ErrOnOom` to do what it says on the tin. `InitOnOom` is similar but inits to the given span if completely uninitialized. Implement `OomHandler` on any struct to implement your own behaviour (the OOM handler state can be accessed from `handle_oom` via `talc.oom_handler`).
- Changed the API and internals of `Span` and other changes to pass `miri`'s Stacked Borrows checks.
    - Span now uses pointers exclusively and carries provenance.
- Updated the benchmarks in a number of ways, notably adding `buddy_alloc` and removing `simple_chunk_allocator`.

