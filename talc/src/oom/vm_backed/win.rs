use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Threading::*;

use core::{num::NonZeroUsize, ptr::{addr_of_mut, null_mut, NonNull}};

use crate::ptr_utils;

use super::ReserveCommitDecommitRelease;


const RESERVED_BLOCK_DEFAULT: usize = 2 << 20;
/// In practice, the Windows allocation granularity, `dwAllocationGranularity`
/// is always 64KiB since Windows NT.
/// The page size is always smaller in practice. 4KiB. (8KiB for Intel Itanium.)
const COMMIT_GRANULARITY_DEFAULT: usize = 64 << 10;

#[derive(Debug)]
pub struct Win32VirtualAllocSource;

unsafe impl ReserveCommitDecommitRelease for Win32VirtualAllocSource {
    #[inline]
    fn reserve(&mut self, min_size: NonZeroUsize) -> Option<NonNull<[u8]>> {
        let reserve_size = (min_size.get() + (RESERVED_BLOCK_DEFAULT-1)) & !(RESERVED_BLOCK_DEFAULT-1);
        
        let memory = unsafe {
            VirtualAlloc(
                null_mut(),
                reserve_size,
                MEM_RESERVE,
                0,
            )
        };

        let memory = NonNull::new(memory.cast::<u8>())?;
        Some(ptr_utils::nonnull_slice_from_raw_parts(memory, reserve_size))
    }

    #[inline]
    unsafe fn release(&mut self, base: NonNull<u8>, _reservation_size: NonZeroUsize) {
        let successful = unsafe {
            VirtualFree(
                base.as_ptr().cast(),
                0,
                MEM_RELEASE,
            )
        };

        assert!(successful != 0);
    }

    #[inline]
    unsafe fn commit(&mut self, base: NonNull<u8>, min_size: NonZeroUsize) -> NonNull<u8> {
        let commit_size = (min_size.get() + (COMMIT_GRANULARITY_DEFAULT-1)) & !(COMMIT_GRANULARITY_DEFAULT-1);
        
        let result = unsafe {
            VirtualAlloc(
                base.as_ptr().cast(),
                commit_size,
                MEM_COMMIT,
                PAGE_READWRITE,
            )
        };

        debug_assert_eq!(result, base.as_ptr().cast());

        NonNull::new_unchecked(base.as_ptr().wrapping_add(commit_size))
    }
    
    unsafe fn decommit(&mut self, top: NonNull<u8>, max_size: NonZeroUsize) -> NonNull<u8> {
        let commit_size = max_size.get() & !(COMMIT_GRANULARITY_DEFAULT-1);
        let base = top.as_ptr().wrapping_sub(commit_size);

        let result = unsafe {
            VirtualAlloc(
                base.cast(),
                commit_size,
                MEM_COMMIT,
                PAGE_READWRITE,
            )
        };

        debug_assert_eq!(result, base.cast());

        NonNull::new_unchecked(base)
    }
}
