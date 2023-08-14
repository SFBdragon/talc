#![feature(allocator_api)]
#![feature(vec_into_raw_parts)]

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

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
    

    struct DlMallocator(lock_api::Mutex::<talc::AssumeUnlockable, Dlmalloc>);

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

#[panic_handler]
fn panic_handler(_: &core::panic::PanicInfo) -> ! {
    loop { }
}

// Box a `u8`!
#[no_mangle]
pub extern "C" fn hello() -> *mut u8 {
    let mut vec = Vec::<u8>::new();
    let _ = vec.try_reserve(42);
    vec.into_raw_parts().0
}

/// Free a `Box<u8>` that we allocated earlier!
/// # Safety
/// `ptr` must be a pointer from `hello` which is used exactly once.
#[no_mangle]
pub unsafe extern "C" fn goodbye(ptr: *mut u8) {
    let _ = Vec::from_raw_parts(ptr, 0, 42);
}

/// Resize a `Box<u8>` that we allocated earlier!
/// # Safety
/// `ptr` must be a pointer from `hello` or `goodbye` which is used exactly once.
#[no_mangle]
pub unsafe extern "C" fn renegotiate(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
    let mut v = Vec::from_raw_parts(ptr, 0, old_size);
    let _ = v.try_reserve(new_size - old_size);
    v.into_raw_parts().0
}
