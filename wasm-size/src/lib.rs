#![no_std]

use core::alloc::Layout;

extern crate alloc;

#[cfg(not(target_family = "wasm"))]
compile_error!("Requires targetting WASM");

#[cfg(all(not(feature = "talc"), not(feature = "dlmalloc"), not(feature = "lol_alloc"), not(feature = "rlsf")))]
mod no_alloc {
    use core::alloc::{GlobalAlloc, Layout};

    struct NoAlloc;
    unsafe impl GlobalAlloc for NoAlloc {
        unsafe fn alloc(&self, _: Layout) -> *mut u8 { core::ptr::null_mut() }
        unsafe fn dealloc(&self, _: *mut u8, _: Layout) { }
    }

    #[global_allocator]
    static NOALLOC: NoAlloc = NoAlloc;
}

#[cfg(all(feature = "talc", not(feature = "talc_arena")))]
#[global_allocator]
static TALC: talc::TalckWasm = unsafe { talc::TalckWasm::new_global() };

#[cfg(all(feature = "rlsf"))]
#[global_allocator]
static RLSF: rlsf::SmallGlobalTlsf = rlsf::SmallGlobalTlsf::new();

#[cfg(all(feature = "talc", feature = "talc_arena"))]
#[global_allocator]
static ALLOCATOR: talc::Talck<talc::locking::AssumeUnlockable, talc::ClaimOnOom> = {
    use core::{mem::MaybeUninit, ptr::addr_of_mut};

    const MEMORY_SIZE: usize = 128 * 1024 * 1024;
    static mut MEMORY: [MaybeUninit<u8>; MEMORY_SIZE] = [MaybeUninit::uninit(); MEMORY_SIZE];
    let span = talc::Span::from_array(addr_of_mut!(MEMORY));
    let oom_handler = unsafe { talc::ClaimOnOom::new(span) };
    talc::Talc::new(oom_handler).lock()
};

#[cfg(feature = "lol_alloc")]
#[global_allocator] 
static LOL_ALLOC: lol_alloc::AssumeSingleThreaded<lol_alloc::FreeListAllocator> = 
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::FreeListAllocator::new()) };

#[cfg(feature = "dlmalloc")]
#[global_allocator]
static DLMALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

// this is necessary, despite rust-analyzer's protests
#[panic_handler]
fn panic_handler(_: &core::panic::PanicInfo) -> ! {
    loop { }
}

#[no_mangle]
pub unsafe extern "C" fn alloc(size: usize) -> *mut u8 {
    alloc::alloc::alloc(Layout::from_size_align_unchecked(size, 8))
}

#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, size: usize) {
    alloc::alloc::dealloc(ptr, Layout::from_size_align_unchecked(size, 8))
}

#[no_mangle]
pub unsafe extern "C" fn realloc(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
    alloc::alloc::realloc(ptr, Layout::from_size_align_unchecked(old_size, 8), new_size)
}
