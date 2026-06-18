//! Chunk layout helpers and the constants that define it.
//!
//! A heap is a contiguous run of chunks, each `[base, end)` with base and size
//! aligned to `CHUNK_UNIT`. The last word, `[end - TAIL_SIZE, end)`, is a
//! metadata slot the upper neighbour reads (via `end_to_tag`) to classify it:
//!
//! - **Allocated:** caller data in `[base, ..)`, with the [`Tag`] in that word
//!   (low bits = flags). `required_chunk_size` reserves it so data can't reach it.
//! - **Gap (≥ `CHUNK_UNIT` = 4 words):** the intrusive `Node` (2 words), a bin
//!   index (`u32`), and its size as a boundary tag at both `base + 3 words` and
//!   the trailing word (they coincide for a minimal gap).
//!
//! To tell the two apart Talc reads that trailing word and tests `ALLOCATED`. A
//! gap's `CHUNK_UNIT`-aligned size has zero low bits (where every flag lives),
//! while an allocation sets the bit. Reading the whole word as a `usize` keeps
//! this independent of byte order; a byte-sized read would only hold on
//! little-endian.

use core::{mem::size_of, ptr::NonNull};

use crate::{base::tag::Tag, node::Node, ptr_utils};

/// Returns whether the two pointers are greater than `CHUNK_UNIT` apart.
#[inline]
pub(crate) fn is_chunk_size(base: *mut u8, end: *mut u8) -> bool {
    end as usize - base as usize >= CHUNK_UNIT
}

#[inline]
pub(crate) const fn required_chunk_size(size: usize) -> usize {
    (size + TAIL_SIZE + (CHUNK_UNIT - 1)) & !(CHUNK_UNIT - 1)
}

#[inline]
pub(crate) unsafe fn alloc_to_end(base: *mut u8, size: usize) -> *mut u8 {
    base.wrapping_add(required_chunk_size(size))
}

/// The minimum size and alignment that Talc will use for chunks.
///
/// It may situationally take on other values in the future.
pub const CHUNK_UNIT: usize = size_of::<usize>() * 4;

/// The trailing metadata word of every chunk: an allocation's [`Tag`] or a gap's
/// size. A gap size is `CHUNK_UNIT`-aligned, so reading this word as a `usize`
/// and testing the low (flag) bits tells the two apart on any endianness.
pub(crate) const TAIL_SIZE: usize = size_of::<usize>();

// The discriminator (and packing END_FLAG into a gap's size) needs CHUNK_UNIT to
// be a power of two and every Tag flag to fit in the low bits that are always
// zero in a CHUNK_UNIT-aligned size.
const _: () = assert!(CHUNK_UNIT.is_power_of_two());
const _: () = assert!(
    (Tag::ALLOCATED_FLAG | Tag::ABOVE_FREE_FLAG | Tag::HEAP_BASE_FLAG | Tag::HEAP_END_FLAG)
        < CHUNK_UNIT,
);

const GAP_NODE_OFFSET: usize = 0;
const GAP_BIN_OFFSET: usize = size_of::<usize>() * 2;
const GAP_LOW_SIZE_OFFSET: usize = size_of::<usize>() * 3;
/// Trailing boundary-tag size, at `end - TAIL_SIZE` (the discriminator slot).
const GAP_HIGH_SIZE_OFFSET: usize = TAIL_SIZE;

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
    end.sub(TAIL_SIZE).cast()
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
