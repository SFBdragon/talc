//! Showcases using [`TalcCell`] and the [`Allocator`](allocator_api2::alloc::Allocator) API.
//!
//! Run with:
//! - `cargo run --example allocator_api`
//! - `cargo miri run --example allocator_api`

#![cfg_attr(feature = "nightly", feature(allocator_api))]

use allocator_api2::vec::Vec;
use talc::{DefaultBinning, TalcCell, min_first_heap_size, source::Manual};

fn main() {
    // Establish some memory for the allocator.
    let mut memory = [0u8; min_first_heap_size::<DefaultBinning>() + 10000];

    // Create the allocator and "claim" the memory.
    let talc = TalcCell::new(Manual);

    println!("Claiming memory...");

    // SAFETY: We know the memory is fine for use.
    // UNWRAP: We know that it's big enough for the metadata.
    let heap_end = unsafe { talc.claim(memory.as_mut_ptr(), memory.len()) }.unwrap();

    println!("Done!");
    println!("Allocating a Vec, extending it, shrinking it...");

    // Allocate, grow, shrink
    let mut vec = Vec::with_capacity_in(100, &talc);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();

    println!("Done!");
    println!("Resizing the heap...");

    // --- Resize the heap while allocations are active! --- //

    // Let's see how to resize the heap, with respect to the allocations
    // already present. We'll use `resize` which automatically determines
    // whether we're asking to `extend` or `truncate` the heap.

    // Let's say we want to have 200 bytes of free space at the top of the heap.
    // However we also need to ensure that we don't try to claim bytes outside of `memory`:
    let alloc_end = unsafe { talc.reserved(heap_end) }.up_to;
    let memory_top = memory.as_mut_ptr_range().end;
    let new_end = alloc_end.as_ptr().wrapping_add(200).min(memory_top);

    // Finally, resize the heap
    unsafe {
        talc.resize(heap_end, new_end).unwrap();
    }

    println!("Done!");
    println!("Shrinking the allocation again...");

    // Shrink again
    vec.truncate(50);
    vec.shrink_to_fit();

    println!("Done!");
    println!("Freeing resources...");

    // deallocate vec
    drop(vec);
}
