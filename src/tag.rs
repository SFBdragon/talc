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
            .field("is_low_free", &self.is_below_free())
            .field("acme_ptr", &format_args!("{:p}", self.acme_ptr()))
            .finish()
    }
}

impl Tag {
    pub const ALLOCATED_FLAG: usize = 1 << 0; // pointers are always aligned to 4 bytes at least
    pub const IS_BELOW_FREE_FLAG: usize = 1 << 1; // pointers are always aligned to 4 bytes at least

    const ACME: usize = !(Self::ALLOCATED_FLAG | Self::IS_BELOW_FREE_FLAG);

    pub fn new(acme: *mut u8, is_below_free: bool) -> Self {
        debug_assert!(acme as usize & !Self::ACME == 0);

        if is_below_free {
            Self(acme.wrapping_add(Self::IS_BELOW_FREE_FLAG | Self::ALLOCATED_FLAG))
        } else {
            Self(acme.wrapping_add(Self::ALLOCATED_FLAG))
        }
    }

    /* pub fn unallocated(acme: *mut u8, low_free: bool) -> Self {
        debug_assert!(acme as usize & !Self::ACME == 0);

        if low_free {
            Self(acme as usize | Self::IS_LOW_FREE_FLAG)
        } else {
            Self(acme as usize)
        }
    } */

    pub fn acme_ptr(self) -> *mut u8 {
        self.0.wrapping_sub(self.0 as usize % ALIGN)
    }

    pub fn is_below_free(self) -> bool {
        self.0 as usize & Self::IS_BELOW_FREE_FLAG != 0
    }

    pub fn is_allocated(self) -> bool {
        self.0 as usize & Self::ALLOCATED_FLAG != 0
    }

    pub unsafe fn set_below_free(ptr: *mut Self) {
        let tag = *ptr;
        debug_assert!(!tag.is_below_free());
        let tag = Self(tag.0.wrapping_add(Self::IS_BELOW_FREE_FLAG));
        debug_assert!(tag.is_below_free());
        *ptr = tag;
    }
    pub unsafe fn clear_below_free(ptr: *mut Self) {
        let tag = *ptr;
        debug_assert!(tag.is_below_free());
        let tag = Self(tag.0.wrapping_sub(Self::IS_BELOW_FREE_FLAG));
        debug_assert!(!tag.is_below_free());
        *ptr = tag;
    }

    /* pub unsafe fn toggle_allocated(ptr: *mut Self) {
        *ptr = Self((*ptr).0 ^ Self::ALLOCATED_FLAG);
    } */

    pub unsafe fn set_acme(ptr: *mut Self, acme: *mut u8) {
        debug_assert!(acme as usize & !Self::ACME == 0);

        *ptr = Tag(acme.wrapping_add((*ptr).0 as usize & !Self::ACME));
    }
}
