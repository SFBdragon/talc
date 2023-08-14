#![doc = include_str!("../README.md")]
#![feature(offset_of)]
#![feature(pointer_is_aligned)]
#![feature(alloc_layout_extra)]
#![feature(slice_ptr_get)]
#![feature(slice_ptr_len)]
#![feature(const_slice_ptr_len)]
#![feature(const_slice_from_raw_parts_mut)]
#![cfg_attr(not(any(test, fuzzing)), no_std)]
#![cfg_attr(feature = "allocator", feature(allocator_api))]

#[cfg(feature = "lock_api")]
mod talck;

mod llist;
mod oom_handler;
mod span;
mod tag;
mod utils;

#[cfg(all(target_family = "wasm", feature = "lock_api"))]
pub use oom_handler::WasmHandler;
pub use oom_handler::{ErrOnOom, InitOnOom, OomHandler};
pub use span::Span;
#[cfg(all(feature = "lock_api", feature = "allocator"))]
pub use talck::TalckRef;
#[cfg(feature = "lock_api")]
pub use talck::{AssumeUnlockable, Talck};

use llist::LlistNode;
use tag::Tag;
use utils::*;

use core::{
    alloc::Layout,
    ptr::{null_mut, NonNull},
};

// Free chunk (3x ptr size minimum):
//   ?? | NODE: LlistNode (2 * ptr), SIZE: usize, ..???.., SIZE: usize | ??
// Reserved chunk (1x ptr size of overhead):
//   ?? |       ???????         , TAG: Tag (ptr) | ??

// TAG contains a pointer to the top of the reserved chunk,
// a is_allocated (set) bit flag differentiating itself from a free chunk
// (the LlistNode contains well-aligned pointers, thus does not have that bit set),
// as well as a is_low_free bit flag which does what is says on the tin

// go check out `utils::bin_of_size(usize)` to see how bucketing works

const WORD_SIZE: usize = core::mem::size_of::<usize>();
const WORD_BITS: usize = usize::BITS as usize;
const ALIGN: usize = core::mem::align_of::<usize>();

const NODE_SIZE: usize = core::mem::size_of::<LlistNode>();
const TAG_SIZE: usize = core::mem::size_of::<Tag>();

const MIN_TAG_OFFSET: usize = NODE_SIZE;
const MIN_CHUNK_SIZE: usize = MIN_TAG_OFFSET + TAG_SIZE;

const BIN_COUNT: usize = usize::BITS as usize * 2;

type Bin = Option<NonNull<LlistNode>>;

/// The Talc Allocator!
///
/// To get started:
/// - Construct with `new` or `with_arena` functions (use [`ErrOnOom`] to ignore OOM handling).
/// - Initialize with `init` or `extend`.
/// - Call [`lock`](Talc::lock) to get a [`Talck`] which supports the
/// [`GlobalAlloc`](core::alloc::GlobalAlloc) and [`Allocator`](core::alloc::Allocator) traits.
pub struct Talc<O: OomHandler> {
    pub oom_handler: O,

    arena: Span,

    allocatable_base: *mut u8,
    allocatable_acme: *mut u8,

    is_base_free: bool,

    /// The low bits of the availability flags.
    availability_low: usize,
    /// The high bits of the availability flags.
    availability_high: usize,

    /// Linked list heads.
    bins: *mut Bin,
}

unsafe impl<O: Send + OomHandler> Send for Talc<O> {}

impl<O: OomHandler> core::fmt::Debug for Talc<O> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Talc")
            .field("arena", &self.arena)
            .field("alloc_base", &self.allocatable_base)
            .field("alloc_acme", &self.allocatable_acme)
            .field("is_base_free", &self.is_base_free)
            .field("availability_low", &format_args!("{:x}", self.availability_low))
            .field("availability_high", &format_args!("{:x}", self.availability_high))
            .finish()
    }
}

impl<O: OomHandler> Talc<O> {
    const fn required_chunk_size(size: usize) -> usize {
        if size <= MIN_CHUNK_SIZE - TAG_SIZE {
            MIN_CHUNK_SIZE
        } else {
            (size + TAG_SIZE + (ALIGN - 1)) & !(ALIGN - 1)
        }
    }

    /// Get the pointer to the `bin`th bin.
    /// # Safety
    /// `bin` must be smaller than `BIN_COUNT`.
    unsafe fn get_bin_ptr(&self, bin: usize) -> *mut Bin {
        debug_assert!(bin < BIN_COUNT);

        self.bins.add(bin)
    }

    /// Sets the availability flag for bin `b`.
    ///
    /// This is done when a chunk is added to an empty bin.
    #[inline]
    fn set_avails(&mut self, b: usize) {
        debug_assert!(b < BIN_COUNT);

        if b < WORD_BITS {
            debug_assert!(self.availability_low & 1 << b == 0);
            self.availability_low ^= 1 << b;
        } else {
            debug_assert!(self.availability_high & 1 << (b - WORD_BITS) == 0);
            self.availability_high ^= 1 << (b - WORD_BITS);
        }
    }
    /// Clears the availability flag for bin `b`.
    ///
    /// This is done when a bin becomes empty.
    #[inline]
    fn clear_avails(&mut self, b: usize) {
        debug_assert!(b < BIN_COUNT);

        // if head is the last node
        if b < WORD_BITS {
            self.availability_low ^= 1 << b;
            debug_assert!(self.availability_low & 1 << b == 0);
        } else {
            self.availability_high ^= 1 << (b - WORD_BITS);
            debug_assert!(self.availability_high & 1 << (b - WORD_BITS) == 0);
        }
    }

    /// Registers memory that may be allocated.
    #[inline]
    unsafe fn register(&mut self, base: *mut u8, acme: *mut u8) {
        debug_assert!(is_chunk_size(base, acme));

        let size = acme as usize - base as usize;
        let bin = bin_of_size(size);
        let chunk = FreeChunk(base);

        let bin_ptr = self.get_bin_ptr(bin);

        if (*bin_ptr).is_none() {
            self.set_avails(bin);
        }

        LlistNode::insert(chunk.node_ptr(), bin_ptr, *bin_ptr);

        debug_assert!((*bin_ptr).is_some());

        // write in high size tag below the node pointers
        chunk.size_ptr().write(size);
        // write in high size tag at the base of the free chunk
        acme.sub(WORD_SIZE).cast::<usize>().write(size);
    }

    /// Deregisters memory, not allowing it to be allocated.
    #[inline]
    unsafe fn deregister(&mut self, node_ptr: *mut LlistNode, bin: usize) {
        debug_assert!((*self.get_bin_ptr(bin)).is_some());

        LlistNode::remove(node_ptr);

        if (*self.get_bin_ptr(bin)).is_none() {
            self.clear_avails(bin);
        }
    }

    /// Ensures the below chunk's `is_above_free` or the `talc.is_base_free` flag is cleared.
    ///
    /// Assumes an allocated chunk's base is at `chunk_acme`.
    #[inline]
    unsafe fn clear_below_free_flag(&mut self, chunk_base: *mut u8) {
        if chunk_base != self.allocatable_base {
            Tag::clear_above_free(chunk_base.sub(TAG_SIZE).cast());
        } else {
            debug_assert!(self.is_base_free);
            self.is_base_free = false;
        }
    }

    /// Allocate a contiguous region of memory according to `layout`, if possible.
    /// # Safety
    /// `layout.size()` must be nonzero.
    pub unsafe fn malloc(&mut self, layout: Layout) -> Result<NonNull<u8>, ()> {
        debug_assert!(layout.size() != 0);

        let (mut chunk_base, chunk_acme, alloc_base) = loop {
            // no checks for initialization are performed, as it would be overhead.
            // this will return None here as the availability flags are initialized
            // to zero; all clear; no memory to allocate, call the OOM handler.
            match self.get_sufficient_chunk(layout) {
                Some(payload) => break payload,
                None => _ = O::handle_oom(self, layout)?,
            }
        };

        // determine the base of the allocated chunk
        // if the amount of memory below the chunk is too small, subsume it, else free it
        let chunk_base_ceil = alloc_base.min(chunk_acme.sub(MIN_CHUNK_SIZE));
        if is_chunk_size(chunk_base, chunk_base_ceil) {
            self.register(chunk_base, chunk_base_ceil);
            chunk_base = chunk_base_ceil;
        } else {
            self.clear_below_free_flag(chunk_base);
        }

        // the word immediately after the allocation
        let post_alloc_ptr = align_up(alloc_base.add(layout.size()));
        // the tag position, accounting for the minimum size of a chunk
        let mut tag_ptr = chunk_base.add(MIN_TAG_OFFSET).max(post_alloc_ptr);
        // the pointer after the lowest possible tag pointer
        let acme = tag_ptr.add(TAG_SIZE);

        // handle the space above the required allocation span
        if is_chunk_size(acme, chunk_acme) {
            self.register(acme, chunk_acme);
            Tag::write(tag_ptr, chunk_base, true);
        } else {
            tag_ptr = chunk_acme.sub(TAG_SIZE);
            Tag::write(tag_ptr, chunk_base, false);
        }

        if tag_ptr != post_alloc_ptr {
            // write the real tag ptr where the tag is expected to be
            post_alloc_ptr.cast::<*mut u8>().write(tag_ptr);
        }

        scan_for_errors(self);

        Ok(NonNull::new_unchecked(alloc_base))
    }

    /// Returns `(chunk_base, chunk_acme, alloc_base)`
    unsafe fn get_sufficient_chunk(
        &mut self,
        layout: Layout,
    ) -> Option<(*mut u8, *mut u8, *mut u8)> {
        let required_chunk_size = Self::required_chunk_size(layout.size());

        let mut bin = self.next_available_bin(bin_of_size(required_chunk_size))?;

        if layout.align() <= ALIGN {
            // the required alignment is most often the machine word size (or less)
            // a faster loop without alignment checking is used in this case
            loop {
                for node_ptr in LlistNode::iter_mut(*self.get_bin_ptr(bin)) {
                    let chunk = FreeChunk(node_ptr.as_ptr().cast());
                    let size = chunk.size_ptr().read();

                    // if the chunk size is sufficient, remove from bookkeeping data structures and return
                    if size >= required_chunk_size {
                        self.deregister(chunk.node_ptr(), bin);
                        return Some((chunk.base(), chunk.base().add(size), chunk.base()));
                    }
                }

                bin = self.next_available_bin(bin + 1)?;
            }
        } else {
            // a larger than word-size alignement is demanded
            // therefore each chunk is manually checked to be sufficient accordingly
            let align_mask = layout.align() - 1;
            let required_size = layout.size() + TAG_SIZE;

            loop {
                for node_ptr in LlistNode::iter_mut(*self.get_bin_ptr(bin)) {
                    let chunk = FreeChunk(node_ptr.as_ptr().cast());
                    let size = chunk.size_ptr().read();

                    if size >= required_chunk_size {
                        // calculate the lowest aligned pointer above the tag-offset free chunk pointer
                        let aligned_ptr = align_up_by(chunk.base(), align_mask);
                        let acme = chunk.base().add(size);

                        // if the remaining size is sufficient, remove the chunk from the books and return
                        if aligned_ptr.add(required_size) <= acme {
                            self.deregister(chunk.node_ptr(), bin);
                            return Some((chunk.base(), acme, aligned_ptr));
                        }
                    }
                }

                bin = self.next_available_bin(bin + 1)?;
            }
        }
    }

    #[inline(always)]
    fn next_available_bin(&self, next_bin: usize) -> Option<usize> {
        if next_bin < usize::BITS as usize {
            // shift flags such that only flags for larger buckets are kept
            let shifted_avails = self.availability_low >> next_bin;

            // find the next up, grab from the high flags, or quit
            if shifted_avails != 0 {
                Some(next_bin + shifted_avails.trailing_zeros() as usize)
            } else if self.availability_high != 0 {
                Some(self.availability_high.trailing_zeros() as usize + WORD_BITS)
            } else {
                None
            }
        } else if next_bin < BIN_COUNT {
            // similar process to the above, but the low flags are irrelevant
            let shifted_avails = self.availability_high >> (next_bin - WORD_BITS);

            if shifted_avails != 0 {
                Some(next_bin + shifted_avails.trailing_zeros() as usize)
            } else {
                return None;
            }
        } else {
            None
        }
    }

    /// Free previously allocated/reallocated memory.
    /// # Safety
    /// `ptr` must have been previously allocated given `layout`.
    pub unsafe fn free(&mut self, ptr: NonNull<u8>, layout: Layout) {
        debug_assert!(self.arena.contains(ptr.as_ptr()));

        // todo, consider a bounds check here for alloc_base < ptr < alloc_acme
        // else hand off to the OOM handler (OOM handler could be able to map its own
        // allocations outside the arena, supporting operations like mmap)?

        let (tag_ptr, tag) = tag_from_alloc_ptr(ptr.as_ptr(), layout.size());
        let mut chunk_base = tag.base_ptr();
        let mut chunk_acme = tag_ptr.add(TAG_SIZE);

        debug_assert!(tag.is_allocated());
        debug_assert!(is_chunk_size(chunk_base, chunk_acme));

        // try recombine below
        if chunk_base != self.allocatable_base {
            union Discriminant {
                tag: Tag,
                size: usize,
            }

            let below_ptr = chunk_base.sub(TAG_SIZE);
            let disc = below_ptr.cast::<Discriminant>().read();

            if disc.tag.is_allocated() {
                Tag::set_above_free(below_ptr.cast());
            } else {
                let below_size = disc.size;
                chunk_base = chunk_base.sub(below_size);
                self.deregister(FreeChunk(chunk_base).node_ptr(), bin_of_size(below_size));
            }
        } else {
            debug_assert!(!self.is_base_free);
            self.is_base_free = true;
        }

        // try recombine above
        if tag.is_above_free() {
            let above = FreeChunk(chunk_acme);
            let above_size = above.size_ptr().read();
            chunk_acme = chunk_acme.add(above_size);
            self.deregister(above.node_ptr(), bin_of_size(above_size));
        }

        // add the full recombined free chunk back into the books
        self.register(chunk_base, chunk_acme);

        scan_for_errors(self);
    }

    /// Grow a previously allocated/reallocated region of memory to `new_size`.
    /// # Safety
    /// `ptr` must have been previously allocated or reallocated given `layout`.
    /// `new_size` must be larger or equal to `layout.size()`.
    pub unsafe fn grow(
        &mut self,
        ptr: NonNull<u8>,
        layout: Layout,
        new_size: usize,
    ) -> Result<NonNull<u8>, ()> {
        debug_assert!(new_size >= layout.size());
        debug_assert!(self.arena.contains(ptr.as_ptr()));

        let old_post_alloc_ptr = align_up(ptr.as_ptr().add(layout.size()));
        let new_post_alloc_ptr = align_up(ptr.as_ptr().add(new_size));

        if old_post_alloc_ptr == new_post_alloc_ptr {
            // this handles a rare short-circuit, but more helpfully
            // also guarantees that we'll never need to add padding to
            // reach minimum chunk size with new_tag_ptr later
            // i.e. new_post_alloc_ptr == new_tag_ptr != old_post_alloc_ptr
            return Ok(ptr);
        }

        let (tag_ptr, tag) = tag_from_alloc_ptr(ptr.as_ptr(), layout.size());

        // tag_ptr may be greater where extra free space needed to be reserved
        if new_post_alloc_ptr <= tag_ptr {
            if new_post_alloc_ptr < tag_ptr {
                new_post_alloc_ptr.cast::<*mut u8>().write(tag_ptr);
            }

            return Ok(ptr);
        }

        let new_tag_ptr = new_post_alloc_ptr;

        let base = tag.base_ptr();
        let acme = tag_ptr.add(TAG_SIZE);

        debug_assert!(tag.is_allocated());
        debug_assert!(is_chunk_size(base, acme));

        // otherwise, check if 1) is free 2) is large enough
        // because free chunks don't border free chunks, this needn't be recursive
        if tag.is_above_free() {
            let above = FreeChunk(acme);
            let above_size = above.size_ptr().read();
            let above_tag_ptr = tag_ptr.add(above_size);

            if new_tag_ptr <= above_tag_ptr {
                self.deregister(above.node_ptr(), bin_of_size(above_size));

                // finally, determine if the remainder of the free block is big enough
                // to be freed again, or if the entire region should be allocated
                if is_chunk_size(new_tag_ptr, above_tag_ptr) {
                    self.register(new_tag_ptr.add(TAG_SIZE), above_tag_ptr.add(TAG_SIZE));
                    Tag::write(new_tag_ptr, base, true);
                } else {
                    Tag::write(above_tag_ptr, base, false);

                    if new_post_alloc_ptr != above_tag_ptr {
                        new_post_alloc_ptr.cast::<*mut u8>().write(above_tag_ptr);
                    }
                }

                scan_for_errors(self);

                return Ok(ptr);
            }
        }

        // grow in-place failed; reallocate the slow way

        let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
        let allocation = self.malloc(new_layout)?;
        allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), layout.size());
        self.free(ptr, layout);

        scan_for_errors(self);

        Ok(allocation)
    }

    /// Shrink a previously allocated/reallocated region of memory to `new_size`.
    ///
    /// This function is infallibe given valid inputs, and the reallocation will always be
    /// done in-place, maintaining the validity of the pointer.
    ///
    /// # Safety
    /// - `ptr` must have been previously allocated or reallocated given `layout`.
    /// - `new_size` must be smaller or equal to `layout.size()`.
    /// - `new_size` should be nonzero.
    pub unsafe fn shrink(&mut self, ptr: NonNull<u8>, layout: Layout, new_size: usize) {
        debug_assert!(new_size != 0);
        debug_assert!(new_size <= layout.size());
        debug_assert!(self.arena.contains(ptr.as_ptr()));

        let (tag_ptr, tag) = tag_from_alloc_ptr(ptr.as_ptr(), layout.size());
        let chunk_base = tag.base_ptr();

        debug_assert!(tag.is_allocated());
        debug_assert!(is_chunk_size(chunk_base, tag_ptr.add(TAG_SIZE)));

        // the word immediately after the allocation
        let new_post_alloc_ptr = align_up(ptr.as_ptr().add(new_size));
        // the tag position, accounting for the minimum size of a chunk
        let mut new_tag_ptr = chunk_base.add(MIN_CHUNK_SIZE - TAG_SIZE).max(new_post_alloc_ptr);

        // if the remainder between the new required size and the originally allocated
        // size is large enough, free the remainder, otherwise leave it
        if is_chunk_size(new_tag_ptr, tag_ptr) {
            let mut acme = tag_ptr.add(TAG_SIZE);
            let new_acme = new_tag_ptr.add(TAG_SIZE);

            if tag.is_above_free() {
                let above = FreeChunk(acme);
                let above_size = above.size_ptr().read();

                self.deregister(above.node_ptr(), bin_of_size(above_size));
                acme = above.base().add(above_size);
            }

            self.register(new_acme, acme);
            Tag::write(new_tag_ptr, chunk_base, true);
        } else {
            new_tag_ptr = tag_ptr;
        }

        if new_tag_ptr != new_post_alloc_ptr {
            new_post_alloc_ptr.cast::<*mut u8>().write(new_tag_ptr);
        }

        scan_for_errors(self);
    }

    /// Returns an uninitialized [`Talc`].
    ///
    /// If you don't want to handle OOM, use [`ErrOnOom`].
    pub const fn new(oom_handler: O) -> Self {
        Self {
            oom_handler,

            arena: Span::empty(),
            allocatable_base: core::ptr::null_mut(),
            allocatable_acme: core::ptr::null_mut(),
            is_base_free: true,

            availability_low: 0,
            availability_high: 0,
            bins: null_mut(),
        }
    }

    /// Contruct and initialize a `Talc` with the given OOM handler and arena.
    ///
    /// If you don't want to handle OOM, use [`ErrOnOom`].
    /// # Safety
    /// See [`init`](Talc::init) for safety requirements.
    pub unsafe fn with_arena(oom_handler: O, arena: Span) -> Self {
        let mut talc = Self::new(oom_handler);
        talc.init(arena);
        talc
    }

    /// Returns the [`Span`] which has been granted to this allocator as allocatable.
    pub const fn get_arena(&self) -> Span {
        self.arena
    }

    /// Returns the [`Span`] in which allocations may be placed.
    pub fn get_allocatable_span(&self) -> Span {
        Span::new(self.allocatable_base, self.allocatable_acme)
    }

    /// Returns the minimum [`Span`] containing all allocated memory.
    pub fn get_allocated_span(&self) -> Span {
        // check if the arena is nonexistant
        if self.get_allocatable_span().size() < MIN_CHUNK_SIZE {
            return Span::empty();
        }

        let mut allocated_acme = self.allocatable_acme;
        let mut allocated_base = self.allocatable_base;

        // check for free space at the arena's top
        let top_disc = allocated_acme.wrapping_sub(TAG_SIZE);
        if !(unsafe { *top_disc.cast::<Tag>() }).is_allocated() {
            let top_size = unsafe { top_disc.cast::<usize>().read() };
            allocated_acme = allocated_acme.wrapping_sub(top_size);
        }

        // check for free memory at the bottom of the arena
        if self.is_base_free {
            let bottom_size = unsafe { FreeChunk(self.allocatable_base).size_ptr().read() };
            allocated_base = allocated_base.wrapping_add(bottom_size);
        }

        // allocated_base might be greater or equal to allocated_acme
        // but that's fine, this'll just become an empty span
        Span::new(allocated_base, allocated_acme)
    }

    /// Initialize the allocator heap.
    ///
    /// Note that metadata will be placed into the bottom of the heap.
    /// It should be on the order of ~1KiB on 64-bit systems.
    /// If the arena isn't big enough, this function will **not** panic.
    /// However, no memory will be made available for allocation.
    ///
    /// # Reinitialization
    /// Calling `init` on the same [`Talc`] multiple times is valid. However,
    /// this will "forget" all prior allocations, as if an entirely new allocator
    /// was constructed.
    ///
    /// # Safety
    /// - The memory within the `arena` must be valid for reads and writes,
    /// and memory therein not allocated to the user must not be mutated
    /// while the allocator is in use.
    ///
    /// # Panics
    /// Panics if `arena` contains the null address.
    pub unsafe fn init(&mut self, arena: Span) {
        // set up the allocator with a new arena
        // we need to store the metadata in the heap
        // by using allocation chunk metadata, it's not a special special case
        // essentially, we want to allocate the metadata by hand

        assert!(!arena.contains(null_mut()), "Arena covers the null address!");

        self.arena = arena;
        self.availability_low = 0;
        self.availability_high = 0;

        let aligned_arena = arena.word_align_inward();

        // if this fails, there's no space to work with
        if let Some((base, acme)) = aligned_arena.get_base_acme() {
            const BIN_ALIGNMENT: usize = core::mem::align_of::<Bin>();
            const BIN_ARRAY_SIZE: usize = core::mem::size_of::<Bin>() * BIN_COUNT;

            // check if there's enough space to bother
            if acme as usize - base as usize >= BIN_ARRAY_SIZE + TAG_SIZE + MIN_CHUNK_SIZE {
                // align the metadata pointer against the base of the arena
                let metadata_ptr = align_up_by(base, BIN_ALIGNMENT - 1);
                // align the tag pointer against the top of the metadata
                let tag_ptr = align_up(metadata_ptr.add(BIN_ARRAY_SIZE));

                self.allocatable_base = base;
                self.allocatable_acme = acme;

                // initialize the bins to None
                for i in 0..BIN_COUNT {
                    let bin_ptr = metadata_ptr.cast::<Bin>().add(i);
                    *bin_ptr = None;
                }

                self.bins = metadata_ptr.cast::<Bin>();
                self.is_base_free = false;

                // check whether there's enough room on top to free
                // add_chunk_to_record only depends on self.bins
                let metadata_chunk_acme = tag_ptr.add(TAG_SIZE);
                if is_chunk_size(metadata_chunk_acme, acme) {
                    self.register(metadata_chunk_acme, acme);
                    Tag::write(tag_ptr, base, true);
                } else {
                    Tag::write(tag_ptr, base, false);
                }

                scan_for_errors(self);

                return;
            }
        }

        // fallthrough from being unable to allocate metadata

        self.allocatable_base = null_mut();
        self.allocatable_acme = null_mut();
        self.bins = null_mut();
        self.is_base_free = false;

        scan_for_errors(self);
    }

    /// Increase the extent of the arena.
    ///
    /// # Safety
    /// The entire new_arena memory but be readable and writable
    /// and unmutated besides that which is allocated. So on and so forth.
    ///
    /// # Panics
    /// This function panics if:
    /// - `new_arena` doesn't contain the old arena (NB: empty arenas are contained by any arena)
    /// - `new_arena` contains the null address
    ///
    /// A recommended pattern for satisfying these criteria is:
    /// ```rust
    /// # use talc::*;
    /// # let mut talc = Talc::new(ErrOnOom);
    /// // compute the new arena as an extention of the old arena
    /// // for the sake of example we avoid the null page too
    /// let new_arena = talc.get_arena().extend(1234, 5678).above(0x400 as *mut u8);
    /// // SAFETY: be sure not to extend into memory we can't use
    /// unsafe { talc.extend(new_arena); }
    /// ```
    pub unsafe fn extend(&mut self, new_arena: Span) {
        assert!(new_arena.contains_span(self.arena), "new_span must contain the current arena");
        assert!(!new_arena.contains(null_mut()), "Arena covers the null address!");

        if !is_chunk_size(self.allocatable_base, self.allocatable_acme) {
            // there's no free or allocated memory, so just init instead
            self.init(new_arena);
            return;
        }

        self.arena = new_arena;

        let old_alloc_base = self.allocatable_base;
        let old_alloc_acme = self.allocatable_acme;

        match new_arena.word_align_inward().get_base_acme() {
            Some((base, acme)) if acme as usize - base as usize >= MIN_CHUNK_SIZE => {
                self.allocatable_base = base;
                self.allocatable_acme = acme;
            }

            // we confirmed the new_arena is bigger than the old arena
            // and that the old allocatable range is bigger than min chunk size
            // thus the aligned result should be big enough
            _ => unreachable!(),
        }

        // if the top chunk is free, extend the block to cover the new extra area
        // otherwise allocate above if possible
        if !(*old_alloc_acme.sub(TAG_SIZE).cast::<Tag>()).is_allocated() {
            let top_size = old_alloc_acme.sub(TAG_SIZE).cast::<usize>().read();
            let top_chunk = FreeChunk(old_alloc_acme.sub(top_size));

            self.deregister(top_chunk.node_ptr(), bin_of_size(top_size));
            self.register(top_chunk.base(), self.allocatable_acme);
        } else if is_chunk_size(old_alloc_acme, self.allocatable_acme) {
            self.register(old_alloc_acme, self.allocatable_acme);
            Tag::set_above_free(old_alloc_acme.sub(TAG_SIZE).cast());
        } else {
            self.allocatable_acme = old_alloc_acme;
        }

        // extend the bottom chunk if it's free, else add free chunk below if possible
        if self.is_base_free {
            let bottom_chunk = FreeChunk(old_alloc_base);
            let bottom_size = bottom_chunk.size_ptr().read();

            self.deregister(bottom_chunk.node_ptr(), bin_of_size(bottom_size));
            self.register(self.allocatable_base, bottom_chunk.base().add(bottom_size));
        } else if is_chunk_size(self.allocatable_base, old_alloc_base) {
            self.register(self.allocatable_base, old_alloc_base);
            self.is_base_free = true;
        } else {
            self.allocatable_base = old_alloc_base;
        }

        scan_for_errors(self);
    }

    /// Reduce the extent of the arena.
    /// The new extent must encompass all current allocations. See below.
    ///
    /// # Panics:
    /// This function panics if:
    /// - old arena doesn't contain `new_arena`
    /// - `new_arena` doesn't contain all the allocated memory
    ///
    /// The recommended pattern for satisfying these criteria is:
    /// ```rust
    /// # use talc::*;
    /// # let mut talc = Talc::new(ErrOnOom);
    /// // note: lock the allocator otherwise a race condition may occur
    /// // in between get_allocated_span and truncate
    ///
    /// // compute the new arena as a reduction of the old arena
    /// let new_arena = talc.get_arena().truncate(1234, 5678).fit_over(talc.get_allocated_span());
    /// // alternatively...
    /// let new_arena = Span::from((1234 as *mut u8)..(5678 as *mut u8))
    ///     .fit_within(talc.get_arena())
    ///     .fit_over(talc.get_allocated_span());
    /// // truncate the arena
    /// talc.truncate(new_arena);
    /// ```
    pub fn truncate(&mut self, new_arena: Span) {
        let new_alloc_span = new_arena.word_align_inward();

        // check that the new_arena is valid
        assert!(self.arena.contains_span(new_arena), "the old arena must contain new_arena!");
        assert!(
            new_alloc_span.contains_span(self.get_allocated_span()),
            "the new_arena must contain the allocated span!"
        );

        // if the old allocatable arena is uninitialized, just reinit
        if self.allocatable_base == null_mut() || self.allocatable_acme == null_mut() {
            unsafe {
                // SAFETY: new_arena is smaller than the current arena
                self.init(new_arena);
            }
            return;
        }

        let new_alloc_base;
        let new_alloc_acme;

        // if it's decimating the entire arena, just reinit, else get the new allocatable extents
        match new_alloc_span.get_base_acme() {
            Some((base, acme)) if is_chunk_size(base, acme) => {
                self.arena = new_arena;
                new_alloc_base = base;
                new_alloc_acme = acme;
            }
            _ => {
                // this shouldn't ever be executed while we're using the heap
                // for metadata, but this code shall remain in case of changes
                unsafe {
                    // SAFETY: new_arena is smaller than the current arena
                    self.init(new_arena);
                }
                return;
            }
        }

        // trim down the arena

        // trim the top
        if new_alloc_acme < self.allocatable_acme {
            let top_size = unsafe { self.allocatable_acme.sub(WORD_SIZE).cast::<usize>().read() };
            let top_chunk = FreeChunk(self.allocatable_acme.wrapping_sub(top_size));

            unsafe {
                self.deregister(top_chunk.node_ptr(), bin_of_size(top_size));
            }

            if is_chunk_size(top_chunk.base(), new_alloc_acme) {
                self.allocatable_acme = new_alloc_acme;

                unsafe {
                    self.register(top_chunk.base(), new_alloc_acme);
                }
            } else {
                self.allocatable_acme = top_chunk.base();

                unsafe {
                    Tag::clear_above_free(top_chunk.base().sub(TAG_SIZE).cast());
                }
            }
        }

        // no need to check if the entire arena vanished;
        // we checked against this possiblity earlier
        // i.e. that new_alloc_span is insignificantly sized

        // check for free memory at the bottom of the arena
        if self.allocatable_base < new_alloc_base {
            debug_assert!(self.is_base_free);

            let bottom_chunk = FreeChunk(self.allocatable_base);
            let bottom_size = unsafe { bottom_chunk.size_ptr().read() };
            let bottom_acme = bottom_chunk.base().wrapping_add(bottom_size);

            unsafe {
                self.deregister(bottom_chunk.node_ptr(), bin_of_size(bottom_size));
            }

            if is_chunk_size(new_alloc_base, bottom_acme) {
                self.allocatable_base = new_alloc_base;

                unsafe {
                    self.register(new_alloc_base, bottom_acme);
                }
            } else {
                self.allocatable_base = bottom_acme;
                self.is_base_free = false;
            }
        }

        scan_for_errors(self);
    }

    /// Wrap in `Talck`, a mutex-locked wrapper struct using [`lock_api`].
    ///
    /// This implements the [`GlobalAlloc`](core::alloc::GlobalAlloc) trait and provides
    /// access to the [`Allocator`](core::alloc::Allocator) API.
    ///
    /// # Examples
    /// ```
    /// # use talc::*;
    /// # use core::alloc::{GlobalAlloc, Layout};
    /// use spin::Mutex;
    /// let talc = Talc::new(ErrOnOom);
    /// let talck = talc.lock::<Mutex<()>>();
    ///
    /// unsafe {
    ///     talck.alloc(Layout::from_size_align_unchecked(32, 4));
    /// }
    /// ```
    #[cfg(feature = "lock_api")]
    pub const fn lock<R: lock_api::RawMutex>(self) -> Talck<R, O> {
        Talck(lock_api::Mutex::new(self))
    }

    #[cfg(feature = "lock_api")]
    pub const unsafe fn lock_assume_single_threaded(self) -> Talck<talck::AssumeUnlockable, O> {
        Talck(lock_api::Mutex::new(self))
    }
}

#[cfg(target_family = "wasm")]
pub type TalckWasm = Talck<AssumeUnlockable, WasmHandler>;

#[cfg(target_family = "wasm")]
impl TalckWasm {
    /// Create a [`Talck`] instance that takes control of WASM memory management.
    ///
    /// # Safety
    /// The runtime evironment must be WASM.
    ///
    /// These restrictions apply while the allocator is in use:
    /// - WASM memory should not manipulated unless allocated.
    pub const unsafe fn new_global() -> Self {
        Talc::new(WasmHandler).lock_assume_single_threaded()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_dealloc_test() {
        const ARENA_SIZE: usize = 10000000;

        let arena = Box::leak(vec![0u8; ARENA_SIZE].into_boxed_slice()) as *mut [_];

        let mut talc = unsafe { Talc::with_arena(ErrOnOom, arena.into()) };

        let layout = Layout::from_size_align(1243, 8).unwrap();

        let a = unsafe { talc.malloc(layout) };
        assert!(a.is_ok());
        unsafe {
            a.unwrap().as_ptr().write_bytes(255, layout.size());
        }

        let mut x = vec![NonNull::dangling(); 100];

        for _ in 0..1 {
            for i in 0..100 {
                let allocation = unsafe { talc.malloc(layout) };
                assert!(allocation.is_ok());
                unsafe {
                    allocation.unwrap().as_ptr().write_bytes(0xab, layout.size());
                }
                x[i] = allocation.unwrap();
            }

            for i in 0..50 {
                unsafe {
                    talc.free(x[i], layout);
                }
            }
            for i in (50..100).rev() {
                unsafe {
                    talc.free(x[i], layout);
                }
            }
        }

        unsafe {
            talc.free(a.unwrap(), layout);
        }

        unsafe {
            drop(Box::from_raw(arena));
        }
    }

    #[test]
    fn init_truncate_extend_test() {
        // not big enough to fit the metadata
        let mut tiny_arena = [0u8; BIN_COUNT * WORD_SIZE / 2];
        let tiny_arena_span: Span = Span::from(&mut tiny_arena);

        // big enough with plenty of extra
        let arena = Box::leak(vec![0u8; BIN_COUNT * WORD_SIZE + 100000].into_boxed_slice());
        let arena_span = Span::from(arena as *mut _);

        let mut talc = Talc::new(ErrOnOom);

        talc.truncate(Span::empty());
        assert!(talc.get_arena().is_empty());
        assert!(talc.allocatable_base.is_null() && talc.allocatable_acme.is_null());
        assert!(!talc.is_base_free);
        assert!(talc.bins.is_null());
        assert!(talc.availability_low == 0 && talc.availability_high == 0);

        unsafe {
            talc.init(tiny_arena_span);
        }

        assert!(talc.get_arena() == tiny_arena_span);
        assert!(talc.allocatable_base.is_null() && talc.allocatable_acme.is_null());
        assert!(!talc.is_base_free);
        assert!(talc.bins.is_null());
        assert!(talc.availability_low == 0 && talc.availability_high == 0);

        talc.truncate(talc.get_arena().truncate(50, 50).fit_over(talc.get_allocated_span()));

        assert!(talc.allocatable_base.is_null() && talc.allocatable_acme.is_null());
        assert!(!talc.is_base_free);
        assert!(talc.bins.is_null());
        assert!(talc.availability_low == 0 && talc.availability_high == 0);

        unsafe {
            talc.init(arena_span);
        }

        assert!(talc.get_arena() == arena_span);
        assert!(!talc.is_base_free);
        assert!(!talc.bins.is_null());

        talc.truncate(talc.get_arena().truncate(500, 500).fit_over(talc.get_allocated_span()));

        let allocation = unsafe {
            let allocation = talc.malloc(Layout::new::<u128>()).unwrap();
            allocation.as_ptr().write_bytes(0, Layout::new::<u128>().size());
            allocation
        };

        talc.truncate(
            talc.get_arena().truncate(100000, 100000).fit_over(talc.get_allocated_span()),
        );

        unsafe {
            talc.extend(talc.get_arena().extend(10000, 10000).fit_within(arena_span));
        }

        unsafe {
            talc.free(allocation, Layout::new::<u128>());
        }

        unsafe {
            drop(Box::from_raw(arena));
        }
    }
}
