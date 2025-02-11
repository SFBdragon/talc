use core::ptr::NonNull;

#[cfg_attr(feature = "disable-realloc-in-place", expect(dead_code))]
#[inline]
pub fn is_aligned_to(ptr: *mut u8, align: usize) -> bool {
    (ptr as usize).trailing_zeros() >= align.trailing_zeros()
}

#[inline]
pub fn nonnull_slice_from_raw_parts(nn: NonNull<u8>, len: usize) -> NonNull<[u8]> {
    // SAFETY: if `nn` is non-null, then the resulting slice is non-null
    unsafe { NonNull::new_unchecked(core::ptr::slice_from_raw_parts_mut(nn.as_ptr(), len)) }
}

#[inline]
pub fn align_down_by(ptr: *mut u8, align_mask: usize) -> *mut u8 {
    // this incantation maintains provenance of ptr for MIRI
    // while allowing the compiler to see through the wrapping_add and optimize it
    ptr.wrapping_sub(ptr as usize & align_mask)
}

#[inline]
pub fn align_up_by(ptr: *mut u8, align_mask: usize) -> *mut u8 {
    // this incantation maintains provenance of ptr for MIRI
    // while allowing the compiler to see through the wrapping_add and optimize it
    ptr.wrapping_add(
        ((ptr as usize).wrapping_add(align_mask) & !align_mask).wrapping_sub(ptr as usize),
    )
    // equivalent to the following:
    // ((ptr as usize + align_mask) & !align_mask) as *mut u8
    // i.e. just align up to the next align_mask + 1
}