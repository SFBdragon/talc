//! A `Tag` just above every allocation and contains a number of bits for the allocation algorithm.

use core::ptr::NonNull;

use crate::{base::CHUNK_UNIT, node::Node};

/// Tag for allocated chunk metadata.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Tag(pub usize);

impl core::fmt::Debug for Tag {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tag")
            .field("ALLOCATED", &self.is_allocated())
            .field(
                if self.is_allocated() { "ABOVE_FREE" } else { "SMALL_FORMAT" },
                &self.is_above_free(),
            )
            .field("HEAP_END", &self.is_heap_end())
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
    pub const FLAGS: usize = 0b111;

    // ========================== ALLOCATED CHUNKS ========================== //
    pub const ALLOCATED_FLAG: usize = 1 << 0; // zero for non-allocated chunks
    pub const ABOVE_FREE_FLAG: usize = 1 << 1;
    pub const HEAP_END_FLAG: usize = 1 << 2;
    pub const HEAP_BASE_FLAG: usize = 1 << 3;

    // ============================ FREE CHUNKS ============================ //
    // ALLOCATED_FlAG is unset
    pub const SMALL_FORMAT_FLAG: usize = 1 << 1;
    // pub const HEAP_END_FLAG: usize = 1 << 2;

    // No more flags are possible as all chunks are at least 8-bytes aligned (CHUNK_UNIT on 32-bit)
    // and log2(8) = 3 bits we can use for flags. (High-bit pointer tagging isn't possible on 32-bit.)
    // If more tags are needed, the metadata memory layout will need to be changed.

    pub const ALLOCATED: Tag = Tag(Self::ALLOCATED_FLAG);
    pub const ABOVE_FREE: Tag = Tag(Self::ABOVE_FREE_FLAG);
    pub const HEAP_BASE: Tag = Tag(Self::HEAP_BASE_FLAG);
    pub const HEAP_END: Tag = Tag(Self::HEAP_END_FLAG);

    #[inline]
    pub fn new_next_of_prev(ptr: *mut Option<NonNull<Node>>) -> Self {
        Tag(ptr as usize)
    }

    #[inline]
    pub fn new_next_of_prev_small(ptr: *mut Option<NonNull<Node>>) -> Self {
        Tag(ptr as usize | Self::SMALL_FORMAT_FLAG)
    }

    #[inline]
    pub fn next_of_prev_small(self) -> *mut Option<NonNull<Node>> {
        (self.0 & !Self::FLAGS) as *mut _
    }

    #[inline]
    pub fn assume_next_of_prev(self) -> *mut Option<NonNull<Node>> {
        self.0 as *mut _
    }

    /// Determine the size of the gap from the tag of a gap.
    ///
    /// This branches based on the `SMALL_FORMAT` flag.
    #[inline]
    pub fn gap_size(self) -> usize {
        if self.is_small_format() { CHUNK_UNIT } else { self.0 & !Self::FLAGS }
    }

    #[inline]
    pub fn is_above_free(self) -> bool {
        self.0 & Self::ABOVE_FREE_FLAG != 0
    }

    #[inline]
    pub fn is_small_format(self) -> bool {
        self.0 & Self::SMALL_FORMAT_FLAG != 0
    }

    #[inline]
    pub fn is_allocated(self) -> bool {
        self.0 & Self::ALLOCATED_FLAG != 0
    }

    #[inline]
    pub fn is_heap_base(self) -> bool {
        self.0 & Self::HEAP_BASE_FLAG == Self::HEAP_BASE_FLAG
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
