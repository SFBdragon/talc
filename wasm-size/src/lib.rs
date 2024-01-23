#![no_std]

extern crate alloc;

use core::alloc::Layout;

#[cfg(not(target_family = "wasm"))]
compile_error!("Requires targetting WASM");

#[cfg(not(any(feature = "dlmalloc", feature = "lol_alloc", feature = "talc_static")))]
#[global_allocator]
static TALC: talc::TalckWasm = unsafe { talc::TalckWasm::new_global() };

#[cfg(feature = "talc_static")]
#[global_allocator]
static ALLOCATOR: talc::Talck<talc::locking::AssumeUnlockable, talc::ClaimOnOom> = {
    static mut MEMORY: [core::mem::MaybeUninit<u8>; 128 * 1024 * 1024] =
        [core::mem::MaybeUninit::uninit(); 128 * 1024 * 1024];
    let span = talc::Span::from_base_size(unsafe { MEMORY.as_ptr() as *mut _ }, 128 * 1024 * 1024);
    talc::Talc::new(unsafe { talc::ClaimOnOom::new(span) }).lock()
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
pub unsafe extern "C" fn grow(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
    alloc::alloc::realloc(ptr, Layout::from_size_align_unchecked(old_size, 8), new_size)
}
