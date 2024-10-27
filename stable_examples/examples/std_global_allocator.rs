use talc::*;

// note:
// - Miri thinks this violates stacked borrows upon program termination.
//   - This only occurs with `#[global_allocator]`.
//   - Consider using the allocator API if you can't have that (see: `examples/stable_allocator_api.rs`)
// - `spin::Mutex<()>`
//   The `spin` crate provides a mutex that is a sensible choice to use.
// - `ClaimOnOom`
//   An OOM handler with support for claiming memory on-demand is required, as allocations may
//   occur prior to the execution of `main()`.

static mut START_ARENA: [u8; 10000] = [0; 10000];

#[global_allocator]
static ALLOCATOR: Talck<spin::Mutex<()>, ClaimOnOom> = Talc::new(unsafe {
    ClaimOnOom::new(Span::from_const_array(core::ptr::addr_of!(START_ARENA)))
}).lock();

fn main() {
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();
}
