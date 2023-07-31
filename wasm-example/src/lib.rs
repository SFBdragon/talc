#![feature(allocator_api)]
#![feature(vec_into_raw_parts)]

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOCATOR: talc::TalckWasm = unsafe { talc::TalckWasm::new_global() };

#[panic_handler]
fn panic_handler(_: &core::panic::PanicInfo) -> ! {
    loop { }
}

// Box a `u8`!
#[no_mangle]
pub extern "C" fn hello() -> *mut u8 {
    Vec::<u8>::with_capacity(42).into_raw_parts().0
}

/// Free a `Box<u8>` that we allocated earlier!
/// # Safety
/// `ptr` must be a pointer from `hello` which is used exactly once.
#[no_mangle]
pub unsafe extern "C" fn goodbye(ptr: *mut u8) {
    let _ = Vec::from_raw_parts(ptr, 0, 42);
}

/// Resize a `Box<u8>` that we allocated earlier!
/// # Safety
/// `ptr` must be a pointer from `hello` or `goodbye` which is used exactly once.
#[no_mangle]
pub unsafe extern "C" fn renegotiate(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
    let mut v = Vec::from_raw_parts(ptr, 0, old_size);
    v.reserve(new_size - old_size);
    v.into_raw_parts().0
}
