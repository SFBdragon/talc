//! The Talc allocator crate.
//!
//! For getting started:
//! - Check out the crate's [README](https://github.com/SFBdragon/talc)
//! - Read check out the `Talc` and `Talck` structures.
//!
//! Your first step will be `Talc::new(...)`, then `claim`.
//! Calling `Talc::lock()` on it will yield a `Talck` which implements
//! [`GlobalAlloc`] and [`Allocator`] (if the appropriate feature flags are set).

#![cfg_attr(not(any(test, feature = "error-scanning-std")), no_std)]

#![cfg_attr(feature = "nightly", feature(allocator_api))]

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#![warn(missing_docs)]
#![allow(type_alias_bounds)]

#[cfg(test)]
#[macro_use]
mod test_utils;
pub(crate) mod ptr_utils;

mod arena;
pub(crate) mod node;

pub mod base;
pub mod cell;
pub mod oom;
pub mod sync;
pub mod wasm;

pub mod ext;

pub use arena::Arena;
pub use base::binning::{Binning, DefaultBinning};
pub use oom::{ClaimOnOom, ErrOnOom};

/// This is a type alias for [`TalcCell`](crate::cell::TalcCell) with the default binning strategy.
pub type TalcCell<O: oom::OomHandler<DefaultBinning>> = cell::TalcCell<O, DefaultBinning>;
/// This is a type alias for [`Talck`](crate::sync::Talck) with the default binning strategy.
pub type Talck<R: lock_api::RawMutex, O: oom::OomHandler<DefaultBinning>> =
    sync::Talck<R, O, DefaultBinning>;

/// [`Talc`](base::Talc) can always successfully claim its first [`Arena`]
/// if the provided `size` is at least the returned value.
///
/// Note that this size is larger than [`min_first_arena_layout`]'s size
/// as extra padding is added to ensure a badly-aligned arena always
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
/// let talc = TalcCell::new(ErrOnOom);
/// let arena = unsafe {
///     talc.claim(ARENA.as_mut_ptr().cast(), ARENA.len()).unwrap()
/// };
/// ```
pub const fn min_first_arena_size<B: Binning>() -> usize {
    let size = crate::base::Talc::<crate::ErrOnOom, B>::required_chunk_size(
        B::BIN_COUNT as usize * core::mem::size_of::<*mut u8>(),
    );

    let max_overhead = crate::base::CHUNK_UNIT + core::mem::align_of::<usize>() - 1;

    size + max_overhead
}

/// [`Talc`](base::Talc) can always successfully claim its first [`Arena`]
/// if the provided `base` and `size` fit the returned
/// [`Layout`](::core::alloc::Layout).
///
/// # Example
///
/// ```rust
/// # extern crate talc;
/// use talc::*;
///
/// let talc = TalcCell::new(ErrOnOom);
/// let arena_layout = min_first_arena_layout::<DefaultBinning>();
///
/// unsafe {
///     let arena_ptr = std::alloc::alloc(arena_layout);
///
///     if !arena_ptr.is_null() {
///         let arena = talc.claim(arena_ptr, arena_layout.size()).unwrap();
///     }
///     # unsafe { std::alloc::dealloc(arena_ptr, arena_layout) };
/// }
/// ```
pub const fn min_first_arena_layout<B: Binning>() -> ::core::alloc::Layout {
    let size = B::BIN_COUNT as usize * core::mem::size_of::<usize>();

    let max_overhead = crate::base::CHUNK_UNIT;

    unsafe {
        ::core::alloc::Layout::from_size_align_unchecked(
            size + max_overhead,
            core::mem::align_of::<usize>(),
        )
    }
}
