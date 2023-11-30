use talc::*;

// note: miri thinks this violates stacked borrows upon program termination.
// This only occurs if #[global_allocator] is used. 
// Use the allocator API if you can't have that.

static mut START_ARENA: [u8; 10000] = [0; 10000];

#[global_allocator]
// The mutex provided by the `spin` crate is used here as it's a sensible choice
static ALLOCATOR: Talck<spin::Mutex<()>, ClaimOnOom> =
    // An OOM handler such as ClaimOnOom is required, as allocations
    // may occur prior to invoking the program entrypoint `main()`,
    // thus needs to support claiming memory on demand
    Talc::new(unsafe { ClaimOnOom::new(
        Span::from_base_size(&START_ARENA as *const _ as *mut _, 10000)
        // A better alternative, but requires the unstable attribute #[feature(const_mut_refs)]
        // Span::from_array(&mut START_ARENA)
    ) }).lock();

fn main() {
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();
}
