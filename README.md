# Talloc
_The TauOS Allocator_

Talloc is a performant and flexible `no_std`-compatible memory allocator suitable for projects such as operating system kernels, or arena allocation for normal apps.

Using Talloc as a simple arena allocator is easy, but practical concerns in `no_std` environments are facilitated, such as custom OOM handling, changing the arena size, managing the metadata, and avoiding holes of usable memory in the arena. Less practical `no_std` concerns are also handled, such as spanning an arena over the zero address (fun!).

## Usage

Use it as a global allocator as follows:
```rust
use talloc::*;

const MIN_SIZE: usize = 0x80;

#[global_allocator]
static ALLOCATOR: Tallock<SPEED_BIAS> = Talloc::<SPEED_BIAS>::new_empty(MIN_SIZE, alloc_error)
    .wrap_spin_lock();

// initialize it later...
let arena = Span::from(0x100000..0x10000000);
unsafe { ALLOCATOR.lock().extend(arena, MemMode::Automatic); }
```

Use it as an arena allocator via the `Allocator` API like so:
```rust
let arena = vec![0u8; SIZE];
let min_block_size = 0x20;
let tallock = Talloc::<SPEED_BIAS>::new_arena(&mut arena, min_block_size).wrap_spin_lock();

tallock.allocate(...);
```

The `BIAS` parameter, `Talloc::extend`, and `Talloc::release` functions give plenty of flexibility for niche applications, as detailed later.

## Performance
O(log n) worst case allocations and deallocations. In practice, it's often O(1) and very fast even when it isn't. See the benchmarks below.

Growing memory always forces a memory copy. Shrinking is always in-place.

Compiling with `RUST_FLAGS="-C target-cpu=native"` or `RUST_FLAGS="-C target-feature=+lzcnt"` is recommended to take advantage of the `lzcnt` instruction on most AMD64/Intel64 CPUs.

## Benchmarks

While results vary for different values of Talloc's `BIAS` parameter, the results aren't that different between `SPEED_BIAS` and `EFFICIENCY_BIAS`.

### galloc's benchmarks:

Note: the original benchmarks have been modified slightly (e.g. replacing `rand` with `fastrand`) in order to alleviate the bottleneck on Talloc.

#### Random Actions Benchmark Results

![Random Actions Benchmark Results](/benchmark_graphs/random_actions.png)

Talloc outperforms the alternatives. 

#### Heap Exhaustion Benchmark Results

![Heap Exhaustion Benchmark Results](/benchmark_graphs/heap_exhaustion.png)

Here we see Talloc's lower memory effeciency gets penalized. 

Using EFFICIENCY_BIAS, Talloc is brought onto par with linked_list_allocator in Heap Exhaustion, but the lead in Random Actions is reduced to below 200%.

### simple_chunk_allocator's benchmarks

Output of `RUST_FLAGS="-C target-cpu=native" cargo run --release --example simple_chunk_allocator_bench` with BENCH_DURATION set to `1.0` 

```
RESULTS OF BENCHMARK: Chunk Allocator
     64179 allocations,  19967 successful_allocations,  44212 deallocations
    median=   672 ticks, average=   744 ticks, min=    63 ticks, max= 72177 ticks

RESULTS OF BENCHMARK: Linked List Allocator
     41374 allocations,  12156 successful_allocations,  29218 deallocations
    median= 13629 ticks, average= 23748 ticks, min=    42 ticks, max=982002 ticks

RESULTS OF BENCHMARK: Galloc
     65958 allocations,  19772 successful_allocations,  46186 deallocations
    median=   105 ticks, average=   418 ticks, min=    42 ticks, max=637371 ticks

RESULTS OF BENCHMARK: Talloc
     66392 allocations,  19058 successful_allocations,  47334 deallocations
    median=   105 ticks, average=   104 ticks, min=    42 ticks, max=  1449 ticks
```

While both Talloc and Galloc have equal median and minimum allocation times (although `rdtscp` seems to be giving fairly quantized results), Talloc's consistency gives it a significantly better average performance.

Output of `RUST_FLAGS="-C target-cpu=native" cargo run --release --example simple_chunk_allocator_bench` with BENCH_DURATION set to `10.0` 

```
RESULTS OF BENCHMARK: Chunk Allocator
     81941 allocations,  24240 successful_allocations,  57701 deallocations
    median=   630 ticks, average=216010 ticks, min=   105 ticks, max=4864734 ticks

RESULTS OF BENCHMARK: Linked List Allocator
    105066 allocations,  29561 successful_allocations,  73234 deallocations
    median= 49623 ticks, average=100834 ticks, min=    42 ticks, max=1910580 ticks

RESULTS OF BENCHMARK: Galloc
    133349 allocations,  31999 successful_allocations,  93681 deallocations
    median=   126 ticks, average= 60083 ticks, min=    42 ticks, max=1158990 ticks

RESULTS OF BENCHMARK: Talloc
    172667 allocations,  27590 successful_allocations, 122150 deallocations
    median=   105 ticks, average=   194 ticks, min=    42 ticks, max= 28644 ticks
```

Similar trend, but the effect is far more pronounced under overbearing heap pressure.

## Efficiency
Efficiency depends primarily on the choice of the `BIAS` parameter. With `0`, average waste is a quarter of the allocation, although situations where align is greater than size can make this far worse. `min_size` determines the minimum allocatable size - many allocations smaller than this may also be wasteful.

Average waste is halved for each increment of `BIAS` above `1`. `SPEED_BIAS=2` has an average waste of an eighth, while `EFFICIENCY_BIAS=3` has an average waste of a sixteenth of the allocation size (regardless of alignment).

Metadata is required by `Talloc`. The arena will be used unless otherwise specified. The bitmap grows proportionally with the arena, taking up the most space. For large arenas with `min_size <= 0x10` bytes, this can be as much as 1/64 of the arena (1/32 if the arena size is just over a power of two), but for each power of two greater of min_size, the requirement is divided by two. For example, an 8GiB heap with `min_size = 128` bytes requires just over 16MiB of metadata. Not too shabby.

## Methods
A buddy system of power-of-two block size is used. A linked list tracks free blocks while a bitmap tracks the availability of pairs to each other for reclaiming contiguous chunks of memory. Finally, a small additional bitmap is used to track which block sizes are available. 

In combination, Talloc never needs to search for free memory to allocate or re-combine. Allocation failure is known after a few bit manipulations. This makes Talloc's performance not only fast, but very consistant - Talloc doesn't really experience heap pressure.

The main trade-off is complexity.

## Testing

A fair bit of fuzzing has been conducted.

## TODO

- Tests and doctests
- Doc linking
- Maybe make it usable jemallocator-style in std environments with page management
    - Figure out how to do this efficiently (can we tell the system to page low-use pages aggressively?)
    - Maybe using mmap-rs?
    - Then benchmark against jemallocator/the systems allocator for a few laughs and tears
    - See how many platforms I'd care to support?

## General Usage

Here is the list of methods:
* Allocation:
    * `alloc`
    * `dealloc`
    * `shrink`
* Information:
    * `get_arena` - returns the current arena memory region
    * `get_meta_mem` - returns the current metadata memory region
    * `req_meta_mem` - returns the necessary metadata memory for an extended arena
* Management:
    * `extend` - initialize or extend the arena region
    * `release` - release specified memory in the arena for allocation
    * `wrap_spin_lock` - wraps the Talloc in a Tallock, which supports the `GlobalAlloc` and `Allocator` APIs

Constructors:
* `new` - creates a new Talloc with an empty arena
* `new_arena` - creates and fully initializes a Talloc for the provided arena

See their docs for more info.

`Span` is a handy-dandy little type for describing memory regions, because trying to manipulate `Range<*mut u8>` or `*mut [u8]` or `base_ptr`-`size` pairs gets annoying. See `Span::from*` and `span.to_*` functions for conversions.

## Advanced Usage

The least self-explanatory components are the `extend` and `release`, as well as the `oom_handler` function
required by every Talloc (giving is `talloc::alloc_error` is the simplest method, returns AllocError on OOM).

See their docs (including the `OomHandler` type's) for more details.

An example of how these components can be put together is given below.

```rust
fn oom_handler(talloc: &mut Talloc<BIAS>, layout: Layout) -> Result<(), AllocError> {
    // alloc doesn't have enough memory, and we just got called! we must free up some memory
    // we'll go through an example of how to handle this situation manually

    // we can inspect `layout` to estimate how much we should free up for this allocation
    // or we can extend by any amount (increasing powers of two has good time complexity)

    // this function will be repeatly called until we free up enough memory or 
    // we return Err(AllocError) at which point the allocation will too


    // Talloc may round the arena edges inwards, so always fetch it manually
    let old_arena: Span = talloc.get_arena();
    // we're going to extend the arena upward to the next power of two size
    let new_arena: Span = old_arena.extend(0, old_arena.size());

    // let's fetch the current metadata memory to maybe free it later
    let (old_meta_ptr, old_meta_layout) = talloc.get_meta_mem();

    // we're going to manually create some seperate space for metadata, if necessary
    let new_meta_memory: Option<(*mut u8, Layout)> = talloc.req_meta_mem(new_arena)
        .map(|layout: Layout| (unsafe { std::alloc::alloc(layout) }, layout));

    unsafe {
        // SAFETY: we use MemMode::Manual, no auto-release
        // and we've allocated enough writable metadata memory 
        // and it doesn't conflict with released arena memory
        talloc.extend(
            new_arena, 
            // MemMode::Automatic puts the metadata within the new arena, and frees the rest
            MemMode::Manual { 
                // we want to specify what to release for allocation
                auto_release: false,
                // we want to use a special region for metadata, not necesarily within the arena
                metadata_memory: new_meta_memory.map(|m| m.0),
            }
        ).unwrap(); // we've allocated the requested metadata memory, this'll never be Err

        // note that extend doesn't touch any memory we haven't told it to
        // it just prepared the allocator to handle a larger arena
    };

    // release some memory for allocation
    let new_memory = new_arena.above(old_arena.acme);
    let bad_memory = Span::from(0x2000..0x4000);
    unsafe {
        // SAFETY: we promise everything except bad_memory is valid for read/writes
        // we're also not freeing metadata memory, because it's supposedly outside new_arena
        talloc.release(new_memory.above(bad_memory.acme));
        // this'll probably release nothing, but that's ok
        talloc.release(new_memory.below(bad_memory.base));
    }

    // manage our old metadata memory, if replaced
    if new_meta_memory.is_some() {
        unsafe {
            // SAFETY: we got this memory from std::alloc::alloc
            std::alloc::dealloc(old_meta_ptr, old_meta_layout);
        }
    }

    Ok(())
}
```
