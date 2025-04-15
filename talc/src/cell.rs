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

use crate::{Binning, base::Talc, oom::OomHandler, ptr_utils::nonnull_slice_from_raw_parts};

use core::ptr::NonNull;

use allocator_api2::alloc::{AllocError, Allocator};

/// [`TalcCell`] implements [`GlobalAlloc`] and [`Allocator`] without locking,
/// but is [`!Sync`](Sync).
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
/// use talc::{TalcCell, ErrOnOom};
///
/// static mut ARENA: [u8; 2048] = [0; 2048];
///
/// let talc = TalcCell::new(ErrOnOom);
/// let arena = unsafe { talc.claim(ARENA.as_mut_ptr().cast(), ARENA.len()).unwrap() };
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
/// OOM handlers. The OOM handler is thus an unsafe trait to implement, and the
/// implementation must uphold that they don't use the
/// [`TalcCell`]/[`Talck`](crate::sync::Talck) directly or indirectly
/// (e.g. resizing a `Vec` when [`Talck`](crate::sync::Talck) is the global allocator)
/// in the [`OomHandler::handle_oom`] implementation.
/// This requirement is not unique to [`TalcCell`] however. If
/// [`Talck`](crate::sync::Talck) is used in the OOM handler, this will cause a deadlock.
///
/// To help catch bad OOM handler implementations, [`TalcCell`] tracks
/// borrows when `debug_assertions` are enabled, similar to a
/// [`RefCell`](core::cell::RefCell).
#[derive(Debug)]
pub struct TalcCell<O: OomHandler<B>, B: Binning> {
    cell: UnsafeCell<Talc<O, B>>,

    #[cfg(debug_assertions)]
    borrowed_at: core::cell::Cell<Option<&'static core::panic::Location<'static>>>,
}

impl<O: OomHandler<B>, B: Binning> TalcCell<O, B> {
    /// Create a new [`TalcCell`].
    #[inline]
    pub const fn new(oom_handler: O) -> Self {
        Self {
            cell: UnsafeCell::new(Talc::new(oom_handler)),

            #[cfg(debug_assertions)]
            borrowed_at: core::cell::Cell::new(None),
        }
    }

    /// Returns a mutable reference to the inner [`Talc`].
    #[inline]
    pub fn get_mut(&mut self) -> &mut Talc<O, B> {
        self.cell.get_mut()
    }

    /// Consumes the [`TalcCell`], returning the inner [`Talc`].
    #[inline]
    pub fn into_inner(self) -> Talc<O, B> {
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
    ///     - There is an exception to this. [`OomHandler::handle_oom`] provides user
    ///         code with a mutable reference to the inner [`Talc`]. Implementing
    ///         [`OomHandler`] is unsafe because the implementor must uphold that they
    ///         do not touch the outer [`TalcCell`] within the [`OomHandler::handle_oom`]
    ///         implementation. [`TalcCell`] relies on this for correctness here.
    #[inline]
    #[track_caller]
    unsafe fn borrow(&self) -> BorrowedTalc<'_, O, B> {
        #[cfg(debug_assertions)]
        {
            if let Some(borrowed_at) = self.borrowed_at.take() {
                panic!(
                    "Tried to borrow the Talc, was borrowed previously at {}:{}:{}. Did the OOM handler attempt to use the TalcCell?",
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

    /// Swaps out the [`Talc`]'s OOM handler for another.
    #[inline]
    #[track_caller]
    pub fn replace_oom_handler(&self, oom_handler: O) -> O {
        unsafe {
            // SAFETY: See `Self::borrow`'s safety docs
            core::mem::replace(&mut self.borrow().oom_handler, oom_handler)
        }
    }

    /// Obtain a clone of the inner allocation statistics.
    #[cfg(feature = "counters")]
    #[inline]
    #[track_caller]
    pub fn counters(&self) -> crate::base::Counters {
        unsafe {
            // SAFETY: See `Self::borrow`'s safety docs
            self.borrow().counters().clone()
        }
    }

    /// Returns the extent of reserved bytes in `arena`.
    ///
    /// `arena.base()..arena.base().add(talc.reserved(&arena))`
    /// are reserved due to allocations in the arena using this memory.
    /// [`Talc::truncate`] and [`Talc::resize`] will not release these bytes.
    ///
    ///
    /// ```not_rust
    ///
    ///     ├──Arena──────────────────────────────────┤
    /// ────┬─────┬───────────┬─────┬───────────┬─────┬────
    /// ... | Gap | Allocated | Gap | Allocated | Gap | ...
    /// ────┴─────┴───────────┴─────┴───────────┴─────┴────
    ///     ├──Reserved─────────────────────────┤
    ///
    /// ```
    ///
    /// # Safety
    /// - `arena` must be managed by this instance of the allocator.
    #[inline]
    #[track_caller]
    pub unsafe fn reserved(&self, arena_acme: *mut u8) -> Option<NonNull<u8>> {
        // SAFETY: See `Self::borrow`'s safety docs
        // SAFETY: `Talc` function safety requirements guaranteed by caller
        self.borrow().reserved(arena_acme)
    }

    /// Establish a new [`Arena`] to allocate into.
    ///
    /// This does not "combine" with neighboring arenas. Use [`TalcCell::extend`] to achieve this.
    ///
    /// Due to alignment requirements, the resulting [`Arena`] may be slightly smaller
    /// than the provided memory on either side. The resulting [`Arena`] can and will not have
    /// well-aligned boundaries though.
    ///
    /// # Failure modes
    ///
    /// The first [`Arena`] needs to hold [`Talc`]'s allocation metadata,
    /// this has a fixed size that depends on the [`Binning`] configuration.
    /// Currently, it's a little more than `BIN_COUNT * size_of::<usize>()`
    /// but this is subject to change.
    ///
    /// Use [`min_first_arena_layout`](crate::min_first_arena_layout) or
    /// [`min_first_arena_size`](crate::min_first_arena_size) to guarantee a
    /// successful first claim.
    /// Using a large constant is fine too.
    /// The size requirement won't more-than-quadruple without a major version bump.
    ///
    /// Once the first [`Arena`] is established, the allocation metadata permanently
    /// reserves the start of that [`Arena`] and all subsequent claims are subject to
    /// a much less stringent requirement: `None` is returned only if `size` is too
    /// small to tag the base and have enough left over to fit a chunk.
    ///
    /// # Safety
    /// The region of memory described by `base` and `size` must be exclusively writable
    /// by the allocator, up until the memory is released with [`TalcCell::truncate`]
    /// or the allocator is no longer active.
    ///
    /// This rule does not apply to memory that will be allocated by `self`.
    /// That's the caller's memory until deallocated.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate talc;
    /// # use talc::{TalcCell, ErrOnOom};
    /// static mut ARENA: [u8; 5000] = [0; 5000];
    ///
    /// let talc = TalcCell::new(ErrOnOom);
    /// let arena = unsafe { talc.claim((&raw mut ARENA).cast(), 5000).unwrap() };
    /// ```
    #[inline]
    #[track_caller]
    pub unsafe fn claim(&self, base: *mut u8, size: usize) -> Option<NonNull<u8>> {
        // SAFETY: See `Self::borrow`'s safety docs
        // SAFETY: `Talc` function safety requirements guaranteed by caller
        self.borrow().claim(base, size)
    }

    /// Extend the `arena`'s up to `new_size`.
    ///
    /// Due to alignment requirements, the `arena` may not be quite `new_size`.
    /// The difference will be less than [`CHUNK_UNIT`](crate::base::CHUNK_UNIT).
    ///
    /// If `new_size` isn't large enough to extend `arena`, this call does nothing.
    ///
    /// # Safety
    /// - `arena` must be managed by this instance of the allocator.
    /// - The memory in `arena.base()..arena.base().add(new_size)`
    ///     must be exclusively writeable by this instance of the allocator for
    ///     the lifetime `arena` unless truncated away or the allocator is no longer active.
    ///     - Note that any memory not contained within `arena` after `extend` returns
    ///         is unclaimed by the allocator and not subject to this requirement.
    ///     - Note that any memory in the resulting `arena` that is allocated by
    ///         `self` later on is also not subject to this requirement for the duration
    ///         of the allocation.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate talc;
    /// # use talc::{TalcCell, ErrOnOom};
    /// static mut ARENA: [u8; 5000] = [0; 5000];
    ///
    /// let talc = TalcCell::new(ErrOnOom);
    /// let mut arena = unsafe { talc.claim((&raw mut ARENA).cast(), 2500).unwrap() };
    /// unsafe { talc.extend(&mut arena, 5000) };
    /// ```
    #[inline]
    #[track_caller]
    pub unsafe fn extend(&self, arena_acme: *mut u8, new_acme: *mut u8) -> NonNull<u8> {
        // SAFETY: See `Self::borrow`'s safety docs
        // SAFETY: `Talc` function safety requirements guaranteed by caller
        self.borrow().extend(arena_acme, new_acme)
    }

    /// Reduce the size of `arena` to `new_size`.
    ///
    /// This function will never truncate more than what is legal.
    /// The extent cannot be reduced further than what is indicated
    /// by [`TalcCell::reserved`]. Attempting to do so (e.g. setting `new_size` to `0`)
    /// will truncate as much as possible.
    ///
    /// If `new_size` is too big, this call does nothing.
    ///
    /// If the resulting [`Arena`] is too small to allocate into, `None` is returned.
    ///
    /// Due to alignment requirements, the resulting [`Arena`]
    /// might have a slightly smaller resulting size than requested
    /// by a difference of less than [`CHUNK_UNIT`](crate::base::CHUNK_UNIT).
    ///
    /// All memory in `arena` not contained by the resulting [`Arena`], if any,
    /// is released back to the caller. You no longer need to guarantee that it's
    /// exclusively writable by `self`.
    ///
    /// # Safety
    /// `arena` must be managed by this instance of the allocator.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate talc;
    /// # use talc::{TalcCell, ErrOnOom};
    /// static mut ARENA: [u8; 5000] = [0; 5000];
    ///
    /// let talc = TalcCell::new(ErrOnOom);
    /// let arena = unsafe { talc.claim((&raw mut ARENA).cast(), ARENA.len()).unwrap() };
    /// // do some allocator operations...
    ///
    /// // reclaim as much of the arena as possible
    /// let opt_arena = unsafe { talc.truncate(arena, 0) };
    /// ```
    #[inline]
    #[track_caller]
    pub unsafe fn truncate(&self, arena_acme: *mut u8, new_acme: *mut u8) -> Option<NonNull<u8>> {
        // SAFETY: See `Self::borrow`'s safety docs
        // SAFETY: `Talc` function safety requirements guaranteed by caller
        self.borrow().truncate(arena_acme, new_acme)
    }

    #[inline]
    #[track_caller]
    pub unsafe fn resize(&self, arena_acme: *mut u8, new_acme: *mut u8) -> Option<NonNull<u8>> {
        self.borrow().resize(arena_acme, new_acme)
    }
}

impl<O: OomHandler<B> + Clone, B: Binning> TalcCell<O, B> {
    /// Returns a clone of [`Talc`]'s OOM handler.
    ///
    /// To set the OOM handler instead, use [`TalcCell::replace_oom_handler`].
    #[inline]
    #[track_caller]
    pub fn clone_oom_handler(&self) -> O {
        unsafe {
            // SAFETY: See `Self::borrow`'s safety docs
            self.borrow().oom_handler.clone()
        }
    }
}

struct BorrowedTalc<'b, O: OomHandler<B>, B: Binning> {
    ptr: NonNull<Talc<O, B>>,
    _phantom: PhantomData<&'b ()>,

    #[cfg(debug_assertions)]
    borrow_release: &'b core::cell::Cell<Option<&'static core::panic::Location<'static>>>,
}
impl<O: OomHandler<B>, B: Binning> Drop for BorrowedTalc<'_, O, B> {
    #[inline]
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        {
            self.borrow_release.set(None);
        }
    }
}
impl<O: OomHandler<B>, B: Binning> Deref for BorrowedTalc<'_, O, B> {
    type Target = Talc<O, B>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}
impl<O: OomHandler<B>, B: Binning> DerefMut for BorrowedTalc<'_, O, B> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

unsafe impl<O: OomHandler<B>, B: Binning> GlobalAlloc for TalcCell<O, B> {
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

unsafe impl<O: OomHandler<B>, B: Binning> Allocator for TalcCell<O, B> {
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
/// unsoundness. Strongly consider [`Talck`](crate::sync::Talck) instead.
///
/// This type implements [`Self::new`] and [`GlobalAlloc`]
/// making it usable as a global allocator.
///
/// See [`TalcCellAssumeSingleThreaded::new`].
pub struct TalcCellAssumeSingleThreaded<O: OomHandler<B>, B: Binning>(TalcCell<O, B>);

unsafe impl<O: OomHandler<B>, B: Binning> Sync for TalcCellAssumeSingleThreaded<O, B> {}

impl<O: OomHandler<B>, B: Binning> TalcCellAssumeSingleThreaded<O, B> {
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
    /// locking is cheap. Strongly consider using a [`Talck`](crate::sync::Talck)
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
    /// use talc::{ClaimOnOom, TalcCell, DefaultBinning};
    ///
    /// #[global_allocator]
    /// static ALLOC: talc::cell::TalcCellAssumeSingleThreaded<ClaimOnOom, DefaultBinning> = unsafe {
    ///     use core::mem::MaybeUninit;
    ///     static mut ARENA: [MaybeUninit<u8>; 100000] = [MaybeUninit::uninit(); 100000];
    ///     talc::cell::TalcCellAssumeSingleThreaded::new(TalcCell::new(ClaimOnOom::array(&raw mut ARENA)))
    /// };
    /// ```
    pub const unsafe fn new(talc: TalcCell<O, B>) -> Self {
        Self(talc)
    }
}

unsafe impl<O: OomHandler<B>, B: Binning> GlobalAlloc for TalcCellAssumeSingleThreaded<O, B> {
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
