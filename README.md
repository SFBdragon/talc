# Talloc, the TauOS allocator
(TauOS is on the list of projects that I'll "definitely get back to someday," don't pay it any mind)

Talloc is a `no_std` allocator suitable for projects such as operating system kernels. Performance and flexibility were the primary considerations during development.

Practical concerns in `no_std` environments are also catered to, such as custom OOM handling, expanding the arena, managing the metadata, and avoiding holes of usable memory in the arena (such as for physical memory allocation). Less practical `no_std` concerns are also accounted for, such as stretching an arena over the zero address. (This use-case is relatively untested, be warned.)

On the other hand, using Talloc as a simple arena allocator is made easy too.

## Usage

Use it as a global allocator as follows:
```rust
#[global_allocator]
static ALLOCATOR: Tallock<{talloc::SPEED_BIAS}> = 
    talloc::Talloc::<{talloc::SPEED_BIAS}>::new_empty(MIN_SIZE, talloc::alloc_error)
    .wrap_spin_lock();

// initialize it later...
let arena = talloc::Span::from(0x0..0x100000);
unsafe { ALLOCATOR.lock().extend(arena, MIN_SIZE, talloc::MemMode::Automatic, talloc::alloc_error); }
```

Use it as an arena allocator via the `Allocator` API like so:
```rust
let arena = vec![0u8; SIZE];
let min_block_size = 0x20;
let tallock = Talloc::<{talloc::SPEED_BIAS}>::new_arena(&mut arena, min_block_size)
    .wrap_spin_lock();

tallock.allocate(...);
```

The `Talloc::new`, `Talloc::extend`, and `Talloc::release` functions give plenty of flexibility for more niche applications.

## Performance
O(log n) worst case allocations and deallocations. In practice, it's often O(1) and very fast even when it isn't. See the benchmarks below.

Growing memory always forces a memory copy. Shrinking is always in-place.

Getting the leading zero bit count is used a lot. Compiling with `RUST_FLAGS="-C target-cpu=native"` is recommended to take advantage of the `lzcnt` instruction on most AMD64/Intel64 CPUs. `RUST_FLAGS="-C target-feature=+lzcnt"` is a more general way to achieve this, if portability is desired over other performance optimizations.

## Efficiency
Efficiency depends primarily on the choice of the `BIAS` parameter. With `0`, average waste is a quarter of the allocation, although situations where align is greater than size can make this far worse.

Average waste is halved for each increment of `BIAS` over zero. `SPEED_BIAS=2` has an average waste of an eighth, while `EFFICIENCY_BIAS=3` has an average waste of a sixteenth of the allocation size.

Metadata is required by `Talloc`. The arena will be used unless otherwise specified. The bitmap grows proportionally with the arena, taking up the most space. For large arenas with minimal min_size, this can be as much as 1/64 of the arena, but for each power of two greater greater of min_size, the requirement is divided by two.

## Methods
A buddy system of power-of-two block size is used. A linked list tracks free blocks while a bitmap tracks the availability of pairs to each other for reclaiming contiguous chunks of memory. Finally, a small additional bitmap is used to track which block sizes are available. 

In combination, Talloc never needs to search for free memory to allocate or re-combine. Allocation failure is known after a few bit manipulations. This makes Talloc's performance not only fast, but very stable, as Talloc doesn't really experience heap pressure.

The main trade-off is complexity.

## Testing

A fair bit of fuzzing has been conducted.

TODO :)

## Benchmarks

While results vary for different values of Talloc's `BIAS` parameter, the results aren't that different between `SPEED_BIAS` and `EFFICIENCY_BIAS`.

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

### galloc's benchmarks:

Note: the original benchmarks have been modified slightly (e.g. replacing `rand` with `fastrand`) in order to alleviate the bottleneck on Talloc.

#### Random Actions Benchmark Results

![Random Actions Benchmark Results](/benchmark_graphs/random_actions.png)

Talloc outperforms the alternatives. 

#### Heap Exhaustion Benchmark Results

![Heap Exhaustion Benchmark Results](/benchmark_graphs/heap_exhaustion.png)

Here we see Talloc's lower memory effeciency gets penalized. 

Using EFFICIENCY_BIAS, Talloc is brought onto par with linked_list_allocator in Heap Exhaustion, but the lead in Random Actions is reduced to below 200%.


## Advanced Usage

OOM handling, extending, and releasing memory. See the example below.

```rust
fn oom_handler(talloc: &mut Talloc<BIAS>, layout: Layout) -> Result<(), AllocError> {
    // an allocation or reallocation has failed! we must free up some memory
    // we'll go through a toy example of how to handle this situation

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
