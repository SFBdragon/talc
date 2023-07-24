#![feature(allocator_api)]

use talc::*;

static mut ARENA: [u8; 10000] = [0; 10000];

fn oom_handler(talc: &mut Talc, _: core::alloc::Layout) -> Result<(), ()> {
    let arena_span = Span::from(unsafe { core::ptr::addr_of_mut!(ARENA) });
    
    if talc.get_arena() == arena_span {
        Err(())
    } else {
        unsafe {
            talc.init(arena_span);
        }
        
        Ok(())
    }
}

fn main() {
    let talc = Talc::with_oom_handler(oom_handler).lock::<spin::Mutex<()>>();

    let mut vec = Vec::with_capacity_in(100, talc.allocator_api_ref());
    vec.extend(0..300usize);
}