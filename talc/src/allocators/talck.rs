//! Home of Talck, a mutex-locked wrapper of Talc.

use crate::talc::{alignment::ChunkAlign, bucket_config::BucketConfig, oom_handler::OomHandler, Talc};

use core::{
    alloc::{GlobalAlloc, Layout},
    cmp::Ordering,
    ptr::null_mut,
};

#[cfg(feature = "allocator_api")]
use core::alloc::{AllocError, Allocator};

#[cfg(all(feature = "allocator-api2", not(feature = "allocator_api")))]
use allocator_api2::alloc::{AllocError, Allocator};

#[cfg(any(feature = "allocator_api", feature = "allocator-api2"))]
pub(crate) fn is_aligned_to(ptr: *mut u8, align: usize) -> bool {
    (ptr as usize).trailing_zeros() >= align.trailing_zeros()
}

const RELEASE_LOCK_ON_REALLOC_LIMIT: usize = 0x10000;

/// Talc lock, contains a mutex-locked [`Talc`].
///
/// # Example
/// ```rust
/// # use talc::*;
/// let talc = Talc::new(ErrOnOom);
/// let talck = talc.lock::<spin::Mutex<()>>();
/// ```
#[derive(Debug)]
pub struct Talck<R: lock_api::RawMutex, O: OomHandler<Cfg, A>, Cfg: BucketConfig, A: ChunkAlign> {
    mutex: lock_api::Mutex<R, Talc<O, Cfg, A>>,
}

impl<R: lock_api::RawMutex, O: OomHandler<Cfg, A>, Cfg: BucketConfig, A: ChunkAlign> Talck<R, O, Cfg, A> {
    /// Create a new `Talck`.
    pub const fn new(talc: Talc<O, Cfg, A>) -> Self {
        Self { mutex: lock_api::Mutex::new(talc) }
    }

    /// Lock the mutex and access the inner `Talc`.
    pub fn lock(&self) -> lock_api::MutexGuard<R, Talc<O, Cfg, A>> {
        self.mutex.lock()
    }

    /// Try to lock the mutex and access the inner `Talc`.
    pub fn try_lock(&self) -> Option<lock_api::MutexGuard<R, Talc<O, Cfg, A>>> {
        self.mutex.try_lock()
    }

    /// Retrieve the inner `Talc`.
    pub fn into_inner(self) -> Talc<O, Cfg, A> {
        self.mutex.into_inner()
    }
}

unsafe impl<R: lock_api::RawMutex, O: OomHandler<Cfg, A>, Cfg: BucketConfig, A: ChunkAlign> GlobalAlloc for Talck<R, O, Cfg, A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.lock().alloc(layout).map_or(null_mut(), |nn| nn.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.lock().dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        match new_size.cmp(&old_layout.size()) {
            Ordering::Greater => {
                // first try to grow in-place before manually re-allocating

                if let Ok(nn) = self.lock().grow_in_place(ptr, old_layout, new_size) {
                    return nn.as_ptr();
                }

                // grow in-place failed, reallocate manually

                let new_layout = Layout::from_size_align_unchecked(new_size, old_layout.align());

                let mut lock = self.lock();
                let allocation = match lock.alloc(new_layout) {
                    Ok(ptr) => ptr,
                    Err(_) => return null_mut(),
                };

                if old_layout.size() > RELEASE_LOCK_ON_REALLOC_LIMIT {
                    drop(lock);
                    allocation.as_ptr().copy_from_nonoverlapping(ptr, old_layout.size());
                    lock = self.lock();
                } else {
                    allocation.as_ptr().copy_from_nonoverlapping(ptr, old_layout.size());
                }

                lock.dealloc(ptr, old_layout);
                allocation.as_ptr()
            }

            Ordering::Less => {
                self.lock().shrink(ptr, old_layout, new_size);
                ptr
            }

            Ordering::Equal => ptr,
        }
    }
}

/// Convert a nonnull and length to a nonnull slice.
#[cfg(any(feature = "allocator_api", feature = "allocator-api2"))]
fn nonnull_slice_from_raw_parts(ptr: NonNull<u8>, len: usize) -> NonNull<[u8]> {
    unsafe { NonNull::new_unchecked(core::ptr::slice_from_raw_parts_mut(ptr.as_ptr(), len)) }
}

#[cfg(any(feature = "allocator_api", feature = "allocator-api2"))]
unsafe impl<R: lock_api::RawMutex, O: OomHandler> Allocator for Talck<R, O> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() == 0 {
            return Ok(nonnull_slice_from_raw_parts(NonNull::dangling(), 0));
        }

        unsafe { self.lock().malloc(layout) }
            .map(|nn| nonnull_slice_from_raw_parts(nn, layout.size()))
            .map_err(|_| AllocError)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        if layout.size() != 0 {
            self.lock().free(ptr, layout);
        }
    }

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
            if let Ok(nn) = self.lock().grow_in_place(ptr, old_layout, new_layout.size()) {
                return Ok(nonnull_slice_from_raw_parts(nn, new_layout.size()));
            }
        }

        // can't grow in place, reallocate manually

        let mut lock = self.lock();
        let allocation = lock.malloc(new_layout).map_err(|_| AllocError)?;

        if old_layout.size() > RELEASE_LOCK_ON_REALLOC_LIMIT {
            drop(lock);
            allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), old_layout.size());
            lock = self.lock();
        } else {
            allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), old_layout.size());
        }

        lock.free(ptr, old_layout);

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

    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(new_layout.size() <= old_layout.size());

        if new_layout.size() == 0 {
            if old_layout.size() > 0 {
                self.lock().free(ptr, old_layout);
            }

            return Ok(nonnull_slice_from_raw_parts(NonNull::dangling(), 0));
        }

        if !is_aligned_to(ptr.as_ptr(), new_layout.align()) {
            let mut lock = self.lock();
            let allocation = lock.malloc(new_layout).map_err(|_| AllocError)?;

            if new_layout.size() > RELEASE_LOCK_ON_REALLOC_LIMIT {
                drop(lock);
                allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), new_layout.size());
                lock = self.lock();
            } else {
                allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), new_layout.size());
            }

            lock.free(ptr, old_layout);
            return Ok(nonnull_slice_from_raw_parts(allocation, new_layout.size()));
        }

        self.lock().shrink(ptr, old_layout, new_layout.size());

        Ok(nonnull_slice_from_raw_parts(ptr, new_layout.size()))
    }
}

impl< O: OomHandler<Cfg, A>, Cfg: BucketConfig, A: ChunkAlign> Talc<O, Cfg, A> {
    /// Wrap in `Talck`, a mutex-locked wrapper struct using [`lock_api`].
    ///
    /// This implements the [`GlobalAlloc`](core::alloc::GlobalAlloc) trait and provides
    /// access to the [`Allocator`](core::alloc::Allocator) API.
    ///
    /// # Examples
    /// ```
    /// # use talc::*;
    /// # use core::alloc::{GlobalAlloc, Layout};
    /// use spin::Mutex;
    /// let talc = Talc::new(ErrOnOom);
    /// let talck = talc.lock::<Mutex<()>>();
    ///
    /// unsafe {
    ///     talck.alloc(Layout::from_size_align_unchecked(32, 4));
    /// }
    /// ```
    pub const fn lock<R: lock_api::RawMutex>(self) -> Talck<R, O, Cfg, A> {
        Talck::new(self)
    }
}

#[cfg(all(target_family = "wasm"))]
impl TalckWasm {
    /// Create a [`Talck`] instance that takes control of WASM memory management.
    ///
    /// # Safety
    /// The runtime environment must be single-threaded WASM.
    ///
    /// Note: calls to memory.grow during use of the allocator is allowed.
    pub const unsafe fn new_global() -> Self {
        Talc::new(crate::WasmHandler::new()).lock()
    }
}

#[cfg(all(target_family = "wasm"))]
pub type TalckWasm = Talck<crate::locking::AssumeUnlockable, crate::WasmHandler>;
