# Talc

![License](https://img.shields.io/crates/l/talc?style=flat-square) ![Downloads](https://img.shields.io/crates/d/talc?style=flat-square) ![docs.rs](https://img.shields.io/docsrs/talc?style=flat-square)


Talc is a performant and flexible `no_std`-compatible memory allocator suitable for projects such as operating system kernels, or arena allocation for normal single-threaded apps.

Using Talc as a simple arena allocator is easy, but practical concerns in `no_std` environments are facilitated too, such as custom OOM handling, as well as powerful features like extending and reducing the allocation arena dynamically.

## Usage

Use it as a global allocator as follows:
```rust ignore
use talc::*;

#[global_allocator]
static ALLOCATOR: Talck = Talc::new().spin_lock();
static mut ARENA: [u8; 1000] = [0; 1000];

fn main() {
    // initialize it later...
    unsafe { ALLOCATOR.talc().init(ARENA.as_mut_slice().into()); }
}
```

Use it as an arena allocator via the `Allocator` API as follows:
```rust ignore
use talc::*;

fn main () {
    let mut arena = Box::leak(vec![0u8; 10000].into_boxed_slice());
    
    let talck = Talc::new().spin_lock();
    unsafe { talck.talc().init(arena.into()); }

    let allocator = talck.allocator_api_ref();
    
    allocator.allocate(..);
}
```

## Performance
O(n) worst case allocations. In practice, it's usually very fast, compared to other allocators. See the benchmarks below.

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

#### Heap Exhaustion Benchmark Results

The number of allocation when filling and flushing the heap with a penalty for each cycle.

![Heap Exhaustion Benchmark Results](/benchmark_graphs/heap_exhaustion.png)

#### Heap Efficiency Benchmark Results

The average occupied capacity once filled with random allocations.

```
             ALLOCATOR | HEAP EFFICIENCY
-----------------------|----------------
                  talc | 99.82%
                galloc | 99.82%
           buddy_alloc | 59.45%
 linked_list_allocator | 99.82%
```

Note that:
- no attempt is made to account for interrupts in these timings, however, the results are fairly consistent on my computer.
- alignment requirements are inversely exponentially frequent, ranging from 2^2 bytes to 2^18, with 2^2 and 2^3 being most common

### Microbenchmarks (based on simple_chunk_allocator's benchmark)

Note: pre-fail allocations account for all allocations up until the first allocation failure, at which point heap pressure has become a major factor. Some allocators deal with heap pressure better than others, and many applications aren't concerned with such cases (where allocation failure results in a panic), hence they are seperated out for seperate consideration.

```
RESULTS OF BENCHMARK: Talc
 2206572 allocation attempts, 1557742 successful allocations,   25885 pre-fail allocations, 1547084 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42      63      63      84      84     105     105     210   32298 |     132   ticks
Pre-Fail Allocations |       42      63      63      84      84      84     105     126    1890 |     100   ticks
       Deallocations |       42      84      84     105     105     210     252     357   23037 |     191   ticks

RESULTS OF BENCHMARK: Buddy Allocator
 2367839 allocation attempts, 1662802 successful allocations,   20721 pre-fail allocations, 1656207 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       21      42      42      42      42      63      63      63   15246 |      51   ticks
Pre-Fail Allocations |       21      42      42      42      42      63      63      63  317478 |      72   ticks
       Deallocations |       42      63      63      63      63      84      84     126   15918 |      98   ticks

RESULTS OF BENCHMARK: Galloc
  275165 allocation attempts,  200172 successful allocations,   25253 pre-fail allocations,  189938 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42      63      63     378   12642   28077   42525   47355  104622 |   19699   ticks
Pre-Fail Allocations |       42      42      42      42      63      63      63    1365   23079 |     879   ticks
       Deallocations |       42      63      84      84     105     231     315     714   14847 |     272   ticks

RESULTS OF BENCHMARK: Linked List Allocator
  136860 allocation attempts,  106338 successful allocations,   26098 pre-fail allocations,   96117 deallocations
            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE
---------------------|--------------------------------------------------------------------------|---------
     All Allocations |       42    3654    9072   15603   23772   34356   47397   59451  661773 |   29312   ticks
Pre-Fail Allocations |       42     588    1743    3402    5607    8358   11865   17115  670194 |   10440   ticks
       Deallocations |       42    2625    6300   10626   15582   21462   28686   38304   84231 |   18772   ticks
```

Note that:
- no attempt is made to account for interrupts in these timings, however, the results are fairly consistent on my computer.
- number of pre-fail allocations is more noise than signal due to random allocation sizes
- alignment requirements are inversely exponentially frequent, ranging from 2^2 bytes to 2^18, with 2^2 and 2^3 being most common



## Algorithms
This is a dlmalloc-style implementation with boundary tagging and bucketing aimed at general-purpose use cases.

The main differences compared to Galloc, using a similar algorithm, is that Talc doesn't bucket by alignment at all, assuming most allocations will require at most a machine-word size alignment, so expect Galloc to be faster where lots of small, large alignment allocations are made. Instead, a much broader range of bucket sizes are used, which should often be more efficient.

Additionally, the layout of chunk metadata is rearranged to allow for smaller minimum-size chunks to reduce memory overhead of small allocations.

## Testing
Test coverage on most of the helper types and some sanity checking on the allocation.

Other than that, lots of fuzzing of the allocator. See `/fuzz/fuzz_targets/fuzz_arena.rs`

## Features
* `spin` (default): Provides the `Talck` type (a spin mutex wrapper) that implements `GlobalAlloc`.
* `allocator` (default): Provides an `Allocator` trait implementation via `Talck`.

## General Usage

Here is the list of methods:
* Constructors:
    * `new`
    * `with_oom_handler`
* Information:
    * `get_arena` - returns the current arena memory region
    * `get_allocatable_span` - returns the current memory region in which allocations could occur
    * `get_allocated_span` - returns the minimum span containing all allocated memory
* Management:
    * `mov` - safely move an initialized Talc to the specified destination
    * `init` - initialize or re-initialize the arena (forgets all previous allocations, if any)
    * `extend` - initialize or extend the arena region
    * `truncate` - reduce the extent of the arena region
    * `spin_lock` - wraps the Talc in a talc, which supports the `GlobalAlloc` and `Allocator` APIs
* Allocation:
    * `malloc`
    * `free`
    * `grow`
    * `shrink`

See their docs for more info.

`Span` is a handy little type for describing memory regions, because trying to manipulate `Range<*mut u8>` or `*mut [u8]` or `base_ptr`-`size` pairs tends to be inconvenient or annoying. See `Span::from*` and `span.to_*` functions for conversions.

## Advanced Usage

Instead of using `Talc::new`, use `Talc::with_oom_handler` and pass in a function pointer. This function will now be called upon OOM. This can be useful for a number of reasons, but one possiblity is dynamically extending the arena as required.

```rust
#![feature(allocator_api)]
use talc::*;
use core::alloc::Layout;

fn oom_handler(talc: &mut Talc, layout: Layout) -> Result<(), AllocError> {
    // alloc doesn't have enough memory, and we just got called! we must free up some memory
    // we'll go through an example of how to handle this situation

    // we can inspect `layout` to estimate how much we should free up for this allocation
    // or we can extend by any amount (increasing powers of two has good time complexity)

    // this function will be repeatly called until we free up enough memory or 
    // we return Err(AllocError) at which point the allocation will too.
    // be careful to avoid conditions where the arena isn't sufficiently extended
    // indefinitely, causing an infinite loop

    // some limit for the sake of example
    const ARENA_TOP_LIMIT: *mut u8 = 0x80000000 as *mut u8;

    let old_arena: Span = talc.get_arena();

    // we're going to extend the arena upward, doubling its size
    // but we'll be sure not to extend past the limit
    let new_arena: Span = old_arena.extend(0, old_arena.size()).below(ARENA_TOP_LIMIT);

    if new_arena == old_arena {
        // we won't be extending the arena, so we should return AllocError
        return Err(AllocError);
    }

    unsafe {
        // we're assuming the new memory up to ARENA_TOP_LIMIT is allocatable
        talc.extend(new_arena);
    };

    Ok(())
}
```

## Caveat for consideration - multithreaded performance

I don't know why, but Talc gets hit harder by extreme multithreaded contention than Galloc does. While this is not so relevant to most use cases, I'd like to resolve this issue if possible. If anyone has some tips for alleviating this issue, please open an issue!

I've tried aligning the bucket array to be on seperate cache lines, but no change in result. Other than that, I don't know what to try.

![2-Thread Random Actions Benchmark Results](/benchmark_graphs/random_actions_multi.png)

