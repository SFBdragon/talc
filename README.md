# Talloc
_The TauOS Allocator_

Talloc is a performant and flexible `no_std`-compatible memory allocator suitable for projects such as operating system kernels, or arena allocation for normal single-threaded apps.

Using Talloc as a simple arena allocator is easy, but practical concerns in `no_std` environments are facilitated too, such as custom OOM handling, as well as powerful features like extending and reducing the allocation arena dynamically.

## Usage

Use it as a global allocator as follows:
```rust
use talloc::*;

#[global_allocator]
static ALLOCATOR: Tallock = Talloc:::new().spin_lock();

// initialize it later...
let arena = Span::from(0x100000..0x10000000);
unsafe { ALLOCATOR.0.lock().init(arena)); }
```

Use it as an arena allocator via the `Allocator` API like so:
```rust
let mut arena = vec![0u8; SIZE];

let tallock = Talloc::new().spin_lock();
tallock.0.lock().init(arena.deref_mut().into());
let allocator = tallock.allocator_api_ref();

allocator.allocate(...);
```

## Performance
O(n) worst case allocations. In practice, it's usually very fast, compared to other allocators. See the benchmarks below.

Deallocation is always O(1), reallocation is usually O(1) unless in-place allocation fails.

## Memory Overhead
Allocations have a overhead of one `usize` each, typically. The chunk size is at minumum `3 * usize`, so tiny allocations will have a lot of overhead.

This improves on Galloc (another boundary-tagging allocator), which has a minimum chunk size of `4 * usize`.

## Benchmarks

### galloc's benchmarks:

The original benchmarks have been modified (e.g. replacing `rand` with `fastrand`) in order to alleviate the overhead.

#### Random Actions Benchmark Results

![Random Actions Benchmark Results](/benchmark_graphs/random_actions.png)

Talloc outperforms the alternatives. 

#### Heap Exhaustion Benchmark Results

![Heap Exhaustion Benchmark Results](/benchmark_graphs/heap_exhaustion.png)

Talloc falls slightly behind if the time penalization is set right.

Note that:
- no attempt is made to account for interrupts in these timings, however, the results are fairly consistent on my computer.
- alignment requirements are inversely exponentially frequent, ranging from 2^2 bytes to 2^18, with 2^2 and 2^3 being most common

### Allocator Microbenchmarks (based on simple_chunk_allocator's benchmark)

Note: pre-fail allocations account for all allocations up until the first allocation failure, at which point heap pressure has become a major factor. Some allocators deal with heap pressure better than others, and many applications aren't concerned with such cases (where allocation failure results in a panic), hence they are seperated out for seperate consideration.

```rust
RESULTS OF BENCHMARK: Chunk Allocator
   25714 allocation attempts,   25535 successful allocations,   22479 pre-fail allocations,   18067 deallocations
            CATEGORY | OCTILE 0     1     2     3     4     5     6      7        8 | AVERAGE
---------------------|--------------------------------------------------------------|---------
     All Allocations |       63  1176  1659  2058  2457  2940  4179  37569 18199587 |  240918  ticks
Pre-Fail Allocations |       84  1134  1596  1953  2289  2688  3234   8400  1562757 |   15753  ticks
       Deallocations |       42   147   231   315   420   504   588    672     1932 |     420  ticks

RESULTS OF BENCHMARK: Linked List Allocator
  136818 allocation attempts,  106743 successful allocations,   26004 pre-fail allocations,   96369 deallocations
            CATEGORY | OCTILE 0     1     2      3      4      5      6      7       8 | AVERAGE
---------------------|-----------------------------------------------------------------|---------
     All Allocations |       42  4032  9261  15729  23772  33999  46977  59010  621642 |   29190  ticks
Pre-Fail Allocations |       42   840  2121   3780   5964   8694  12243  17514  614250 |   10781  ticks
       Deallocations |       42  3045  6615  10878  15813  21672  28602  38031  107877 |   18880  ticks

RESULTS OF BENCHMARK: Galloc
  282102 allocation attempts,  206544 successful allocations,   22751 pre-fail allocations,  196616 deallocations
            CATEGORY | OCTILE 0   1   2    3      4      5      6      7       8 | AVERAGE
---------------------|-----------------------------------------------------------|---------
     All Allocations |       42  63  63  378  12474  27027  41559  45549  100527 |   19129  ticks
Pre-Fail Allocations |       42  42  42   42     63     63     63    861   21714 |     691  ticks
       Deallocations |       42  63  84   84    105    231    294    693   15771 |     262  ticks

RESULTS OF BENCHMARK: Talloc
 2193976 allocation attempts, 1545626 successful allocations,   24585 pre-fail allocations, 1534743 deallocations
            CATEGORY | OCTILE 0   1   2    3    4    5    6    7      8 | AVERAGE
---------------------|--------------------------------------------------|---------
     All Allocations |       42  63  63   84   84  105  126  210  38871 |     133  ticks
Pre-Fail Allocations |       42  63  63   84   84   84  105  126   3927 |     100  ticks
       Deallocations |       42  84  84  105  105  147  252  336  17115 |     187  ticks
```

Talloc performs the best, with only Galloc coming close when not under heap pressure. Galloc often allocates slightly faster than Talloc but otherwise takes much longer, whereas Talloc's performance is much more stable. (Galloc uses dedicated bins covering a smaller range of allocations, while Talloc's binning makes a broader range of allocations quick).

Note that:
- no attempt is made to account for interrupts in these timings, however, the results are fairly consistent on my computer.
- number of pre-fail allocations is more noise than signal due to random allocation sizes
- alignment requirements are inversely exponentially frequent, ranging from 2^2 bytes to 2^18, with 2^2 and 2^3 being most common



## Method
This is a dlmalloc-style implementation with boundary tagging and bucketing used to efficiently do general-purpose allocation.

The main differences compared to Galloc is that Talloc doesn't bucket by alignment at all, assuming most allocations will require at most a machine-word size alignment. Instead, a much broader range of bucket sizes are used, which should often be more efficient.

Additionally, the layout of chunk metadata is rearranged to allow for smaller minimum-size chunks to reduce memory overhead of small allocations.

## Testing
Test coverage on most of the helper types and some sanity checking on the allocation.

Other than that, lots of fuzzing of the allocator. See `/fuzz/fuzz_targets/fuzz_arena.rs`

## Features
* `spin` (default): Provides the `Tallock` type (a spin mutex wrapper) that implements `GlobalAlloc`.
* `allocator` (default): Provides an `Allocator` trait implementation via `Tallock`.

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
    * `mov` - safely move an initialized Talloc to the specified destination
    * `init` - initialize or re-initialize the arena (forgets all previous allocations, if any)
    * `extend` - initialize or extend the arena region
    * `truncate` - reduce the extent of the arena region
    * `spin_lock` - wraps the Talloc in a Tallock, which supports the `GlobalAlloc` and `Allocator` APIs
* Allocation:
    * `malloc`
    * `free`
    * `grow`
    * `shrink`

See their docs for more info.

`Span` is a handy little type for describing memory regions, because trying to manipulate `Range<*mut u8>` or `*mut [u8]` or `base_ptr`-`size` pairs tends to be inconvenient or annoying. See `Span::from*` and `span.to_*` functions for conversions.

## Advanced Usage

Instead of using `Talloc::new`, use `Talloc::with_oom_handler` and pass in a function pointer. This function will now be called upon OOM. This can be useful for a number of reasons, but one possiblity is dynamically extending the arena as required.

```rust
fn oom_handler(talloc: &mut Talloc, layout: Layout) -> Result<(), AllocError> {
    // alloc doesn't have enough memory, and we just got called! we must free up some memory
    // we'll go through an example of how to handle this situation

    // we can inspect `layout` to estimate how much we should free up for this allocation
    // or we can extend by any amount (increasing powers of two has good time complexity)

    // this function will be repeatly called until we free up enough memory or 
    // we return Err(AllocError) at which point the allocation will too.
    // be careful to avoid conditions where the arena isn't sufficiently extended
    // indefinitely, causing an infinite loop

    // some limit for the sake of example
    const ARENA_TOP_LIMIT: isize = 0x80000000;

    let old_arena: Span = talloc.get_arena();

    if old_arena.acme == ARENA_TOP_LIMIT {
        // we won't free any more, so return AllocError
        return Err(AllocError);
    }

    // we're going to extend the arena upward, doubling its size
    // but we'll be sure not to extend past the limit
    let new_arena: Span = old_arena.extend(0, old_arena.size()).below(ARENA_TOP_LIMIT);

    unsafe {
        // we're assuming the new memory up to ARENA_TOP_LIMIT is allocatable
        talloc.extend(new_arena);
    };

    Ok(())
}
```

## Caveat for consideration - multithreaded performance

I don't know why, but Talloc gets hit harder by extreme multithreaded contention than Galloc does. While this is not so relevant to most use cases, I'd like to resolve this issue if possible. If anyone has some tips for alleviating this issue, please open an issue!

I've tried aligning the bucket array to be on seperate cache lines, but no change in result. Other than that, I don't know what to try.

![2-Thread Random Actions Benchmark Results](/benchmark_graphs/random_actions_multi.png)

