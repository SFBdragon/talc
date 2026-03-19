//! A bunch of utility functions for working with chunks.
//!
//!

use core::{mem::size_of, ptr::NonNull};

use crate::{base::tag::Tag, node::Node, ptr_utils};

/// Returns whether the two pointers are greater than `CHUNK_UNIT` apart.
#[inline]
pub(crate) fn is_chunk_size(base: *mut u8, end: *mut u8) -> bool {
    end as usize - base as usize >= CHUNK_UNIT
}

#[inline]
pub(crate) const fn required_chunk_size(size: usize) -> usize {
    (size + size_of::<Tag>() + (CHUNK_UNIT - 1)) & !(CHUNK_UNIT - 1)
}

#[inline]
pub(crate) unsafe fn alloc_to_end(base: *mut u8, size: usize) -> *mut u8 {
    base.wrapping_add(required_chunk_size(size))
}

/// The minimum size and alignment that Talc will use for chunks.
///
/// It may situationally take on other values in the future.
pub const CHUNK_UNIT: usize = size_of::<usize>() * 4;

const GAP_NODE_OFFSET: usize = 0;
const GAP_BIN_OFFSET: usize = size_of::<usize>() * 2;
const GAP_LOW_SIZE_OFFSET: usize = size_of::<usize>() * 3;
const GAP_HIGH_SIZE_OFFSET: usize = size_of::<usize>();

pub const END_FLAG: usize = Tag::HEAP_END_FLAG as usize;

// WASM perf tanks if these #[inline]'s are not present
#[inline]
pub(crate) unsafe fn gap_base_to_node(base: *mut u8) -> *mut Node {
    base.add(GAP_NODE_OFFSET).cast()
}
#[inline]
pub(crate) unsafe fn gap_base_to_bin(base: *mut u8) -> *mut u32 {
    base.add(GAP_BIN_OFFSET).cast()
}
#[inline]
pub(crate) unsafe fn gap_base_to_size(base: *mut u8) -> *mut usize {
    base.add(GAP_LOW_SIZE_OFFSET).cast()
}
#[inline]
pub(crate) unsafe fn gap_end_to_size_and_flag(end: *mut u8) -> *mut usize {
    end.sub(GAP_HIGH_SIZE_OFFSET).cast()
}
#[inline]
pub(crate) unsafe fn gap_node_to_base(node: NonNull<Node>) -> *mut u8 {
    node.as_ptr().cast::<u8>().sub(GAP_NODE_OFFSET).cast()
}
#[inline]
pub(crate) unsafe fn gap_node_to_size(node: NonNull<Node>) -> *mut usize {
    node.as_ptr().cast::<u8>().sub(GAP_NODE_OFFSET).add(GAP_LOW_SIZE_OFFSET).cast()
}
#[inline]
pub(crate) unsafe fn end_to_tag(end: *mut u8) -> *mut Tag {
    end.sub(size_of::<Tag>()).cast()
}

/// Aligns `ptr` up by `CHUNK_UNIT`.
#[inline]
pub(crate) fn align_up(ptr: *mut u8) -> *mut u8 {
    ptr_utils::align_up_by(ptr, CHUNK_UNIT)
}
/// Aligns `ptr` down by `CHUNK_UNIT`.
#[inline]
pub(crate) fn align_down(ptr: *mut u8) -> *mut u8 {
    ptr_utils::align_down_by(ptr, CHUNK_UNIT)
}
