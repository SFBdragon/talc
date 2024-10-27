use std::ptr::addr_of;

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
//
// ## Arena pointer conversion from `*const` to `*mut`
// Without `const_mut_refs` being stable just yet, we need to get a mutable pointer
// indirectly. It's not clear whether this is acceptable (see: https://github.com/SFBdragon/talc/issues/32)
// but MIRI is fine with it and Rust's aliasing/provenance rules don't stipulate yet.
// Once Rust 1.83 lands with `const_mut_refs`, this example will be changed
// to just use `&raw mut`.

static mut ARENA: [u8; 10000] = [0; 10000];

#[global_allocator]
static ALLOCATOR: Talck<spin::Mutex<()>, ClaimOnOom> = Talc::new(unsafe {
    ClaimOnOom::new(Span::from_array(addr_of!(ARENA) as *mut [u8; 10000]))
}).lock();

fn main() {
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();
}
