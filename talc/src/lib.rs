//! The Talc allocator crate.
//!
//! For getting started:
//! - Check out the crate's [README](https://github.com/SFBdragon/talc)
//! - Read check out the `Talc` and `Talck` structures.
//!
//! Your first step will be `Talc::new(...)`, then `claim`.
//! Calling `Talc::lock()` on it will yield a `Talck` which implements
//! [`GlobalAlloc`] and [`Allocator`] (if the appropriate feature flags are set).

// #![cfg_attr(not(any(test, feature = "fuzzing")), no_std)]
#![cfg_attr(feature = "allocator_api", feature(allocator_api))]

#![warn(missing_docs)]

mod allocators;
mod ptr_utils;
mod span;
pub mod talc;

pub use span::Span;
pub use talc::oom_handler::{OomHandler, ErrOnOom, ClaimOnOom};

pub type Talc<O: talc::oom_handler::OomHandler<talc::bucket_config::TwoUsizeBucketConfig, talc::alignment::DefaultAlign>> = talc::Talc<O, talc::bucket_config::TwoUsizeBucketConfig, talc::alignment::DefaultAlign>;
pub type TalcWithCacheAlignment<O: talc::oom_handler::OomHandler<talc::bucket_config::TwoUsizeBucketConfig, talc::alignment::CacheAligned>> = talc::Talc<O, talc::bucket_config::TwoUsizeBucketConfig, talc::alignment::CacheAligned>;

const ALLOC: Talc<ErrOnOom> = Talc::new(ErrOnOom);
