use talc::*;

// note: miri thinks this violates stacked borrows upon program termination.
// This only occurs if #[global_allocator] is used. 
// Use the allocator API if you can't have that.

static mut START_ARENA: [u8; 10000] = [0; 10000];

#[global_allocator]
// the mutex provided by the `spin` crate is used here as it's a sensible choice
static ALLOCATOR: Talck<spin::Mutex<()>, ClaimOnOom> =
    // we need to use the ClaimOnOom OOM handler or similar as allocations may
    // occur prior to invocation of the program entrypoint main(), so claiming some
    // memory must be done on-demand
    Talc::new(unsafe { ClaimOnOom::new(
        Span::from_base_size(&START_ARENA as *const _ as *mut _, 10000)
        // Span::from_array(&mut ARENA) - better but requires unstable #[feature(const_mut_refs)]
    ) }).lock();

fn main() {
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();
}
