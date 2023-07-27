#![doc = include_str!("../README.md")]
#![feature(offset_of)]
#![feature(pointer_is_aligned)]
#![feature(alloc_layout_extra)]
#![feature(slice_ptr_get)]
#![feature(slice_ptr_len)]
#![feature(const_mut_refs)]
#![feature(const_slice_ptr_len)]
#![feature(const_slice_from_raw_parts_mut)]
#![cfg_attr(not(test), no_std)]
#![cfg_attr(feature = "allocator", feature(allocator_api))]

#[cfg(feature = "lock_api")]
mod talck;

mod llist;
mod oom_handler;
mod span;
mod tag;
mod utils;

pub use oom_handler::{ErrOnOom, InitOnOom, OomHandler};
pub use span::Span;
#[cfg(feature = "lock_api")]
pub use talck::Talck;
#[cfg(all(feature = "lock_api", feature = "allocator"))]
pub use talck::TalckRef;

use llist::LlistNode;
use tag::Tag;
use utils::*;

use core::{
    alloc::Layout,
    ptr::{null_mut, NonNull},
};

// Free chunk (3x ptr size minimum):
//   ?? | NODE: LlistNode (2 * ptr) SIZE: usize, ..???.., SIZE: usize | ??
// Reserved chunk (1x ptr size of overhead):
//   ?? | TAG: Tag (usize),       ???????         | ??

// TAG contains a pointer to the top of the reserved chunk,
// a is_allocated (set) bit flag differentiating itself from a free chunk
// (the LlistNode contains well-aligned pointers, thus does not have that bit set),
// as well as a is_low_free bit flag which does what is says on the tin

// go check out bucket_of_size to see how bucketing works

const WORD_SIZE: usize = core::mem::size_of::<usize>();
const ALIGN: usize = core::mem::align_of::<usize>();

const NODE_SIZE: usize = core::mem::size_of::<LlistNode>();
const TAG_SIZE: usize = core::mem::size_of::<Tag>();

/// Minimum chunk size.
const MIN_CHUNK_SIZE: usize = NODE_SIZE + WORD_SIZE;

const BIN_COUNT: usize = usize::BITS as usize * 2;

type Bin = Option<NonNull<LlistNode>>;

/// The Talc Allocator!
///
/// Initialize with `new`, `with_arena`, `with_oom_handler` or `with_arena_and_oom_handler`.
///
/// Call [`lock`](Talc::lock) to get a [`Talck`] which supports the
/// [`GlobalAlloc`](core::alloc::GlobalAlloc) and [`Allocator`](core::alloc::Allocator) traits.
pub struct Talc<O: OomHandler> {
    pub oom_handler: O,

    arena: Span,

    allocatable_base: *mut u8,
    allocatable_acme: *mut u8,

    is_top_free: bool,

    /// The low bits of the availability flags.
    availability_low: usize,
    /// The high bits of the availability flags.
    availability_high: usize,

    /// Linked list heads.
    bins: *mut [Bin],
}

unsafe impl<O: Send + OomHandler> Send for Talc<O> {}

impl<O: OomHandler> core::fmt::Debug for Talc<O> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Talc")
            .field("arena", &self.arena)
            .field("alloc_base", &self.allocatable_base)
            .field("alloc_acme", &self.allocatable_acme)
            .field("is_top_free", &self.is_top_free)
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

        self.bins.get_unchecked_mut(bin)
    }

    /// Sets the availability flag for bin `b`.
    ///
    /// This is done when a chunk is added to an empty bin.
    #[inline]
    fn set_avails(&mut self, b: usize) {
        debug_assert!(b < BIN_COUNT);

        if b < 64 {
            debug_assert!(self.availability_low & 1 << b == 0);
            self.availability_low ^= 1 << b;
        } else {
            debug_assert!(self.availability_high & 1 << (b - 64) == 0);
            self.availability_high ^= 1 << (b - 64);
        }
    }
    /// Clears the availability flag for bin `b`.
    ///
    /// This is done when a bin becomes empty.
    #[inline]
    fn clear_avails(&mut self, b: usize) {
        debug_assert!(b < BIN_COUNT);

        // if head is the last node
        if b < 64 {
            self.availability_low ^= 1 << b;
            debug_assert!(self.availability_low & 1 << b == 0);
        } else {
            self.availability_high ^= 1 << (b - 64);
            debug_assert!(self.availability_high & 1 << (b - 64) == 0);
        }
    }

    /// Registers memory that may be allocated.
    #[inline]
    unsafe fn register(&mut self, base: *mut u8, acme: *mut u8) {
        debug_assert!(is_chunk_size(base, acme));

        let size = acme as usize - base as usize;

        let bin = bin_of_size(size);
        let free_chunk = FreeChunk(base);

        let bin_ptr = self.get_bin_ptr(bin);
        if (*bin_ptr).is_none() {
            self.set_avails(bin);
        }

        LlistNode::insert(free_chunk.node_ptr(), bin_ptr, *bin_ptr);

        debug_assert!((*bin_ptr).is_some());

        // write in low size tag above the node pointers
        *free_chunk.size_ptr() = size;
        // write in high size tag at the end of the free chunk
        *acme.cast::<usize>().sub(1) = size;
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

    /// Ensures the above chunk's `is_below_free` or the `is_top_free` flag is cleared.
    ///
    /// Assumes an allocated chunk's base is at `chunk_acme`.
    #[inline]
    unsafe fn clear_below_free(&mut self, chunk_acme: *mut u8) {
        if chunk_acme != self.allocatable_acme {
            Tag::clear_below_free(chunk_acme.cast());
        } else {
            debug_assert!(self.is_top_free);
            self.is_top_free = false;
        }
    }

    /// Like `clear_below_acme` but doesn't assume the above chunk is allocated,
    /// in which case it is deregistered and `chunk_acme` is raised to cover it.
    #[inline]
    unsafe fn try_recombine_above(&mut self, chunk_acme: &mut *mut u8) {
        if *chunk_acme != self.allocatable_acme {
            match identify_above(*chunk_acme) {
                AboveChunk::Allocated(above_tag_ptr) => Tag::set_below_free(above_tag_ptr),
                AboveChunk::Free(above) => {
                    let above_size = *above.size_ptr();
                    *chunk_acme = above.base().add(above_size);
                    self.deregister(above.node_ptr(), bin_of_size(above_size));
                }
            }
        } else {
            debug_assert!(!self.is_top_free);
            self.is_top_free = true;
        }
    }

    /// Allocate a contiguous region of memory according to `layout`, if possible.
    /// # Safety
    /// `layout.size()` must be nonzero.
    pub unsafe fn malloc(&mut self, layout: Layout) -> Result<NonNull<u8>, ()> {
        debug_assert!(layout.size() != 0);

        // no checks for initialization are performed, as it would be overhead.
        // this will return None here as the availability flags are initialized
        // to zero; all clear; no memory to allocate, call the OOM handler.
        let (chunk_base, chunk_acme, alloc_base) = loop {
            match self.get_sufficient_chunk(layout) {
                Some(payload) => break payload,
                // todo change OomHandler trait or figure out handling of outside allocations
                None => _ = O::handle_oom(self, layout)?,
            }
        };

        // the tag position immediately before the allocation
        let pre_alloc_ptr = align_down(alloc_base.sub(TAG_SIZE));
        // the tag position, accounting for the minimum size of a chunk
        let mut tag_ptr = chunk_acme.sub(MIN_CHUNK_SIZE).min(pre_alloc_ptr);

        let is_below_free = is_chunk_size(chunk_base, tag_ptr);
        if is_below_free {
            self.register(chunk_base, tag_ptr);
        } else {
            // the space below the tag is too small to register, so lower the tag
            tag_ptr = chunk_base;
        }

        if tag_ptr != pre_alloc_ptr {
            // write the real tag ptr where the tag is expected to be
            *pre_alloc_ptr.cast::<*mut u8>() = tag_ptr;
        }

        let req_acme = required_acme(alloc_base, layout.size(), tag_ptr);

        // handle the space above the required allocation span
        if is_chunk_size(req_acme, chunk_acme) {
            self.register(req_acme, chunk_acme);

            tag_ptr.cast::<Tag>().write(Tag::new(req_acme, is_below_free));
        } else {
            self.clear_below_free(chunk_acme);

            tag_ptr.cast::<Tag>().write(Tag::new(chunk_acme, is_below_free));
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

        let mut bin = self.next_bin(bin_of_size(required_chunk_size))?;

        if layout.align() <= ALIGN {
            // the required alignment is most often the machine word size (or less)
            // a faster loop without alignment checking is used in this case
            loop {
                for node_ptr in LlistNode::iter_mut(*self.get_bin_ptr(bin)) {
                    let free_chunk = FreeChunk(node_ptr.as_ptr().cast());
                    let chunk_size = *free_chunk.size_ptr();

                    // if the chunk size is sufficient, remove from bookkeeping data structures and return
                    if chunk_size >= required_chunk_size {
                        self.deregister(free_chunk.node_ptr(), bin as usize);

                        return Some((
                            free_chunk.base(),
                            free_chunk.base().add(chunk_size),
                            free_chunk.base().add(TAG_SIZE),
                        ));
                    }
                }

                bin = self.next_bin(bin + 1)?;
            }
        } else {
            // a larger than word-size alignement is demanded
            // therefore each chunk is manually checked to be sufficient accordingly
            let align_mask = layout.align() - 1;

            loop {
                for node_ptr in LlistNode::iter_mut(*self.get_bin_ptr(bin)) {
                    let free_chunk = FreeChunk(node_ptr.as_ptr().cast());
                    let chunk_size = *free_chunk.size_ptr();

                    if chunk_size >= required_chunk_size {
                        // calculate the lowest aligned pointer above the tag-offset free chunk pointer
                        let aligned_ptr = align_up_by(free_chunk.base().add(TAG_SIZE), align_mask);
                        let chunk_acme = free_chunk.base().add(chunk_size);

                        // if the remaining size is sufficient, remove the chunk from the books and return
                        if aligned_ptr.add(layout.size()) <= chunk_acme {
                            self.deregister(free_chunk.node_ptr(), bin);
                            return Some((free_chunk.base(), chunk_acme, aligned_ptr));
                        }
                    }
                }

                bin = self.next_bin(bin + 1)?;
            }
        }
    }

    #[inline(always)]
    fn next_bin(&self, bin: usize) -> Option<usize> {
        if bin < usize::BITS as usize {
            // shift flags such that only flags for larger buckets are kept
            let shifted_avails = self.availability_low >> bin;

            // find the next up, grab from the high flags, or quit
            if shifted_avails != 0 {
                Some(bin + shifted_avails.trailing_zeros() as usize)
            } else if self.availability_high != 0 {
                Some(self.availability_high.trailing_zeros() as usize + 64)
            } else {
                None
            }
        } else if bin < BIN_COUNT {
            // similar process to the above, but the low flags are irrelevant
            let shifted_avails = self.availability_high >> (bin - 64);

            if shifted_avails != 0 {
                Some(bin + shifted_avails.trailing_zeros() as usize)
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
    pub unsafe fn free(&mut self, ptr: NonNull<u8>, _: Layout) {
        let (mut chunk_base, tag) = chunk_ptr_from_alloc_ptr(ptr.as_ptr());
        let mut chunk_acme = tag.acme_ptr();

        debug_assert!(tag.is_allocated());
        debug_assert!(is_chunk_size(chunk_base, chunk_acme));

        self.try_recombine_above(&mut chunk_acme);

        // try recombine below
        if tag.is_below_free() {
            // grab the size off the top of the free chunk
            let low_chunk_size = *chunk_base.cast::<usize>().sub(1);
            chunk_base = chunk_base.sub(low_chunk_size);

            self.deregister(FreeChunk(chunk_base).node_ptr(), bin_of_size(low_chunk_size));
        }

        // add the full recombined free chunk back into the books
        self.register(chunk_base, chunk_acme);

        scan_for_errors(self);
    }

    /// Grow a previously allocated/reallocated region of memory to `new_size`.
    /// # Safety
    /// `ptr` must have been previously allocated or reallocated given `old_layout`.
    /// `new_size` must be larger or equal to `old_layout.size()`.
    pub unsafe fn grow(
        &mut self,
        ptr: NonNull<u8>,
        layout: Layout,
        new_size: usize,
    ) -> Result<NonNull<u8>, ()> {
        debug_assert!(new_size >= layout.size());

        let (chunk_base, tag) = chunk_ptr_from_alloc_ptr(ptr.as_ptr());
        let chunk_acme = tag.acme_ptr();

        debug_assert!(tag.is_allocated());
        debug_assert!(is_chunk_size(chunk_base, chunk_acme));

        let new_req_acme = required_acme(ptr.as_ptr(), new_size, chunk_base);

        // short-circuit if the chunk is already large enough
        if new_req_acme <= chunk_acme {
            return Ok(ptr);
        }

        // otherwise, check if the chunk above 1) exists 2) is free 3) is large enough
        // because free chunks don't border free chunks, this needn't be recursive
        if chunk_acme != self.allocatable_acme {
            if let AboveChunk::Free(above) = identify_above(chunk_acme) {
                let above_size = *above.size_ptr();
                let above_acme = chunk_acme.add(above_size);

                // is the additional memory sufficient?
                if above_acme >= new_req_acme {
                    self.deregister(above.node_ptr(), bin_of_size(above_size));

                    // finally, determine if the remainder of the free block is big enough
                    // to be freed again, or if the entire region should be allocated
                    if is_chunk_size(new_req_acme, above_acme) {
                        self.register(new_req_acme, above_acme);

                        Tag::set_acme(chunk_base.cast(), new_req_acme);
                    } else {
                        self.clear_below_free(above_acme);

                        Tag::set_acme(chunk_base.cast(), above_acme);
                    }

                    scan_for_errors(self);
                    return Ok(ptr);
                }
            }
        }

        // grow in-place failed; reallocate the slow way

        scan_for_errors(self);
        let allocation =
            self.malloc(Layout::from_size_align_unchecked(new_size, layout.align()))?;
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
    /// - `ptr` must have been previously allocated or reallocated given `old_layout`.
    /// - `new_size` must be smaller or equal to `old_layout.size()`.
    /// - `new_size` should be nonzero.
    pub unsafe fn shrink(&mut self, ptr: NonNull<u8>, layout: Layout, new_size: usize) {
        debug_assert!(new_size != 0);
        debug_assert!(new_size <= layout.size());

        let (chunk_ptr, tag) = chunk_ptr_from_alloc_ptr(ptr.as_ptr());
        let mut chunk_acme = tag.acme_ptr();

        debug_assert!(tag.is_allocated());
        debug_assert!(is_chunk_size(chunk_ptr, chunk_acme));

        let new_req_acme = required_acme(ptr.as_ptr(), new_size, chunk_ptr);

        // if the remainder between the new required size and the originally allocated
        // size is large enough, free the remainder, otherwise leave it
        if is_chunk_size(new_req_acme, chunk_acme) {
            self.try_recombine_above(&mut chunk_acme);

            self.register(new_req_acme, chunk_acme);

            Tag::set_acme(chunk_ptr.cast(), new_req_acme);
        }

        scan_for_errors(self);
    }

    pub const fn new(oom_handler: O) -> Self {
        Self {
            oom_handler,

            arena: Span::empty(),
            allocatable_base: core::ptr::null_mut(),
            allocatable_acme: core::ptr::null_mut(),
            is_top_free: true,

            availability_low: 0,
            availability_high: 0,
            bins: core::ptr::slice_from_raw_parts_mut(null_mut(), 0),
        }
    }

    /// Contruct and initialize a `Talc` with the given arena and OOM handler function.
    /// # Safety
    /// See [`init`](Talc::init) for safety requirements.
    pub unsafe fn with_arena(oom_handler: O, arena: Span) -> Self {
        let mut talc = Self::new(oom_handler);
        talc.init(arena);
        talc
    }

    /// Returns the [`Span`] which has been granted to this
    /// allocator as allocatable.
    pub const fn get_arena(&self) -> Span {
        self.arena
    }

    /// Returns the [`Span`] in which allocations may be placed.
    pub fn get_allocatable_span(&self) -> Span {
        Span::from(self.allocatable_base..self.allocatable_acme)
    }

    /// Returns the minimum [`Span`] containing all allocated memory.
    pub fn get_allocated_span(&self) -> Span {
        // check if the arena is nonexistant
        if MIN_CHUNK_SIZE > self.allocatable_acme as usize - self.allocatable_base as usize {
            return Span::empty();
        }

        let mut allocated_acme = self.allocatable_acme;
        let mut allocated_base = self.allocatable_base;

        // check for free space at the arena's top
        if self.is_top_free {
            let top_free_size = unsafe { *self.allocatable_acme.cast::<usize>().sub(1) };

            allocated_acme = allocated_acme.wrapping_sub(top_free_size);
        }

        // check for free memory at the bottom of the arena
        if !(unsafe { *self.allocatable_base.cast::<Tag>() }).is_allocated() {
            let free_bottom_chunk = FreeChunk(self.allocatable_base);
            let free_bottom_size = unsafe { *free_bottom_chunk.size_ptr() };

            allocated_base = allocated_base.wrapping_add(free_bottom_size);
        }

        // allocated_base might be greater or equal to allocated_acme
        // but that's fine, this'll just become a Span::Empty
        Span::new(allocated_base, allocated_acme)
    }

    /// Initialize the allocator heap.
    ///
    /// Note that metadata will be placed into the bottom of the heap.
    /// It should be ~1KiB. Note that if the arena isn't big enough,
    /// this function will **not** fail. However, no memory will be made
    /// available for allocation, and allocations will signal OOM.
    ///
    /// # Safety
    /// - The memory within the `arena` must be valid for reads and writes,
    /// and memory therein not allocated to the user must not be mutated
    /// for the lifetime of all the allocations of this allocator.
    ///
    /// # Panics
    /// Panics if `arena` contains the null address.
    pub unsafe fn init(&mut self, arena: Span) {
        // set up the allocator with a new arena
        // we need to store the metadata in the heap
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

            // check if aligning up and adding TAG_SIZE is possible (if not, there's not enough space)
            if BIN_ALIGNMENT - 1 + TAG_SIZE <= usize::MAX - base as usize {
                // allocated metadata chunk tag at the bottom of the arena
                let tag_ptr = base;

                // add TAG_SIZE to the base pointer (to allow sufficient space for it) and align up for bin
                // tag_ptr.wrapping_add(TAG_SIZE) is probably already correct, unless BIN_ALIGNMENT > ALIGN
                let metadata_ptr = align_up_by(tag_ptr.add(TAG_SIZE), BIN_ALIGNMENT - 1);

                // finally, check if there's enough space to allocate the bin array
                if acme as usize - metadata_ptr as usize > BIN_ARRAY_SIZE {
                    self.allocatable_base = base;
                    self.allocatable_acme = acme;

                    // this shouldn't be necessary unless the align of Bin changes to be >8
                    let tag_ptr_ptr = align_down(metadata_ptr.sub(ALIGN));
                    if tag_ptr_ptr != tag_ptr {
                        *tag_ptr_ptr.cast::<*mut Tag>() = tag_ptr.cast();
                    }

                    let metadata_acme = metadata_ptr.add(BIN_ARRAY_SIZE);

                    // write the value for the tag in
                    tag_ptr.cast::<Tag>().write(Tag::new(metadata_acme, false));

                    // initialize the bins to None
                    for i in 0..BIN_COUNT {
                        let bin_ptr = metadata_ptr.cast::<Bin>().add(i);
                        *bin_ptr = None;
                    }

                    self.bins =
                        core::ptr::slice_from_raw_parts_mut(metadata_ptr.cast::<Bin>(), BIN_COUNT);

                    // check whether there's enough room on top to free
                    // add_chunk_to_record only depends on self.bins
                    if is_chunk_size(metadata_acme, acme) {
                        self.register(metadata_acme, acme);
                        self.is_top_free = true;
                    } else {
                        self.is_top_free = false;
                    }

                    scan_for_errors(self);

                    return;
                }
            }
        }

        // fallthrough from being unable to allocate metadata

        self.allocatable_base = null_mut();
        self.allocatable_acme = null_mut();
        self.is_top_free = false;

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
    /// let new_arena = talc.get_arena().extend(1234, 5678).above(0x1000 as *mut u8);
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
        if self.is_top_free {
            let top_size = *old_alloc_acme.cast::<usize>().sub(1);
            let top_chunk = FreeChunk(old_alloc_acme.sub(top_size));

            self.deregister(top_chunk.node_ptr(), bin_of_size(top_size));
            self.register(top_chunk.base(), self.allocatable_acme);
        } else if is_chunk_size(old_alloc_acme, self.allocatable_acme) {
            self.register(old_alloc_acme, self.allocatable_acme);

            self.is_top_free = true;
        } else {
            self.allocatable_acme = old_alloc_acme;
        }

        // extend the bottom chunk if it's free, else add free chunk below if possible
        if !(*old_alloc_base.cast::<Tag>()).is_allocated() {
            let bottom_chunk = FreeChunk(old_alloc_base);
            let bottom_size = *bottom_chunk.size_ptr();

            self.deregister(bottom_chunk.node_ptr(), bin_of_size(bottom_size));
            self.register(self.allocatable_base, bottom_chunk.base().add(bottom_size));
        } else if is_chunk_size(self.allocatable_base, old_alloc_base) {
            self.register(self.allocatable_base, old_alloc_base);

            Tag::set_below_free(old_alloc_base.cast());
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
            debug_assert!(self.is_top_free);

            let top_free_size = unsafe { *self.allocatable_acme.cast::<usize>().sub(1) };

            let top_free_chunk = FreeChunk(self.allocatable_acme.wrapping_sub(top_free_size));

            unsafe {
                self.deregister(top_free_chunk.node_ptr(), bin_of_size(top_free_size));
            }

            if is_chunk_size(top_free_chunk.base(), new_alloc_acme) {
                self.allocatable_acme = new_alloc_acme;

                unsafe {
                    self.register(top_free_chunk.base(), new_alloc_acme);
                }
            } else {
                self.allocatable_acme = top_free_chunk.base();
                self.is_top_free = false;
            }
        }

        // no need to check if the entire arena vanished;
        // we checked against this possiblity earlier
        // i.e. that new_alloc_span is insignificantly sized

        // check for free memory at the bottom of the arena
        if new_alloc_base > self.allocatable_base {
            let bottom_chunk = FreeChunk(self.allocatable_base);
            let bottom_chunk_size = unsafe { *bottom_chunk.size_ptr() };
            let bottom_acme = bottom_chunk.base().wrapping_add(bottom_chunk_size);

            unsafe {
                self.deregister(bottom_chunk.node_ptr(), bin_of_size(bottom_chunk_size));
            }

            if is_chunk_size(new_alloc_base, bottom_acme) {
                self.allocatable_base = new_alloc_base;

                unsafe {
                    self.register(new_alloc_base, bottom_acme);
                }
            } else {
                self.allocatable_base = bottom_acme;

                unsafe {
                    Tag::clear_below_free(bottom_acme.cast());
                }
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

    #[test]
    fn talc_test() {
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
    fn init_truncate_test() {
        let mut tiny_arena = [0u64; 100];
        let tiny_arena_span: Span = Span::from(&mut tiny_arena);
        let arena = Box::leak(vec![0u8; 1000000].into_boxed_slice());
        let arena_span = Span::from(arena as *mut _);

        let mut talc = Talc::new(ErrOnOom);

        talc.truncate(Span::empty());
        assert!(talc.get_arena().is_empty());
        assert!(talc.allocatable_base.is_null() && talc.allocatable_acme.is_null());
        assert!(!talc.is_top_free);
        assert!(talc.bins.len() == 0 && talc.bins.as_mut_ptr().is_null());
        assert!(talc.availability_low == 0 && talc.availability_high == 0);

        unsafe {
            talc.init(tiny_arena_span);
        }

        assert!(talc.get_arena() == tiny_arena_span);
        assert!(talc.allocatable_base.is_null() && talc.allocatable_acme.is_null());
        assert!(!talc.is_top_free);
        assert!(talc.bins.len() == 0 && talc.bins.as_mut_ptr().is_null());
        assert!(talc.availability_low == 0 && talc.availability_high == 0);

        talc.truncate(talc.get_arena().truncate(50, 50).fit_over(talc.get_allocated_span()));

        assert!(talc.allocatable_base.is_null() && talc.allocatable_acme.is_null());
        assert!(!talc.is_top_free);
        assert!(talc.bins.len() == 0 && talc.bins.as_mut_ptr().is_null());
        assert!(talc.availability_low == 0 && talc.availability_high == 0);

        unsafe {
            talc.init(arena_span);
        }

        assert!(talc.get_arena() == arena_span);
        assert!(talc.is_top_free);
        assert!(talc.bins.len() == BIN_COUNT);

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
            talc.free(allocation, Layout::new::<u128>());
        }

        unsafe {
            drop(Box::from_raw(arena));
        }
    }
}
