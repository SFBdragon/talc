# Allocator Internals

## Type

This is a dlmalloc-style linked list allocator with boundary tagging and binning, aimed at general-purpose use cases. Allocation is O(n) worst case (but in practice its near-constant time, see microbenchmarks), while in-place reallocations and deallocations are O(1).

This allocator is quite similar to the TLSF (Two-Level Segregated Fit) algorithm. Some notable differences:
- Talc allocates bins in a similar way to TLSF, but uses a single index calculated in
  a fairly optimized way to achieve approximately the same thing, reducing some register pressure.
  (In my rough testing, switching Talc to use TLSF's approach hurt performance.)
- Talc doesn't give up early if the fast acquisition path for an allocation fails:
  it falls back to a full search, making allocations technically O(n).
  (This reduces fragmentation and makes OOM failures a little less likely.)

(If you want a more pure TLSF implementation, do check out `rlsf` - it's a great `no_std` allocator.)

## Heaps

Talc allows for an arbitrary number of heaps to facilitate using discontiguous memory
(useful for using both static and dynamically allocated memory, or managing dicontinuous memory maps,
and in-general supporting mmap-like memory acquistion patterns over brk/sbrk)
provisioning different amounts of memory at different times and in different ways
- e.g. statically allocating a small amount for any allocations before the program can provision larger blocks from the OS or UEFI memory maps

We need to ensure the allocation

## Memory Layout

Because deallocated allocations are coalesced with neighbouring gaps,
the allocation must track whether its neighbours are allocated
(and we can't look at possibly-allocated memory to determine this).

To avoid allocations needing to track whether there's a gap above and below them,
they have a footer that distinguishes them as allocated and tracks whether the chunk
above is allocated. When deallocating, an allocation will check its own tag to see if
the above is free, and will check the tag of the chunk below to see if it's free.
It's safe to access a neighbouring gap because both free and allocated chunks have a
metadata footer.

(We need to be able to update whether the neighbouring chunks are free when those chunks
are allocated and deallocated.)

Because we need to track such metadata for arbitrary numbers of allocations, it's stored
next to the allocation. You can store it high-side (and track ABOVE_FREE) or low-side
(and track BELOW_FREE). The advantage of storing it high-side is that aligned allocations
are more likely to have the tag in the same cache line as the allocation data.
i.e. 

Allocations need to track the following things:
- It needs to be distinguishable from 
- Is the chunk immediately above me free or allocated? (`ABOVE_FREE`)


Gap:
- !ALLOCATED
- ?SMALL_FORMAT
- HEAP_END

Allocation


    pub const ALLOCATED_FLAG: usize = 1 << 0;
    pub const ABOVE_FREE_FLAG: usize = 1 << 1;
    pub const SMALL_FORMAT_FLAG: usize = Self::ABOVE_FREE_FLAG;
    pub const HEAP_END_FLAG: usize = 1 << 2;

    pub const ALLOCATED: Tag = Tag(Self::ALLOCATED_FLAG);
    pub const ABOVE_FREE: Tag = Tag(Self::ABOVE_FREE_FLAG);
    pub const HEAP_END: Tag = Tag(Self::HEAP_END_FLAG);
