use core::ptr::NonNull;

use crate::{base::Talc, base::binning::Binning};

use super::Source;

/// A [`Source`] implementation that attempts to claim the memory on-demand.
///
/// This source has two states:
/// - ready: there is memory to be claimed and it hasn't been claimed yet.
/// - standby: there is nothing to claim, but there may be the result of the [`Talc::claim`] call.
///
/// If the allocator invokes [`Claim::acquire`]
/// - ready: [`Talc::claim`] is called on the memory.
/// - standby: acquire fails. Acts like [`Manual`](super::Manual).
///
/// If the [`Talc::claim`] call is successful, [`Claim`] holds onto the
/// return value of [`Talc::claim`], which you can take using [`Claim::take_claim`].
#[derive(Debug)]
pub struct Claim(ClaimInner);

#[derive(Debug)]
enum ClaimInner {
    Ready { base: *mut u8, size: usize },
    Standby { maybe_claim_result: Option<NonNull<u8>> },
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
        Self(ClaimInner::Ready { base, size })
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

    /// Creates a [`Claim`] in the standby state.
    ///
    /// If [`Talc`] calls [`Claim::acquire`] it'll act like [`Manual`](super::Manual).
    ///
    /// Can be swapped into [`Talc`]'s source to prevent claiming the provided region.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate talc;
    /// use talc::{TalcCell, source::Claim};
    ///
    /// static mut HEAP: [u8; 4096] = [0; 4096];
    ///
    /// let talc = TalcCell::new(unsafe { Claim::array(&raw mut HEAP) });
    /// let _old_source = talc.replace_source(Claim::standby());
    /// ```
    #[inline]
    pub const fn standby() -> Self {
        Self(ClaimInner::Standby { maybe_claim_result: None })
    }

    /// Check if the memory region is still available to be claimed.
    #[inline]
    pub const fn is_ready(&self) -> bool {
        matches!(&self.0, ClaimInner::Ready { .. })
    }

    /// If [`Source::acquire`] has been called and the claim was successful,
    /// the resulting heap end can be taken using this function.
    /// Otherwise, this return `None`.
    pub fn take_claim(&mut self) -> Option<NonNull<u8>> {
        match &mut self.0 {
            ClaimInner::Standby { maybe_claim_result } => maybe_claim_result.take(),
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
            ClaimInner::Ready { base, size } => {
                talc.source.0 = ClaimInner::Standby {
                    // SAFETY: guaranteed by the creator of the `Claim`
                    maybe_claim_result: unsafe { talc.claim(base, size) },
                };

                Ok(())
            }
            ClaimInner::Standby { maybe_claim_result: _ } => Err(()),
        }
    }
}
