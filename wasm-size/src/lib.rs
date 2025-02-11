#![no_std]

use core::alloc::Layout;

extern crate alloc;

#[cfg(feature = "no_alloc")]
mod no_alloc {
    use core::alloc::{GlobalAlloc, Layout};

    struct NoAlloc;
    unsafe impl GlobalAlloc for NoAlloc {
        unsafe fn alloc(&self, _: Layout) -> *mut u8 {
            core::ptr::null_mut()
        }
        unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
    }

    #[global_allocator]
    static NOALLOC: NoAlloc = NoAlloc;
}

#[cfg(all(feature = "talc", not(feature = "talc_arena")))]
#[global_allocator]
static TALC: talc::wasm::WasmDynamicTalc = unsafe { talc::wasm::new_wasm_dynamic_allocator() };

#[cfg(all(feature = "talc", feature = "talc_arena"))]
#[global_allocator]
static TALC_ARENA: talc::wasm::WasmArenaTalc = {
    use core::mem::MaybeUninit;
    static mut MEMORY: [MaybeUninit<u8>; 128 * 1024 * 1024] =
        [MaybeUninit::uninit(); 128 * 1024 * 1024];

    unsafe { talc::wasm::new_wasm_arena_allocator(&raw mut MEMORY) }
};

#[cfg(all(feature = "rlsf", not(feature = "rlsf_small")))]
#[global_allocator]
static RLSF: rlsf::GlobalTlsf = rlsf::GlobalTlsf::new();

#[cfg(feature = "rlsf_small")]
#[global_allocator]
static RLSF: rlsf::SmallGlobalTlsf = rlsf::SmallGlobalTlsf::new();

#[cfg(feature = "lol_alloc")]
#[global_allocator]
static LOL_ALLOC: lol_alloc::AssumeSingleThreaded<lol_alloc::FreeListAllocator> =
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::FreeListAllocator::new()) };

#[cfg(feature = "dlmalloc")]
#[global_allocator]
static DLMALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[cfg(not(test))]
#[panic_handler]
fn panic_handler(_: &core::panic::PanicInfo) -> ! {
    loop {}
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
