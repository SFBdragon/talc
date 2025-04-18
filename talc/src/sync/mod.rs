//! [`TalcLock`] facilitates using [`Talc`](crate::base::Talc) as a Rust
//! global allocator, or other usage across multiple threads.
//!
//! See [`TalcLock`].

use core::ptr::{NonNull, null_mut};

use allocator_api2::alloc::{AllocError, Allocator, GlobalAlloc, Layout};

use crate::{base::Talc, base::binning::Binning, src::Source};

#[doc(hidden)]
#[cfg(all(feature = "system-backed", target_family = "unix"))]
pub mod unix;

#[doc(hidden)]
#[cfg(all(feature = "system-backed", target_family = "windows"))]
pub mod win;

const RELEASE_LOCK_ON_REALLOC_LIMIT: usize = 0x4000;

/// Wraps a mutex-locked [`Talc`].
///
/// # Example
/// ```rust
/// # use talc::Manual;
/// use spin::Mutex;
///
/// let talc = talc::TalcLock::<Mutex<()>, _>::new(talc::src::Global(std::alloc::System)); // TODO
/// ```
#[derive(Debug)]
pub struct TalcLock<R: lock_api::RawMutex, S: Source, B: Binning> {
    mutex: lock_api::Mutex<R, Talc<S, B>>,
}

impl<R: lock_api::RawMutex, S: Source, B: Binning> TalcLock<R, S, B> {
    /// Create a new [`TalcLock`].
    pub const fn new(src: S) -> Self {
        Self { mutex: lock_api::Mutex::new(Talc::new(src)) }
    }

    /// Lock the mutex and access the inner [`Talc`].
    #[track_caller]
    pub fn lock(&self) -> lock_api::MutexGuard<R, Talc<S, B>> {
        self.mutex.lock()
    }

    /// Try to lock the mutex and access the inner [`Talc`].
    pub fn try_lock(&self) -> Option<lock_api::MutexGuard<R, Talc<S, B>>> {
        self.mutex.try_lock()
    }

    /// Returns a mutable reference to the inner [`Talc`].
    ///
    /// This avoids locking, as having a mutable reference statically
    /// guarantees that `self` is not locked.
    pub fn get_mut(&mut self) -> &mut Talc<S, B> {
        self.mutex.get_mut()
    }

    /// Retrieve the inner [`Talc`].
    pub fn into_inner(self) -> Talc<S, B> {
        self.mutex.into_inner()
    }
}

unsafe impl<R: lock_api::RawMutex, S: Source, B: Binning> GlobalAlloc for TalcLock<R, S, B> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.lock().allocate(layout).map_or(null_mut(), |nn| nn.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.lock().deallocate(ptr, layout)
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

#[cfg_attr(feature = "disable-realloc-in-place", expect(dead_code))]
#[inline(always)]
fn is_aligned_to(ptr: *mut u8, align: usize) -> bool {
    (ptr as usize).trailing_zeros() >= align.trailing_zeros()
}

#[inline(always)]
fn nonnull_slice_from_raw_parts(nn: NonNull<u8>, len: usize) -> NonNull<[u8]> {
    // SAFETY: if `nn` is non-null, then the resulting slice is non-null
    unsafe { NonNull::new_unchecked(core::ptr::slice_from_raw_parts_mut(nn.as_ptr(), len)) }
}

unsafe impl<R: lock_api::RawMutex, S: Source, B: Binning> Allocator for TalcLock<R, S, B> {
    #[inline]
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() == 0 {
            return Ok(nonnull_slice_from_raw_parts(NonNull::dangling(), 0));
        }

        // SAFETY: Ensured the size is not zero above.
        unsafe { self.lock().allocate(layout) }
            .map(|nn| nonnull_slice_from_raw_parts(nn, layout.size()))
            .ok_or(AllocError)
    }
    #[inline]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        if layout.size() != 0 {
            self.lock().deallocate(ptr.as_ptr(), layout);
        }
    }

    #[cfg(not(any(feature = "disable-grow-in-place", feature = "disable-realloc-in-place")))]
    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(new_layout.size() >= old_layout.size());

        if old_layout.size() == 0 {
            return self.allocate(new_layout);
        } else if is_aligned_to(ptr.as_ptr(), new_layout.align()) {
            // alignment is fine, try to allocate in-place
            if self.lock().try_grow_in_place(ptr.as_ptr(), old_layout, new_layout.size()) {
                return Ok(nonnull_slice_from_raw_parts(ptr, new_layout.size()));
            }
        }

        // can't grow in place, reallocate manually

        let mut lock = self.lock();
        let allocation = lock.allocate(new_layout).ok_or(AllocError)?;

        if old_layout.size() > RELEASE_LOCK_ON_REALLOC_LIMIT {
            drop(lock);
            allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), old_layout.size());
            lock = self.lock();
        } else {
            allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), old_layout.size());
        }

        lock.deallocate(ptr.as_ptr(), old_layout);

        Ok(nonnull_slice_from_raw_parts(allocation, new_layout.size()))
    }

    unsafe fn grow_zeroed(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let res = self.grow(ptr, old_layout, new_layout);

        if let Ok(allocation) = res {
            allocation
                .as_ptr()
                .cast::<u8>()
                .add(old_layout.size())
                .write_bytes(0, new_layout.size() - old_layout.size());
        }

        res
    }

    #[cfg(not(feature = "disable-realloc-in-place"))]
    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(new_layout.size() <= old_layout.size());

        if new_layout.size() == 0 {
            if old_layout.size() > 0 {
                self.lock().deallocate(ptr.as_ptr(), old_layout);
            }

            return Ok(nonnull_slice_from_raw_parts(NonNull::dangling(), 0));
        }

        if !is_aligned_to(ptr.as_ptr(), new_layout.align()) {
            let mut lock = self.lock();
            let allocation = lock.allocate(new_layout).ok_or(AllocError)?;

            if new_layout.size() > RELEASE_LOCK_ON_REALLOC_LIMIT {
                drop(lock);
                allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), new_layout.size());
                lock = self.lock();
            } else {
                allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), new_layout.size());
            }

            lock.deallocate(ptr.as_ptr(), old_layout);
            return Ok(nonnull_slice_from_raw_parts(allocation, new_layout.size()));
        }

        self.lock().shrink(ptr.as_ptr(), old_layout, new_layout.size());

        Ok(nonnull_slice_from_raw_parts(ptr, new_layout.size()))
    }
}
