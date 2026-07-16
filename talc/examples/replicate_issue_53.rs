//! Showcases using [`TalcCell`] and the [`Allocator`](allocator_api2::alloc::Allocator) API.
//!
//! Run with:
//! - `cargo run --example allocator_api`
//! - `cargo miri run --example allocator_api`

use std::alloc::{Layout, alloc};

use allocator_api2::{boxed::Box, vec::Vec};
use talc::{DefaultBinning, cell::TalcCell, source::Manual};

fn main() {
    unsafe {
        // Allocate a 8MiB-aligned 8MiB-sized heap from the system/global allocator
        let heap_size = 8 << 20;
        let heap_layout = Layout::from_size_align(heap_size, heap_size).unwrap();
        let heap = alloc(heap_layout);
        assert!(!heap.is_null());

        // Claim it
        let talc = TalcCell::<Manual, DefaultBinning>::new(Manual);
        talc.claim(heap, heap_size).unwrap();

        // Allocate and drop box
        let b = Box::new_in(1, &talc);
        drop(b);

        eprintln!("Allocated and deallocated box.");

        // Allocate 64-byte vec
        let mut v = Vec::<u8, _>::with_capacity_in(64, &talc);
        v.fill(0);

        eprintln!("Allocated and filled vec.");

        // This triggers a scan before deallocating.
        drop(v);

        eprintln!("Dropped. Done.");
    }
}
