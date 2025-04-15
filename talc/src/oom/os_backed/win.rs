use windows_sys::Win32::System::Memory::*;

use core::{
    num::NonZeroUsize,
    ptr::{NonNull, null_mut},
};

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
    const INIT: Self = Self;

    #[inline]
    fn reserve(&mut self, min_size: NonZeroUsize) -> Option<NonNull<[u8]>> {
        let reserve_size =
            (min_size.get() + (RESERVED_BLOCK_DEFAULT - 1)) & !(RESERVED_BLOCK_DEFAULT - 1);

        let memory = unsafe { VirtualAlloc(null_mut(), reserve_size, MEM_RESERVE, 0) };

        let memory = NonNull::new(memory.cast::<u8>())?;
        Some(ptr_utils::nonnull_slice_from_raw_parts(memory, reserve_size))
    }

    #[inline]
    unsafe fn release(&mut self, base: NonNull<u8>, _reservation_size: usize) {
        let successful = unsafe { VirtualFree(base.as_ptr().cast(), 0, MEM_RELEASE) };

        assert!(successful != 0);
    }

    #[inline]
    unsafe fn commit(&mut self, base: NonNull<u8>, size: usize) {
        let result =
            unsafe { VirtualAlloc(base.as_ptr().cast(), size, MEM_COMMIT, PAGE_READWRITE) };

        debug_assert_eq!(result, base.as_ptr().cast());
    }

    unsafe fn decommit(&mut self, top: NonNull<u8>, size: usize) {
        let decommit_base = top.as_ptr().wrapping_sub(size).cast();
        let result = unsafe { VirtualFree(decommit_base, size, MEM_DECOMMIT) };

        debug_assert!(result != 0);
    }

    fn commit_granularity(&mut self) -> usize {
        COMMIT_GRANULARITY_DEFAULT
    }
}
