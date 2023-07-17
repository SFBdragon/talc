//! A `Tag` is a size with flags in the least significant
//! bits and most significant bit for allocated chunks.

// const UNUSED_BITS: usize = 2; //crate::ALIGN.ilog2();
// on 64 bit machines we have unused 3 bits to work with but
// let's keep it more portable for now.

/// Tag for allocated chunk metadata.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Tag(pub(crate) usize);

impl core::fmt::Debug for Tag {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tag")
            .field("is_allocated", &self.is_allocated())
            .field("is_low_free", &self.is_low_free())
            .field("acme_ptr", &format_args!("{:p}", self.acme_ptr()))
            .finish()
    }
}

impl Tag {
    pub const ALLOCATED_FLAG: usize = 1 << 0; // pointers are always aligned to 4 bytes at least
    pub const IS_LOW_FREE_FLAG: usize = 1 << 1; // pointers are always aligned to 4 bytes at least

    const ACME: usize = !(Self::ALLOCATED_FLAG | Self::IS_LOW_FREE_FLAG);

    pub fn new(acme: *mut u8, low_free: bool) -> Self {
        debug_assert!(acme as usize & !Self::ACME == 0);

        if low_free {
            Self(acme as usize | Self::IS_LOW_FREE_FLAG | Self::ALLOCATED_FLAG)
        } else {
            Self(acme as usize | Self::ALLOCATED_FLAG)
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
        (self.0 & Self::ACME) as *mut u8
    }

    pub fn is_low_free(self) -> bool {
        self.0 & Self::IS_LOW_FREE_FLAG != 0
    }

    pub fn is_allocated(self) -> bool {
        self.0 & Self::ALLOCATED_FLAG != 0
    }

    pub unsafe fn set_low_free(ptr: *mut Self) {
        let tag = *ptr;
        debug_assert!(!tag.is_low_free());
        let tag = Self(tag.0 ^ Self::IS_LOW_FREE_FLAG);
        debug_assert!(tag.is_low_free());
        *ptr = tag;
    }
    pub unsafe fn clear_low_free(ptr: *mut Self) {
        let tag = *ptr;
        debug_assert!(tag.is_low_free());
        let tag = Self(tag.0 ^ Self::IS_LOW_FREE_FLAG);
        debug_assert!(!tag.is_low_free());
        *ptr = tag;
    }

    /* pub unsafe fn toggle_allocated(ptr: *mut Self) {
        *ptr = Self((*ptr).0 ^ Self::ALLOCATED_FLAG);
    } */

    pub unsafe fn set_acme(ptr: *mut Self, acme: *mut u8) {
        debug_assert!(acme as usize & !Self::ACME == 0);

        *ptr = Tag(acme as usize | ((*ptr).0 & !Self::ACME));
    }
}
