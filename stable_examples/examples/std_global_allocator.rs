use std::alloc::{GlobalAlloc, Layout};

use talc::*;

// Run with:
// `cargo +stable run -p stable_examples --example std_global_allocator`
// `cargo miri run -p stable_examples --example std_global_allocator`

// Notes:
// 
// ## Using `spin::Mutex<()>`
// The `spin` crate provides a simple mutex we can use on most platforms.
// We'll use it for the sake of example.
//
// ## Using `ClaimOnOom`
// An OOM handler with support for claiming memory on-demand is required, as allocations may
// occur prior to the execution of `main`.

static mut INITIAL_ARENA: [u8; 10000] = [0; 10000];

#[global_allocator]
static ALLOCATOR: Talck<spin::Mutex<()>, ClaimOnOom> = Talc::new(unsafe {
    ClaimOnOom::new(Span::from_array(&raw mut INITIAL_ARENA))
}).lock();

fn main() {
    eprint!("Doing some small allocations... ");

    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();

    eprintln!("Done!");

    eprint!("Dynamically obtaining a larger arena to allocate into... ");

    let size = 0x1000000;
    let ptr = unsafe {
        // Safety: Layout size is nonzero.
        std::alloc::System.alloc(Layout::from_size_align(size, 1).unwrap())
    };
    let main_arena = Span::from_base_size(ptr, size);
    
    let _main_heap = unsafe {
        // Safety: The memory in `main_arena` is same for reads and writes,
        // and does not overlap with any other arena.
        ALLOCATOR.lock().claim(main_arena).unwrap()
    };

    eprintln!("Done!");

    eprint!("Taking advantage of the larger arena... ");

    vec.extend(0..100000usize);
    drop(vec);

    eprintln!("Done! Ending...");
}
