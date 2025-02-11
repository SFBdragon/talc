use std::alloc::{GlobalAlloc, Layout, System};

use talc::*;

// Run with:
// `cargo run --example global_allocator`

// Notes:
//
// ## Using `spin::Mutex<()>`
// The `spin` crate provides a simple mutex we can use on most platforms.
// We'll use it for the sake of example.
//
// ## Using `ClaimOnOom`
// An OOM handler with support for claiming memory on-demand is required,
// as allocations may occur prior to the execution of `main`.

#[global_allocator]
#[cfg(not(miri))]
static TALC: Talck<spin::Mutex<()>, ClaimOnOom> = Talck::new(unsafe {
    static mut INITIAL_ARENA: [u8; 100000] = [0; 100000];
    ClaimOnOom::array(&raw mut INITIAL_ARENA)
    // For older Rust versions: ClaimOnOom::array(core::ptr::addr_of!(INITIAL_ARENA) as *mut _)
});

fn main() {
    eprint!("Doing some small allocations... ");

    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();

    eprintln!("Done!");

    eprint!("Dynamically obtaining a larger arena to allocate into... ");

    let size = 0x1000000;
    let layout = Layout::from_size_align(size, 1).unwrap();
    let ptr = unsafe {
        // Safety: Layout size is nonzero.
        System.alloc(layout)
    };

    #[cfg(not(miri))]
    let _main_heap = unsafe {
        // Safety: The memory in `main_arena` is same for reads and writes,
        // and does not overlap with any other arena.
        TALC.lock().claim(ptr, size).unwrap()
    };

    eprintln!("Done!");

    eprint!("Taking advantage of the larger arena... ");

    vec.extend(0..100000usize);
    drop(vec);

    eprintln!("Done! Ending...");

    unsafe {
        System.dealloc(ptr, layout);
    }
}
