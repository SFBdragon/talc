use core::alloc::GlobalAlloc;

use crate::{base::Talc, oom::OomHandler, Binning};



pub trait AsTalc<O: OomHandler<B>, B: Binning> {
    fn as_talc(&mut self) -> &mut Talc<O, B>;
}

macro_rules! impl_global_alloc {
    ($talc:ty) => {
        unsafe impl<O: ::crate::OomHandler<B>, B: ::crate::Binning, T: AsTalc<O, B>> GlobalAlloc for T {
            #[inline]
            #[track_caller]
            unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
                talc.as_talc().allocate(layout).unwrap_or(core::ptr::null_mut(), |nn| nn.as_ptr())
            }
        
            #[inline]
            #[track_caller]
            unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
                talc.as_talc().deallocate(ptr, layout);
            }
            
            #[inline]
            #[track_caller]
            unsafe fn alloc_zeroed(&self, layout: core::alloc::Layout) -> *mut u8 {
                let size = layout.size();
                // SAFETY: the safety contract for `alloc` must be upheld by the caller.
                let ptr = unsafe { self.alloc(layout) };
                if !ptr.is_null() {
                    // SAFETY: as allocation succeeded, the region from `ptr`
                    // of size `size` is guaranteed to be valid for writes.
                    unsafe { core::ptr::write_bytes(ptr, 0, size) };
                }
                ptr
            }

            #[cfg(not(any(feature = "disable-grow-in-place", feature = "disable-realloc-in-place")))]
            #[track_caller]
            unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
                let mut lock = self.lock();

                // SAFETY: guaranteed by caller that `ptr` was previously allocated by
                // this allocator given the layout `old_layout`.
                if lock.try_realloc_in_place(ptr, old_layout, new_size) {
                    return ptr;
                }

                // grow in-place failed, reallocate manually

                // SAFETY: guaranteed by caller that `new_size` is a valid layout size
                let new_layout = Layout::from_size_align_unchecked(new_size, old_layout.align());

                // SAFETY: guaranteed by caller that `new_size` is nonzero
                let allocation = match lock.allocate(new_layout) {
                    Some(ptr) => ptr.as_ptr(),
                    None => return null_mut(),
                };

                // Shrink always succeeds, only growing the allocation might fail,
                // so the `old_layout.size() < new_size` here, and thus we just copy
                // all the old allocation bytes.

                if old_layout.size() > RELEASE_LOCK_ON_REALLOC_LIMIT {
                    drop(lock);
                    allocation.copy_from_nonoverlapping(ptr, old_layout.size());
                    lock = self.lock();
                } else {
                    allocation.copy_from_nonoverlapping(ptr, old_layout.size());
                }

                lock.deallocate(ptr, old_layout);

                allocation
            }

            #[cfg(all(feature = "disable-grow-in-place", not(feature = "disable-realloc-in-place")))]
            #[track_caller]
            unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
                let mut lock = self.lock();

                if new_size <= old_layout.size() {
                    // SAFETY: guaranteed by caller that `ptr` was previously allocated by
                    // this allocator given the layout `old_layout`.
                    lock.shrink(ptr, old_layout, new_size);
                    return ptr;
                }

                // grow in-place failed, reallocate manually

                // SAFETY: guaranteed by caller that `new_size` is a valid layout size
                let new_layout = Layout::from_size_align_unchecked(new_size, old_layout.align());

                // SAFETY: guaranteed by caller that `new_size` is nonzero
                let allocation = match lock.allocate(new_layout) {
                    Some(ptr) => ptr.as_ptr(),
                    None => return null_mut(),
                };

                // Shrink always succeeds, only growing the allocation might fail,
                // so the `old_layout.size() < new_size` here, and thus we just copy
                // all the old allocation bytes.

                if old_layout.size() > RELEASE_LOCK_ON_REALLOC_LIMIT {
                    drop(lock);
                    allocation.copy_from_nonoverlapping(ptr, old_layout.size());
                    lock = self.lock();
                } else {
                    allocation.copy_from_nonoverlapping(ptr, old_layout.size());
                }

                lock.deallocate(ptr, old_layout);

                allocation
            }

            #[cfg(feature = "disable-realloc-in-place")]
            unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
                let mut lock = self.lock();

                // SAFETY: the caller must ensure that the `new_size` does not overflow.
                // `layout.align()` comes from a `Layout` and is thus guaranteed to be valid.
                let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, old_layout.align()) };
                // SAFETY: the caller must ensure that `new_layout` is greater than zero.
                let allocation = match unsafe { lock.allocate(new_layout) } {
                    Some(new_ptr) => new_ptr.as_ptr(),
                    None => return null_mut(),
                };

                // SAFETY: the previously allocated block cannot overlap the newly allocated block.
                // The safety contract for `dealloc` must be upheld by the caller.
                unsafe {
                    let copy_count = core::cmp::min(old_layout.size(), new_size);

                    if copy_count > RELEASE_LOCK_ON_REALLOC_LIMIT {
                        drop(lock);
                        allocation.copy_from_nonoverlapping(ptr, copy_count);
                        lock = self.lock();
                    } else {
                        allocation.copy_from_nonoverlapping(ptr, copy_count);
                    }

                    core::ptr::copy_nonoverlapping(ptr, allocation, copy_count);
                    lock.deallocate(ptr, old_layout);
                }

                allocation
            }
        }
        
    };
}

unsafe impl<O: OomHandler<B>, B: Binning, T: AsTalc<O, B>> GlobalAlloc for T {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        todo!()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        todo!()
    }
    
    unsafe fn alloc_zeroed(&self, layout: core::alloc::Layout) -> *mut u8 {
        let size = layout.size();
        // SAFETY: the safety contract for `alloc` must be upheld by the caller.
        let ptr = unsafe { self.alloc(layout) };
        if !ptr.is_null() {
            // SAFETY: as allocation succeeded, the region from `ptr`
            // of size `size` is guaranteed to be valid for writes.
            unsafe { core::ptr::write_bytes(ptr, 0, size) };
        }
        ptr
    }
    
    unsafe fn realloc(&self, ptr: *mut u8, layout: core::alloc::Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller must ensure that the `new_size` does not overflow.
        // `layout.align()` comes from a `Layout` and is thus guaranteed to be valid.
        let new_layout = unsafe { core::alloc::Layout::from_size_align_unchecked(new_size, layout.align()) };
        // SAFETY: the caller must ensure that `new_layout` is greater than zero.
        let new_ptr = unsafe { self.alloc(new_layout) };
        if !new_ptr.is_null() {
            // SAFETY: the previously allocated block cannot overlap the newly allocated block.
            // The safety contract for `dealloc` must be upheld by the caller.
            unsafe {
                core::ptr::copy_nonoverlapping(ptr, new_ptr, core::cmp::min(layout.size(), new_size));
                self.dealloc(ptr, layout);
            }
        }
        new_ptr
    }
}
