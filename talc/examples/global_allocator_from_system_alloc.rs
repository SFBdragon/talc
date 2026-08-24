use std::alloc::System;

#[cfg(not(miri))]
use spinning_top::RawSpinlock;

#[cfg(not(miri))]
use talc::{TalcLock, source::GlobalAllocSource};

// Run with:
// `cargo run --example global_allocator`

// Notes:
//
// ## Using `spinning_top::RawSpinlock`
// The `spinning_top` crate provides a simple mutex we can use on most platforms.
// We'll use it for the sake of example.

#[global_allocator]
#[cfg(not(miri))]
static TALC: TalcLock<RawSpinlock, GlobalAllocSource<System>> =
    TalcLock::new(GlobalAllocSource::new(System));

fn main() {
    eprint!("Doing some small allocations... ");

    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();

    eprintln!("Done!");

    eprintln!("How about a little more!");

    vec.extend(0..100000usize);
    drop(vec);

    eprintln!("Done! Ending...");
}
