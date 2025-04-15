#![no_std]
#![no_main]

extern crate alloc;

use core::alloc::Layout;

use alloc::{alloc::alloc, vec::Vec};

use talc::{ClaimOnOom, Talck};

// todo: what is going on here

#[global_allocator]
static TALC: Talck<spin::Mutex<()>, ClaimOnOom> = Talck::new(unsafe {
    static mut INITIAL_ARENA: [u8; 10000] = [0; 10000];
    ClaimOnOom::array(&raw mut INITIAL_ARENA)
    // For older Rust versions: ClaimOnOom::array(core::ptr::addr_of!(INITIAL_ARENA) as *mut _)
});

#[no_mangle]
pub fn main() {
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();

    // let mut stack_chonker = [0u8; 20000];
    // unsafe {
    // TALC.lock().claim((&raw mut stack_chonker).cast(), 20000);
    // }

    /* let mut slice: Box<[core::mem::MaybeUninit<u8>]> = Box::new_uninit_slice(400);
    slice.fill(MaybeUninit::new(0x2B)); */

    unsafe {
        let layout = Layout::from_size_align(1000, 1).unwrap();
        let alloc = alloc(layout);
        alloc.write_bytes(0x2b, 1000);
        TALC.lock().deallocate(alloc, layout);
    }
}

#[cfg(miri)]
#[no_mangle]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    main();
    0
}
