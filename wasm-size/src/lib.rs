#![no_std]

extern crate alloc;

use core::alloc::Layout;

#[cfg(not(target_arch = "wasm32"))]
compile_error!("Requires --target wasm32-unknown-unknown");

#[cfg(not(any(feature = "dlmalloc", feature = "lol_alloc")))]
#[global_allocator]
static ALLOCATOR: talc::TalckWasm = unsafe { talc::TalckWasm::new_global() };

#[cfg(feature = "lol_alloc")]
#[global_allocator] 
static LOL_ALLOC: lol_alloc::AssumeSingleThreaded<lol_alloc::FreeListAllocator> = 
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::FreeListAllocator::new()) };

#[cfg(feature = "dlmalloc")]
mod dlmalloc {
    use alloc::alloc::Layout;
    use dlmalloc::Dlmalloc;
    use alloc::alloc::GlobalAlloc;

    #[global_allocator]
    static ALLOC: DlMallocator = DlMallocator(lock_api::Mutex::new(Dlmalloc::new()));
    

    struct DlMallocator(lock_api::Mutex::<talc::locking::AssumeUnlockable, Dlmalloc>);

    unsafe impl GlobalAlloc for DlMallocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            self.0.lock().malloc(layout.size(), layout.align())
        }
    
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            self.0.lock().free(ptr, layout.size(), layout.align());
        }
    
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            self.0.lock().realloc(ptr, layout.size(), layout.align(), new_size)
        }
    
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            self.0.lock().calloc(layout.size(), layout.align())
        }
    }    
}

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
