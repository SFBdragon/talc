use core::ptr::NonNull;

use crate::{base::Talc, base::binning::Binning};

use super::Source;

/// An source that attempts to claim the memory on-demand.
///
/// This source has two states:
/// - unclaimed: there is memory to be claimed and it hasn't been claimed yet.
/// - cannot-claim: there is no memory for the allocator to claim.
///
/// If the allocator invokes [`Claim::acquire`]
/// - unclaimed: [`Talc::claim`] is called on the memory.
/// - cannot-claim: acquire fails. Acts like [`Manual`](super::Manual).
///
/// If the [`Talc::claim`] call is successful, [`Claim`] holds onto the
/// return value of [`Talc::claim`], which you can take using [`Claim::take_claim`].
///
#[derive(Debug)]
pub struct Claim(ClaimInner);

#[derive(Debug)]
enum ClaimInner {
    Unclaimed { base: *mut u8, size: usize },
    CannotClaim(Option<NonNull<u8>>),
}

unsafe impl Send for ClaimInner {}

impl Claim {
    /// Create a new [`Claim`] source.
    ///
    /// # Safety
    /// The caller must guarantee that the safety contract of [`Talc::claim`]
    /// would be upheld if/when [`Talc`] invokes [`Claim::acquire`]
    /// on the returned [`Claim`].
    #[inline]
    pub const unsafe fn new(base: *mut u8, size: usize) -> Self {
        Self(ClaimInner::Unclaimed { base, size })
    }

    /// Create a new [`Claim`] source from an array.
    ///
    /// # Safety
    /// The caller must guarantee that the safety contract of [`Talc::claim`]
    /// would be upheld if/when [`Talc`] invokes [`Claim::acquire`]
    /// on the returned [`Claim`].
    #[inline]
    pub const unsafe fn array<T, const N: usize>(array: *mut [T; N]) -> Self {
        Self::new(array.cast(), N * core::mem::size_of::<T>())
    }

    /// Creates a [`Claim`] in the cannot-claim state.
    ///
    /// If [`Talc`] calls [`Claim::acquire`] it'll act like [`Manual`](super::Manual).
    ///
    /// Can be swapped into [`Talc`]'s source to prevent claiming the provided region.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate talc;
    /// use talc::{TalcCell, Claim};
    ///
    /// static mut HEAP: [u8; 4096] = [0; 4096];
    ///
    /// let talc = TalcCell::new(unsafe { Claim::array(&raw mut HEAP) });
    /// let _old_source = talc.replace_source(Claim::cannot());
    /// ```
    // todo rename to empty or something??
    #[inline]
    pub const fn cannot() -> Self {
        Self(ClaimInner::CannotClaim(None))
    }

    /// Check if the memory is still available to be claimed.
    #[inline]
    pub const fn is_unclaimed(&self) -> bool {
        matches!(&self.0, ClaimInner::Unclaimed { .. })
    }

    /// If [`Source::acquire`] has been called and the claim was successful,
    /// the resulting heap end can be taken using this function.
    /// Otherwise, this return `None`.
    pub fn take_claim(&mut self) -> Option<NonNull<u8>> {
        match &mut self.0 {
            ClaimInner::CannotClaim(heap_end) => heap_end.take(),
            _ => None,
        }
    }
}

// SAFETY: `acquire` does not touch any exterior allocator.
unsafe impl Source for Claim {
    fn acquire<B: Binning>(
        talc: &mut Talc<Self, B>,
        _layout: core::alloc::Layout,
    ) -> Result<(), ()> {
        match talc.source.0 {
            ClaimInner::Unclaimed { base, size } => {
                talc.source.0 = ClaimInner::CannotClaim(
                    // SAFETY: guaranteed by the creator of the `Claim`
                    unsafe { talc.claim(base, size) },
                );

                Ok(())
            }
            ClaimInner::CannotClaim(_) => Err(()),
        }
    }
}
