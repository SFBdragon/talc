# Talc

![Crates.io](https://img.shields.io/crates/v/talc?style=flat-square&color=orange) ![docs.rs](https://img.shields.io/docsrs/talc?style=flat-square) ![Downloads](https://img.shields.io/crates/d/talc?style=flat-square) ![License](https://img.shields.io/crates/l/talc?style=flat-square) 

Talc is a performant and flexible `no_std`-compatible memory allocator suitable for projects such as operating system kernels, or arena allocation for normal single-threaded apps. 

Practical concerns in `no_std` environments are facilitated, such as custom OOM handling, as well as powerful features like extending and reducing the allocation arena dynamically.

## Usage

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

It can be used as a global allocator as follows:
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


## Performance
O(n) worst case allocations. In practice, it's usually fast. See the benchmarks below.

Deallocation is always O(1), reallocation is usually O(1) unless in-place allocation fails.

## Memory Overhead
Allocations have a overhead of one `usize` each, typically. The chunk size is at minumum `3 * usize`, so tiny allocations will have a lot of overhead.

This improves on Galloc (another boundary-tagging allocator), which has a minimum chunk size of `4 * usize`.

## Benchmarks

### Macrobenchmarks (based on galloc's benchmarks)

The original benchmarks have been modified (e.g. replacing `rand` with `fastrand`) in order to alleviate the overhead.

#### Random Actions Benchmark Results

The number of successful allocations, deallocations, and reallocations within the allotted time.

![Random Actions Benchmark Results](/benchmark_graphs/random_actions.png)

#### Heap Efficiency Benchmark Results

The average occupied capacity once filled with random allocations.

``` ignore
             ALLOCATOR | HEAP EFFICIENCY
-----------------------|----------------
                  talc | 99.82%
                galloc | 99.82%
           buddy_alloc | 59.45%
 linked_list_allocator | 99.82%
```

#### Heap Exhaustion Benchmark Results

The number of allocation when filling and flushing the heap with a penalty for each cycle.

![Heap Exhaustion Benchmark Results](/benchmark_graphs/heap_exhaustion.png)

Notes:
- alignment requirements are inversely exponentially frequent, ranging from 2^2 bytes to 2^18, with 2^2 and 2^3 being most common

### Microbenchmarks (based on simple_chunk_allocator's benchmark)

Pre-fail allocations account for all allocations up until the first allocation failure, at which point heap pressure has become a major factor. Some allocators deal with heap pressure better than others, and many applications aren't concerned with such cases (where allocation failure results in a panic), hence they are seperated out for seperate consideration.

``` ignore
RESULTS OF BENCHMARK: Talc
 2035430 allocation attempts, 1437720 successful allocations,   25718 pre-fail allocations, 1427160 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42      42      63      84     105     105     147     273   65205 |     193   ticks
Pre-Fail Allocations |       42      84      84     105     105     126     147     315    9030 |     291   ticks
       Deallocations |       42     147     168     231     294     357     441     567   28308 |     348   ticks

RESULTS OF BENCHMARK: Buddy Allocator
 2318380 allocation attempts, 1632315 successful allocations,   17755 pre-fail allocations, 1625750 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       21      42      42      42      42      63      63      63   18837 |      57   ticks
Pre-Fail Allocations |       21      42      42      42      42      63      63     168   12621 |     256   ticks
       Deallocations |       42      84      84      84     105     105     105     210   17472 |     133   ticks

RESULTS OF BENCHMARK: Galloc
  107633 allocation attempts,   85752 successful allocations,   25069 pre-fail allocations,   76048 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42      84     189    1911    7602   51303  114618  162645  276843 |   54936   ticks
Pre-Fail Allocations |       42      63      63     273    1638    1785    2058    3318   52101 |    2247   ticks
       Deallocations |       42     147     273     336     420     483     567     798   31437 |     474   ticks

RESULTS OF BENCHMARK: Linked List Allocator
   60976 allocation attempts,   52372 successful allocations,   25858 pre-fail allocations,   42917 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42    3654   10626   22092   41286   71253  115773  167055  261576 |   66885   ticks
Pre-Fail Allocations |       42    1575    3864    7476   12369   19383   31857   55839  163212 |   23543   ticks
       Deallocations |       42    1995    6993   15183   27825   47124   75537  114135  214305 |   46387   ticks
```

Notes:
- number of pre-fail allocations is more noise than signal due to random allocation sizes
- alignment requirements are inversely exponentially frequent, ranging from 2^2 bytes to 2^18, with 2^2 and 2^3 being most common


## Algorithm
This is a dlmalloc-style linked list allocator with boundary tagging and bucketing, aimed at general-purpose use cases.

The main differences compared to Galloc, using a similar algorithm, is that Talc doesn't bucket by alignment at all, assuming most allocations will require at most a machine-word size alignment, so expect Galloc to be faster where lots of small, large alignment allocations are made. Instead, a much broader range of bucket sizes are used, which should often be more efficient.

Additionally, the layout of chunk metadata is rearranged to allow for smaller minimum-size chunks to reduce memory overhead of small allocations.

## Testing
Tests on most of the helper types and Talc functions.

Other than that, lots of fuzzing of the allocator.

## Features
* `lock_api` (default): Provides the `Talck` locking wrapper type that implements `GlobalAlloc`.
* `allocator` (default): Provides an `Allocator` trait implementation via `Talck`.

## General Usage

Here is the list of methods:
* Constructors:
    * `new`
    * `with_arena`
* Information:
    * `get_arena` - returns the current arena memory region
    * `get_allocatable_span` - returns the current memory region in which allocations could occur
    * `get_allocated_span` - returns the minimum span containing all allocated memory
* Management:
    * `init` - initialize or re-initialize the arena (forgets all previous allocations, if any)
    * `extend` - extend the arena or initialize, if uninitialized
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

## Support Me
This'll go towards keeping me alive, getting me through university, and allowing me to keep working on my OSS projects. 

[![Paypal](/donate.png)](https://www.paypal.com/donate/?hosted_button_id=8CSQ92VV58VPQ)

Appreciate it! 
