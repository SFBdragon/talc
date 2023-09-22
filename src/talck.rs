use crate::{OomHandler, Talc};

use core::{
    alloc::{GlobalAlloc, Layout},
    cmp::Ordering,
    ptr::{self, NonNull},
};

#[cfg(feature = "allocator")]
use core::alloc::AllocError;


#[cfg(feature = "allocator")]
pub(crate) fn is_aligned_to(ptr: *mut u8, align: usize) -> bool {
    (ptr as usize).trailing_zeros() >= align.trailing_zeros()
}

/// Talc lock: wrapper struct containing a mutex-locked [`Talc`].
///
/// In order to access the [`Allocator`](core::alloc::Allocator) 
/// API, call [`allocator`](Talc::allocator).
///
/// # Example
/// ```rust
/// # use talc::*;
/// let talc = Talc::new(ErrOnOom);
/// let talck = talc.lock::<spin::Mutex<()>>();
/// ```
#[derive(Debug)]
pub struct Talck<R: lock_api::RawMutex, O: OomHandler>(pub lock_api::Mutex<R, Talc<O>>);

impl<R: lock_api::RawMutex, O: OomHandler> Talck<R, O> {
    /// Get a reference that implements the [`Allocator`](core::alloc::Allocator) API.
    #[cfg(feature = "allocator")]
    pub fn allocator(&self) -> TalckRef<'_, R, O> {
        TalckRef(self)
    }

    /// Lock the mutex and access the inner `Talc`.
    pub fn talc(&self) -> lock_api::MutexGuard<'_, R, Talc<O>> {
        self.0.lock()
    }
}

unsafe impl<R: lock_api::RawMutex, O: OomHandler> GlobalAlloc for Talck<R, O> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0.lock().malloc(layout).map_or(ptr::null_mut(), |nn: _| nn.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.lock().free(NonNull::new_unchecked(ptr), layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        match layout.size().cmp(&new_size) {
            Ordering::Less => self
                .0
                .lock()
                .grow(NonNull::new_unchecked(ptr), layout, new_size)
                .map_or(ptr::null_mut(), |nn| nn.as_ptr()),

            Ordering::Greater => {
                self.0.lock().shrink(NonNull::new_unchecked(ptr), layout, new_size);
                ptr
            }

            Ordering::Equal => ptr,
        }
    }
}

/// A reference to a [`Talck`] that implements the [`Allocator`](core::alloc::Allocator) trait.
#[cfg(feature = "allocator")]
#[derive(Debug, Clone, Copy)]
pub struct TalckRef<'a, R: lock_api::RawMutex, O: OomHandler>(pub &'a Talck<R, O>);

#[cfg(feature = "allocator")]
unsafe impl<'a, R: lock_api::RawMutex, O: OomHandler> core::alloc::Allocator
    for TalckRef<'a, R, O>
{
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, core::alloc::AllocError> {
        if layout.size() == 0 {
            return Ok(NonNull::slice_from_raw_parts(NonNull::dangling(), 0));
        }

        unsafe { self.0.0.lock().malloc(layout) }
            .map(|nn| NonNull::slice_from_raw_parts(nn, layout.size()))
            .map_err(|_| AllocError)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        if layout.size() != 0 {
            self.0.0.lock().free(ptr, layout);
        }
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, core::alloc::AllocError> {
        debug_assert!(new_layout.size() >= old_layout.size());

        if old_layout.size() == 0 {
            return self.allocate(new_layout);
        }

        if !is_aligned_to(ptr.as_ptr(), new_layout.align()) {
            let allocation = self.0.0.lock().malloc(new_layout).map_err(|_| AllocError)?;
            allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), new_layout.size());
            self.0.0.lock().free(ptr, old_layout);
            return Ok(NonNull::slice_from_raw_parts(allocation, new_layout.size()));
        }

        self.0
            .0
            .lock()
            .grow(ptr, old_layout, new_layout.size())
            .map(|nn| NonNull::slice_from_raw_parts(nn, new_layout.size()))
            .map_err(|_| AllocError)
    }

    unsafe fn grow_zeroed(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, core::alloc::AllocError> {
        let res = self.grow(ptr, old_layout, new_layout);

        if let Ok(allocation) = res {
            (allocation.as_ptr() as *mut u8)
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
    ) -> Result<NonNull<[u8]>, core::alloc::AllocError> {
        debug_assert!(new_layout.size() <= old_layout.size());

        if new_layout.size() == 0 {
            if old_layout.size() > 0 {
                self.0.0.lock().free(ptr, old_layout);
            }

            return Ok(NonNull::slice_from_raw_parts(NonNull::dangling(), 0));
        }

        if !is_aligned_to(ptr.as_ptr(), new_layout.align()) {
            let allocation = self.0.0.lock().malloc(new_layout).map_err(|_| AllocError)?;
            allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), new_layout.size());
            self.0.0.lock().free(ptr, old_layout);
            return Ok(NonNull::slice_from_raw_parts(allocation, new_layout.size()));
        }

        self.0.0.lock().shrink(ptr, old_layout, new_layout.size());

        Ok(NonNull::slice_from_raw_parts(ptr, new_layout.size()))
    }
}

/// A dummy RawMutex implementation to skip synchronization on single threaded systems.
///
/// # Safety
/// This is very unsafe and may cause undefined behaviour if multiple threads enter
/// a critical section syncronized by this, even without explicit unsafe code.
pub struct AssumeUnlockable;

// SAFETY: nope
unsafe impl lock_api::RawMutex for AssumeUnlockable {
    const INIT: AssumeUnlockable = AssumeUnlockable;

    // A spinlock guard can be sent to another thread and unlocked there
    type GuardMarker = lock_api::GuardSend;

    fn lock(&self) {}

    fn try_lock(&self) -> bool {
        true
    }

    unsafe fn unlock(&self) {}
}
