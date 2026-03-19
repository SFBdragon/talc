//! A `Tag` just above every allocation and contains a number of bits for the allocation algorithm.

/// Tag for allocated chunk metadata.
#[derive(Clone, Copy)]
pub struct Tag(pub u8);

impl core::fmt::Debug for Tag {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tag")
            .field("is_allocated", &self.is_allocated())
            .field("is_above_free", &self.is_above_free())
            .field("is_heap_base", &self.is_heap_base())
            .field("is_heap_end", &self.is_heap_end())
            .finish()
    }
}

impl core::ops::BitOr for Tag {
    type Output = Tag;

    #[inline]
    fn bitor(self, rhs: Self) -> Self::Output {
        Tag(self.0 | rhs.0)
    }
}

impl core::ops::BitAnd for Tag {
    type Output = Tag;

    #[inline]
    fn bitand(self, rhs: Self) -> Self::Output {
        Tag(self.0 & rhs.0)
    }
}

impl core::ops::BitOrAssign for Tag {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl Tag {
    pub const ALLOCATED_FLAG: u8 = 1 << 0;
    pub const ABOVE_FREE_FLAG: u8 = 1 << 1;
    pub const HEAP_BASE_FLAG: u8 = 1 << 2;
    pub const HEAP_END_FLAG: u8 = 1 << 3;

    pub const ALLOCATED: Tag = Tag(Self::ALLOCATED_FLAG);
    pub const ABOVE_FREE: Tag = Tag(Self::ABOVE_FREE_FLAG);
    pub const HEAP_BASE: Tag = Tag(Self::HEAP_BASE_FLAG);
    pub const HEAP_END: Tag = Tag(Self::HEAP_END_FLAG);

    #[inline]
    pub fn is_above_free(self) -> bool {
        self.0 & Self::ABOVE_FREE_FLAG != 0
    }

    #[inline]
    pub fn is_allocated(self) -> bool {
        self.0 & Self::ALLOCATED_FLAG != 0
    }

    #[inline]
    pub fn is_heap_base(self) -> bool {
        self.0 & Self::HEAP_BASE_FLAG != 0
    }

    #[inline]
    pub fn is_heap_end(self) -> bool {
        self.0 & Self::HEAP_END_FLAG != 0
    }

    #[inline]
    #[track_caller]
    pub unsafe fn set_above_free(ptr: *mut Self) {
        debug_assert!((*ptr).0 & Self::ABOVE_FREE_FLAG == 0);
        (*ptr).0 |= Self::ABOVE_FREE_FLAG;
    }

    #[inline]
    #[track_caller]
    pub unsafe fn clear_above_free(ptr: *mut Self) {
        debug_assert!((*ptr).0 & Self::ABOVE_FREE_FLAG != 0);
        (*ptr).0 ^= Self::ABOVE_FREE_FLAG;
    }

    #[inline]
    #[track_caller]
    pub unsafe fn set_end_flag(ptr: *mut Self) {
        debug_assert!((*ptr).0 & Self::HEAP_END_FLAG == 0);
        (*ptr).0 ^= Self::HEAP_END_FLAG;
    }

    #[inline]
    #[track_caller]
    pub unsafe fn clear_end_flag(ptr: *mut Self) {
        debug_assert!((*ptr).0 & Self::HEAP_END_FLAG != 0);
        (*ptr).0 ^= Self::HEAP_END_FLAG;
    }
}
