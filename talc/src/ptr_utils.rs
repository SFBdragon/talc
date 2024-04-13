//! Generic utilities for pointer handling and sizing.

pub const WORD_SIZE: usize = core::mem::size_of::<usize>();
pub const WORD_BITS: usize = usize::BITS as usize;
pub const ALIGN: usize = core::mem::align_of::<usize>();

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

pub fn align_down(ptr: *mut u8) -> *mut u8 {
    ptr.wrapping_sub(ptr as usize % ALIGN)
}
pub fn align_up_overflows(ptr: *mut u8) -> bool {
    ALIGN - 1 > usize::MAX - ptr as usize
}
pub fn align_up(ptr: *mut u8) -> *mut u8 {
    debug_assert!(!align_up_overflows(ptr));

    let offset_ptr = ptr.wrapping_add(ALIGN - 1);
    offset_ptr.wrapping_sub(offset_ptr as usize % ALIGN)
}

#[cfg(test)]
mod tests {
    use core::ptr::null_mut;

    use super::*;

    #[test]
    fn align_ptr_test() {
        assert!(!align_up_overflows(null_mut()));
        assert!(!align_up_overflows(null_mut::<u8>().wrapping_sub(ALIGN)));
        assert!(align_up_overflows(null_mut::<u8>().wrapping_sub(ALIGN - 1)));
        assert!(align_up_overflows(null_mut::<u8>().wrapping_sub(ALIGN - 2)));
        assert!(align_up_overflows(null_mut::<u8>().wrapping_sub(ALIGN - 3)));

        assert!(align_up(null_mut()) == null_mut());
        assert!(align_down(null_mut()) == null_mut());

        assert!(align_up(null_mut::<u8>().wrapping_add(1)) == null_mut::<u8>().wrapping_add(ALIGN));
        assert!(align_up(null_mut::<u8>().wrapping_add(2)) == null_mut::<u8>().wrapping_add(ALIGN));
        assert!(align_up(null_mut::<u8>().wrapping_add(3)) == null_mut::<u8>().wrapping_add(ALIGN));
        assert!(
            align_up(null_mut::<u8>().wrapping_add(ALIGN)) == null_mut::<u8>().wrapping_add(ALIGN)
        );

        assert!(align_down(null_mut::<u8>().wrapping_add(1)) == null_mut::<u8>());
        assert!(align_down(null_mut::<u8>().wrapping_add(2)) == null_mut::<u8>());
        assert!(align_down(null_mut::<u8>().wrapping_add(3)) == null_mut::<u8>());
        assert!(
            align_down(null_mut::<u8>().wrapping_add(ALIGN))
                == null_mut::<u8>().wrapping_add(ALIGN)
        );
    }
}
