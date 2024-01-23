# Talc

[![Crates.io](https://img.shields.io/crates/v/talc?style=flat-square&color=orange)](https://crates.io/crates/talc) ![Downloads](https://img.shields.io/crates/d/talc?style=flat-square) [![docs.rs](https://img.shields.io/docsrs/talc?style=flat-square)](https://docs.rs/talc/latest/talc/) [![License](https://img.shields.io/crates/l/talc?style=flat-square)](https://github.com/SFBdragon/talc/blob/master/LICENSE.md)

Talc is a performant and flexible memory allocator, with first class support for **`no_std`** and **WebAssembly**. It's suitable for projects such as operating system kernels, website backends, or arena allocation in single-threaded contexts.

Is your project targeting WASM? Check out [usage and comparisons here](./README_WASM.md).

### Table of Contents
- [Setup](#setup)
- [Benchmarks](#benchmarks)
- [Algorithm](#algorithm)
- [Testing](#testing)
- [General Usage](#general-usage)
- [Advanced Usage](#advanced-usage)
- [Conditional Features](#conditional-features)
- [Stable Rust and MSRV](#stable-rust-and-msrv)
- [Support Me](#support-me)
- [Changelog](#changelog)

## Setup

Use it as an arena allocator via the `Allocator` API with `spin` as follows:
```rust
#![feature(allocator_api)]
use talc::*;
use core::alloc::{Allocator, Layout};

static mut ARENA: [u8; 10000] = [0; 10000];

fn main () {
    let talck = Talc::new(ErrOnOom).lock::<spin::Mutex<()>>();
    unsafe { talck.lock().claim(ARENA.as_mut().into()); }
    
    talck.allocate(Layout::new::<[u32; 16]>());
}
```

Or as a global allocator:
```rust
#![feature(const_mut_refs)]
use talc::*;

static mut ARENA: [u8; 10000] = [0; 10000];

#[global_allocator]
static ALLOCATOR: Talck<spin::Mutex<()>, ClaimOnOom> = Talc::new(unsafe {
    // if we're in a hosted environment, the Rust runtime may allocate before
    // main() is called, so we need to initialize the arena automatically
    ClaimOnOom::new(Span::from_array(&mut ARENA))
}).lock();

fn main() {
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
}
```

Note that any lock implementing `lock_api` can be used.

See [the `std_global_allocator` example](/examples/std_global_allocator.rs), [General Usage](#general-usage) and [Advanced Usage](#advanced-usage) for more details.

## Benchmarks

### Macrobenchmarks (based on galloc's benchmarks)

The original benchmarks have been modified (e.g. replacing `rand` with `fastrand`) in order to alleviate the overhead. Additionally, alignment requirements are inversely exponentially frequent, ranging from 2^2 bytes to 2^18, with 2^2 and 2^3 being most common.

#### Random Actions Benchmark Results

The number of successful allocations, deallocations, and reallocations within the allotted time.

![Random Actions Benchmark Results](/benchmark_graphs/random_actions.png)

Note that these results are sensitive to the allocation sizes, ratio of allocations to deallocations, and other such factors.

#### Heap Efficiency Benchmark Results

The average occupied capacity upon first allocation failure when randomly allocating/deallocating/reallocating.

|             Allocator | Average Random Actions Heap Efficiency |
| --------------------- | -------------------------------------- |
|              dlmalloc |                                 99.07% |
|              **talc** |                                 98.87% |
| linked_list_allocator |                                 98.28% |
|                galloc |                                 95.86% |
|           buddy_alloc |                                 58.75% |


### Microbenchmarks (based on simple_chunk_allocator's benchmark)

Pre-fail allocations account for all allocations up until the first allocation failure, at which point heap pressure has become a major factor. Some allocators deal with heap pressure better than others, and many applications aren't concerned with such cases (where allocation failure results in a panic), hence they are separated out for separate consideration. Actual number of pre-fail allocations can be quite noisy due to random allocation sizes.

``` ignore
RESULTS OF BENCHMARK: Talc
 1919774 allocation attempts, 1556400 successful allocations,   74575 pre-fail allocations, 1537392 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
       Normal Allocs |       42      63      63      84      84     105     105     126    7812 |     116   (ticks)
High-Pressure Allocs |       42      63      84      84     105     105     126     294   49980 |     183   (ticks)
            Deallocs |       42      84     126     210     252     294     399     462   32235 |     289   (ticks)

RESULTS OF BENCHMARK: Buddy Allocator
 2201546 allocation attempts, 1769552 successful allocations,   53684 pre-fail allocations, 1757592 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
       Normal Allocs |       21      42      42      42      63      63      63      63    5250 |      60   (ticks)
High-Pressure Allocs |       21      42      42      63      63      63      63      63   20181 |      58   (ticks)
            Deallocs |       42      63      63      63      84      84     105     147   67158 |     103   (ticks)

RESULTS OF BENCHMARK: Dlmalloc
 1935460 allocation attempts, 1564035 successful allocations,   78340 pre-fail allocations, 1544000 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
       Normal Allocs |       42      84     126     147     168     189     210     315   17115 |     187   (ticks)
High-Pressure Allocs |       42      84     126     168     189     189     231     315   57687 |     192   (ticks)
            Deallocs |       42     126     147     231     273     357     399     462   65394 |     307   (ticks)

RESULTS OF BENCHMARK: Galloc
  201507 allocation attempts,  179724 successful allocations,   76734 pre-fail allocations,  162520 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
       Normal Allocs |       42      42      42      42      63      63      84    3045   66255 |    1784   (ticks)
High-Pressure Allocs |       42      63      84     315   23415   50694   87570  114933  367752 |   43719   (ticks)
            Deallocs |       42      63      84     126     231     273     399     672   66024 |     312   (ticks)

RESULTS OF BENCHMARK: Linked List Allocator
  105312 allocation attempts,  101486 successful allocations,   78507 pre-fail allocations,   84896 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
       Normal Allocs |       42    1386    3864    7098   11319   16905   25494   42882  739809 |   20006   (ticks)
High-Pressure Allocs |       63   13356   28938   47439   68985   93555  121443  148491  203007 |   76488   (ticks)
            Deallocs |       42    1701    4620    8589   13860   21609   35133   64554  210252 |   26608   (ticks)
```

\* The reason Buddy Allocator appears so much better here than in Random Actions is that reallocation effeciency is not measured at all.

## Algorithm
This is a dlmalloc-style linked list allocator with boundary tagging and bucketing, aimed at general-purpose use cases. Allocation is O(n) worst case, while in-place reallocations and deallocations are O(1). In practice, it's speedy.

The main algorithmic difference between Talc and Galloc, using a similar algorithm, is that Talc doesn't bucket by alignment at all, assuming most allocations will require at most a machine-word size alignment. Instead, a much broader range of bucket sizes are used, which should often be more efficient.

Additionally, the layout of chunk metadata is rearranged to allow for smaller minimum-size chunks to reduce memory overhead of small allocations. The minimum chunk size is `3 * usize`, with a single `usize` being reserved per allocation.

## Testing
Tests on most of the helper types and Talc functions.

Other than that, lots of fuzzing of the allocator.

## General Usage

Here is the list of important `Talc` methods:
* Constructors:
    * `new`
* Information:
    * `get_allocated_span` - returns the minimum heap span containing all allocated memory in an established heap
* Management:
    * `claim` - claim memory to establishing a new heap
    * `extend` - extend an established heap
    * `truncate` - reduce the extent of an established heap
    * `lock` - wraps the `Talc` in a `Talck`, which supports the `GlobalAlloc` and `Allocator` APIs
* Allocation:
    * `malloc`
    * `free`
    * `grow`
    * `shrink`

Read their [documentation](https://docs.rs/talc/latest/talc/struct.Talc.html) for more info.

[`Span`](https://docs.rs/talc/latest/talc/struct.Span.html) is a handy little type for describing memory regions, as trying to manipulate `Range<*mut u8>` or `*mut [u8]` or `base_ptr`-`size` pairs tends to be inconvenient or annoying.

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

## Conditional Features
* `"lock_api"` (default): Provides the `Talck` locking wrapper type that implements `GlobalAlloc`.
* `"allocator"` (default, requires nightly): Provides an `Allocator` trait implementation via `Talck`.
* `"nightly_api"` (default, requires nightly): Provides the `Span::from(*mut [T])` and `Span::from_slice` functions.
* `"counters"`: `Talc` will track heap and allocation metrics. Use `Talc::get_counters` to access them.

## Stable Rust and MSRV
Talc can be built on stable Rust by disabling `"allocator"` and `"nightly_api"`. The MSRV is 1.67.1.

Disabling `"nightly_api"` makes `Span::from(*mut [T])` and `Span::from_slice` unavailable. See the [`std_global_allocator` example](examples/std_global_allocator.rs) for how to get around this restriction in certain contexts.


## Support Me
If you find the project useful, please consider donating: [Paypal](https://www.paypal.com/donate/?hosted_button_id=8CSQ92VV58VPQ)

## Changelog

#### v4.1.0

- Added optional tracking of allocation metrics. Thanks [Ken Hoover](https://github.com/khoover) for the suggestion!
    - Enable the `"counters"` feature. Access the data via `talc.get_counters()`
    - Metrics include allocation count, bytes available, fragmentation, overhead, and [more](https://docs.rs/talc/latest/talc/struct.Span.html)
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
    - Find more details [here](./README_WASM.md).


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

