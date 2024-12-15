//! Generic utilities for pointer handling and sizing.

/// Aligns `ptr` up to the next `align_mask + 1`.
///
/// `align_mask` must be a power of two minus one.
#[inline]
pub fn align_up_by(ptr: *mut u8, align_mask: usize) -> *mut u8 {
    debug_assert!((align_mask + 1).is_power_of_two());

    // this incantation maintains provenance of ptr
    // while allowing the compiler to see through the wrapping_add and optimize it
    ptr.wrapping_add(((ptr as usize + align_mask) & !align_mask) - ptr as usize)
    // equivalent to the following:
    // ((ptr as usize + align_mask) & !align_mask) as *mut u8
    // i.e. just align up to the next align_mask + 1
}

/// Aligns `ptr` down to `align_mask + 1`.
///
/// `align_mask` must be a power of two minus one.
pub fn align_down_by(ptr: *mut u8, align_mask: usize) -> *mut u8 {
    debug_assert!((align_mask + 1).is_power_of_two());

    ptr.wrapping_sub(ptr as usize & align_mask)
}

