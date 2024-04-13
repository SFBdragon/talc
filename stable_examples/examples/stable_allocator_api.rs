use talc::{ErrOnOom, Talc};
use allocator_api2::vec::Vec;

// This uses the `allocator-api2` crate to compile successfully on stable Rust.

// Run with:
// `cargo +stable run -p stable_examples --example stable_allocator_api`

fn main() {
    // Establish some memory for the allocator.
    let mut arena = [0u8; 10000];

    // Create the allocator and "claim" the memory.
    let talck = Talc::new(ErrOnOom).lock::<spin::Mutex<()>>();

    // We know the memory is fine for use (unsafe) and that it's big enough for the metadata (unwrap).
    let heap = unsafe {
        talck.lock().claim(arena.as_mut().into()).unwrap()
    };

    // Allocate, grow, shrink
    let mut vec = Vec::with_capacity_in(100, &talck);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();

    // --- Resize the arena while allocations are active! --- //

    // Let's see how to shrink the arena, as this is more complicated than extending it,
    // as we need to respect the allocations that are currently present.

    // First, lock the allocator. We don't want a race condition between
    // getting the allocated span (see below) and truncating.
    // If the minimum heap span changes and we try to truncate to an invalid
    // heap, a panic will occur.
    let mut talc = talck.lock();

    // Retrieve the shrink-wrapped span of memory in this heap.
    let allocated_span = unsafe { talc.get_allocated_span(heap) };

    // Let's say we want to leave only a little bit of memory on either side,
    // and free the rest of the heap. 
    // Additionally, make sure we don't "truncate" to beyond the original heap's boundary.
    let new_heap = allocated_span.extend(200, 200).fit_within(heap);

    // Finally, truncate the heap!
    let _heap2 = unsafe {
        talc.truncate(heap, new_heap)
    };

    // and we're done!
    drop(talc);

    // deallocate vec
    drop(vec);
}