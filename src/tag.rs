//! A `Tag` is a size with flags in the least significant
//! bits and most significant bit for allocated chunks.

// const UNUSED_BITS: usize = 2; //crate::ALIGN.ilog2();
// on 64 bit machines we have unused 3 bits to work with but
// let's keep it more portable for now.

use crate::ALIGN;

/// Tag for allocated chunk metadata.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Tag(pub(crate) *mut u8);

impl core::fmt::Debug for Tag {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tag")
            .field("is_allocated", &self.is_allocated())
            .field("is_above_free", &self.is_above_free())
            .field("base_ptr", &format_args!("{:p}", self.base_ptr()))
            .finish()
    }
}

impl Tag {
    pub const ALLOCATED_FLAG: usize = 1 << 0; // pointers are always aligned to 4 bytes at least
    pub const IS_ABOVE_FREE_FLAG: usize = 1 << 1; // pointers are always aligned to 4 bytes at least

    const BASE: usize = !(Self::ALLOCATED_FLAG | Self::IS_ABOVE_FREE_FLAG);

    pub unsafe fn write(tag_ptr: *mut u8, base: *mut u8, is_above_free: bool) {
        debug_assert!(base as usize & !Self::BASE == 0);

        tag_ptr.cast::<Tag>().write(if is_above_free {
            Self(base.wrapping_add(Self::IS_ABOVE_FREE_FLAG | Self::ALLOCATED_FLAG))
        } else {
            Self(base.wrapping_add(Self::ALLOCATED_FLAG))
        })
    }

    pub fn base_ptr(self) -> *mut u8 {
        self.0.wrapping_sub(self.0 as usize % ALIGN)
    }

    pub fn is_above_free(self) -> bool {
        self.0 as usize & Self::IS_ABOVE_FREE_FLAG != 0
    }

    pub fn is_allocated(self) -> bool {
        self.0 as usize & Self::ALLOCATED_FLAG != 0
    }

    pub unsafe fn set_above_free(ptr: *mut Self) {
        let tag = *ptr;
        debug_assert!(!tag.is_above_free());
        let tag = Self(tag.0.wrapping_add(Self::IS_ABOVE_FREE_FLAG));
        debug_assert!(tag.is_above_free());
        *ptr = tag;
    }
    pub unsafe fn clear_above_free(ptr: *mut Self) {
        let tag = *ptr;
        debug_assert!(tag.is_above_free());
        let tag = Self(tag.0.wrapping_sub(Self::IS_ABOVE_FREE_FLAG));
        debug_assert!(!tag.is_above_free());
        *ptr = tag;
    }
}
