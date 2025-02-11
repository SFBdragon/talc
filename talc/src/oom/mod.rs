use core::{alloc::Layout, fmt::Debug};

use crate::Binning;

use crate::base::Talc;

// mod allocator_backed;
mod claim_on_oom;
mod vm_backed;

// pub use allocator_backed::AllocOnOom;
pub use claim_on_oom::ClaimOnOom;
pub use vm_backed::GetSysMemOnOom;


// pub(crate) mod backed;

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

    #[inline]
    unsafe fn handle_basereg(&mut self, arena_base: *mut u8, chunk_acme: *mut u8) -> bool {
        false
    }


    /* /// If `self` does not support shrinking a span, this returns `None`.
    /// 
    /// If `self` does, then this returns the minimum amount of free space
    /// from the top of the span that ought to be available before
    /// `try_realloc_in_place` will get called to shrink a span.
    /// 
    /// If `self` allocates discrete pages, it may be best to return that
    /// here, or a higher size, so that [`Backed`] never asks to shrink
    /// spans when there isn't enough space available for `self` to even
    /// release a single page.
    /// 
    /// Setting this to a reasonably high value is also an optimization
    /// to the reclaim routine, at the cost of holding onto additional
    /// memory (which can also benefit performance, of course). 
    ///
    /// The returned value must be constant for a particular instance of `Self`.
    #[inline]
    fn supports_shrink_with_delta_of(&mut self) -> Option<core::num::NonZeroUsize> {
        None
    }
    #[inline]
    unsafe fn shrink(
        &mut self,
        arena_acme: *mut u8,
        min_new_acme: *mut u8,
    ) -> *mut u8 {
        let _ = min_new_acme;
        arena_acme
    }

    #[inline]
    fn could_be_arena_acme(&mut self, arena_acme: *mut u8) -> bool {
        true
    } */
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


