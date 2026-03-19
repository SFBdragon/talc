//! An allocator needs a source of memory to allocate from.
//! With [`Talc`] this can be manually provided, e.g. using [`Talc::claim`] directly.
//!
//! However, it's often desirable to implement either
//! - a routine to obtain memory if the allocator exhausts all available heap space
//! - a fully automatic acquire/resize/release solution, e.g. obtaining and managing memory from an operating system
//!
//! Implementations of the [`Source`] trait provide different approaches.
//! If you need something slightly different - this is quite common, especially on
//! embedded systems and other unusual compilation targets - it's recommended that you
//! look at some of the more advanced implementations in this crate
//! (e.g. [`GlobalAllocSource`] or `VirtualHeapsSource` [currently unused]).
//!
//! Some key ones to be aware of:
//! - [`Manual`] does nothing. Returns `Err(())` if `acquire` is called.

use core::{alloc::Layout, fmt::Debug};

use crate::base::binning::Binning;

use crate::base::Talc;

mod allocator;
mod claim;
mod global_alloc;
// pub mod vheaps;

pub use allocator::AllocatorSource;
pub use claim::Claim;
pub use global_alloc::GlobalAllocSource;

/// Acquire and manage heaps of memory for [`Talc`] to utilize.
///
/// A [`Source`] implementation has two parts:
///
/// - [`Source::acquire`] is called when [`Talc`] needs more memory.
///     The source is expected to use heap management APIs such as `claim`/`extend` to achieve this.
///
/// - Optionally, implementing [`Source::resize`] allows for automatically managing heaps
///     with free space available to be reclaimed.
///     Set [`Source::TRACK_HEAP_END`] to `true` to enable this.
///
/// # The old `OomHandler` way
///
/// Just implement [`Source::acquire`] as usual and ignore `resize`. The API is almost the same.
///
/// # Safety
///
/// Do not use the parent [`TalcLock`](crate::sync::TalcLock) or [`TalcCell`](crate::cell::TalcCell)
/// in the [`Source`] implementation.
/// - The former will deadlock.
/// - The latter will result in a panic if debug assertion are enabled, or else undefined behavior.
///
/// The main cause for concern is using the global allocator, directly or
/// indirectly, if the global allocator might be the parent [`TalcLock`](crate::sync::TalcLock) or
/// [`TalcSyncCell`](crate::cell::TalcSyncCell).
///
/// For examples of how this could go wrong:
/// - Calling `println!` or `dbg!` or a similar operation in the [`Source`] implementation
///     which may allocate. This would make the [`Source`] implementation unusable
///     in global allocators.
/// - Manipulating a `Vec` of metadata, as this may allocate to the same effect.
///     (Linked lists are generally more applicable for memory management.)
///
/// If dynamic memory is needed, make sure to pre-allocate it
/// beforehand, or have a known-distinct allocator.
///
/// # Drop implementation
///
/// Keep in mind that if the [`Source`] is keeping track of heaps associated with
/// resources such as mmap'd system memory, it's a good idea to implement [`Drop`]
/// on a [`Source`] implementation such that when the allocator is disposed of,
/// the resources can be disposed of properly in turn.
pub unsafe trait Source: Debug + Sized {
    /// The allocator ran out of available memory and has thus called [`Source::acquire`]
    /// your options are as follows:
    /// - use [`Talc::claim`] or [`Talc::extend`] to establish/extend the amount of memory available
    ///     for `talc` to allocate from, and then return `Ok(())`
    /// - allow allocation failure to occur by returning `Err(())`, which is typically a last resort
    ///
    /// If `Ok(())` is returned, but the allocator still finds itself without sufficient memory,
    /// [`Source::acquire`] is invoked again.
    /// Therefore an infinite loop will occur if `Ok(())` is repeatedly returned
    /// without extending or claiming new memory. To avoid this, don't return `Ok(())`
    /// if no additional memory has been made available to the allocator.
    ///
    /// # Statefulness
    ///
    /// The source may be stateful.
    ///
    /// Use `talc.source` to access the data associated with the [`Source`] implementation.
    ///
    /// # Safety, Panicking, and Deadlocking
    /// Implementors of [`Source`] must be vigilant about not interacting
    /// with the busy-allocating instance of [`Talc`] through anything except
    /// the provided mutable reference, `talc`.
    ///
    /// Implementors must uphold that they do not interact with the [`TalcLock`](crate::sync::TalcLock)
    /// or [`TalcCell`](crate::cell::TalcCell) that wraps the provided [`Talc`].
    /// This includes indirect calls to the global allocator (which might be a wrapper around [`Talc`])
    /// because of something like `println!` or `dbg!` or `Vec::with_capacity`.
    ///
    /// See [`Source`]'s documentation for more information.
    fn acquire<B: Binning>(talc: &mut Talc<Self, B>, layout: Layout) -> Result<(), ()>;

    /// Configures whether [`Talc`] tracks the end of heaps.
    ///
    /// If this is `true`, then [`Talc`] will recognize when it's working with
    /// a
    /// This must be `true` for the [`Source::resize`] implementation to have any effect.
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

    /// The allocator has released memory near the top of the heap.
    /// As a result, [`Talc`] called [`Source::resize`] to give the source an opportunity
    /// to resize the heap. This effectively gives the implementation an opportunity
    /// to reclaim the memory (or claim more memory, if desirable for some reason).
    ///
    /// The return value is where the new top of the heap will be.
    ///
    /// This function is never called if [`Source::TRACK_HEAP_END`] is `false`.
    ///
    /// To set the stage:
    /// - `chunk_base` is the top of the reserved region. See [`Talc::reserved`].
    ///     In short, it's either the pointer above the last allocated byte or the base of the heap.
    /// - `heap_end` is the top of the heap. This is what you're adjusting up or down.
    /// - `is_heap_base` is whether `chunk_base` is actually the base of the entire heap.
    ///
    /// Implementation details for [`Source`] implementations.
    /// - `chunk_base` is aligned to [`CHUNK_UNIT`](crate::base::CHUNK_UNIT)
    /// - `heap_end` is aligned to [`CHUNK_UNIT`](crate::base::CHUNK_UNIT)
    /// - If `is_heap_base`: `base + 1 <= chunk_base <= base + CHUNK_UNIT`
    ///
    /// # Performance
    ///
    /// If [`Source::TRACK_HEAP_END`] is `true` then [`Talc`] will call [`Source::resize`]
    /// whenever the free space at the end of a heap gets larger.
    /// This will result in quite a lot of calls!
    /// So avoid doing anything expensive until you know that there's a decent amount
    /// of memory to free up, or whatever you're planning to do.
    ///
    /// For example, you'll typically want a quick check at the start, e.g.
    /// `if heap_end as usize - chunk_base as usize > PAGE_SIZE`
    /// before doing any real work.
    ///
    /// # Safety
    ///
    /// Callers must guarantee that
    /// - `chunk_base` and `heap_end` is aligned to [`CHUNK_UNIT`](crate::base::CHUNK_UNIT)
    /// - `is_heap_base` must be a true reflection of whether the heap will be deleted if the caller returns `chunk_base`
    ///
    /// Implementors must guarantee that
    /// - The returned pointer is greater than or equal to `chunk_base`
    /// - The returned pointer is aligned to [`CHUNK_UNIT`](crate::base::CHUNK_UNIT)
    /// - The memory within `heap_end..return_ptr`, if any, is subject to the same safety contract as [`Talc::extend`]
    ///
    /// Note that this constitutes a resizing of the heap, and a change of the heap end,
    /// as far as [`Talc::extend`]/[`Talc::truncate`]/[`Talc::resize`] are concerned,
    /// and thus this affects their safety contracts.
    ///
    /// # Do not use manual heap management
    ///
    /// [`Source::resize`] implementations should not call heap management functions:
    /// [`Talc::claim`], [`Talc::extend`], [`Talc::truncate`], and [`Talc::resize`].
    /// Instead, the extent of the heap being worked on is entirely controlled by
    /// the returned pointer.
    ///
    /// Note that this function does not receive a pointer to the [`Talc`] instance,
    /// just a reference to the [`Source`] instance. This is intentional.
    ///
    /// # Consider forbidding the user from doing manual heap management
    ///
    /// Typically, [`Source::resize`] implementations rely on metadata around the heaps they manage.
    ///
    /// Unless the user manually takes care to replicate the metadata that the [`Source`]
    /// implementation maintains about the heaps it manages, then manual heap management
    /// while a resizing-[`Source`]-implementation is active will probably lead to
    /// erroneous memory accessed and UB.
    ///
    /// If manual heap management use may lead to UB, document this clearly on the
    /// implementation struct's docs.
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

/// A [`Source`] implementation that does not provide or reclaim memory.
///
/// Allocation error occurs immediately upon any attempt to call [`Source::acquire`].
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
