use core::{alloc::Layout, fmt::Debug};

use crate::base::binning::Binning;

use crate::base::Talc;

mod allocator;
mod global_alloc;
pub mod rcdr;
mod span;

pub use allocator::AllocatorSource;
pub use global_alloc::GlobalAllocSource;
pub use span::Claim;

#[cfg(all(feature = "system-backed", any(unix, windows)))]
pub use rcdr::Os;

/// Source and manage regions of memory for [`Talc`] to utilize.
///
/// # Safety
///
/// Do not use the parent [`TalcLock`](crate::sync::TalcLock) or [`TalcCell`](crate::cell::TalcCell)
/// in the [`Source`] implementation.
/// - The former will deadlock.
/// - The latter will result in a panic if debug assertion are enabled, otherwise undefined behavior.
///
/// The main cause for concern is using the global allocator, directly or
/// indirectly, if the global allocator might be the parent [`TalcLock`](crate::sync::TalcLock) or
/// [`TalcCellAssumeSingleThreaded`](crate::cell::TalcCellAssumeSingleThreaded).
///
/// For examples of how this could go wrong:
/// - Calling [`println!`] or a similar operation in the [`Source`] implementation
///     which may allocate. This would make the [`Source`] implementation unusable
///     in global allocators.
/// - Manipulating a [`Vec`] of metadata, as this may allocate to the same effect.
///     (Linked lists are generally more applicable for memory management.)
///
/// If dynamic memory is needed, make sure to pre-allocate it
/// beforehand, or have a separate allocator.
pub unsafe trait Source: Debug + Sized {
    /// Given the allocator and the `layout` of the allocation that caused
    /// OOM, resize or claim and return `Ok(())` or fail by returning `Err(())`.
    ///
    /// This function is called repeatedly if the allocator is still out of memory.
    /// Therefore an infinite loop will occur if `Ok(())` is repeatedly returned
    /// without extending or claiming new memory. To avoid this, don't return `Ok(())`
    /// if no additional memory has been made available to the allocator.
    ///
    /// # Statefulness
    ///
    /// The source may be stateful.
    ///
    /// Use `talc.source` to access the data associated with the source.
    ///
    /// # Safety, Panicking, and Deadlocking
    /// Implementors of [`Source`] must be vigilant about not interacting
    /// with the busy-allocating instance of [`Talc`] through anything except
    /// the provided mutable reference, `talc`.
    ///
    /// Implementors must uphold that they do not interact with the [`TalcLock`](crate::sync::TalcLock)
    /// or [`TalcCell`](crate::cell::TalcCell) that wraps the provided [`Talc`].
    /// This includes indirect calls to the global allocator
    /// (which might be a wrapper around [`Talc`]) because of something like [`println!`].
    ///
    /// See [`Source`]'s documentation for more information.
    fn acquire<B: Binning>(talc: &mut Talc<Self, B>, layout: Layout) -> Result<(), ()>;

    /// Configures whether [`Talc`] tracks the end of heaps.
    ///
    /// This must be `true` for [`Source::resize`] to have any effect.
    ///
    /// Because tracking the end of the heap incurs some overhead,
    /// leave this as `false` if you don't need to automatically reduce the
    /// size of the heap.
    ///
    /// Note that this does not allow for querying the ends of heaps.
    /// This just means that Talc "knows it when it sees it" and can
    /// call [`Source::resize`] to provide implementors
    /// the opportunity to change the size of the heap if they'd like.
    /// See [`Source::resize`] for more details on that.
    const TRACK_HEAP_END: bool = false;

    /// TODO
    ///
    ///
    /// `chunk_base` is effectively the return value of [`Talc::reserved`].
    ///
    /// Not called unless `TRACK_HEAP_END` is set to true.
    /// If `is_heap_base`, `base + 1 <= chunk_base <= base + CHUNK_UNIT`.
    /// `chunk_case` is aligned to CHUNK_UNIT.
    /// `heap_end` is aligned to CHUNK_UNIT.
    #[inline]
    unsafe fn resize(
        &mut self,
        chunk_base: *mut u8,
        heap_end: *mut u8,
        is_heap_base: bool,
    ) -> *mut u8 {
        let _ = (chunk_base, heap_end, is_heap_base);
        heap_end
    }
}

/// Does not provide or reclaim memory.
///
/// Allocation error occurs immediately upon any attempt to acquire memory.
///
/// [`Talc::claim`], [`Talc::extend`], [`Talc::resize`], and [`Talc::truncate`]
/// must be used to provision the allocator with memory regions to allocate from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Manual;

// SAFETY: `acquire` does not touch any exterior allocator.
unsafe impl Source for Manual {
    #[inline]
    fn acquire<B: Binning>(_talc: &mut Talc<Self, B>, _layout: Layout) -> Result<(), ()> {
        Err(())
    }
}
