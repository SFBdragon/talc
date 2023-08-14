# Talc

![Crates.io](https://img.shields.io/crates/v/talc?style=flat-square&color=orange) ![Downloads](https://img.shields.io/crates/d/talc?style=flat-square) ![docs.rs](https://img.shields.io/docsrs/talc?style=flat-square) ![License](https://img.shields.io/crates/l/talc?style=flat-square) 

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
- [Changelog](#changelog)
- [Support Me](#support-me)

## Setup

Use it as an arena allocator via the `Allocator` API as follows:
```rust
#![feature(allocator_api)]
use talc::*;
use core::alloc::{Allocator, Layout};

static mut ARENA: [u8; 10000] = [0; 10000];

fn main () {
    let talck = unsafe {
        Talc::with_arena(ErrOnOom, ARENA.as_mut().into()).lock::<spin::Mutex<()>>()
    };
    
    talck.allocator().allocate(Layout::new::<[u32; 16]>());
}
```

Or as a global allocator:
```rust
use talc::*;

static mut ARENA: [u8; 10000] = [0; 10000];

#[global_allocator]
static ALLOCATOR: Talck<spin::Mutex<()>, InitOnOom> = Talc::new(unsafe {
    // if we're in a hosted environment, the Rust runtime may allocate before
    // main() is called, so we need to initialize the arena automatically
    InitOnOom::new(Span::from_slice(ARENA.as_slice() as *const [u8] as *mut [u8]))
}).lock();

fn main() {
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
}
```

See [General Usage](#general-usage) and [Advanced Usage](#advanced-usage) for more details.

## Benchmarks

### Macrobenchmarks (based on galloc's benchmarks)

The original benchmarks have been modified (e.g. replacing `rand` with `fastrand`) in order to alleviate the overhead. Additionally, alignment requirements are inversely exponentially frequent, ranging from 2^2 bytes to 2^18, with 2^2 and 2^3 being most common.

#### Random Actions Benchmark Results

The number of successful allocations, deallocations, and reallocations within the allotted time.

![Random Actions Benchmark Results](/benchmark_graphs/random_actions.png)

#### Heap Efficiency Benchmark Results

The average occupied capacity upon first allocation failure when randomly allocating/deallocating/reallocating.

|             Allocator | Average Random Actions Heap Efficiency |
| --------------------- | -------------------------------------- |
|              dlmalloc |                                 97.34% |
|                  talc |                                 97.12% |
| linked_list_allocator |                                 96.54% |
|                galloc |                                 94.47% |
|           buddy_alloc |                                 57.70% |


### Microbenchmarks (based on simple_chunk_allocator's benchmark)

Pre-fail allocations account for all allocations up until the first allocation failure, at which point heap pressure has become a major factor. Some allocators deal with heap pressure better than others, and many applications aren't concerned with such cases (where allocation failure results in a panic), hence they are seperated out for seperate consideration. Actual number of pre-fail allocations can be quite noisy due to random allocation sizes.

``` ignore
RESULTS OF BENCHMARK: Talc
 2221032 allocation attempts, 1564703 successful allocations,   26263 pre-fail allocations, 1553755 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       21      42      63      63      84      84     105     189   54327 |     123   ticks
Pre-Fail Allocations |       42      63      63      63      84      84     105     126    1743 |      93   ticks
       Deallocations |       21      63      84      84     105     126     231     315   21357 |     178   ticks

RESULTS OF BENCHMARK: Buddy Allocator
 2370094 allocation attempts, 1665891 successful allocations,   17228 pre-fail allocations, 1659287 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       21      42      42      42      42      63      63      63   15519 |      52   ticks
Pre-Fail Allocations |       21      42      42      42      42      63      63      63     756 |      75   ticks
       Deallocations |       42      63      63      63      63      84      84     126   16107 |      94   ticks

RESULTS OF BENCHMARK: Dlmalloc
 2176317 allocation attempts, 1531543 successful allocations,   25560 pre-fail allocations, 1520414 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42      63      84     147     168     189     210     294   19026 |     170   ticks
Pre-Fail Allocations |       42      63     105     147     147     168     189     273   16863 |     168   ticks
       Deallocations |       42     105     126     126     189     252     294     399   19509 |     240   ticks

RESULTS OF BENCHMARK: Galloc
  282268 allocation attempts,  207553 successful allocations,   23284 pre-fail allocations,  197680 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42      63      63     294   12306   26901   41748   45906  128877 |   19106   ticks
Pre-Fail Allocations |       42      42      42      42      63      63      63     630   21147 |     663   ticks
       Deallocations |       42      63      84      84     147     252     378     735   18018 |     288   ticks

RESULTS OF BENCHMARK: Linked List Allocator
  137396 allocation attempts,  107083 successful allocations,   24334 pre-fail allocations,   96915 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42    4452    9786   16296   24108   33894   45801   56763 1199415 |   28868   ticks
Pre-Fail Allocations |       42     924    2310    4032    6216    8883   12537   18039  902979 |   11427   ticks
       Deallocations |       42    3423    7224   11550   16485   22092   28833   37569   98679 |   19085   ticks
```

Why does Buddy Allocator perform much better here than in the random actions benchmark? The buddy allocator's performance is heavily dependant on the size of allocations in random actions, as it doesn't appear to reallocate efficiently. The microbenchmark results only measure allocation and deallocation, with no regard to reallocation. (The currently-used sizes of about 100 to 100000 bytes puts Talc and Buddy Allocator roughly on par, but this is just a coincidence.)

## Algorithm
This is a dlmalloc-style linked list allocator with boundary tagging and bucketing, aimed at general-purpose use cases. Allocation is O(n) worst case, while in-place reallocations and deallocations are O(1).

The main differences compared to Galloc, using a similar algorithm, is that Talc doesn't bucket by alignment at all, assuming most allocations will require at most a machine-word size alignment, so expect Galloc to be faster where lots of small, large alignment allocations are made. Instead, a much broader range of bucket sizes are used, which should often be more efficient.

Additionally, the layout of chunk metadata is rearranged to allow for smaller minimum-size chunks to reduce memory overhead of small allocations. The minimum chunk size is `3 * usize`, with a single `usize` being reserved per allocation.

## Testing
Tests on most of the helper types and Talc functions.

Other than that, lots of fuzzing of the allocator.

## General Usage

Here is the list of `Talc` methods:
* Constructors:
    * `new`
    * `with_arena`
* Information:
    * `get_arena` - returns the current arena memory region
    * `get_allocatable_span` - returns the current memory region in which allocations could occur
    * `get_allocated_span` - returns the minimum span containing all allocated memory
* Management:
    * `init` - initialize or re-initialize the arena (forgets all previous allocations, if any)
    * `extend` - extend the arena (or initialize, if uninitialized)
    * `truncate` - reduce the extent of the arena
    * `lock` - wraps the `Talc` in a `Talck`, which supports the `GlobalAlloc` and `Allocator` APIs
* Allocation:
    * `malloc`
    * `free`
    * `grow`
    * `shrink`

See their docs for more info.

`Span` is a handy little type for describing memory regions, because trying to manipulate `Range<*mut u8>` or `*mut [u8]` or `base_ptr`-`size` pairs tends to be inconvenient or annoying. See `Span::from*` and `span.to_*` functions for conversions.

## Advanced Usage

The most powerful feature of the allocator is that it has a modular OOM handling system, allowing you to perform any actions, including directly on the allocator or reporting the offending allocation, allowing you to fail out of or recover from allocation failure easily. As an example, recovering my extending the arena is implemented below.

```rust
use talc::*;

struct MyOomHandler;

impl OomHandler for MyOomHandler {
    fn handle_oom(talc: &mut Talc<Self>, layout: core::alloc::Layout) -> Result<(), ()> {
        // alloc doesn't have enough memory, and we just got called! we must free up some memory
        // we'll go through an example of how to handle this situation
    
        // we can inspect `layout` to estimate how much we should free up for this allocation
        // or we can extend by any amount (increasing powers of two has good time complexity)
    
        // this function will be repeatly called until we free up enough memory or 
        // we return Err(()) causing allocation failure. Be careful to avoid conditions where 
        // the arena isn't sufficiently extended indefinitely, causing an infinite loop
    
        // an arbitrary address limit for the sake of example
        const ARENA_TOP_LIMIT: *mut u8 = 0x80000000 as *mut u8;
    
        let old_arena: Span = talc.get_arena();
    
        // we're going to extend the arena upward, doubling its size
        // but we'll be sure not to extend past the limit
        let new_arena: Span = old_arena.extend(0, old_arena.size()).below(ARENA_TOP_LIMIT);
    
        if new_arena == old_arena {
            // we won't be extending the arena, so we should return Err
            return Err(());
        }
    
        unsafe {
            // we're assuming the new memory up to ARENA_TOP_LIMIT is allocatable
            talc.extend(new_arena);
        };
    
        Ok(())
    }
}
```

## Conditional Features
* `lock_api` (default): Provides the `Talck` locking wrapper type that implements `GlobalAlloc`.
* `allocator` (default): Provides an `Allocator` trait implementation via `Talck`.

## Changelog

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


## Support Me
I'm looking for part-time programming work for which South Africans are eligible. If you know of any suitable vacancies, please get in touch. [Here's my LinkedIn.](https://www.linkedin.com/in/shaun-beautement-9101a823b/)

On the other hand, if you find the project useful, please consider donating via [Paypal](https://www.paypal.com/donate/?hosted_button_id=8CSQ92VV58VPQ). Thanks!
