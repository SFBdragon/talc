use talc::*;

// note: miri thinks this violates stacked borrows.
// this only occurs if #[global_allocator] is used.
// use the allocator API if you want nice things.

static mut ARENA: [u8; 10000] = [0; 10000];
#[global_allocator]
static ALLOCATOR: Talck<spin::Mutex<()>, ClaimOnOom> = Talc::new(unsafe {
    ClaimOnOom::new(Span::from_slice(ARENA.as_slice() as *const [u8] as *mut [u8]))
})
.lock();

fn main() {
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();
}
