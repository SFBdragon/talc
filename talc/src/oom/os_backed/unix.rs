use core::{num::NonZeroUsize, ptr::NonNull};

use crate::ptr_utils;

use super::ReserveCommitDecommitRelease;

const RESERVED_BLOCK_DEFAULT: usize = 4 << 20;
const COMMIT_GRANULARITY_DEFAULT: usize = 128 << 10;

#[derive(Debug)]
pub struct UnixMMapSource;

unsafe impl ReserveCommitDecommitRelease for UnixMMapSource {
    const INIT: Self = Self;

    #[inline]
    fn reserve(&mut self, min_size: NonZeroUsize) -> Option<NonNull<[u8]>> {
        let unit_l1 = RESERVED_BLOCK_DEFAULT - 1;
        let size = (min_size.get() + unit_l1) & !unit_l1;

        let x = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                size,
                libc::PROT_NONE,
                libc::MAP_ANONYMOUS | libc::MAP_PRIVATE,
                -1,
                0,
            )
        };

        if x == libc::MAP_FAILED {
            return None;
        }

        // eprintln!("RESERVE  {:p}..{:p}", x.cast::<u8>(), x.cast::<u8>().wrapping_add(size));

        NonNull::new(core::ptr::slice_from_raw_parts_mut(x.cast(), size))
    }

    #[inline]
    unsafe fn release(&mut self, base: NonNull<u8>, reservation_size: usize) {
        let result = unsafe { libc::munmap(base.as_ptr().cast(), reservation_size) };

        // eprintln!(
        //     "RELEASE  {:p}..{:p}",
        //     base.as_ptr(),
        //     base.as_ptr().wrapping_add(reservation_size)
        // );

        // using debug_assert may result in allocations and thus would violate the impl safety contract
        // in practice, this would typically result in a deadlock
        #[cfg(debug_assertions)]
        if result != 0 {
            libc::abort();
        }
    }

    #[inline]
    unsafe fn commit(&mut self, base: NonNull<u8>, size: usize) {
        let is_base_aligned = ptr_utils::is_aligned_to(base.as_ptr(), COMMIT_GRANULARITY_DEFAULT);
        let is_size_aligned = size % COMMIT_GRANULARITY_DEFAULT == 0;

        // eprintln!("COMMIT   {:p}..{:p}", base.as_ptr(), base.as_ptr().wrapping_add(size));

        // using debug_assert may result in allocations and thus would violate the impl safety contract
        // in practice, this would typically result in a deadlock
        #[cfg(debug_assertions)]
        if !is_base_aligned || !is_size_aligned {
            libc::abort();
        }

        let result = unsafe {
            libc::mprotect(base.as_ptr().cast(), size, libc::PROT_READ | libc::PROT_WRITE)
        };

        #[cfg(debug_assertions)]
        if result != 0 {
            libc::abort();
        }
    }

    #[inline]
    unsafe fn decommit(&mut self, base: NonNull<u8>, size: usize) {
        let is_base_aligned = ptr_utils::is_aligned_to(base.as_ptr(), COMMIT_GRANULARITY_DEFAULT);
        let is_size_aligned = size % COMMIT_GRANULARITY_DEFAULT == 0;

        // eprintln!("DECOMMIT {:p}..{:p}", base.as_ptr(), base.as_ptr().wrapping_add(size));

        // using debug_assert may result in allocations and thus would violate the impl safety contract
        // in practice, this would typically result in a deadlock
        #[cfg(debug_assertions)]
        if !is_base_aligned || !is_size_aligned {
            libc::abort();
        }

        #[cfg(target_os = "linux")]
        let result = unsafe { libc::madvise(base.as_ptr().cast(), size, libc::MADV_FREE) };

        #[cfg(not(target_os = "linux"))]
        let result = unsafe { libc::mprotect(base.cast(), size, libc::PROT_NONE) };

        // using debug_assert may result in allocations and thus would violate the impl safety contract
        // in practice, this would typically result in a deadlock
        #[cfg(debug_assertions)]
        if result != 0 {
            libc::abort();
        }
    }

    fn commit_granularity(&mut self) -> usize {
        COMMIT_GRANULARITY_DEFAULT
    }
}
