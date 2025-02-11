use crate::{base::Talc, Arena, Binning};

use super::OomHandler;

/// An out-of-memory handler that attempts to claim the memory within a given [`Span`] upon OOM.
///
/// This OOM handler has two states:
/// - unclaimed: there is memory for the allocator to try claiming, and it hasn't been claimed yet.
/// - cannot-claim: there is no memory for the allocator to claim.
///
/// If the allocator invokes [`ClaimOnOom::handle_oom`]
/// - unclaimed: [`Talc::claim`] is called.
/// - cannot-claim: indicates that OOM could not be handled. Acts like [`ErrOnOom`].
///
/// If the [`Talc::claim`] call is successful, [`ClaimOnOom`] holds onto the
/// returned [`Arena`], which you can take using [`ClaimOnOom::take_claimed_arena`].
///
#[derive(Debug)]
pub struct ClaimOnOom(ClaimOnOomInner);

#[derive(Debug)]
enum ClaimOnOomInner {
    Unclaimed { base: *mut u8, size: usize },
    CannotClaim(Option<Arena>),
}

unsafe impl Send for ClaimOnOomInner {}

impl ClaimOnOom {
    /// Create a new [`ClaimOnOom`] OOM handler.
    ///
    /// # Safety
    /// The caller must guarantee that the safety contract of [`Talc::claim`]
    /// would be upheld if/when [`Talc`] invokes [`ClaimOnOom::handle_oom`]
    /// on the returned [`ClaimOnOom`].
    #[inline]
    pub const unsafe fn new(base: *mut u8, size: usize) -> Self {
        Self(ClaimOnOomInner::Unclaimed { base, size })
    }

    /// Create a new [`ClaimOnOom`] OOM handler.
    ///
    /// # Safety
    /// The caller must guarantee that the safety contract of [`Talc::claim`]
    /// would be upheld if/when [`Talc`] invokes [`ClaimOnOom::handle_oom`]
    /// on the returned [`ClaimOnOom`].
    #[inline]
    pub const unsafe fn array<T, const N: usize>(array: *mut [T; N]) -> Self {
        Self::new(array.cast(), N * core::mem::size_of::<T>())
    }

    /// Creates a [`ClaimOnOom`] in the cannot-claim state.
    ///
    /// If [`Talc`] calls [`ClaimOnOom::handle_oom`] it'll act like [`ErrOnOom`].
    ///
    /// Can be swapped into [`Talc::oom_handler`] to prevent
    /// [`ClaimOnOom::handle_oom`] from claiming memory on OOM.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate talc;
    /// use talc::{TalcCell, ClaimOnOom};
    ///
    /// static mut ARENA: [u8; 4096] = [0; 4096];
    ///
    /// let talc = TalcCell::new(unsafe { ClaimOnOom::array(&raw mut ARENA) });
    /// let _old_oom_handler = talc.replace_oom_handler(ClaimOnOom::err_on_oom());
    /// ```
    #[inline]
    pub const fn err_on_oom() -> Self {
        Self(ClaimOnOomInner::CannotClaim(None))
    }

    /// Check if the memory is still available to be claimed.
    #[inline]
    pub const fn is_unclaimed(&self) -> bool {
        matches!(&self.0, ClaimOnOomInner::Unclaimed { .. })
    }

    /// If `handle_oom` has been called and an [`Arena`] is successfully claimed,
    /// the [`Arena`] can be taken using this function.
    /// Otherwise, this return `None`.
    pub fn take_claimed_arena(&mut self) -> Option<Arena> {
        match &mut self.0 {
            ClaimOnOomInner::CannotClaim(arena) => arena.take(),
            _ => None,
        }
    }
}

// SAFETY: `handle_oom` does not touch any exterior allocator.
unsafe impl<B: Binning> OomHandler<B> for ClaimOnOom {
    fn handle_oom(talc: &mut Talc<Self, B>, _layout: core::alloc::Layout) -> Result<(), ()> {
        match talc.oom_handler.0 {
            ClaimOnOomInner::Unclaimed { base, size } => {
                talc.oom_handler.0 = ClaimOnOomInner::CannotClaim(
                    // SAFETY: guaranteed by the creator of the `ClaimOnOom`
                    unsafe { talc.claim(base, size) },
                );

                Ok(())
            }
            ClaimOnOomInner::CannotClaim(_) => Err(()),
        }
    }
}
