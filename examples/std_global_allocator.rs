use talc::*;

#[global_allocator]
static ALLOCATOR: Talck<spin::Mutex<()>> = Talc::with_oom_handler(oom_handler).lock();
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
    let mut vec = Vec::with_capacity(100);
    vec.extend(0..300usize);
    vec.truncate(100);
    vec.shrink_to_fit();
}