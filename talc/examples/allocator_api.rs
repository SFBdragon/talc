//! Showcases using [`TalcCell`] and the [`Allocator`](allocator_api2::alloc::Allocator) API.

#![cfg_attr(feature = "nightly", feature(allocator_api))]

use allocator_api2::vec::Vec;
use talc::{ErrOnOom, TalcCell};

// Run with:
// `cargo run --example allocator_api`
// `cargo miri run --example allocator_api`

fn main() {
    // Establish some memory for the allocator.
    let mut memory = [0u8; 10000];

    // Create the allocator and "claim" the memory.
    let talc = TalcCell::new(ErrOnOom);

    // We know the memory is fine for use (unsafe) and that it's big enough for the metadata (unwrap).
    let end = unsafe { talc.claim(memory.as_mut_ptr(), memory.len()) }.unwrap().as_ptr();

    // Allocate, grow, shrink
    let mut vec = Vec::with_capacity_in(100, &talc);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();

    // --- Resize the arena while allocations are active! --- //

    // Let's see how to resize the arena, with respect to the allocations
    // already present. We'll use `resize` which automatically determines
    // whether we're asking to `extend` or `truncate` the arena.

    // Let's say we want to have 200 bytes of free space at the top of the heap.
    // However we also need to ensure that we don't try to claim bytes outside of `memory`:
    // TODO EXPLAIN UNWRAP
    let allocated = unsafe { talc.reserved(end) }.unwrap();
    let new_size = allocated.as_ptr().wrapping_add(200).min(memory.as_mut_ptr_range().end);

    // Finally, resize the heap!
    let _arena = unsafe { talc.resize(end, new_size) }.unwrap();

    // Shrink again
    vec.truncate(50);
    vec.shrink_to_fit();

    // deallocate vec
    drop(vec);
}
