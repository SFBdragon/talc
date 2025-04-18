//! The Talc allocator crate.
//!
//! For getting started:
//! - Check out the crate's [README](https://github.com/SFBdragon/talc)
//! - Read check out the `Talc` and `TalcLock` structures.
//!
//! Your first step will be `Talc::new(...)`, then `claim`.
//! Calling `Talc::lock()` on it will yield a `TalcLock` which implements
//! [`GlobalAlloc`] and [`Allocator`] (if the appropriate feature flags are set).
//!
//! TODO ^^^

#![cfg_attr(not(any(test, feature = "error-scanning-std")), no_std)]
#![cfg_attr(feature = "nightly", feature(allocator_api))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]
#![allow(type_alias_bounds)]

#[cfg(test)]
#[macro_use]
mod test_utils;
pub(crate) mod node;
pub(crate) mod ptr_utils;

pub mod base;
pub mod cell;
pub mod src;
pub mod sync;
pub mod wasm;

pub mod prelude {
    #[cfg(feature = "counters")]
    pub use crate::base::Counters;
    pub use crate::base::Reserved;
    pub use crate::base::Talc;
    pub use crate::base::binning::{Binning, DefaultBinning};
    pub use crate::src::{AllocatorSource, Claim, GlobalAllocSource, Manual, Source};

    #[cfg(feature = "system-backed")]
    pub use crate::src::Os;

    /// This is a type alias for [`TalcCell`](crate::cell::TalcCell) with the default binning strategy.
    pub type TalcCell<S: Source> = crate::cell::TalcCell<S, DefaultBinning>;
    /// This is a type alias for [`TalcLock`](crate::sync::TalcLock) with the default binning strategy.
    pub type TalcLock<R: lock_api::RawMutex, S: Source> =
        crate::sync::TalcLock<R, S, DefaultBinning>;
}

/// [`Talc`](base::Talc) can always successfully perform the first claim
/// if the provided `size` is at least the returned value.
///
/// Note that this size is larger than [`min_first_heap_layout`]'s size
/// as extra padding is added to ensure a badly-aligned heap always
/// has enough well-aligned memory.
///
/// # Example
///
/// ```rust
/// # extern crate talc;
/// use talc::*;
///
/// static mut ARENA: [u8; min_first_arena_size::<DefaultBinning>()] = [0; min_first_arena_size::<DefaultBinning>()];
///
/// let talc = TalcCell::new(Manual);
/// let arena = unsafe {
///     talc.claim(ARENA.as_mut_ptr().cast(), ARENA.len()).unwrap()
/// };
/// ```
pub const fn min_first_heap_size<B: base::binning::Binning>() -> usize {
    let size = crate::base::Talc::<crate::src::Manual, B>::required_chunk_size(
        B::BIN_COUNT as usize * core::mem::size_of::<*mut u8>(),
    );

    let max_overhead = crate::base::CHUNK_UNIT + core::mem::align_of::<usize>() - 1;

    size + max_overhead
}

/// [`Talc`](base::Talc) can always successfully perform the first claim
/// if the provided `base` and `size` fit the returned
/// [`Layout`](::core::alloc::Layout).
///
/// # Example
///
/// ```rust
/// # extern crate talc;
/// use talc::*;
///
/// let talc = TalcCell::new(Manual);
/// let heap_layout = min_first_heap_layout::<DefaultBinning>();
///
/// unsafe {
///     let arena_ptr = std::alloc::alloc(heap_layout);
///
///     if !heap_ptr.is_null() {
///         let _heap_end = talc.claim(heap_ptr, heap_layout.size()).unwrap();
///     }
///     
///     // ...
///
///     unsafe { std::alloc::dealloc(heap_ptr, heap_layout) };
/// }
/// ```
pub const fn min_first_heap_layout<B: base::binning::Binning>() -> ::core::alloc::Layout {
    let size = B::BIN_COUNT as usize * core::mem::size_of::<usize>();

    let max_overhead = crate::base::CHUNK_UNIT;

    unsafe {
        ::core::alloc::Layout::from_size_align_unchecked(
            size + max_overhead,
            core::mem::align_of::<usize>(),
        )
    }
}
