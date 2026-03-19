//! UNFINISHED - OPEN AN ISSUE IF YOU WANT OS VIRTUAL MEMORY INTEGRATION
//!
//! An attempt at making a decent abstraction over memory allocation via anon&private `mmap` mappings.
//!
//! If you know a thing or two about low-level system allocation on UNIX platforms,
//! scrutiny on my implementation would be appreciated.

use core::{num::NonZeroUsize, ptr::NonNull};

use crate::ptr_utils;

const RESERVED_BLOCK: usize = 8 << 20;
const COMMIT_GRANULARITY: usize = 256 << 10;

#[derive(Debug)]
pub struct UnixMMapSource;

// SAFETY:
// Does not interact with any Rust allocators, and thus does not create a second mutable reference to Talc.
// See [`Source`]'s safety contract for why this is important.
unsafe impl super::VirtualHeaps for UnixMMapSource {
    const INIT: Self = Self;

    #[inline]
    fn reserve(&mut self, min_size: NonZeroUsize) -> Option<NonNull<[u8]>> {
        let unit_l1 = RESERVED_BLOCK - 1;
        let size = (min_size.get() + unit_l1) & !unit_l1;

        let result = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                size,
                libc::PROT_NONE,
                libc::MAP_ANONYMOUS | libc::MAP_PRIVATE,
                -1,
                0,
            )
        };

        if result == libc::MAP_FAILED {
            return None;
        }

        NonNull::new(core::ptr::slice_from_raw_parts_mut(result.cast(), size))
    }

    #[inline]
    unsafe fn release(
        &mut self,
        base: NonNull<u8>,
        _commited_size: usize,
        reservation_size: usize,
    ) {
        let result = unsafe { libc::munmap(base.as_ptr().cast(), reservation_size) };

        debug_assert_eq!(result, 0);
    }

    #[inline]
    unsafe fn commit(&mut self, base: NonNull<u8>, size: usize) -> Result<(), ()> {
        let is_base_aligned = ptr_utils::is_aligned_to(base.as_ptr(), COMMIT_GRANULARITY);
        let is_size_aligned = size % COMMIT_GRANULARITY == 0;

        debug_assert!(is_base_aligned);
        debug_assert!(is_size_aligned);

        let result = unsafe {
            libc::mprotect(base.as_ptr().cast(), size, libc::PROT_READ | libc::PROT_WRITE)
        };

        (result == 0).then_some(()).ok_or(())
    }

    #[inline]
    unsafe fn discard(&mut self, base: NonNull<u8>, size: usize) {
        let is_base_aligned = ptr_utils::is_aligned_to(base.as_ptr(), COMMIT_GRANULARITY);
        let is_size_aligned = size % COMMIT_GRANULARITY == 0;

        debug_assert!(is_base_aligned);
        debug_assert!(is_size_aligned);

        #[cfg(target_os = "linux")]
        let result = unsafe { libc::madvise(base.as_ptr().cast(), size, libc::MADV_FREE) };

        #[cfg(not(target_os = "linux"))]
        let result = unsafe { libc::mprotect(base.cast(), size, libc::PROT_NONE) };

        debug_assert_eq!(result, 0);
    }

    #[inline]
    unsafe fn decommit(&mut self, base: NonNull<u8>, size: usize) {
        let is_base_aligned = ptr_utils::is_aligned_to(base.as_ptr(), COMMIT_GRANULARITY);
        let is_size_aligned = size % COMMIT_GRANULARITY == 0;

        debug_assert!(is_base_aligned);
        debug_assert!(is_size_aligned);

        #[cfg(target_os = "linux")]
        let result = unsafe { libc::madvise(base.as_ptr().cast(), size, libc::MADV_FREE) };

        #[cfg(not(target_os = "linux"))]
        let result = unsafe { libc::mprotect(base.cast(), size, libc::PROT_NONE) };

        debug_assert_eq!(result, 0);
    }

    fn commit_granularity(&mut self) -> usize {
        COMMIT_GRANULARITY
    }
}
