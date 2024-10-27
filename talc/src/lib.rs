//! The Talc allocator crate.
//!
//! For getting started:
//! - Check out the crate's [README](https://github.com/SFBdragon/talc)
//! - Read check out the `Talc` and `Talck` structures.
//!
//! Your first step will be `Talc::new(...)`, then `claim`.
//! Calling `Talc::lock()` on it will yield a `Talck` which implements
//! [`GlobalAlloc`] and [`Allocator`] (if the appropriate feature flags are set).

#![cfg_attr(not(any(test, feature = "fuzzing")), no_std)]
#![cfg_attr(feature = "allocator", feature(allocator_api))]

mod oom_handler;
mod ptr_utils;
mod span;
mod talc;

#[cfg(feature = "lock_api")]
pub mod locking;
#[cfg(feature = "lock_api")]
mod talck;

pub use oom_handler::{ClaimOnOom, ErrOnOom, OomHandler};
pub use span::Span;
pub use talc::Talc;

#[cfg(feature = "lock_api")]
pub use talck::Talck;
#[cfg(all(target_family = "wasm", feature = "lock_api"))]
pub use talck::TalckWasm;

#[cfg(all(target_family = "wasm", feature = "lock_api"))]
pub use oom_handler::WasmHandler;
