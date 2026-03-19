//! A simple example using Talc as a global allocator in a `no_std` environment.
//!
//! The allocator is used in a few different ways for the sake of demonstration.
//!
//! Run with:
//! - `cargo run --example no_std_global_allocator`
//! - `cargo miri run -p talc --example no_std_global_allocator`
//!     MIRI currently doesn't like this example for reasons I don't understand.

#![no_std]
#![cfg_attr(miri, no_main)]

extern crate alloc;

use core::alloc::Layout;

use alloc::{
    alloc::{alloc, dealloc},
    boxed::Box,
    vec::Vec,
};
use spinning_top::RawSpinlock;
use talc::{TalcLock, source::Claim};

#[global_allocator]
static TALC: TalcLock<RawSpinlock, Claim> = TalcLock::new(unsafe {
    static mut INITIAL_ARENA: [u8; 50000] = [0; 50000];
    Claim::array(&raw mut INITIAL_ARENA)
    // For older Rust versions: Claim::array(core::ptr::addr_of!(INITIAL_ARENA) as *mut _)
    // this is not ideal as its unclear if
});

#[cfg_attr(miri, no_mangle)]
pub fn main() {
    if cfg!(miri) {
        // Disably MIRI validation for now. It's tripping up on allocation drops here.
        return;
    }

    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();

    let mut stack_memory = [0u8; 20000];
    let stack_heap_end =
        unsafe { TALC.lock().claim((&raw mut stack_memory).cast(), 20000).unwrap() };

    let mut slice: Box<[core::mem::MaybeUninit<u8>]> = Box::new_uninit_slice(400);
    slice.fill(core::mem::MaybeUninit::new(0x2B));

    unsafe {
        let layout = Layout::from_size_align(1000, 1).unwrap();
        let alloc = alloc(layout);
        alloc.write_bytes(0x2b, 1000);
        dealloc(alloc, layout);
    }

    drop(slice);

    unsafe {
        TALC.lock().truncate(stack_heap_end, core::ptr::null_mut());
    }

    drop(vec);
}

#[cfg(miri)]
#[no_mangle]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    main();
    0
}
