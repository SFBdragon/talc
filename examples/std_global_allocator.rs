use talc::*;

// note: miri thinks this violates stacked borrows upon program termination.
// This only occurs if #[global_allocator] is used.
// Use the allocator API if you can't have that.

static mut START_ARENA: [u8; 10000] = [0; 10000];

// The mutex provided by the `spin` crate is used here as it's a sensible choice

// Allocations may occur prior to the execution of `main()`, thus support for
// claiming memory on-demand is required, such as the ClaimOnOom OOM handler.

#[global_allocator]
static ALLOCATOR: Talck<spin::Mutex<()>, ClaimOnOom> = Talc::new(unsafe {
        ClaimOnOom::new(Span::from_const_array(std::ptr::addr_of!(START_ARENA)))
    }).lock();

fn main() {
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();
}
