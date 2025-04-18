//! [`TalcCell`] allows using [`Talc`](crate::base::Talc) as a Rust allocator
//! for single-threaded unsynchronized locking.
//!
//! See [`TalcCell`].

use core::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    ptr::null_mut,
};

use crate::{
    base::binning::Binning,
    base::{Reserved, Talc},
    ptr_utils::nonnull_slice_from_raw_parts,
    src::Source,
};

use core::ptr::NonNull;

use allocator_api2::alloc::{AllocError, Allocator};

/// [`TalcCell`] implements [`GlobalAlloc`] and [`Allocator`]
/// without locking, but is [`!Sync`](Sync).
///
/// This type has similar semantics to a [`Cell`](core::cell::Cell).
///
/// # Example
/// ```rust
/// # #![cfg_attr(feature = "nightly", feature(allocator_api))]
/// # extern crate allocator_api2;
/// # extern crate talc;
///
/// use allocator_api2::alloc::{Allocator, Layout};
/// use allocator_api2::vec::Vec;
/// use talc::{TalcCell, Manual};
///
/// static mut HEAP: [u8; 2048] = [0; 2048];
///
/// let talc = TalcCell::new(Claim::array(&raw mut HEAP));
///
/// let mut my_vec = Vec::<u32, _>::with_capacity_in(42, &talc);
/// my_vec.push(123);
/// ```
///
/// # Safety
/// [`TalcCell`]'s API does not expose references to the inner [`Talc`] within
/// an [`UnsafeCell`] and is `!Sync`, so it's safe to mutate the inner data
/// through a shared reference.
///
/// There is an exception to this; a reference to the inner [`Talc`] is exposed to
/// sources. [`Source`] is thus an unsafe trait to implement, and the
/// implementation must uphold that they don't use the
/// [`TalcCell`]/[`TalcLock`](crate::sync::TalcLock) directly or indirectly
/// (e.g. calling `dbg!` in [`Source::resize`] when [`TalcLock`](crate::sync::TalcLock) is the global allocator)
/// in the implementation.
/// This requirement is not unique to [`TalcCell`].
/// If [`TalcLock`](crate::sync::TalcLock) is used in the source impl, it'll deadlock.
///
/// To help catch bad [`Source`] implementations, [`TalcCell`] tracks
/// borrows when `debug_assertions` are enabled, similar to a
/// [`RefCell`](core::cell::RefCell).
#[derive(Debug)]
pub struct TalcCell<S: Source, B: Binning> {
    cell: UnsafeCell<Talc<S, B>>,

    #[cfg(debug_assertions)]
    borrowed_at: core::cell::Cell<Option<&'static core::panic::Location<'static>>>,
}

impl<S: Source, B: Binning> TalcCell<S, B> {
    /// Create a new [`TalcCell`].
    #[inline]
    pub const fn new(src: S) -> Self {
        Self {
            cell: UnsafeCell::new(Talc::new(src)),

            #[cfg(debug_assertions)]
            borrowed_at: core::cell::Cell::new(None),
        }
    }

    /// Returns a mutable reference to the inner [`Talc`].
    #[inline]
    pub fn get_mut(&mut self) -> &mut Talc<S, B> {
        self.cell.get_mut()
    }

    /// Consumes the [`TalcCell`], returning the inner [`Talc`].
    #[inline]
    pub fn into_inner(self) -> Talc<S, B> {
        self.cell.into_inner()
    }

    /// Borrow the inner [`Talc`] mutably.
    ///
    /// # Safety
    /// Creating aliasing references must be avoided.
    /// [`TalcCell`] ensures against this in the following ways:
    ///
    /// - [`TalcCell`]'s functions do not call [`TalcCell::borrow`] more than once.
    /// - [`TalcCell`]'s functions do not call another [`TalcCell`] function while holding a [`BorrowedTalc`].
    /// - [`TalcCell`]'s API does not expose references to the inner [`Talc`].
    ///     - There is an exception to this. [`Source::acquire`] provides user
    ///         code with a mutable reference to the inner [`Talc`]. Implementing
    ///         [`Source`] is unsafe because the implementor must uphold that they
    ///         do not touch the outer [`TalcCell`] within the [`Source::acquire`]
    ///         implementation. [`TalcCell`] relies on this for correctness here.
    #[inline]
    #[track_caller]
    unsafe fn borrow(&self) -> BorrowedTalc<'_, S, B> {
        #[cfg(debug_assertions)]
        {
            if let Some(borrowed_at) = self.borrowed_at.take() {
                panic!(
                    "Tried to borrow the Talc, was borrowed previously at {}:{}:{}. Did the source attempt to use the TalcCell?",
                    borrowed_at.file(),
                    borrowed_at.line(),
                    borrowed_at.column(),
                );
            }

            self.borrowed_at.set(Some(core::panic::Location::caller()));
        }

        BorrowedTalc {
            ptr: unsafe { NonNull::new_unchecked(self.cell.get()) },
            _phantom: PhantomData,

            #[cfg(debug_assertions)]
            borrow_release: &self.borrowed_at,
        }
    }

    /// Swaps out the source for another.
    ///
    /// If you just want to clone the source, see [`TalcCell::clone_source`].
    #[inline]
    #[track_caller]
    pub fn replace_source(&self, src: S) -> S {
        unsafe {
            // SAFETY: See `Self::borrow`'s safety docs
            core::mem::replace(&mut self.borrow().source, src)
        }
    }

    /// Obtain the inner allocation statistics.
    #[cfg(feature = "counters")]
    #[inline]
    #[track_caller]
    pub fn counters(&self) -> crate::base::Counters {
        unsafe {
            // SAFETY: See `Self::borrow`'s safety docs
            self.borrow().counters().clone()
        }
    }

    #[inline]
    #[track_caller]
    pub unsafe fn reserved(&self, heap_end: *mut u8) -> Reserved {
        // SAFETY: See `Self::borrow`'s safety docs
        // SAFETY: `Talc` function safety requirements guaranteed by caller
        self.borrow().reserved(heap_end)
    }

    #[inline]
    #[track_caller]
    pub unsafe fn claim(&self, base: *mut u8, size: usize) -> Option<NonNull<u8>> {
        // SAFETY: See `Self::borrow`'s safety docs
        // SAFETY: `Talc` function safety requirements guaranteed by caller
        self.borrow().claim(base, size)
    }

    #[inline]
    #[track_caller]
    pub unsafe fn extend(&self, heap_end: *mut u8, new_end: *mut u8) -> NonNull<u8> {
        // SAFETY: See `Self::borrow`'s safety docs
        // SAFETY: `Talc` function safety requirements guaranteed by caller
        self.borrow().extend(heap_end, new_end)
    }

    #[inline]
    #[track_caller]
    pub unsafe fn truncate(&self, heap_end: *mut u8, new_end: *mut u8) -> Option<NonNull<u8>> {
        // SAFETY: See `Self::borrow`'s safety docs
        // SAFETY: `Talc` function safety requirements guaranteed by caller
        self.borrow().truncate(heap_end, new_end)
    }

    #[inline]
    #[track_caller]
    pub unsafe fn resize(&self, heap_end: *mut u8, new_end: *mut u8) -> Option<NonNull<u8>> {
        self.borrow().resize(heap_end, new_end)
    }
}

impl<S: Source + Clone, B: Binning> TalcCell<S, B> {
    /// Returns a clone of [`Talc`]'s source.
    ///
    /// To set the source instead, use [`TalcCell::replace_source`].
    #[inline]
    #[track_caller]
    pub fn clone_source(&self) -> S {
        unsafe {
            // SAFETY: See `Self::borrow`'s safety docs
            self.borrow().source.clone()
        }
    }
}

struct BorrowedTalc<'b, S: Source, B: Binning> {
    ptr: NonNull<Talc<S, B>>,
    _phantom: PhantomData<&'b ()>,

    #[cfg(debug_assertions)]
    borrow_release: &'b core::cell::Cell<Option<&'static core::panic::Location<'static>>>,
}
impl<S: Source, B: Binning> Drop for BorrowedTalc<'_, S, B> {
    #[inline]
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        {
            self.borrow_release.set(None);
        }
    }
}
impl<S: Source, B: Binning> Deref for BorrowedTalc<'_, S, B> {
    type Target = Talc<S, B>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}
impl<S: Source, B: Binning> DerefMut for BorrowedTalc<'_, S, B> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

unsafe impl<S: Source, B: Binning> GlobalAlloc for TalcCell<S, B> {
    #[inline]
    #[track_caller]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: See `Self::borrow`'s safety docs
        // SAFETY: guaranteed by caller
        self.borrow().allocate(layout).map_or(null_mut(), |nn| nn.as_ptr())
    }
    #[inline]
    #[track_caller]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: See `Self::borrow`'s safety docs
        // SAFETY: guaranteed by caller
        self.borrow().deallocate(ptr, layout)
    }

    #[inline]
    #[track_caller]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
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
        // SAFETY: See `Self::borrow`'s safety docs
        let mut talc = self.borrow();

        // SAFETY: guaranteed by caller that `ptr` was previously allocated by
        // this allocator given the layout `old_layout`.
        if talc.try_realloc_in_place(ptr, old_layout, new_size) {
            return ptr;
        }

        // grow in-place failed, reallocate manually

        // SAFETY: guaranteed by caller that `new_size` is a valid layout size
        let new_layout = Layout::from_size_align_unchecked(new_size, old_layout.align());

        // SAFETY: guaranteed by caller that `new_size` is nonzero
        let allocation = match talc.allocate(new_layout) {
            Some(ptr) => ptr.as_ptr(),
            None => return null_mut(),
        };

        // Shrink always succeeds, only growing the allocation might fail,
        // so the `old_layout.size() < new_size` here, and thus we just copy
        // all the old allocation bytes.
        allocation.copy_from_nonoverlapping(ptr, old_layout.size());

        talc.deallocate(ptr, old_layout);

        allocation
    }

    #[cfg(all(feature = "disable-grow-in-place", not(feature = "disable-realloc-in-place")))]
    #[track_caller]
    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: See `Self::borrow`'s safety docs
        let mut talc = self.borrow();

        if new_size <= old_layout.size() {
            // SAFETY: guaranteed by caller that `ptr` was previously allocated by
            // this allocator given the layout `old_layout`.
            talc.shrink(ptr, old_layout, new_size);
            return ptr;
        }

        // grow in-place failed, reallocate manually

        // SAFETY: guaranteed by caller that `new_size` is a valid layout size
        let new_layout = Layout::from_size_align_unchecked(new_size, old_layout.align());

        // SAFETY: guaranteed by caller that `new_size` is nonzero
        let Some(allocation) = talc.allocate(new_layout) else { return null_mut() };

        // Shrink always succeeds, only growing the allocation might fail,
        // so the `old_layout.size() < new_size` here, and thus we just copy
        // all the old allocation bytes.
        allocation.as_ptr().copy_from_nonoverlapping(ptr, old_layout.size());

        talc.deallocate(ptr, old_layout);

        allocation.as_ptr()
    }

    #[cfg(feature = "disable-realloc-in-place")]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller must ensure that the `new_size` does not overflow.
        // `layout.align()` comes from a `Layout` and is thus guaranteed to be valid.
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        // SAFETY: the caller must ensure that `new_layout` is greater than zero.
        let new_ptr = unsafe { self.alloc(new_layout) };
        if !new_ptr.is_null() {
            // SAFETY: the previously allocated block cannot overlap the newly allocated block.
            // The safety contract for `dealloc` must be upheld by the caller.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    ptr,
                    new_ptr,
                    core::cmp::min(layout.size(), new_size),
                );
                self.dealloc(ptr, layout);
            }
        }
        new_ptr
    }
}

unsafe impl<S: Source, B: Binning> Allocator for TalcCell<S, B> {
    #[inline]
    #[track_caller]
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() == 0 {
            let dangling = unsafe { NonNull::new_unchecked(layout.align() as *mut u8) };
            return Ok(nonnull_slice_from_raw_parts(dangling, layout.size()));
        }

        // SAFETY: See `Self::borrow`'s safety docs
        // SAFETY: Ensured the size is not zero above.
        match unsafe { self.borrow().allocate(layout) } {
            Some(allocation) => Ok(nonnull_slice_from_raw_parts(allocation, layout.size())),
            None => Err(AllocError),
        }
    }
    #[inline]
    #[track_caller]
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        if layout.size() != 0 {
            // SAFETY: See `Self::borrow`'s safety docs
            self.borrow().deallocate(ptr.as_ptr(), layout)
        }
    }

    #[inline]
    #[track_caller]
    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let ptr = self.allocate(layout)?;
        // SAFETY: `alloc` returns a valid memory block
        unsafe { ptr.cast::<u8>().as_ptr().write_bytes(0, ptr.len()) }
        Ok(ptr)
    }
    #[inline]
    #[track_caller]
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

    #[cfg(not(any(feature = "disable-grow-in-place", feature = "disable-realloc-in-place")))]
    #[track_caller]
    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(new_layout.size() >= old_layout.size());

        if old_layout.size() == 0 {
            return Allocator::allocate(self, new_layout);
        } else if crate::ptr_utils::is_aligned_to(ptr.as_ptr(), new_layout.align()) {
            // alignment is fine, try to allocate in-place
            // SAFETY: See `Self::borrow`'s safety docs
            if self.borrow().try_grow_in_place(ptr.as_ptr(), old_layout, new_layout.size()) {
                return Ok(nonnull_slice_from_raw_parts(ptr, new_layout.size()));
            }
        }

        // can't grow in place, reallocate manually
        // SAFETY: See `Self::borrow`'s safety docs
        let allocation = self.borrow().allocate(new_layout).ok_or(AllocError)?;
        allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), old_layout.size());
        // SAFETY: See `Self::borrow`'s safety docs
        self.borrow().deallocate(ptr.as_ptr(), old_layout);

        Ok(nonnull_slice_from_raw_parts(allocation, new_layout.size()))
    }

    // Default implementations

    #[cfg(any(feature = "disable-grow-in-place", feature = "disable-realloc-in-place"))]
    #[inline]
    #[track_caller]
    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(new_layout.size() >= old_layout.size());

        let new_ptr = self.allocate(new_layout)?;

        // SAFETY: because `new_layout.size()` must be greater than or equal to
        // `old_layout.size()`, both the old and new memory allocation are valid for reads and
        // writes for `old_layout.size()` bytes. Also, because the old allocation wasn't yet
        // deallocated, it cannot overlap `new_ptr`. Thus, the call to `copy_nonoverlapping` is
        // safe. The safety contract for `dealloc` must be upheld by the caller.
        unsafe {
            core::ptr::copy_nonoverlapping(
                ptr.as_ptr(),
                new_ptr.as_ptr().cast(),
                old_layout.size(),
            );
            self.deallocate(ptr, old_layout);
        }

        Ok(new_ptr)
    }

    #[cfg(not(feature = "disable-realloc-in-place"))]
    #[track_caller]
    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(new_layout.size() <= old_layout.size());

        // SAFETY: See `Self::borrow`'s safety docs
        let mut talc = self.borrow();

        if new_layout.size() == 0 {
            if old_layout.size() > 0 {
                talc.deallocate(ptr.as_ptr(), old_layout);
            }

            let dangling = unsafe { NonNull::new_unchecked(new_layout.align() as *mut u8) };
            return Ok(nonnull_slice_from_raw_parts(dangling, new_layout.size()));
        }

        if !crate::ptr_utils::is_aligned_to(ptr.as_ptr(), new_layout.align()) {
            let allocation = talc.allocate(new_layout).ok_or(AllocError)?;
            allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), new_layout.size());
            talc.deallocate(ptr.as_ptr(), old_layout);
            return Ok(nonnull_slice_from_raw_parts(allocation, new_layout.size()));
        }

        talc.shrink(ptr.as_ptr(), old_layout, new_layout.size());

        Ok(nonnull_slice_from_raw_parts(ptr, new_layout.size()))
    }

    #[cfg(feature = "disable-realloc-in-place")]
    #[track_caller]
    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert!(new_layout.size() <= old_layout.size());

        let new_ptr = self.allocate(new_layout)?;

        // SAFETY: because `new_layout.size()` must be lower than or equal to
        // `old_layout.size()`, both the old and new memory allocation are valid for reads and
        // writes for `new_layout.size()` bytes. Also, because the old allocation wasn't yet
        // deallocated, it cannot overlap `new_ptr`. Thus, the call to `copy_nonoverlapping` is
        // safe. The safety contract for `dealloc` must be upheld by the caller.
        unsafe {
            core::ptr::copy_nonoverlapping(
                ptr.as_ptr(),
                new_ptr.as_ptr().cast(),
                new_layout.size(),
            );
            self.deallocate(ptr, old_layout);
        }

        Ok(new_ptr)
    }
}

/// Wraps [`TalcCell`], but making it [`Sync`]. This easily leads to
/// unsoundness. Strongly consider [`TalcLock`](crate::sync::TalcLock) instead.
///
/// This type implements [`Self::new`] and [`GlobalAlloc`]
/// making it usable as a global allocator.
///
/// See [`TalcCellAssumeSingleThreaded::new`].
pub struct TalcCellAssumeSingleThreaded<S: Source, B: Binning>(TalcCell<S, B>);

unsafe impl<S: Source, B: Binning> Sync for TalcCellAssumeSingleThreaded<S, B> {}

impl<S: Source, B: Binning> TalcCellAssumeSingleThreaded<S, B> {
    /// Create a [`TalcCellAssumeSingleThreaded`] from a [`TalcCell`].
    ///
    /// [`TalcCellAssumeSingleThreaded`] is useful if your program is exclusively
    /// single-threaded (no multi-threading, no interrupts, no signal handling)
    /// and you want a global allocator that doesn't lock.
    ///
    /// This is primarily intended for use with atomic-less WebAssembly, where
    /// these requirements apply.
    ///
    /// Note that this is primarily for convenience. Contention-less
    /// locking is cheap. Strongly consider using a [`TalcLock`](crate::sync::TalcLock)
    /// with a spin-lock instead.
    ///
    /// # Safety
    /// [`TalcCellAssumeSingleThreaded`] is inherently unsafe by implementing
    /// [`Sync`] on [`TalcCell`], which has the semantics of a [`Cell`](core::cell::Cell).
    ///
    /// Calling a [`GlobalAlloc`] function on this type from two threads simultaneously is UB.
    ///
    /// # Example
    ///
    /// ```rust
    /// use talc::{Claim, TalcCell, DefaultBinning};
    ///
    /// #[global_allocator]
    /// static ALLOC: talc::cell::TalcCellAssumeSingleThreaded<Claim, DefaultBinning> = unsafe {
    ///     use core::mem::MaybeUninit;
    ///     static mut ARENA: [MaybeUninit<u8>; 100000] = [MaybeUninit::uninit(); 100000];
    ///     talc::cell::TalcCellAssumeSingleThreaded::new(TalcCell::new(Claim::array(&raw mut ARENA)))
    /// };
    /// ```
    pub const unsafe fn new(talc: TalcCell<S, B>) -> Self {
        Self(talc)
    }
}

unsafe impl<S: Source, B: Binning> GlobalAlloc for TalcCellAssumeSingleThreaded<S, B> {
    #[track_caller]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0.alloc(layout)
    }
    #[track_caller]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.dealloc(ptr, layout)
    }
    #[track_caller]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.0.realloc(ptr, layout, new_size)
    }
}
