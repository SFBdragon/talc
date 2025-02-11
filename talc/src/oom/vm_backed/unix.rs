use core::{num::NonZeroUsize, ptr::{addr_of_mut, NonNull}};

use super::ReserveCommitDecommitRelease;



const RESERVED_BLOCK_DEFAULT: usize = 2 << 20;
const COMMIT_GRANULARITY_DEFAULT: usize = 64 << 10;

#[derive(Debug)]
pub struct UnixMMapSource;

unsafe impl ReserveCommitDecommitRelease for UnixMMapSource {
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

        NonNull::new(core::ptr::slice_from_raw_parts_mut(x.cast(), size))
    }

    #[inline]
    unsafe fn release(&mut self, base: NonNull<u8>, reservation_size: NonZeroUsize) {
        let result = unsafe {
            libc::munmap(base.as_ptr().cast(), reservation_size.get())
        };

        // using debug_assert may result in allocations and thus would violate the impl safety contract
        // in practice, this would typically result in a deadlock
        #[cfg(debug_assertions)]
        if result != 0 {
            libc::abort();
        }
    }

    #[inline]
    unsafe fn commit(&mut self, base: NonNull<u8>, min_size: NonZeroUsize) -> NonNull<u8> {
        let unit_l1 = COMMIT_GRANULARITY_DEFAULT - 1;
        let size = (min_size.get() + unit_l1) & !unit_l1;
        
        let result = unsafe {
            libc::mprotect(
                base.as_ptr().cast(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
            )
        };

        // using debug_assert may result in allocations and thus would violate the impl safety contract
        // in practice, this would typically result in a deadlock
        #[cfg(debug_assertions)]
        if result != 0 {
            libc::abort();
        }

        NonNull::new_unchecked(base.as_ptr().wrapping_add(size))
    }
    
    #[inline]
    unsafe fn decommit(&mut self, top: NonNull<u8>, max_size: NonZeroUsize) -> NonNull<u8> {
        let unit_l1 = COMMIT_GRANULARITY_DEFAULT - 1;
        let size = max_size.get() & !unit_l1;
        let base = top.as_ptr().wrapping_sub(size);

        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::madvise(
                base.cast(),
                size,
                libc::MADV_FREE,
            )
        };
        
        #[cfg(not(target_os = "linux"))]
        let result = unsafe {
            libc::mprotect(
                base.cast(),
                size,
                libc::PROT_NONE,
            )
        };

        // using debug_assert may result in allocations and thus would violate the impl safety contract
        // in practice, this would typically result in a deadlock
        #[cfg(debug_assertions)]
        if result != 0 {
            libc::abort();
        }

        NonNull::new_unchecked(base)
    }
}
