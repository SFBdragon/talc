use core::{alloc::Layout, fmt::Debug};

use crate::Binning;

use crate::base::Talc;

// mod allocator_backed;
mod claim_on_oom;
mod os_backed;

pub use allocator_backed::AllocOnOom;
pub use claim_on_oom::ClaimOnOom;

#[cfg(any(unix, windows))]
pub use os_backed::WithSysMem;

pub(crate) mod allocator_backed;

/// Handle [`Talc`]'s out-of-memory state.
///
/// # Safety
/// Do not use the parent [`TalcCell`](crate::cell::TalcCell) or
/// [`Talck`](crate::sync::Talck) in the [`OomHandler::handle_oom`] implementation.
/// The latter will deadlock.
/// The former will result in a panic if debug assertion are enabled, undefined behavior otherwise.
///
/// The main cause for concern is using the global allocator, directly or
/// indirectly, as the global allocator might be the parent
/// [`TalcCellAssumeSingleThreaded`](crate::cell::TalcCellAssumeSingleThreaded)
/// or [`Talck`](crate::sync::Talck). This includes manipulating a [`Vec`]
/// through a `static` variable and calling [`println!`], both of which may allocate.
///
/// If dynamic memory is needed, make sure to pre-allocate it
/// beforehand, or have a separate source of allocations.
pub unsafe trait OomHandler<B: Binning>: Debug + Sized {
    /// Given the allocator and the `layout` of the allocation that caused
    /// OOM, resize or claim and return `Ok(())` or fail by returning `Err(())`.
    ///
    /// This function is called repeatedly if the allocator is still out of memory.
    /// Therefore an infinite loop will occur if `Ok(())` is repeatedly returned
    /// without extending or claiming new memory.
    ///
    /// # Statefulness
    ///
    /// The OOM handler may be stateful.
    ///
    /// Use `talc.oom_handler` to access the data associated with the OOM handler.
    ///
    /// # Safety, Panicking, and Deadlocking
    /// Implementors of [`OomHandler`] must be vigilant about not interacting
    /// with the busy-allocating instance of [`Talc`] through anything except
    /// the provided mutable reference, `talc`.
    ///
    /// Implementors must uphold that they do not interact with the [`Talck`](crate::sync::Talck)
    /// or [`TalcCell`](crate::cell::TalcCell) that wraps the provided [`Talc`].
    /// This includes indirect calls to the global allocator
    /// (which might be a wrapper around [`Talc`]) because of something like [`println!`].
    ///
    /// See [`OomHandler`]'s documentation for more information.
    fn handle_oom(talc: &mut Talc<Self, B>, layout: Layout) -> Result<(), ()>;

    /// Configures whether [`Talc`] tracks the end of the arena.
    ///
    /// This must be `true` for [`OomHandler::maybe_resize_arena`] to have any effect.
    ///
    /// Because tracking the end of the arena incurs some overhead,
    /// leave this as `false` if you don't need to automatically reduce the
    /// size of the arena.
    ///
    /// Note that this does not allow for querying the ends of arenas.
    /// This just means that Talc "knows it when it sees it" and can
    /// call [`OomHandler::maybe_resize_arena`] to provide implementors
    /// the opportunity to change the size of the arena if they'd like.
    /// See [`OomHandler::maybe_resize_arena`] for more details on that.
    const TRACK_ARENA_END: bool = false;

    /// TODO
    /// Not called unless `TRACK_ARENA_END` is set to true.
    /// If `is_arena_base`, `base + 1 <= chunk_base <= base + CHUNK_UNIT`.
    /// `chunk_case` is aligned to CHUNK_UNIT.
    /// `arena_end` is aligned to CHUNK_UNIT.
    #[inline]
    unsafe fn maybe_resize_arena(
        &mut self,
        chunk_base: *mut u8,
        arena_end: *mut u8,
        is_arena_base: bool,
    ) -> *mut u8 {
        let _ = (chunk_base, arena_end, is_arena_base);
        arena_end
    }
}

/// Doesn't handle out-of-memory conditions, immediate allocation error occurs.
#[derive(Debug, Clone, Copy)]
pub struct ErrOnOom;

// SAFETY: `handle_oom` does not touch any exterior allocator.
unsafe impl<B: Binning> OomHandler<B> for ErrOnOom {
    #[inline]
    fn handle_oom(_talc: &mut Talc<Self, B>, _layout: Layout) -> Result<(), ()> {
        Err(())
    }
}
