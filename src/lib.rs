#![doc = include_str!("../README.md")]
#![cfg_attr(not(any(test, fuzzing)), no_std)]
#![cfg_attr(feature = "allocator", feature(allocator_api))]
#![cfg_attr(feature = "nightly_api", feature(slice_ptr_len))]
#![cfg_attr(feature = "nightly_api", feature(const_slice_ptr_len))]

#[cfg(feature = "lock_api")]
mod talck;

mod llist;
mod oom_handler;
mod span;
mod tag;
mod utils;

#[cfg(all(target_family = "wasm", feature = "lock_api"))]
pub use oom_handler::WasmHandler;
pub use oom_handler::{ClaimOnOom, ErrOnOom, OomHandler};
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
const MIN_HEAP_SIZE: usize = MIN_CHUNK_SIZE + TAG_SIZE;

const BIN_COUNT: usize = usize::BITS as usize * 2;

type Bin = Option<NonNull<LlistNode>>;

/// The Talc Allocator!
///
/// One way to get started:
/// 1. Construct with [`new`](Talc::new) (supply [`ErrOnOom`] to ignore OOM handling).
/// 2. Establish any number of heaps with [`claim`](Talc::claim).
/// 3. Call [`lock`](Talc::lock) to get a [`Talck`] which supports the
/// [`GlobalAlloc`](core::alloc::GlobalAlloc) and [`Allocator`](core::alloc::Allocator) traits.
pub struct Talc<O: OomHandler> {
    /// The user-specified OOM handler. 
    /// 
    /// Its state is entirely maintained by the user.
    pub oom_handler: O,

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
            .field("availability_low", &format_args!("{:x}", self.availability_low))
            .field("availability_high", &format_args!("{:x}", self.availability_high))
            .field("metadata_ptr", &self.bins)
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

    /// Allocate a contiguous region of memory according to `layout`, if possible.
    /// # Safety
    /// `layout.size()` must be nonzero.
    pub unsafe fn malloc(&mut self, layout: Layout) -> Result<NonNull<u8>, ()> {
        debug_assert!(layout.size() != 0);

        let (mut chunk_base, chunk_acme, alloc_base) = loop {
            // this returns None if there are no heaps or allocatable memory
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
            Tag::clear_above_free(chunk_base.sub(TAG_SIZE).cast());
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

        // if there are no valid heaps, availability is zero, and next_available_bin returns None
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
            // a larger than word-size alignment is demanded
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
        let (tag_ptr, tag) = tag_from_alloc_ptr(ptr.as_ptr(), layout.size());
        let mut chunk_base = tag.base_ptr();
        let mut chunk_acme = tag_ptr.add(TAG_SIZE);

        debug_assert!(tag.is_allocated());
        debug_assert!(is_chunk_size(chunk_base, chunk_acme));

        // try recombine below
        let below_ptr = chunk_base.sub(TAG_SIZE).cast::<Tag>();
        let below_tag = below_ptr.read();
        if below_tag.is_allocated() {
            Tag::set_above_free(below_ptr);
        } else {
            // if the below tag doesn't have the allocated flag set,
            // it's actually a size usize of a free chunk!
            let below_size = below_tag.base_ptr() as usize;
            chunk_base = chunk_base.sub(below_size);
            self.deregister(FreeChunk(chunk_base).node_ptr(), bin_of_size(below_size));
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
    /// This function is infallible given valid inputs, and the reallocation will always be
    /// done in-place, maintaining the validity of the pointer.
    ///
    /// # Safety
    /// - `ptr` must have been previously allocated or reallocated given `layout`.
    /// - `new_size` must be smaller or equal to `layout.size()`.
    /// - `new_size` should be nonzero.
    pub unsafe fn shrink(&mut self, ptr: NonNull<u8>, layout: Layout, new_size: usize) {
        debug_assert!(new_size != 0);
        debug_assert!(new_size <= layout.size());

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
        Self { oom_handler, availability_low: 0, availability_high: 0, bins: null_mut() }
    }

    /// Returns the minimum [`Span`] containing this heap's allocated memory.
    /// # Safety
    /// `heap` must be the return value of a heap manipulation function.
    pub unsafe fn get_allocated_span(&self, heap: Span) -> Span {
        assert!(heap.size() >= MIN_HEAP_SIZE);

        let (mut base, mut acme) = heap.get_base_acme().unwrap();

        // check for free space at the heap's top
        let top_disc = acme.wrapping_sub(TAG_SIZE);
        if !unsafe { top_disc.cast::<Tag>().read() }.is_allocated() {
            let top_size = unsafe { top_disc.cast::<usize>().read() };
            acme = acme.wrapping_sub(top_size);
        }

        // check for free memory at the bottom of the heap using the base tag
        if unsafe { base.cast::<Tag>().read() }.is_above_free() {
            let bottom_base = base.wrapping_add(TAG_SIZE);
            let bottom_size = unsafe { FreeChunk(bottom_base).size_ptr().read() };
            base = base.wrapping_add(bottom_size - TAG_SIZE);
        }

        // base might be greater that acme for an empty heap
        // but that's fine, this'll just become an empty span
        Span::new(base, acme)
    }

    /// Attempt to initialize a new heap for the allocator.
    ///
    /// Note:
    /// * Each heap reserves a `usize` at the bottom as fixed overhead.
    /// * Metadata will be placed into the bottom of the first successfully established heap.
    /// It is currently ~1KiB on 64-bit systems (less on 32-bit). This is subject to change.
    ///
    /// # Return Values
    /// The resulting [`Span`] is the actual heap extent, and may
    /// be slightly smaller than requested. Use this to resize the heap.
    /// Any memory outside the claimed heap is free to use.
    ///
    /// Returns [`Err`] where
    /// * allocator metadata is not yet established, and there's insufficient memory to do so.
    /// * allocator metadata is established, but the heap is too small
    /// (less than around `4 * usize` for now).
    ///
    /// # Safety
    /// - The memory within the `memory` must be valid for reads and writes,
    /// and memory therein not allocated to the user must not be mutated
    /// while the allocator is in use.
    /// - `memory` should not overlap with any other active heap.
    ///
    /// # Panics
    /// Panics if `memory` contains the null address.
    pub unsafe fn claim(&mut self, memory: Span) -> Result<Span, ()> {
        const BIN_ARRAY_SIZE: usize = core::mem::size_of::<Bin>() * BIN_COUNT;

        // create a new heap
        // if bins is null, we will need to try put the metadata in this heap
        // this metadata is allocated 'by hand' to be isomorphic with other chunks

        assert!(!memory.contains(null_mut()), "heap covers the null address!");

        let aligned_heap = memory.word_align_inward();

        // if this fails, there's no space to work with
        if let Some((base, acme)) = aligned_heap.get_base_acme() {
            // the allocator has already successfully allocated its metadata
            if !self.bins.is_null() {
                // check if there's enough space to establish a free chunk
                if acme as usize - base as usize >= MIN_HEAP_SIZE {
                    // write in the base tag
                    Tag::write(base, null_mut(), true);

                    // register the free memory
                    let chunk_base = base.wrapping_add(TAG_SIZE);
                    self.register(chunk_base, acme);

                    scan_for_errors(self);

                    return Ok(aligned_heap);
                }
            } else {
                // check if there's enough space to allocate metadata and establish a free chunk
                if acme as usize - base as usize >= TAG_SIZE + BIN_ARRAY_SIZE + TAG_SIZE {
                    Tag::write(base, null_mut(), false);

                    // align the metadata pointer against the base of the heap
                    let metadata_ptr = base.add(TAG_SIZE);
                    // align the tag pointer against the top of the metadata
                    let post_metadata_ptr = metadata_ptr.add(BIN_ARRAY_SIZE);

                    // initialize the bins to None
                    for i in 0..BIN_COUNT {
                        let bin_ptr = metadata_ptr.cast::<Bin>().add(i);
                        bin_ptr.write(None);
                    }

                    self.bins = metadata_ptr.cast::<Bin>();

                    // check whether there's enough room on top to free
                    // add_chunk_to_record only depends on self.bins
                    let metadata_chunk_acme = post_metadata_ptr.add(TAG_SIZE);
                    if is_chunk_size(metadata_chunk_acme, acme) {
                        self.register(metadata_chunk_acme, acme);
                        Tag::write(post_metadata_ptr, base, true);
                    } else {
                        let tag_ptr = acme.sub(TAG_SIZE);
                        post_metadata_ptr.cast::<*mut u8>().write(tag_ptr);
                        Tag::write(tag_ptr, base, false);
                    }

                    scan_for_errors(self);

                    return Ok(aligned_heap);
                }
            }
        }

        // fallthrough from insufficient size

        Err(())
    }

    /// Increase the extent of a heap. The new extent of the heap is returned,
    /// and will be equal to or slightly smaller than requested.
    ///
    /// # Safety
    /// - `old_heap` must be the return value of a heap-manipulation function
    /// of this allocator instance.
    /// - The entire `new_heap` memory but be readable and writable
    /// and unmutated besides that which is allocated so long as the heap is in use.
    ///
    /// # Panics
    /// This function panics if:
    /// - `old_heap` is too small or heap metadata is not yet allocated
    /// - `new_heap` doesn't contain `old_heap`
    /// - `new_heap` contains the null address
    ///
    /// A recommended pattern for satisfying these criteria is:
    /// ```rust
    /// # use talc::*;
    /// # let mut talc = Talc::new(ErrOnOom);
    /// let mut heap = [0u8; 2000];
    /// let old_heap = Span::from(&mut heap[300..1700]);
    /// let old_heap = unsafe { talc.claim(old_heap).unwrap() };
    ///
    /// // compute the new heap span as an extension of the old span
    /// let new_heap = old_heap.extend(250, 500).fit_within((&mut heap[..]).into());
    ///
    /// // SAFETY: be sure not to extend into memory we can't use
    /// let new_heap = unsafe { talc.extend(old_heap, new_heap) };
    /// ```
    pub unsafe fn extend(&mut self, old_heap: Span, new_heap: Span) -> Span {
        assert!(!self.bins.is_null());
        assert!(old_heap.size() >= MIN_HEAP_SIZE);
        assert!(new_heap.contains_span(old_heap), "new_heap must contain old_heap");
        assert!(!new_heap.contains(null_mut()), "new_heap covers the null address!");

        let (old_base, old_acme) = old_heap.word_align_inward().get_base_acme().unwrap();
        let (new_base, new_acme) = new_heap.word_align_inward().get_base_acme().unwrap();
        let old_chunk_base = old_base.add(TAG_SIZE);
        let new_chunk_base = new_base.add(TAG_SIZE);
        let mut ret_base = new_base;
        let mut ret_acme = new_acme;

        // if the top chunk is free, extend the block to cover the new extra area
        // otherwise allocate above if possible
        if !(old_acme.sub(TAG_SIZE).cast::<Tag>().read()).is_allocated() {
            let top_size = old_acme.sub(TAG_SIZE).cast::<usize>().read();
            let top_chunk = FreeChunk(old_acme.sub(top_size));

            self.deregister(top_chunk.node_ptr(), bin_of_size(top_size));
            self.register(top_chunk.base(), new_acme);
        } else if is_chunk_size(old_acme, new_acme) {
            self.register(old_acme, new_acme);
            Tag::set_above_free(old_acme.sub(TAG_SIZE).cast());
        } else {
            ret_acme = old_acme;
        }

        // extend the bottom chunk if it's free, else add free chunk below if possible
        if unsafe { old_base.cast::<Tag>().read() }.is_above_free() {
            let bottom_chunk = FreeChunk(old_chunk_base);
            let bottom_size = bottom_chunk.size_ptr().read();

            self.deregister(bottom_chunk.node_ptr(), bin_of_size(bottom_size));
            self.register(new_chunk_base, bottom_chunk.base().add(bottom_size));
            Tag::write(new_base, null_mut(), true);
        } else if is_chunk_size(new_chunk_base, old_chunk_base) {
            self.register(new_chunk_base, old_chunk_base);
            Tag::write(new_base, null_mut(), true);
        } else {
            ret_base = old_base;
        }

        scan_for_errors(self);

        Span::new(ret_base, ret_acme)
    }

    /// Reduce the extent of a heap.
    /// The new extent must encompass all current allocations. See below.
    ///
    /// The resultant heap is always equal to or slightly smaller than `new_heap`.
    ///
    /// Truncating to an empty [`Span`] is valid for heaps where no memory is
    /// allocated within it, where [`get_allocated_span`](Talc::get_allocated_span) is empty.
    /// In all cases where the return value is empty, the heap no longer exists.
    /// You may do what you like with the memory. The empty span should not be
    /// used as input to [`truncate`](Talc::truncate), [`extend`](Talc::extend),
    /// or [`get_allocated_span`](Talc::get_allocated_span).
    ///
    /// # Safety
    /// `old_heap` must be the return value of a heap-manipulation function
    /// of this allocator instance.
    ///
    /// # Panics:
    /// This function panics if:
    /// - `old_heap` doesn't contain `new_heap`
    /// - `new_heap` doesn't contain all the allocated memory in `old_heap`
    /// - the heap metadata is not yet allocated
    ///
    /// A recommended pattern for satisfying these criteria is:
    /// ```rust
    /// # use talc::*;
    /// # let mut talc = Talc::new(ErrOnOom);
    /// let mut heap = [0u8; 2000];
    /// let old_heap = Span::from(&mut heap[300..1700]);
    /// let old_heap = unsafe { talc.claim(old_heap).unwrap() };
    ///
    /// // note: lock a `Talck` otherwise a race condition may occur
    /// // in between Talc::get_allocated_span and Talc::truncate
    ///
    /// // compute the new heap span as a truncation of the old span
    /// let new_heap = old_heap
    ///     .truncate(250, 300)
    ///     .fit_over(unsafe { talc.get_allocated_span(old_heap) });
    ///
    /// // truncate the heap
    /// unsafe { talc.truncate(old_heap, new_heap); }
    /// ```
    pub unsafe fn truncate(&mut self, old_heap: Span, new_heap: Span) -> Span {
        assert!(!self.bins.is_null(), "no heaps have been successfully established?");

        let new_heap = new_heap.word_align_inward();

        // check that the new_heap is valid
        assert!(old_heap.contains_span(new_heap), "the old_heap must contain new_heap!");
        assert!(
            new_heap.contains_span(unsafe { self.get_allocated_span(old_heap) }),
            "new_heap must contain all the heap's allocated memory! see `get_allocated_span`"
        );

        let (old_base, old_acme) = old_heap.get_base_acme().unwrap();
        let old_chunk_base = old_base.add(TAG_SIZE);

        // if the entire heap is decimated, just return an empty span
        if new_heap.size() < MIN_HEAP_SIZE {
            self.deregister(
                old_chunk_base.cast(),
                bin_of_size(old_acme as usize - old_chunk_base as usize),
            );

            return Span::empty();
        }

        let (new_base, new_acme) = new_heap.get_base_acme().unwrap();
        let new_chunk_base = new_base.add(TAG_SIZE);
        let mut ret_base = new_base;
        let mut ret_acme = new_acme;

        // trim the top
        if new_acme < old_acme {
            let top_size = unsafe { old_acme.sub(WORD_SIZE).cast::<usize>().read() };
            let top_chunk = FreeChunk(old_acme.wrapping_sub(top_size));

            self.deregister(top_chunk.node_ptr(), bin_of_size(top_size));

            if is_chunk_size(top_chunk.base(), new_acme) {
                self.register(top_chunk.base(), new_acme);
            } else {
                ret_acme = top_chunk.base();
                Tag::clear_above_free(top_chunk.base().sub(TAG_SIZE).cast());
            }
        }

        // no need to check if the entire heap vanished;
        // we checked against this possibility earlier

        // trim the bottom
        if old_base < new_base {
            debug_assert!(old_base.cast::<Tag>().read().is_above_free());

            let bottom_chunk = FreeChunk(old_chunk_base);
            let bottom_size = unsafe { bottom_chunk.size_ptr().read() };
            let bottom_acme = bottom_chunk.base().add(bottom_size);

            self.deregister(bottom_chunk.node_ptr(), bin_of_size(bottom_size));

            if is_chunk_size(new_chunk_base, bottom_acme) {
                self.register(new_chunk_base, bottom_acme);
                Tag::write(new_base, null_mut(), true);
            } else {
                ret_base = bottom_acme.sub(TAG_SIZE);
                Tag::write(ret_base, null_mut(), false);
            }
        }

        scan_for_errors(self);

        Span::new(ret_base, ret_acme)
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

    /// Wrap in a `Talck` without a synchronizing lock. 
    /// 
    /// **Not generally recommended.** Use [`lock`](Talc::lock) with a 
    /// spin lock instead if you're unsure.
    /// # Safety
    /// You must maintain exclusivity of access to the lock, whether via platform
    /// specific constrains, application thread usage, or some form of synchronization.
    #[cfg(feature = "lock_api")]
    pub const unsafe fn lock_assume_single_threaded(self) -> Talck<talck::AssumeUnlockable, O> {
        Talck(lock_api::Mutex::new(self))
    }
}

#[cfg(all(target_family = "wasm", feature = "lock_api"))]
pub type TalckWasm = Talck<AssumeUnlockable, WasmHandler>;

#[cfg(all(target_family = "wasm", feature = "lock_api"))]
impl TalckWasm {
    /// Create a [`Talck`] instance that takes control of WASM memory management.
    ///
    /// # Safety
    /// The runtime environment must be single-threaded WASM.
    ///
    /// These restrictions apply while the allocator is in use:
    /// - WASM memory should not manipulated unless allocated.
    /// - Talc's heap resizing functions must not be used.
    pub const unsafe fn new_global() -> Self {
        Talc::new(WasmHandler::new()).lock_assume_single_threaded()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_assumptions_hold() {
        // claim assumes this
        assert!(ALIGN == std::mem::align_of::<Bin>() && ALIGN == std::mem::size_of::<Bin>());
    }

    #[test]
    fn alloc_dealloc_test() {
        const ARENA_SIZE: usize = 10000000;

        let arena = Box::leak(vec![0u8; ARENA_SIZE].into_boxed_slice()) as *mut [_];

        let mut talc = Talc::new(ErrOnOom);

        unsafe {
            talc.claim(arena.as_mut().unwrap().into()).unwrap();
        }

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
    fn claim_truncate_extend_test() {
        // not big enough to fit the metadata
        let mut tiny_heap = [0u8; BIN_COUNT * WORD_SIZE / 2];
        let tiny_heap_span: Span = Span::from(&mut tiny_heap);

        // big enough with plenty of extra
        let big_heap = Box::leak(vec![0u8; BIN_COUNT * WORD_SIZE + 100000].into_boxed_slice());
        let big_heap_span = Span::from(big_heap.as_mut());

        let mut talc = Talc::new(ErrOnOom);

        unsafe {
            talc.claim(tiny_heap_span).unwrap_err();
        }

        assert!(talc.bins.is_null());
        assert!(talc.availability_low == 0 && talc.availability_high == 0);

        let alloc_big_heap = unsafe { talc.claim(big_heap_span).unwrap() };

        assert!(!talc.bins.is_null());

        let alloc_big_heap = unsafe {
            talc.truncate(
                alloc_big_heap,
                alloc_big_heap.truncate(500, 500).fit_over(talc.get_allocated_span(alloc_big_heap)),
            )
        };

        let _alloc_tiny_heap = unsafe { talc.claim(tiny_heap_span).unwrap() };

        let allocation = unsafe {
            let allocation = talc.malloc(Layout::new::<u128>()).unwrap();
            allocation.as_ptr().write_bytes(0, Layout::new::<u128>().size());
            allocation
        };

        let alloc_big_heap = unsafe {
            talc.truncate(
                alloc_big_heap,
                alloc_big_heap
                    .truncate(100000, 100000)
                    .fit_over(talc.get_allocated_span(alloc_big_heap)),
            )
        };

        unsafe {
            talc.extend(
                alloc_big_heap,
                alloc_big_heap.extend(10000, 10000).fit_within(big_heap_span),
            );
        }

        unsafe {
            talc.free(allocation, Layout::new::<u128>());
        }

        unsafe {
            drop(Box::from_raw(big_heap));
        }
    }
}
