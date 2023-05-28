use crate::{Talloc, AllocError};

#[cfg(feature = "allocator")]
use core::alloc::Allocator;

use core::{alloc::{GlobalAlloc, Layout}, ptr::{NonNull, self}};

/// Concurrency synchronisation layer on top of `Talloc`, see its documentation for more.
/// 
/// This is just a thin wrapper containing a spin mutex which implements the allocator
/// traits as the underlying allocator is not internally synchronized.
#[derive(Debug)]
pub struct Tallock<const BIAS: usize>(pub spin::Mutex<Talloc<BIAS>>);

impl<const BIAS: usize> Tallock<BIAS> {
    #[inline]
    pub const fn new(talloc: Talloc<BIAS>) -> Self {
        Self(spin::Mutex::new(talloc))
    }

    /// Acquire the lock on the `Talloc`.
    #[inline]
    pub fn lock(&self) -> spin::MutexGuard<Talloc<BIAS>> {
        self.0.lock()
    }
}

impl<const BIAS: usize> Talloc<BIAS> {
    pub const fn wrap_spin_lock(self) -> Tallock<BIAS> {
        Tallock::new(self)
    }
}

unsafe impl<const BIAS: usize> GlobalAlloc for Tallock<BIAS> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.lock().alloc(layout).map_or(core::ptr::null_mut(), |nn| nn.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: caller guaranteed that the given ptr was allocated
        // where null means allocation failure, thus ptr is not null
        self.lock().dealloc(NonNull::new_unchecked(ptr), layout);
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        match self.lock().alloc(layout) {
            Ok(ptr) => {
                ptr.as_ptr().write_bytes(0, layout.size());
                ptr.as_ptr()
            },
            Err(_) => ptr::null_mut(),
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: see dealloc
        if old_layout.size() < new_size {
            let allocation = Talloc::alloc(
                &mut self.lock(),
                Layout::from_size_align_unchecked(new_size, old_layout.align())
            );
            
            match allocation {
                Ok(allocd_ptr) => {
                    ptr::copy_nonoverlapping(ptr, allocd_ptr.as_ptr(), old_layout.size());
                    self.dealloc(ptr, old_layout);
                    allocd_ptr.as_ptr()
                },
                Err(_) => ptr::null_mut(),
            }
        } else {
            self.lock().shrink(
                NonNull::new_unchecked(ptr), 
                old_layout, 
                Layout::from_size_align_unchecked(new_size, old_layout.align())
            );
            ptr
        }
    }
}

#[cfg(feature = "allocator")]
unsafe impl<const BIAS: usize> Allocator for Tallock<BIAS> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() != 0 {
            unsafe {
                self.lock().alloc(layout).map(|nn| 
                    NonNull::slice_from_raw_parts(nn, layout.size())
                )
            }
        } else {
            Ok(NonNull::slice_from_raw_parts(layout.dangling(), 0))
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        if layout.size() != 0 {
            self.lock().dealloc(ptr, layout)
        }
    }

    unsafe fn shrink(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout)
    -> Result<NonNull<[u8]>, AllocError> {
        if new_layout.size() != 0 {

            // we might need to reallocate here if the required alignment got bigger
            if core::intrinsics::unlikely(new_layout.align() > old_layout.align()) {
                let t = self.lock();
                let old_size = t.layout_to_size(old_layout);
                let new_size = t.layout_to_size(new_layout);
                drop(t);

                if new_size > old_size {
                    let allocation = self.lock().alloc(new_layout)?;
                    allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), new_layout.size());
                    self.lock().dealloc(ptr, old_layout);

                    return Ok(NonNull::slice_from_raw_parts(allocation, new_layout.size()));
                }
            }

            // SAFETY: caller guaranteed
            self.lock().shrink(ptr, old_layout, new_layout);
            Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()))
        } else {
            self.deallocate(ptr, old_layout);
            Ok(NonNull::slice_from_raw_parts(new_layout.dangling(), 0))
        }
    }


    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let ptr = self.allocate(layout)?;
        // SAFETY: `alloc` returns a valid memory block
        unsafe { ptr.as_non_null_ptr().as_ptr().write_bytes(0, ptr.len()) }
        Ok(ptr)
    }

    unsafe fn grow(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(
            new_layout.size() >= old_layout.size(),
            "`new_layout.size()` must be greater than or equal to `old_layout.size()`"
        );

        // IS THIS WORTH IT?
        //if old_layout.size() > 0 {
        //    let t = self.lock();
        //    let old_size = t.layout_to_size(old_layout);
        //    let new_size = t.layout_to_size(new_layout);
        //    let sub_g = BIAS.min(t.llists.len() - t.g_of_size(old_size) - 1);
        //    drop(t);
    //
        //    let alignment = (old_size >> sub_g) - 1;
        //    let old_acme = (old_layout.size() + alignment) & !alignment;
    //
        //    if unlikely(old_acme >= new_size) {
        //        return Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()));
        //    }
        //}

        let new_ptr = self.allocate(new_layout)?;

        // SAFETY: because `new_layout.size()` must be greater than or equal to
        // `old_layout.size()`, both the old and new memory allocation are valid for reads and
        // writes for `old_layout.size()` bytes. Also, because the old allocation wasn't yet
        // deallocated, it cannot overlap `new_ptr`. Thus, the call to `copy_nonoverlapping` is
        // safe. The safety contract for `dealloc` must be upheld by the caller.
        unsafe {
            ptr::copy_nonoverlapping(ptr.as_ptr(), new_ptr.as_mut_ptr(), old_layout.size());
            self.deallocate(ptr, old_layout);
        }

        Ok(new_ptr)
    }

    unsafe fn grow_zeroed(&self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> Result<NonNull<[u8]>, AllocError> {

        let new_ptr = self.grow(ptr, old_layout, new_layout)?;

        new_ptr
            .as_mut_ptr()
            .wrapping_add(old_layout.size())
            .write_bytes(0, new_layout.size() - old_layout.size());

        Ok(new_ptr)
    }
}
