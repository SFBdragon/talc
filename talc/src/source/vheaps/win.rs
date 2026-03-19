//! UNFINISHED - OPEN AN ISSUE IF YOU WANT OS VIRTUAL MEMORY INTEGRATION
//!
//! An attempt at making a decent abstraction over Window's Virtual Allocation API.
//!
//! If you know a thing or two about low-level system allocation on Windows,
//! scrutiny on my implementation would be appreciated.

use windows_sys::Win32::System::Memory::*;

use core::{
    num::NonZeroUsize,
    ptr::{NonNull, null_mut},
};

use crate::ptr_utils;

use super::VirtualHeaps;

const RESERVED_BLOCK_DEFAULT: usize = 8 << 20;
/// In practice, the Windows allocation granularity, `dwAllocationGranularity`
/// is always 64KiB since Windows NT.
/// The page size is always smaller in practice. 4KiB. (8KiB for Intel Itanium.)
const COMMIT_GRANULARITY_DEFAULT: usize = 256 << 10;

#[derive(Debug)]
pub struct Win32VirtualAllocSource;

// SAFETY:
// Does not interact with any Rust allocators, and thus does not create a second mutable reference to Talc.
// See [`Source`]'s safety contract for why this is important.
unsafe impl VirtualHeaps for Win32VirtualAllocSource {
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

        debug_assert!(successful != 0);
    }

    #[inline]
    unsafe fn commit(&mut self, base: NonNull<u8>, size: usize) -> Result<(), ()> {
        let result =
            unsafe { VirtualAlloc(base.as_ptr().cast(), size, MEM_COMMIT, PAGE_READWRITE) };

        debug_assert_eq!(result, base.as_ptr().cast());
    }

    unsafe fn decommit(&mut self, top: NonNull<u8>, size: usize) {
        let decommit_base = top.as_ptr().wrapping_sub(size).cast();
        let result = unsafe { DiscardVirtualHeaps(decommit_base, size) };

        debug_assert!(result != 0);
    }

    fn commit_granularity(&mut self) -> usize {
        COMMIT_GRANULARITY_DEFAULT
    }
}
