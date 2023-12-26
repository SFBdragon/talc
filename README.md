# Talc

[![Crates.io](https://img.shields.io/crates/v/talc?style=flat-square&color=orange)](https://crates.io/crates/talc) ![Downloads](https://img.shields.io/crates/d/talc?style=flat-square) [![docs.rs](https://img.shields.io/docsrs/talc?style=flat-square)](https://docs.rs/talc/latest/talc/) [![License](https://img.shields.io/crates/l/talc?style=flat-square)](https://github.com/SFBdragon/talc/blob/master/LICENSE.md)

Talc is a performant and flexible memory allocator, with first class support for `no_std` and WebAssembly. It's suitable for projects such as operating system kernels, website backends, or arena allocation in single-threaded contexts.

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

Use it as an arena allocator via the `Allocator` API as follows:
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

Note that both of these examples use the `spin` crate's mutex as a locking mechanism. Any lock implementing `lock_api` will do, though.

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
|                  talc |                                 98.87% |
| linked_list_allocator |                                 98.28% |
|                galloc |                                 95.86% |
|           buddy_alloc |                                 58.75% |


### Microbenchmarks (based on simple_chunk_allocator's benchmark)

Pre-fail allocations account for all allocations up until the first allocation failure, at which point heap pressure has become a major factor. Some allocators deal with heap pressure better than others, and many applications aren't concerned with such cases (where allocation failure results in a panic), hence they are separated out for separate consideration. Actual number of pre-fail allocations can be quite noisy due to random allocation sizes.

``` ignore
RESULTS OF BENCHMARK: Talc
 2011833 allocation attempts, 1419683 successful allocations,   26972 pre-fail allocations, 1408883 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42      42      63      84      84     105     126     189   48468 |     133   ticks
Pre-Fail Allocations |       42      63      63      84      84     105     105     126    6489 |     102   ticks
       Deallocations |       42      84     105     105     189     252     273     399   31899 |     228   ticks

RESULTS OF BENCHMARK: Buddy Allocator
 2201551 allocation attempts, 1543457 successful allocations,   16227 pre-fail allocations, 1536871 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       21      42      42      63      63      63      63      63   21693 |      57   ticks
Pre-Fail Allocations |       21      42      42      42      63      63      63      84    4578 |      77   ticks
       Deallocations |       42      63      63      63      63      84      84     126   18795 |      99   ticks

RESULTS OF BENCHMARK: Dlmalloc
 1993087 allocation attempts, 1404059 successful allocations,   23911 pre-fail allocations, 1392832 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42      63      84     147     168     189     231     315   26166 |     181   ticks
Pre-Fail Allocations |       42      63     105     147     168     189     210     273    1218 |     172   ticks
       Deallocations |       42     105     126     147     231     273     336     420   45507 |     257   ticks

RESULTS OF BENCHMARK: Galloc
  276978 allocation attempts,  203844 successful allocations,   24233 pre-fail allocations,  193851 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42      63      84     294   12201   26859   41937   46116  127512 |   19259   ticks
Pre-Fail Allocations |       42      42      42      63      63      63      63     735   35007 |     663   ticks
       Deallocations |       42      63      84     210     231     294     399     651   19635 |     324   ticks

RESULTS OF BENCHMARK: Linked List Allocator
  134333 allocation attempts,  103699 successful allocations,   24836 pre-fail allocations,   93275 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42    4242    9723   16359   24633   35448   48027   59094 1060941 |   29863   ticks
Pre-Fail Allocations |       42     798    2205    3969    6216    9051   12747   18375 1126293 |   11534   ticks
       Deallocations |       42    3171    6972   11319   16254   22029   29211   38661  100044 |   19274   ticks
```

Q: Why does Buddy Allocator perform much better here than in the random actions benchmark? 

A: The buddy allocator's performance is heavily dependant on the size of allocations in random actions, as it doesn't appear to reallocate efficiently. The microbenchmark results only measure allocation and deallocation, with no regard to reallocation. (The currently-used sizes of 1 to 20000 bytes leads to the results above in Random Actions.)

## Algorithm
This is a dlmalloc-style linked list allocator with boundary tagging and bucketing, aimed at general-purpose use cases. Allocation is O(n) worst case, while in-place reallocations and deallocations are O(1). In practice, it's speedy.

The main algorithmic difference between Talc and Galloc, using a similar algorithm, is that Talc doesn't bucket by alignment at all, assuming most allocations will require at most a machine-word size alignment. Instead, a much broader range of bucket sizes are used, which should often be more efficient.

Additionally, the layout of chunk metadata is rearranged to allow for smaller minimum-size chunks to reduce memory overhead of small allocations. The minimum chunk size is `3 * usize`, with a single `usize` being reserved per allocation.

## Testing
Tests on most of the helper types and Talc functions.

Other than that, lots of fuzzing of the allocator.

## General Usage

Here is the list of `Talc` methods:
* Constructors:
    * `new`
* Information:
    * `get_allocated_span` - returns the minimum span containing all allocated memory
* Management:
    * `claim` - claim memory to establishing a new heap
    * `extend` - extend the extent of a heap
    * `truncate` - reduce the extent of a heap
    * `lock` - wraps the `Talc` in a `Talck`, which supports the `GlobalAlloc` and `Allocator` APIs
* Allocation:
    * `malloc`
    * `free`
    * `grow`
    * `shrink`

Read their [documentation](https://docs.rs/talc/latest/talc/struct.Talc.html) for more info.

[`Span`](https://docs.rs/talc/latest/talc/struct.Span.html) is a handy little type for describing memory regions, because trying to manipulate `Range<*mut u8>` or `*mut [u8]` or `base_ptr`-`size` pairs tends to be inconvenient or annoying.

## Advanced Usage

The most powerful feature of the allocator is that it has a modular OOM handling system, allowing you to fail out of or recover from allocation failure easily. 

As an example, recovering by extending the heap is implemented below.

```rust
use talc::*;

struct MyOomHandler {
    heap: Span,
}

impl OomHandler for MyOomHandler {
    fn handle_oom(talc: &mut Talc<Self>, layout: core::alloc::Layout) -> Result<(), ()> {
        // alloc doesn't have enough memory, and we just got called! we must free up some memory
        // we'll go through an example of how to handle this situation
    
        // we can inspect `layout` to estimate how much we should free up for this allocation
        // or we can extend by any amount (increasing powers of two has good time complexity)
        // creating another heap would also work, but this isn't covered here
    
        // this function will be repeatedly called until we free up enough memory or 
        // we return Err(()) causing allocation failure. Be careful to avoid conditions where 
        // the heap isn't sufficiently extended indefinitely, causing an infinite loop
    
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
            // we're assuming the new memory up to HEAP_TOP_LIMIT is allocatable
            talc.oom_handler.heap = talc.extend(old_heap, new_heap);
        }
    
        Ok(())
    }
}
```

## Conditional Features
* `lock_api` (default): Provides the `Talck` locking wrapper type that implements `GlobalAlloc`.
* `allocator` (default, requires nightly): Provides an `Allocator` trait implementation via `Talck`.
* `nightly_api` (default, requires nightly): Provides the `Span::from(*mut [T])` and `Span::from_slice` functions.

## Stable Rust and MSRV
Talc can be built on stable Rust by using `--no-default-features --features=lock_api` (`lock_api` isn't strictly necessary). 

Disabling `nightly_api` makes `Span::from(*mut [T])` and `Span::from_slice` unavailable. See the [`std_global_allocator` example](examples/std_global_allocator.rs) for how to get around this restriction in certain contexts.

The MSRV is currently 1.67.1

## Support Me
If you find the project useful, please consider donating via [Paypal](https://www.paypal.com/donate/?hosted_button_id=8CSQ92VV58VPQ). Thanks!

On the other hand, I'm looking for part-time programming work for which South Africans are eligible. If you know of any suitable vacancies, please get in touch. [Here's my LinkedIn.](https://www.linkedin.com/in/shaun-beautement-9101a823b/)

## Changelog

#### v4.0.0
- Changed `Talck`'s API to be more inline with Rust norms. 
    - `Talck` now hides its internal structure (no more `.0`).
    - `Talck::talc()` has been replaced by `Talck::lock()`. 
    - `Talck::new()` and `Talck::into_inner(self)` have been added.
    - Removed `TalckRef` and implemented the `Allocator` trait on `Talck` directly. No need to call `talck.allocator()` anymore.
- Changed API for provided locking mechanism
    - Moved `AssumeUnlockable` into `talc::locking::AssumeUnlockable`
    - Removed `Talc::lock_assume_single_threaded`, use `.lock::<talc::locking::AssumeUnlockable>()` directly instead.
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
    - This will have the largest impact on avoiding false sharing, where previously, the allocation metadata for one allocation would infringe on the cache-line of the allocation before it, even if a sufficiently high alignment was demanded. A marginal/negligible increase in single-threaded performance resulted, too.
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

