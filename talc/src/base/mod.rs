//! This module provides the core allocation mechanism via the [`Talc`] type and related configuration.

use crate::{node::Node, oom::OomHandler};
use binning::Binning;
use bitfield::BitField;
use core::{
    alloc::Layout,
    fmt::Debug,
    mem::{align_of, size_of},
    ptr::NonNull,
};
use tag::Tag;

use crate::ptr_utils;

pub mod binning;
pub mod bitfield;
mod tag;

#[cfg(feature = "counters")]
mod counters;
#[cfg(feature = "counters")]
pub use counters::Counters;

/// The minimum size and alignment that Talc will use for chunks.
///
/// Currently, this value changes if the `"cache-aligned-allocations"`
/// feature is set. It may take on other values in the future.
#[cfg(not(feature = "cache-aligned-allocations"))]
pub const CHUNK_UNIT: usize = size_of::<usize>() * 4;

/// The minimum size and alignment that Talc will use for chunks.
#[cfg(feature = "cache-aligned-allocations")]
pub const CHUNK_UNIT: usize =
    if size_of::<usize>() * 4 < align_of::<crossbeam_utils::CachePadded<u8>>() {
        align_of::<crossbeam_utils::CachePadded<u8>>()
    } else {
        size_of::<usize>() * 4
    };

const GAP_NODE_OFFSET: usize = 0;
const GAP_BIN_OFFSET: usize = size_of::<usize>() * 2;
const GAP_LOW_SIZE_OFFSET: usize = size_of::<usize>() * 3;
const GAP_HIGH_SIZE_OFFSET: usize = size_of::<usize>();

const END_FLAG: usize = tag::Tag::ARENA_END_FLAG as usize;

// WASM perf tanks if these #[inline]'s are not present
#[inline]
unsafe fn gap_base_to_node(base: *mut u8) -> *mut Node {
    base.add(GAP_NODE_OFFSET).cast()
}
#[inline]
unsafe fn gap_base_to_bin(base: *mut u8) -> *mut u32 {
    base.add(GAP_BIN_OFFSET).cast()
}
#[inline]
unsafe fn gap_base_to_size(base: *mut u8) -> *mut usize {
    base.add(GAP_LOW_SIZE_OFFSET).cast()
}
#[inline]
unsafe fn gap_end_to_size_and_flag(end: *mut u8) -> *mut usize {
    end.sub(GAP_HIGH_SIZE_OFFSET).cast()
}
#[inline]
unsafe fn gap_node_to_base(node: NonNull<Node>) -> *mut u8 {
    node.as_ptr().cast::<u8>().sub(GAP_NODE_OFFSET).cast()
}
#[inline]
unsafe fn gap_node_to_size(node: NonNull<Node>) -> *mut usize {
    node.as_ptr().cast::<u8>().sub(GAP_NODE_OFFSET).add(GAP_LOW_SIZE_OFFSET).cast()
}
#[inline]
unsafe fn end_to_tag(acme: *mut u8) -> *mut Tag {
    acme.sub(size_of::<Tag>()).cast()
}

/// The core allocator type.
///
/// To use [`Talc`] across multiple threads, e.g. as a global allocator, use [`Talck`](crate::sync::Talck).
///
/// To use [`Talc`] in a single thread, e.g. via the
/// [`Allocator`](allocator_api2::alloc::Allocator) API, use [`TalcCell`](crate::cell::TalcCell).
///
/// [`Talc`] itself does not exhibit interior mutability.
/// You need a mutable reference to allocate using [`Talc`], therefore it doesn't implement
/// [`Allocator`](allocator_api2::alloc::Allocator) or [`GlobalAlloc`](allocator_api2::alloc::GlobalAlloc)
/// itself.
///
/// # Generic Parameters
/// See the [`OomHandler`] and [`Binning`] trait documentation for more info, but in short:
///
/// - The OOM handler is effectively a callback to get more memory if allocation failure occurs.
///     Though they can be more TODO
///     Common choices are [`ErrOnOom`](crate::ErrOnOom) and [`ClaimOnOom`](crate::ClaimOnOom).
///     TODO backing allocator?
///
/// - The binning implementation determines the internal types and operations [`Talc`] uses
///     to classify chunks into free-lists and keeps track of free-list occupancy.
///     The default implementation is [`DefaultBinning`](crate::DefaultBinning).
pub struct Talc<O: OomHandler<B>, B: Binning> {
    /// Allocation statistics for this arena.
    #[cfg(feature = "counters")]
    counters: Counters,

    avails: B::AvailabilityBitField,
    free_lists: *mut Option<NonNull<Node>>,
    _phantom: core::marker::PhantomData<fn(B) -> B>,

    /// The out-of-memory handler state.
    ///
    /// This is user-accessible and can be mutated by the OOM handler routine.
    ///
    /// [`Talc`] just holds it and calls `handle_oom` on it when necessary.
    /// [`Talc`] doesn't read/write to it after initialization.
    pub oom_handler: O,
}

unsafe impl<O: OomHandler<B> + Send, B: Binning> Send for Talc<O, B> {}
unsafe impl<O: OomHandler<B> + Sync, B: Binning> Sync for Talc<O, B> where
    B::AvailabilityBitField: Sync
{
}

impl<O: OomHandler<B>, B: Binning> Debug for Talc<O, B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug_struct = f.debug_struct("Talc");

        debug_struct
            .field("availability", &self.avails)
            .field(
                "free_lists",
                &core::ptr::slice_from_raw_parts_mut(self.free_lists, B::BIN_COUNT as usize),
            )
            .field("oom_handler", &self.oom_handler);

        #[cfg(feature = "counters")]
        {
            debug_struct.field("counters", &self.counters);
        }

        debug_struct.finish()
    }
}

impl<O: OomHandler<B>, B: Binning> Talc<O, B> {
    /// Aligns `ptr` up by `CHUNK_UNIT`.
    #[inline]
    pub fn align_up(ptr: *mut u8) -> *mut u8 {
        let align_mask = CHUNK_UNIT - 1;
        ptr_utils::align_up_by(ptr, align_mask)
    }

    /// Aligns `ptr` down by `CHUNK_UNIT`.
    #[inline]
    pub fn align_down(ptr: *mut u8) -> *mut u8 {
        let align_mask = CHUNK_UNIT - 1;
        ptr_utils::align_down_by(ptr, align_mask)
    }

    /// Returns whether the two pointers are greater than `CHUNK_UNIT` apart.
    #[inline]
    fn is_chunk_size(base: *mut u8, acme: *mut u8) -> bool {
        acme as usize - base as usize >= CHUNK_UNIT
    }

    #[inline]
    pub(crate) const fn required_chunk_size(size: usize) -> usize {
        (size + size_of::<Tag>() + (CHUNK_UNIT - 1)) & !(CHUNK_UNIT - 1)
    }
    #[inline]
    unsafe fn alloc_to_acme(base: *mut u8, size: usize) -> *mut u8 {
        base.wrapping_add(Self::required_chunk_size(size))
    }

    #[inline]
    unsafe fn free_list_ptr(&self, bin: u32) -> *mut Option<NonNull<Node>> {
        debug_assert!(bin < B::BIN_COUNT);
        self.free_lists.add(bin as usize)
    }

    /// Registers a gap in memory into the free lists.
    #[cfg_attr(not(target_family = "wasm"), inline)]
    unsafe fn register_gap(&mut self, base: *mut u8, acme: *mut u8) {
        debug_assert!(Self::is_chunk_size(base, acme));

        let size = acme as usize - base as usize;
        let bin = B::size_to_bin(size).min(B::BIN_COUNT - 1);
        let bin_ptr = self.free_list_ptr(bin);

        if (*bin_ptr).is_none() {
            debug_assert!(!self.avails.read_bit(bin));
            self.avails.set_bit(bin);
        }

        Node::link_at(gap_base_to_node(base), Node { next: *bin_ptr, next_of_prev: bin_ptr });
        gap_base_to_bin(base).write(bin);
        gap_base_to_size(base).write(size);
        gap_end_to_size_and_flag(acme).write(size);

        debug_assert!((*bin_ptr).is_some());

        #[cfg(feature = "counters")]
        self.counters.account_register_gap(size);
    }

    /// De-registers memory from the free lists.
    #[cfg_attr(not(target_family = "wasm"), inline)]
    unsafe fn deregister_gap(&mut self, base: *mut u8, size: usize) {
        debug_assert!(
            (*self.free_list_ptr(B::size_to_bin(size).min(B::BIN_COUNT - 1))).is_some(),
            "{} {} {:?}",
            size,
            B::size_to_bin(size),
            self.avails
        );

        #[cfg(feature = "counters")]
        self.counters.account_deregister_gap(size);

        Node::unlink(gap_base_to_node(base).read());

        let bin = gap_base_to_bin(base).read();
        if (*self.free_list_ptr(bin)).is_none() {
            debug_assert!(self.avails.read_bit(bin));
            self.avails.clear_bit(bin);
        }
    }

    /// Allocate a contiguous region of memory according to `layout`, if possible.
    ///
    /// # Safety
    /// `layout.size()` must be nonzero.
    pub unsafe fn allocate(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        self.scan_for_errors();

        debug_assert!(layout.size() != 0);

        let required_chunk_size = Self::required_chunk_size(layout.size());

        let (base, chunk_end) = 'search: loop {
            // This is allowed to return values >= B::BIN_COUNT.
            // This indicates that the last bucket is our only bet,
            // and the allocations therein are not necessarily big enough.
            let bin = B::size_to_bin_ceil(required_chunk_size);

            // special case, this is a large allocation, dig around the last bin
            if bin >= (B::BIN_COUNT - 1) {
                if self.avails.read_bit(B::BIN_COUNT - 1) {
                    if let Some(success) = self.full_search_bin(
                        B::BIN_COUNT - 1,
                        required_chunk_size,
                        layout.align() - 1,
                    ) {
                        break 'search success;
                    }
                }

                O::handle_oom(self, layout).ok()?;
                continue 'search;
            }

            let mut b = self.avails.bit_scan_after(bin);

            // Handle the case where it turns out there's no feasible bins available.
            if b >= B::BIN_COUNT {
                if self.avails.read_bit(bin - 1) {
                    if let Some(success) =
                        self.full_search_bin(bin - 1, required_chunk_size, layout.align() - 1)
                    {
                        break 'search success;
                    }
                }

                O::handle_oom(self, layout).ok()?;
                continue 'search;
            }

            if layout.align() <= CHUNK_UNIT {
                let node_ptr = self.free_list_ptr(b).read().unwrap_unchecked();
                let mut size = gap_node_to_size(node_ptr).read();

                if O::TRACK_ARENA_END {
                    size &= !END_FLAG;
                }

                debug_assert!(size >= required_chunk_size);

                let base = gap_node_to_base(node_ptr);
                self.deregister_gap(base, size);

                Tag::clear_above_free(end_to_tag(base));

                break 'search (base, base.add(size));
            } else {
                // a larger than word-size alignment is demanded
                // therefore each chunk is manually checked to be sufficient accordingly
                let align_mask = layout.align() - 1;

                loop {
                    if let Some(res) = self.full_search_bin(b, required_chunk_size, align_mask) {
                        break 'search res;
                    }

                    if b + 1 < B::BIN_COUNT || B::AvailabilityBitField::BITS > B::BIN_COUNT {
                        b = self.avails.bit_scan_after(b + 1);

                        if b < B::BIN_COUNT {
                            continue;
                        }
                    }

                    if let Some(res) =
                        self.full_search_bin(bin - 1, required_chunk_size, align_mask)
                    {
                        break 'search res;
                    }

                    O::handle_oom(self, layout).ok()?;
                    continue 'search;
                }
            }
        };

        debug_assert_eq!(Self::align_down(base), base);

        let end = base.add(required_chunk_size);
        let mut tag = Tag::ALLOCATED;

        if O::TRACK_ARENA_END && *gap_end_to_size_and_flag(chunk_end) & END_FLAG != 0 {
            // handle the space above the required allocation span
            if end != chunk_end {
                self.register_gap(end, chunk_end);
                *gap_end_to_size_and_flag(chunk_end) |= END_FLAG;

                tag |= Tag::ABOVE_FREE;
            } else {
                tag |= Tag::ARENA_END;
            }
        } else {
            // handle the space above the required allocation span
            if end != chunk_end {
                self.register_gap(end, chunk_end);
                tag |= Tag::ABOVE_FREE;
            }
        }

        #[cfg(feature = "counters")]
        self.counters.account_alloc(layout.size());

        end_to_tag(end).write(tag);

        Some(NonNull::new_unchecked(base))
    }

    #[cold]
    unsafe fn full_search_bin(
        &mut self,
        bin: u32,
        required_size: usize,
        align_mask: usize,
    ) -> Option<(*mut u8, *mut u8)> {
        for node_ptr in Node::iter_mut(*self.free_list_ptr(bin)) {
            let mut size = gap_node_to_size(node_ptr).read();

            if O::TRACK_ARENA_END {
                size &= !END_FLAG;
            }

            let base = gap_node_to_base(node_ptr);
            let acme = base.add(size);
            // calculate the lowest aligned pointer above the tag-offset free chunk pointer
            let aligned_base = ptr_utils::align_up_by(base, align_mask);

            // if the remaining size is sufficient, remove the chunk from the books and return
            if aligned_base.add(required_size) <= acme {
                self.deregister_gap(base, size);

                // determine the base of the allocated chunk
                // if the amount of memory below the chunk is too small, subsume it, else free it
                if base != aligned_base {
                    self.register_gap(base, aligned_base);
                } else {
                    Tag::clear_above_free(end_to_tag(base));
                }

                return Some((aligned_base, acme));
            }
        }

        None
    }

    /// Free an allocation.
    ///
    /// # Safety
    /// `ptr` must have been previously allocated given `layout`.
    pub unsafe fn deallocate(&mut self, ptr: *mut u8, layout: Layout) {
        // self.scan_for_errors();

        #[cfg(feature = "counters")]
        self.counters.account_dealloc(layout.size());

        let mut chunk_base = ptr;
        let mut chunk_acme = Self::alloc_to_acme(ptr, layout.size());
        let tag = end_to_tag(chunk_acme).read();

        let mut is_arena_end = tag.is_arena_end();

        debug_assert!(tag.is_allocated());
        debug_assert!(Self::is_chunk_size(chunk_base, chunk_acme));

        // Try to recombine with a gap below, if it's there.
        // This gap is never the end of the arena, so we don't need to worry about the presence of an end flag.
        if !end_to_tag(chunk_base).read().is_allocated() {
            let below_size = gap_end_to_size_and_flag(chunk_base).read();
            debug_assert!(below_size & END_FLAG == 0);

            // Calculate the base pointer for the gap below.
            let below_base = chunk_base.sub(below_size);
            self.deregister_gap(below_base, below_size);
            chunk_base = below_base;
        } else {
            Tag::set_above_free(end_to_tag(chunk_base))
        }

        // Try to recombine with a gap above, if it's there.
        // The end flag is never clobbered by this operation, so we can still read it later.
        if tag.is_above_free() {
            debug_assert!(!tag.is_arena_end());

            let mut above_size = gap_base_to_size(chunk_acme).read();
            if O::TRACK_ARENA_END {
                above_size &= !END_FLAG;
            }

            self.deregister_gap(chunk_acme, above_size);
            chunk_acme = chunk_acme.add(above_size);

            if O::TRACK_ARENA_END {
                if gap_end_to_size_and_flag(chunk_acme).read() & END_FLAG != 0 {
                    is_arena_end = true;
                }
            }
        }

        if O::TRACK_ARENA_END && is_arena_end {
            let is_arena_base = end_to_tag(chunk_base).read().is_arena_base();

            // Give the OOM handler an opportunity to see if the arena can be truncated or deleted.
            let arena_end =
                self.oom_handler.maybe_resize_arena(chunk_base, chunk_acme, is_arena_base);

            debug_assert!(arena_end <= chunk_acme);
            debug_assert!(chunk_base <= arena_end);
            debug_assert!(ptr_utils::is_aligned_to(arena_end, CHUNK_UNIT));

            if arena_end > chunk_base {
                // add the full recombined free chunk back into the books
                self.register_gap(chunk_base, arena_end);
                *gap_end_to_size_and_flag(arena_end) |= END_FLAG;
            } else if !is_arena_base {
                *end_to_tag(chunk_base) = Tag(
                    ((*end_to_tag(chunk_base)).0 | Tag::ARENA_END_FLAG) & !Tag::ABOVE_FREE_FLAG
                );
            }

            #[cfg(feature = "counters")]
            self.counters.account_truncate(
                chunk_acme,
                arena_end,
                is_arena_base && chunk_base == arena_end,
            );
        } else {
            // add the full recombined free chunk back into the books
            self.register_gap(chunk_base, chunk_acme);
        }

        // TODO REMOVE
        self.scan_for_errors();
    }

    /// Attempt to grow a previously allocated/reallocated region of memory to `new_size`.
    ///
    /// The return value indicates whether the operation was successful.
    /// The validity of the pointer is maintained regardless, but the allocation
    /// size does not change if `false` is returned.
    ///
    /// # Safety
    /// `ptr` must have been previously allocated or reallocated given `layout`.
    /// `new_size` must be larger or equal to `layout.size()`.
    pub unsafe fn try_grow_in_place(
        &mut self,
        ptr: *mut u8,
        layout: Layout,
        new_size: usize,
    ) -> bool {
        debug_assert!(new_size >= layout.size());
        self.scan_for_errors();

        let old_acme = Self::alloc_to_acme(ptr, layout.size());
        let new_acme = Self::alloc_to_acme(ptr, new_size);

        if old_acme == new_acme {
            #[cfg(feature = "counters")]
            self.counters.account_grow_in_place(layout.size(), new_size);

            return true;
        }

        let old_tag = end_to_tag(old_acme).read();

        debug_assert!(old_tag.is_allocated());

        // otherwise, check if 1) is free 2) is large enough
        // because free chunks don't border free chunks, this needn't be recursive
        if old_tag.is_above_free() {
            let mut above_size = gap_base_to_size(old_acme).read();
            if O::TRACK_ARENA_END {
                above_size &= !END_FLAG;
            }

            let above_acme = old_acme.add(above_size);

            if new_acme <= above_acme {
                self.deregister_gap(old_acme, above_size);

                let end_flag = if O::TRACK_ARENA_END {
                    gap_end_to_size_and_flag(above_acme).read() & END_FLAG != 0
                } else {
                    false
                };

                // finally, determine if the remainder of the free block is big enough
                // to be freed again, or if the entire region should be allocated
                if new_acme != above_acme {
                    self.register_gap(new_acme, above_acme);

                    if O::TRACK_ARENA_END && end_flag {
                        *gap_end_to_size_and_flag(above_acme) |= END_FLAG;
                    }

                    end_to_tag(new_acme).write(Tag::ALLOCATED | Tag::ABOVE_FREE);
                } else {
                    let tag = if O::TRACK_ARENA_END && end_flag {
                        Tag::ALLOCATED | Tag::ARENA_END
                    } else {
                        Tag::ALLOCATED
                    };
                    end_to_tag(new_acme).write(tag);
                }

                #[cfg(feature = "counters")]
                self.counters.account_grow_in_place(layout.size(), new_size);

                return true;
            }
        }

        false
    }

    /// Shrink an allocation to `new_size`.
    ///
    /// This function is infallible given valid inputs, and the reallocation will always be
    /// done in-place, maintaining the validity of the pointer.
    ///
    /// # Safety
    /// - `ptr` must have been previously allocated or reallocated given `layout`.
    /// - `new_size` must be smaller or equal to `layout.size()`.
    /// - `new_size` must be nonzero.
    pub unsafe fn shrink(&mut self, ptr: *mut u8, layout: Layout, new_size: usize) {
        debug_assert!(new_size != 0);
        debug_assert!(new_size <= layout.size());
        self.scan_for_errors();

        let mut chunk_end = Self::alloc_to_acme(ptr, layout.size());
        let new_end = Self::alloc_to_acme(ptr, new_size);

        debug_assert!(end_to_tag(chunk_end).read().is_allocated());
        debug_assert!(Self::is_chunk_size(ptr, chunk_end));

        // if the remainder between the new required size and the originally allocated
        // size is large enough, free the remainder, otherwise leave it
        if new_end != chunk_end {
            let old_tag = end_to_tag(chunk_end).read();
            let is_arena_end;

            if old_tag.is_above_free() {
                let mut above_size = gap_base_to_size(chunk_end).read();
                if O::TRACK_ARENA_END {
                    above_size &= !END_FLAG;
                }

                self.deregister_gap(chunk_end, above_size);
                chunk_end = chunk_end.add(above_size);

                is_arena_end = *gap_end_to_size_and_flag(chunk_end) & END_FLAG != 0;
            } else {
                is_arena_end = old_tag.is_arena_end();
            }

            let mut tag = Tag::ALLOCATED | Tag::ABOVE_FREE;
            if O::TRACK_ARENA_END && is_arena_end {
                // Give the OOM handler an opportunity to see if the arena can be truncated.
                // The arena cannot be deleted here, and we never pass in the arena base,
                // as part of this allocation still occupied space, at the very least.
                let arena_end = self.oom_handler.maybe_resize_arena(new_end, chunk_end, false);

                debug_assert!(arena_end <= chunk_end);
                debug_assert!(new_end <= arena_end);
                debug_assert!(ptr_utils::is_aligned_to(arena_end, CHUNK_UNIT));

                if arena_end > new_end {
                    // add the full recombined free chunk back into the books
                    self.register_gap(new_end, arena_end);
                    *gap_end_to_size_and_flag(arena_end) |= END_FLAG;
                } else {
                    tag = Tag::ALLOCATED | Tag::ARENA_END;
                }

                #[cfg(feature = "counters")]
                self.counters.account_truncate(chunk_end, arena_end, false);
            } else {
                self.register_gap(new_end, chunk_end);
            }

            end_to_tag(new_end).write(tag);
        }

        #[cfg(feature = "counters")]
        self.counters.account_shrink_in_place(layout.size(), new_size);
    }

    /// Attempt to change the size of an allocation without copying memory.
    ///
    /// The return value indicates whether the operation was successful.
    ///
    /// This just calls [`shrink`](Self::shrink) or [`try_grow_in_place`](Self::try_grow_in_place)
    /// depending on whether `new_size` is larger or smaller.
    ///
    /// If `new_size <= layout.size()`, then this will always succeed.
    ///
    /// # Safety
    /// - `ptr` must have been previously allocated or reallocated given `layout`.
    /// - `new_size` must be nonzero.
    pub unsafe fn try_realloc_in_place(
        &mut self,
        ptr: *mut u8,
        layout: Layout,
        new_size: usize,
    ) -> bool {
        match new_size.cmp(&layout.size()) {
            core::cmp::Ordering::Greater => self.try_grow_in_place(ptr, layout, new_size),
            core::cmp::Ordering::Less => {
                self.shrink(ptr, layout, new_size);
                true
            }
            core::cmp::Ordering::Equal => true,
        }
    }

    /// Create a new [`Talc`]. See [`Talc`]'s documentation for more info on it.
    ///
    /// You won't typically want to use [`Talc`] directly. Consider:
    /// - The cell-like [`TalcCell`](crate::cell::TalcCell), for single-threaded allocation.
    ///     Intended for use with the [`Allocator`](allocator_api2::alloc::Allocator) API.
    /// - The lock-based synchronized [`Talck`](crate::sync::Talck), for multi-threaded allocation.
    ///     Intended for use as a global allocator.
    ///
    /// [`TalcCellAssumeSingleThreaded`](crate::cell::TalcCellAssumeSingleThreaded) is also available, if required.
    ///
    /// [`Talc`] is primarily provided to be wrapped. Making your own wrapper might
    /// be best for you if the above options don't serve your use-case.
    pub const fn new(oom_handler: O) -> Self {
        Self {
            #[cfg(feature = "counters")]
            counters: Counters::new(),

            avails: B::AvailabilityBitField::ZEROES,
            free_lists: core::ptr::null_mut(),
            oom_handler,

            _phantom: core::marker::PhantomData,
        }
    }

    /// Indicates whether `self` has already established its allocator metadata into its heap.
    ///
    /// How should I use this? It's most useful to ensure enough memory is being claimed
    /// in [`OomHandler`] implementations. If you're not implementing [`OomHandler`], either
    /// the [`OomHandler`] implementation you're using will take care of it for you, or
    /// you'll be claiming memory immediately or once-off and will know when to consider
    /// the extra requirement. Use [`min_first_arena_size`](crate::min_first_arena_size).
    ///
    /// What does this imply? The minimum size of contiguous memory to claim must exceed
    /// a few kilobytes to be successful. See [`claim`](Talc::claim) for details.
    ///
    /// When is this the case? No memory has been successfully claimed yet.
    ///
    /// Why?
    /// - [`Talc`], like most allocators, requires a block of metadata to track available memory.
    /// - [`Talc`] thus needs to have enough space in the first claimed memory region to put the metadata.
    /// - This block of metadata is referenced by other pointers, and thus cannot be moved.
    #[inline]
    pub const fn is_metadata_established(&self) -> bool {
        !self.free_lists.is_null()
    }

    // todo fixme
    /// Establish a new [`Arena`] to allocate into.
    ///
    /// This does not "combine" with neighboring arenas. Use [`Talc::extend`] to achieve this.
    ///
    /// Due to alignment requirements, the resulting [`Arena`] may be slightly smaller
    /// than the provided memory on either side. The resulting [`Arena`] can and will not have
    /// well-aligned boundaries though.
    ///
    /// # Failure modes
    ///
    /// The first [`Arena`] needs to hold [`Talc`]'s allocation metadata,
    /// this has a fixed size that depends on the [`Binning`] configuration.
    /// Currently, it's a little more than `BIN_COUNT * size_of::<usize>()`
    /// but this is subject to change.
    ///
    /// Use [`min_first_arena_layout`](crate::min_first_arena_layout) or
    /// [`min_first_arena_size`](crate::min_first_arena_size) to guarantee a
    /// successful first claim.
    /// Using a large constant is fine too.
    /// The size requirement won't more-than-quadruple without a major version bump.
    ///
    /// Once the first [`Arena`] is established, the allocation metadata permanently
    /// reserves the start of that [`Arena`] and all subsequent claims are subject to
    /// a much less stringent requirement: `None` is returned only if `size` is too
    /// small to tag the base and have enough left over to fit a chunk.
    ///
    /// # Safety
    /// The region of memory described by `base` and `size` must be exclusively writable
    /// by the allocator, up until the memory is released with [`Talc::truncate`]
    /// or the allocator is no longer active.
    ///
    /// This rule does not apply to memory that will be allocated by `self`.
    /// That's the caller's memory until deallocated.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate talc;
    /// # use talc::{TalcCell, ErrOnOom};
    /// static mut ARENA: [u8; 5000] = [0; 5000];
    ///
    /// let talc = TalcCell::new(ErrOnOom);
    /// let arena = unsafe { talc.claim((&raw mut ARENA).cast(), 5000).unwrap() };
    /// ```
    pub unsafe fn claim(&mut self, base: *mut u8, size: usize) -> Option<NonNull<u8>> {
        // Check if `base + size` overflows. If so, that's okay, just claim up to the top.
        // Currently we never claim the last CHUNK_UNIT of memory. Talc could be changed
        // to be able to use them (i.e. support the acme wrapping to NULL) however
        // 1. Dealing with this correctly throughout the allocator is very tricky.
        // 2. It's not easy to verify that this code works as intended.
        // 3. I doubt anyone really cares much about those last few bytes.
        let arena_acme = Self::align_down(ptr_utils::saturating_ptr_add(base, size));
        let arena_base;
        let gap_base;

        if self.free_lists.is_null() {
            // If `memory` starts at null, it's probably a user bug, but maybe
            // it's a weird bare-metal device and the user just wants the heap at the bottom.
            // We need to dodge the null pointer as attempting to allocate
            // or dereference the null pointer is a bad idea
            // (currently UB in talc due to use of `NonNull::new_unchecked` in `allocate`)
            let base = if base.is_null() { base.wrapping_add(1) } else { base };
            arena_base = ptr_utils::align_up_by(base, align_of::<Option<NonNull<Node>>>() - 1);

            let free_lists_size = size_of::<Option<NonNull<Node>>>() * B::BIN_COUNT as usize;
            gap_base = Self::align_up(arena_base.wrapping_add(free_lists_size + size_of::<Tag>()));

            // if calculating gap_base overflowed OR the meta_acme=gap_base is higher than arena_acme
            // there isn't enough memory to allocate the metadata and cap it off with a tag
            if gap_base < arena_base || arena_acme < gap_base {
                return None;
            }

            let mut tag = Tag::ALLOCATED;
            if gap_base < arena_acme {
                tag |= Tag::ABOVE_FREE;
            }
            end_to_tag(gap_base).write(tag);

            self.free_lists = arena_base.cast();
            for b in 0..B::BIN_COUNT {
                self.free_list_ptr(b).write(None);
            }
        } else {
            // Note that adding the header size and aligning up automatically dodges
            // the possibility of claiming null, if `memory` started at null.
            gap_base = Self::align_up(base.wrapping_add(size_of::<Tag>()));

            // if calculating gap_base overflowed OR there isn't a CHUNK_UNIT between
            // gap_base and arena_acme, then there isn't enough memory to claim
            if gap_base.wrapping_add(CHUNK_UNIT) < base
                || arena_acme < gap_base.wrapping_add(CHUNK_UNIT)
            {
                return None;
            }

            arena_base = end_to_tag(gap_base).cast();

            arena_base.cast::<Tag>().write(Tag::ALLOCATED | Tag::ABOVE_FREE | Tag::ARENA_BASE);
        }

        #[cfg(feature = "counters")]
        self.counters.account_claim(arena_acme as usize - arena_base as usize);

        if gap_base < arena_acme {
            self.register_gap(gap_base, arena_acme);

            if O::TRACK_ARENA_END {
                *gap_end_to_size_and_flag(arena_acme) |= END_FLAG;
            }
        }

        // todo why always nonnull?
        NonNull::new(arena_acme)
    }

    /// Returns the end of the allocated regions.
    ///
    /// Returns `arena_end` if there's no unallocated space at the end.
    ///
    /// [`Talc::truncate`] and [`Talc::resize`] will not release bytes below
    /// the returned pointer. (You can pass null into them and they'll truncate
    /// down to this return value).
    ///
    ///
    /// ```not_rust
    ///
    ///     ├──Arena──────────────────────────────────┤
    /// ────┬─────┬───────────┬─────┬───────────┬─────┬────
    /// ... | Gap | Allocated | Gap | Allocated | Gap | ...
    /// ────┴─────┴───────────┴─────┴───────────┴─────┴────
    ///     ├──Reserved─────────────────────────┤
    ///
    ///
    /// ```
    ///
    /// # Atomicity
    ///
    /// Be aware that this value may change before you use it if you don't own
    /// the allocator or hold a lock on it.
    ///
    /// However, you can use [`Talc::truncate`] and [`Talc::resize`] correctly without
    /// consulting this value at all. Atomicity isn't an issue for these calls.
    ///
    /// # Safety
    /// - `arena_end`'s arena must be managed by this instance of [`Talc`].
    pub unsafe fn reserved(&self, arena_end: *mut u8) -> Option<NonNull<u8>> {
        // TODO document the some/none conditions. Are these NonNull::news always returning non-nulls?
        if let Some(gap_base) = unsafe { Self::arena_end_to_gap_base(arena_end) } {
            if unsafe { end_to_tag(gap_base).read() }.is_arena_base() {
                None
            } else {
                NonNull::new(gap_base)
            }
        } else {
            NonNull::new(arena_end)
        }
    }

    /// TODO FIXME
    /// Extend the `arena`'s up to `new_size`.
    ///
    /// Due to alignment requirements, the `arena` may not be quite `new_size`.
    /// The difference will be less than [`CHUNK_UNIT`](crate::base::CHUNK_UNIT).
    ///
    /// If `new_size` isn't large enough to extend `arena`, this call does nothing.
    ///
    /// # Safety
    /// - `arena` must be managed by this instance of the allocator.
    /// - The memory in `arena.base()..arena.base().add(new_size)`
    ///     must be exclusively writeable by this instance of the allocator for
    ///     the lifetime `arena` unless truncated away or the allocator is no longer active.
    ///     - Note that any memory not contained within `arena` after `extend` returns
    ///         is unclaimed by the allocator and not subject to this requirement.
    ///     - Note that any memory in the resulting `arena` that is allocated by
    ///         `self` later on is also not subject to this requirement for the duration
    ///         of the allocation.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate talc;
    /// # use talc::{TalcCell, ErrOnOom};
    /// static mut ARENA: [u8; 5000] = [0; 5000];
    ///
    /// let talc = TalcCell::new(ErrOnOom);
    /// let mut arena = unsafe { talc.claim((&raw mut ARENA).cast(), 2500).unwrap() };
    /// unsafe { talc.extend(&mut arena, 5000) };
    /// ```
    pub unsafe fn extend(&mut self, arena_end: *mut u8, new_end: *mut u8) -> NonNull<u8> {
        debug_assert!(ptr_utils::is_aligned_to(arena_end, CHUNK_UNIT));

        let new_end = Self::align_down(new_end);

        if new_end <= arena_end {
            return NonNull::new_unchecked(arena_end);
        }

        debug_assert!(ptr_utils::is_aligned_to(new_end, CHUNK_UNIT));

        let mut free_chunk_base = arena_end;

        if let Some(gap_base) = Self::arena_end_to_gap_base(arena_end) {
            free_chunk_base = gap_base;
            self.deregister_gap(gap_base, arena_end as usize - gap_base as usize);
        } else {
            let tag_ptr = end_to_tag(arena_end);
            Tag::set_above_free(tag_ptr);

            if O::TRACK_ARENA_END {
                Tag::clear_end_flag(tag_ptr);
            }
        }

        self.register_gap(free_chunk_base, new_end);

        if O::TRACK_ARENA_END {
            *gap_end_to_size_and_flag(new_end) |= END_FLAG;
        }

        #[cfg(feature = "counters")]
        self.counters.account_append(arena_end, new_end);

        // SAFETY: todo
        NonNull::new_unchecked(new_end)
    }

    /// Reduce the arena's end from `arena_end` to `new_end`.
    ///
    /// Returns the new arena end, or otherwise `None` if the arena would be
    /// empty or too small to allocate into (less than a
    /// [`CHUNK_UNIT`](crate::base::CHUNK_UNIT)), and is thus deleted.
    ///
    /// If `new_end` is greater or equal to `arena_end`, this returns `arena_end`.
    ///
    /// The extent cannot be reduced further than what is indicated
    /// by [`Talc::reserved`]. Attempting to do so (e.g. setting `new_end` to `null_mut`)
    /// will truncate as much as possible.
    ///
    /// Due to alignment requirements, the resulting arena end
    /// might be slightly lower than requested
    /// by a difference of less than [`CHUNK_UNIT`](crate::base::CHUNK_UNIT).
    ///
    /// All memory between the resulting pointer and `arena_end`, if any,
    /// is released back to the caller. You no longer need to guarantee that
    /// unallocated memory in this region is not mutated.
    ///
    /// # Safety
    /// - The arena must be managed by this instance of the allocator.
    /// - `arena_end` must have been previously returned as an arena end by this
    ///     allocator, and not subsequently modified. i.e. it must be the
    ///     up-to-date arena end.
    ///
    /// # Example
    ///
    /// ```
    /// # extern crate talc;
    /// # use talc::{TalcCell, ErrOnOom};
    /// # use core::ptr::null_mut();
    /// static mut ARENA: [u8; 5000] = [0; 5000];
    ///
    /// let mut talc = TalcCell::new(ErrOnOom);
    /// let end = unsafe { talc.claim((&raw mut ARENA).cast(), ARENA.len()).unwrap() };
    /// // do some allocator operations...
    ///
    /// // reclaim as much of the arena as possible
    /// let opt_new_end = unsafe { talc.truncate(end, null_mut()) };
    /// ```
    pub unsafe fn truncate(&mut self, arena_end: *mut u8, new_end: *mut u8) -> Option<NonNull<u8>> {
        debug_assert!(
            ptr_utils::is_aligned_to(arena_end, CHUNK_UNIT),
            "This is not the end of an arena. Ends of arenas are always aligned to CHUNK_UNIT."
        );

        let new_end = Self::align_down(new_end);
        if new_end >= arena_end {
            return NonNull::new(arena_end);
        }

        if let Some(gap_base) = unsafe { Self::arena_end_to_gap_base(arena_end) } {
            self.deregister_gap(gap_base, arena_end as usize - gap_base as usize);

            let mut is_arena_deleted = false;
            if gap_base < new_end {
                self.register_gap(gap_base, new_end);

                if O::TRACK_ARENA_END {
                    *gap_end_to_size_and_flag(new_end) |= END_FLAG;
                }
            } else if end_to_tag(gap_base).read().is_arena_base() {
                is_arena_deleted = true;
            } else {
                let tag_ptr = end_to_tag(gap_base);
                Tag::clear_above_free(tag_ptr);

                if O::TRACK_ARENA_END {
                    Tag::set_end_flag(tag_ptr);
                }
            };

            let new_end = new_end.max(gap_base);

            #[cfg(feature = "counters")]
            self.counters.account_truncate(arena_end, new_end, is_arena_deleted);

            if !is_arena_deleted { NonNull::new(new_end) } else { None }
        } else {
            NonNull::new(arena_end)
        }
    }

    #[inline]
    pub unsafe fn resize(&mut self, arena_end: *mut u8, new_end: *mut u8) -> Option<NonNull<u8>> {
        match new_end.cmp(&arena_end) {
            core::cmp::Ordering::Less => self.truncate(arena_end, new_end),
            core::cmp::Ordering::Equal => NonNull::new(new_end),
            core::cmp::Ordering::Greater => Some(self.extend(arena_end, new_end)),
        }
    }

    #[inline]
    unsafe fn arena_end_to_gap_base(end: *mut u8) -> Option<*mut u8> {
        // gap size will never have bit 1 set, but a tag will
        let is_gap_below = !end_to_tag(end).read().is_allocated();
        is_gap_below.then(|| {
            if O::TRACK_ARENA_END {
                end.sub(gap_end_to_size_and_flag(end).read() & !END_FLAG)
            } else {
                end.sub(gap_end_to_size_and_flag(end).read())
            }
        })
    }

    #[cfg(not(any(test, feature = "error-scanning-std")))]
    fn scan_for_errors(&self) {}

    #[cfg(any(test, feature = "error-scanning-std"))]
    /// Debugging function for checking various assumptions.
    fn scan_for_errors(&self) {
        use core::ops::Range;

        // allocator-api2 doesn't re-export this correctly
        // because it exports from `alloc` instead of `std`
        // if `std` and `nightly` are enabled
        #[cfg(not(feature = "nightly"))]
        use allocator_api2::alloc::System;
        #[cfg(feature = "nightly")]
        use std::alloc::System;

        let mut vec = allocator_api2::vec::Vec::<Range<*mut u8>, _>::new_in(System);

        if !self.free_lists.is_null() {
            for b in 0..B::BIN_COUNT {
                let mut any = false;
                unsafe {
                    for node in Node::iter_mut(*self.free_list_ptr(b)) {
                        any = true;
                        assert!(self.avails.read_bit(b));

                        let base = gap_node_to_base(node);
                        let mut size = gap_base_to_size(base).read();

                        if size == CHUNK_UNIT + END_FLAG {
                            size = CHUNK_UNIT;
                        }
                        assert_eq!(size % CHUNK_UNIT, 0);

                        let end = base.add(size);
                        let end_size_flag = gap_end_to_size_and_flag(end).read();
                        // let end_flag = end_size_flag & END_FLAG != 0;
                        let end_size = end_size_flag & !END_FLAG;
                        assert_eq!(size, end_size, "{:p} {:x} {:x}", base, size, end_size);

                        // TODO check end flag?

                        let bin = gap_base_to_bin(base).read();
                        assert_eq!(bin, B::size_to_bin(size).min(B::BIN_COUNT - 1));

                        let lower_tag = end_to_tag(base).read();
                        assert!(lower_tag.is_allocated());
                        assert!(lower_tag.is_above_free());

                        let range = base..end;
                        // eprintln!("{:p}..{:p}{}", base, end, if end_flag { "*" } else { "" });
                        for other in &vec {
                            // Interestingly, De Morgan's law doesn't work here, the reason is worth the thought.
                            let overlaps = !(other.end <= range.start || range.end <= other.start);
                            assert!(!overlaps, "{:?} intersects {:?}", range, other);
                        }
                        vec.push(range);
                    }
                }

                if !any {
                    assert!(!self.avails.read_bit(b));
                }
            }
        } else {
            assert!(self.avails.bit_scan_after(0) >= B::BIN_COUNT);
        }
    }
}

#[cfg(test)]
mod tests {
    use core::ptr::null_mut;
    use std::alloc::{alloc, dealloc};

    use crate::{ErrOnOom, min_first_arena_size};

    use super::*;

    #[test]
    fn verify_gap_properties() {
        fn verify_gap_properties_inner<B: Binning>() {
            unsafe {
                let mut talc = Talc::<_, B>::new(ErrOnOom);

                let meta_layout = crate::min_first_arena_layout::<B>();
                let meta_mem = alloc(meta_layout);
                let _meta_arena = talc.claim(meta_mem, meta_layout.size()).unwrap();

                let gap_mem = Box::into_raw(Box::<[u8]>::new_uninit_slice(999));
                let gap_acme = talc.claim(gap_mem.cast(), gap_mem.len()).unwrap().as_ptr();

                assert!(gap_acme <= gap_mem.cast::<u8>().add(gap_mem.len()));
                assert!(gap_acme.add(CHUNK_UNIT) > gap_mem.cast::<u8>().add(gap_mem.len()));

                let gap_base =
                    Talc::<ErrOnOom, B>::align_up(gap_mem.cast::<u8>().add(size_of::<Tag>()));
                let gap_size = gap_acme as usize - gap_base as usize;
                assert!(gap_size <= 999);
                assert!(999 - CHUNK_UNIT * 2 < gap_size);

                let gap_bin = B::size_to_bin(gap_size).min(B::BIN_COUNT - 1);
                assert!(talc.free_list_ptr(gap_bin).read().is_some());
                let gap_node_ptr = talc.free_list_ptr(gap_bin).read().unwrap();
                assert_eq!(gap_node_ptr.as_ptr(), gap_base_to_node(gap_base));
                let gap_node = gap_node_ptr.read();
                assert!(gap_node.next.is_none());
                assert_eq!(gap_node.next_of_prev, talc.free_list_ptr(gap_bin));
                assert_eq!(gap_bin, gap_base_to_bin(gap_base).read());
                assert_eq!(gap_size, gap_base_to_size(gap_base).read());

                assert_eq!(gap_base_to_size(gap_base).read(), gap_size);
                assert_eq!(
                    gap_end_to_size_and_flag(gap_acme),
                    gap_acme.sub(size_of::<usize>()).cast()
                );
                assert_eq!(gap_end_to_size_and_flag(gap_acme).read(), gap_size | END_FLAG);

                talc.deregister_gap(gap_base, gap_size);

                dealloc(meta_mem, meta_layout);
                drop(Box::from_raw(gap_mem));
            }
        }

        for_many_talc_configurations!(verify_gap_properties_inner);
    }

    #[test]
    fn alloc_dealloc_test() {
        fn alloc_dealloc_test_inner<B: Binning>() {
            unsafe {
                let arena = Box::into_raw(Box::<[u8]>::new_uninit_slice(5000));
                let mut talc = Talc::<_, B>::new(ErrOnOom);
                talc.claim(arena.cast(), arena.len()).unwrap();

                let layout = Layout::from_size_align(2435, 8).unwrap();
                let allocation = talc.allocate(layout).unwrap().as_ptr();

                allocation.write_bytes(0xCD, layout.size());

                talc.deallocate(allocation, layout);

                drop(Box::from_raw(arena));
            }
        }

        for_many_talc_configurations!(alloc_dealloc_test_inner);
    }

    #[test]
    fn alloc_fail_test() {
        fn alloc_fail_test_inner<B: Binning>() {
            unsafe {
                let arena = Box::into_raw(Box::<[u8]>::new_uninit_slice(
                    min_first_arena_size::<B>() + 100 + CHUNK_UNIT,
                ));
                let mut talc = Talc::<_, B>::new(ErrOnOom);
                talc.claim(arena.cast(), arena.len()).unwrap();

                talc.allocate(Layout::new::<u64>()).unwrap();

                let layout = Layout::from_size_align(1234 + CHUNK_UNIT, 8).unwrap();
                assert_eq!(talc.allocate(layout), None);

                drop(Box::from_raw(arena));
            }
        }

        for_many_talc_configurations!(alloc_fail_test_inner);
    }

    #[test]
    fn claim_arena_thats_too_small() {
        fn claim_arena_thats_too_small_inner<B: Binning>() {
            unsafe {
                let mut tiny_heap = [0u8; 200];

                let mut talc = Talc::<_, B>::new(crate::ErrOnOom);
                assert!(talc.claim(tiny_heap.as_mut_ptr().cast(), tiny_heap.len()).is_none());

                assert!(talc.free_lists.is_null());
                assert!(talc.avails.bit_scan_after(0) >= B::BIN_COUNT);
            }
        }

        for_many_talc_configurations!(claim_arena_thats_too_small_inner);
    }

    #[test]
    fn claim_small_arena_after_metadata_is_allocated() {
        fn claim_small_arena_after_metadata_is_allocated_inner<B: Binning>() {
            unsafe {
                // big enough with plenty of extra
                let meta_layout = crate::min_first_arena_layout::<B>();
                let big_heap = alloc(meta_layout);

                let mut talc = Talc::<_, B>::new(ErrOnOom);
                let _arena = talc.claim(big_heap.cast(), meta_layout.size()).unwrap();

                assert!(!talc.free_lists.is_null());
                assert!(talc.avails.bit_scan_after(0) >= B::BIN_COUNT);

                let mut tiny_heap = [0u8; 300];
                let _tiny_arena =
                    talc.claim(tiny_heap.as_mut_ptr().cast(), tiny_heap.len()).unwrap();

                dealloc(big_heap, meta_layout);
            }
        }

        for_many_talc_configurations!(claim_small_arena_after_metadata_is_allocated_inner);
    }

    #[test]
    fn claim_truncate_extend_test() {
        fn claim_truncate_extend_test_inner<B: Binning>() {
            unsafe {
                // big enough with plenty of extra
                let big_heap = Box::into_raw(Box::<[u8]>::new_uninit_slice(100000));
                let mut talc = Talc::<_, B>::new(ErrOnOom);
                let arena_acme = talc.claim(big_heap.cast(), big_heap.len()).unwrap().as_ptr();

                let arena_acme = talc.truncate(arena_acme, null_mut()).unwrap().as_ptr();
                assert!(talc.allocate(Layout::new::<u128>()).is_none());

                let arena_acme = talc.extend(arena_acme, arena_acme.add(256)).as_ptr();
                let a1 = talc.allocate(Layout::new::<u128>()).unwrap().as_ptr();
                a1.write_bytes(0, Layout::new::<u128>().size());

                let _arena_acme =
                    talc.extend(arena_acme, big_heap.cast::<u8>().add(big_heap.len()));

                let big_layout = Layout::from_size_align(80000, 8).unwrap();
                let a2 = talc.allocate(big_layout).unwrap();

                talc.deallocate(a1, Layout::new::<u128>());

                talc.deallocate(a2.as_ptr(), big_layout);

                drop(Box::from_raw(big_heap));
            }
        }

        for_many_talc_configurations!(claim_truncate_extend_test_inner);
    }
}
