mod llist;
mod tag;

#[cfg(feature = "counters")]
pub mod counters;

use crate::{ptr_utils::*, OomHandler, Span};
use core::{
    alloc::Layout,
    ptr::{null_mut, NonNull},
};
use llist::LlistNode;
use tag::Tag;

const NODE_SIZE: usize = core::mem::size_of::<LlistNode>();
const TAG_SIZE: usize = core::mem::size_of::<Tag>();

const MIN_TAG_OFFSET: usize = NODE_SIZE;
const MIN_CHUNK_SIZE: usize = MIN_TAG_OFFSET + TAG_SIZE;
const MIN_HEAP_SIZE: usize = MIN_CHUNK_SIZE + TAG_SIZE;

const BIN_COUNT: usize = usize::BITS as usize * 2;

type Bin = Option<NonNull<LlistNode>>;

// Free chunk (3x ptr size minimum):
//   ?? | NODE: LlistNode (2 * ptr), SIZE: usize, ..???.., SIZE: usize | ??
// Reserved chunk (1x ptr size of overhead):
//   ?? |       ???????         , TAG: Tag (ptr) | ??

// TAG contains a pointer to the bottom of the reserved chunk,
// a is_allocated (set) bit flag differentiating itself from a free chunk
// (the LlistNode contains well-aligned pointers, thus does not have that bit set),
// as well as a is_low_free bit flag which does what is says on the tin

const GAP_NODE_OFFSET: usize = 0;
const GAP_LOW_SIZE_OFFSET: usize = NODE_SIZE;
const GAP_HIGH_SIZE_OFFSET: usize = WORD_SIZE;

// WASM perf tanks if these #[inline]'s are not present
#[inline]
unsafe fn gap_base_to_node(base: *mut u8) -> *mut LlistNode {
    base.add(GAP_NODE_OFFSET).cast()
}
#[inline]
unsafe fn gap_base_to_size(base: *mut u8) -> *mut usize {
    base.add(GAP_LOW_SIZE_OFFSET).cast()
}
#[inline]
unsafe fn gap_base_to_acme(base: *mut u8) -> *mut u8 {
    gap_base_to_acme_size(base).0
}
#[inline]
unsafe fn gap_base_to_acme_size(base: *mut u8) -> (*mut u8, usize) {
    let size = gap_base_to_size(base).read();
    (base.add(size), size)
}
#[inline]
unsafe fn gap_acme_to_size(acme: *mut u8) -> *mut usize {
    acme.sub(GAP_HIGH_SIZE_OFFSET).cast()
}
#[inline]
unsafe fn gap_acme_to_base(acme: *mut u8) -> *mut u8 {
    gap_acme_to_base_size(acme).0
}
#[inline]
unsafe fn gap_acme_to_base_size(acme: *mut u8) -> (*mut u8, usize) {
    let size = gap_acme_to_size(acme).read();
    (acme.sub(size), size)
}
#[inline]
unsafe fn gap_node_to_base(node: NonNull<LlistNode>) -> *mut u8 {
    node.as_ptr().cast::<u8>().sub(GAP_NODE_OFFSET).cast()
}
#[inline]
unsafe fn gap_node_to_size(node: NonNull<LlistNode>) -> *mut usize {
    node.as_ptr().cast::<u8>().sub(GAP_NODE_OFFSET).add(GAP_LOW_SIZE_OFFSET).cast()
}
#[inline]
unsafe fn is_gap_below(acme: *mut u8) -> bool {
    // gap size will never have bit 1 set, but a tag will
    gap_acme_to_size(acme).read() & Tag::ALLOCATED_FLAG == 0
}
#[inline]
unsafe fn is_gap_above_heap_base(heap_base: *mut u8) -> bool {
    // there's a tag at every heap base
    heap_base.cast::<Tag>().read().is_above_free()
}

/// Determines the tag pointer and retrieves the tag, given the allocated pointer.
#[inline]
unsafe fn tag_from_alloc_ptr(ptr: *mut u8, size: usize) -> (*mut u8, Tag) {
    let post_alloc_ptr = align_up(ptr.add(size));
    // we're either reading a tag_ptr or a Tag with the base pointer + metadata in the low bits
    let tag_or_tag_ptr = post_alloc_ptr.cast::<*mut u8>().read();

    // if the pointer is greater, it's a tag_ptr
    // if it's less, it's a tag, effectively a base pointer
    // (the low bits of metadata in a tag don't effect the inequality)
    if tag_or_tag_ptr > post_alloc_ptr {
        (tag_or_tag_ptr, tag_or_tag_ptr.cast::<Tag>().read())
    } else {
        (post_alloc_ptr, Tag(tag_or_tag_ptr))
    }
}

/// Returns whether the two pointers are greater than `MIN_CHUNK_SIZE` apart.
#[inline]
fn is_chunk_size(base: *mut u8, acme: *mut u8) -> bool {
    debug_assert!(acme >= base, "!(acme {:p} >= base {:p})", acme, base);
    acme as usize - base as usize >= MIN_CHUNK_SIZE
}

/// `size` should be larger or equal to MIN_CHUNK_SIZE
#[inline]
unsafe fn bin_of_size(size: usize) -> usize {
    // this mess determines the bucketing strategy used by the allocator
    // the default is to have a bucket per multiple of word size from the minimum
    // chunk size up to WORD_BUCKETED_SIZE and double word gap (sharing two sizes)
    // up to DOUBLE_BUCKETED_SIZE, and from there on use pseudo-logarithmic sizes.

    // such sizes are as follows: begin at some power of two (DOUBLE_BUCKETED_SIZE)
    // and increase by some power of two fraction (quarters, on 64 bit machines)
    // until reaching the next power of two, and repeat:
    // e.g. begin at 32, increase by quarters: 32, 40, 48, 56, 64, 80, 96, 112, 128, ...

    // note to anyone adding support for another word size: use buckets.py to figure it out
    const ERRMSG: &str = "Unsupported system word size, open an issue/create a PR!";

    /// up to what size do we use a bin for every multiple of a word
    const WORD_BIN_LIMIT: usize = match WORD_SIZE {
        8 => 256,
        4 => 64,
        _ => panic!("{}", ERRMSG),
    };
    /// up to what size beyond that do we use a bin for every multiple of a doubleword
    const DOUBLE_BIN_LIMIT: usize = match WORD_SIZE {
        8 => 512,
        4 => 128,
        _ => panic!("{}", ERRMSG),
    };
    /// how many buckets are linearly spaced among each power of two magnitude (how many divisions)
    const DIVS_PER_POW2: usize = match WORD_SIZE {
        8 => 4,
        4 => 2,
        _ => panic!("{}", ERRMSG),
    };
    /// how many bits are used to determine the division
    const DIV_BITS: usize = DIVS_PER_POW2.ilog2() as usize;

    /// the bucket index at which the doubleword separated buckets start
    const DBL_BUCKET: usize = (WORD_BIN_LIMIT - MIN_CHUNK_SIZE) / WORD_SIZE;
    /// the bucket index at which the peudo-exponentially separated buckets start
    const EXP_BUCKET: usize = DBL_BUCKET + (DOUBLE_BIN_LIMIT - WORD_BIN_LIMIT) / WORD_SIZE / 2;
    /// Log 2 of (minimum pseudo-exponential chunk size)
    const MIN_EXP_BITS_LESS_ONE: usize = DOUBLE_BIN_LIMIT.ilog2() as usize;

    debug_assert!(size >= MIN_CHUNK_SIZE);

    if size < WORD_BIN_LIMIT {
        // single word separated bucket

        (size - MIN_CHUNK_SIZE) / WORD_SIZE
    } else if size < DOUBLE_BIN_LIMIT {
        // double word separated bucket

        // equiv to (size - WORD_BIN_LIMIT) / 2WORD_SIZE + DBL_BUCKET
        // but saves an instruction
        size / (2 * WORD_SIZE) - WORD_BIN_LIMIT / (2 * WORD_SIZE) + DBL_BUCKET
    } else {
        // pseudo-exponentially separated bucket

        // here's what a size is, bit by bit: 1_div_extra
        // e.g. with four divisions 1_01_00010011000
        // the bucket is determined by the magnitude and the division
        // mag 0 div 0, mag 0 div 1, mag 0 div 2, mag 0 div 3, mag 1 div 0, ...

        let bits_less_one = size.ilog2() as usize;

        // the magnitude the size belongs to.
        // calculate the difference in bit count i.e. difference in power
        let magnitude = bits_less_one - MIN_EXP_BITS_LESS_ONE;
        // the division of the magnitude the size belongs to.
        // slide the size to get the division bits at the bottom and remove the top bit
        let division = (size >> (bits_less_one - DIV_BITS)) - DIVS_PER_POW2;
        // the index into the pseudo-exponential buckets.
        let bucket_offset = magnitude * DIVS_PER_POW2 + division;

        // cap the max bucket at the last bucket
        (bucket_offset + EXP_BUCKET).min(BIN_COUNT - 1)
    }
}

/// The Talc Allocator!
///
/// One way to get started:
/// 1. Construct with [`new`](Talc::new) (supply [`ErrOnOom`] to ignore OOM handling).
/// 2. Establish any number of heaps with [`claim`](Talc::claim).
/// 3. Call [`lock`](Talc::lock) to get a [`Talck`] which supports the
/// [`GlobalAlloc`](core::alloc::GlobalAlloc) and [`Allocator`](core::alloc::Allocator) traits.
///
/// Check out the associated functions `new`, `claim`, `lock`, `extend`, and `truncate`.
pub struct Talc<O: OomHandler> {
    /// The low bits of the availability flags.
    availability_low: usize,
    /// The high bits of the availability flags.
    availability_high: usize,
    /// Linked list heads.
    bins: *mut Bin,

    /// The user-specified OOM handler.
    ///
    /// Its state is entirely maintained by the user.
    pub oom_handler: O,

    #[cfg(feature = "counters")]
    /// Allocation stats.
    counters: counters::Counters,
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
    #[inline]
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
    #[inline]
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

    /// Registers a gap in memory which is allocatable.
    #[inline]
    unsafe fn register_gap(&mut self, base: *mut u8, acme: *mut u8) {
        debug_assert!(is_chunk_size(base, acme));

        let size = acme as usize - base as usize;
        let bin = bin_of_size(size);

        let bin_ptr = self.get_bin_ptr(bin);

        if (*bin_ptr).is_none() {
            self.set_avails(bin);
        }

        LlistNode::insert(gap_base_to_node(base), bin_ptr, *bin_ptr);

        debug_assert!((*bin_ptr).is_some());

        gap_base_to_size(base).write(size);
        gap_acme_to_size(acme).write(size);

        #[cfg(feature = "counters")]
        self.counters.account_register_gap(size);
    }

    /// Deregisters memory, not allowing it to be allocated.
    #[inline]
    unsafe fn deregister_gap(&mut self, base: *mut u8, bin: usize) {
        debug_assert!((*self.get_bin_ptr(bin)).is_some());
        #[cfg(feature = "counters")]
        self.counters.account_deregister_gap(gap_base_to_size(base).read());

        LlistNode::remove(gap_base_to_node(base));

        if (*self.get_bin_ptr(bin)).is_none() {
            self.clear_avails(bin);
        }
    }

    /// Allocate a contiguous region of memory according to `layout`, if possible.
    /// # Safety
    /// `layout.size()` must be nonzero.
    pub unsafe fn malloc(&mut self, layout: Layout) -> Result<NonNull<u8>, ()> {
        debug_assert!(layout.size() != 0);
        self.scan_for_errors();

        let (mut free_base, free_acme, alloc_base) = loop {
            // this returns None if there are no heaps or allocatable memory
            match self.get_sufficient_chunk(layout) {
                Some(payload) => break payload,
                None => _ = O::handle_oom(self, layout)?,
            }
        };

        // determine the base of the allocated chunk
        // if the amount of memory below the chunk is too small, subsume it, else free it
        let chunk_base_ceil = alloc_base.min(free_acme.sub(MIN_CHUNK_SIZE));
        if is_chunk_size(free_base, chunk_base_ceil) {
            self.register_gap(free_base, chunk_base_ceil);
            free_base = chunk_base_ceil;
        } else {
            Tag::clear_above_free(free_base.sub(TAG_SIZE).cast());
        }

        // the word immediately after the allocation
        let post_alloc_ptr = align_up(alloc_base.add(layout.size()));
        // the tag position, accounting for the minimum size of a chunk
        let mut tag_ptr = free_base.add(MIN_TAG_OFFSET).max(post_alloc_ptr);
        // the pointer after the lowest possible tag pointer
        let min_alloc_chunk_acme = tag_ptr.add(TAG_SIZE);

        // handle the space above the required allocation span
        if is_chunk_size(min_alloc_chunk_acme, free_acme) {
            self.register_gap(min_alloc_chunk_acme, free_acme);
            Tag::write(tag_ptr.cast(), free_base, true);
        } else {
            tag_ptr = free_acme.sub(TAG_SIZE);
            Tag::write(tag_ptr.cast(), free_base, false);
        }

        if tag_ptr != post_alloc_ptr {
            // write the real tag ptr where the tag is expected to be
            post_alloc_ptr.cast::<*mut u8>().write(tag_ptr);
        }

        #[cfg(feature = "counters")]
        self.counters.account_alloc(layout.size());

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
                    let size = gap_node_to_size(node_ptr).read();

                    // if the chunk size is sufficient, remove from bookkeeping data structures and return
                    if size >= required_chunk_size {
                        let base = gap_node_to_base(node_ptr);
                        self.deregister_gap(base, bin);
                        return Some((base, base.add(size), base));
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
                    let size = gap_node_to_size(node_ptr).read();

                    if size >= required_chunk_size {
                        let base = gap_node_to_base(node_ptr);
                        let acme = base.add(size);
                        // calculate the lowest aligned pointer above the tag-offset free chunk pointer
                        let aligned_ptr = align_up_by(base, align_mask);

                        // if the remaining size is sufficient, remove the chunk from the books and return
                        if aligned_ptr.add(required_size) <= acme {
                            self.deregister_gap(base, bin);
                            return Some((base, acme, aligned_ptr));
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
        self.scan_for_errors();
        #[cfg(feature = "counters")]
        self.counters.account_dealloc(layout.size());

        let (tag_ptr, tag) = tag_from_alloc_ptr(ptr.as_ptr(), layout.size());
        let mut chunk_base = tag.chunk_base();
        let mut chunk_acme = tag_ptr.add(TAG_SIZE);

        debug_assert!(tag.is_allocated());
        debug_assert!(is_chunk_size(chunk_base, chunk_acme));

        // try recombine below
        if is_gap_below(chunk_base) {
            let (below_base, below_size) = gap_acme_to_base_size(chunk_base);
            self.deregister_gap(below_base, bin_of_size(below_size));

            chunk_base = below_base;
        } else {
            Tag::set_above_free(chunk_base.sub(TAG_SIZE).cast());
        }

        // try recombine above
        if tag.is_above_free() {
            let above_size = gap_base_to_size(chunk_acme).read();
            self.deregister_gap(chunk_acme, bin_of_size(above_size));

            chunk_acme = chunk_acme.add(above_size);
        }

        // add the full recombined free chunk back into the books
        self.register_gap(chunk_base, chunk_acme);
    }

    /// Grow a previously allocated/reallocated region of memory to `new_size`.
    /// # Safety
    /// `ptr` must have been previously allocated or reallocated given `layout`.
    /// `new_size` must be larger or equal to `layout.size()`.
    pub unsafe fn grow(
        &mut self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_size: usize,
    ) -> Result<NonNull<u8>, ()> {

        match self.grow_in_place(ptr, old_layout, new_size) {
            Err(_) => {
                // grow in-place failed; reallocate the slow way
                let new_layout = Layout::from_size_align_unchecked(new_size, old_layout.align());
                let allocation = self.malloc(new_layout)?;
                allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), old_layout.size());
                self.free(ptr, old_layout);
    
                Ok(allocation)
            }
            res => res,
        }
    }

    /// Attempt to grow a previously allocated/reallocated region of memory to `new_size`.
    /// 
    /// Returns `Err` if reallocation could not occur in-place. 
    /// Ownership of the memory remains with the caller.
    /// # Safety
    /// `ptr` must have been previously allocated or reallocated given `layout`.
    /// `new_size` must be larger or equal to `layout.size()`.
    pub unsafe fn grow_in_place(
        &mut self,
        ptr: NonNull<u8>,
        old_layout: Layout,
        new_size: usize,
    ) -> Result<NonNull<u8>, ()> {
        debug_assert!(new_size >= old_layout.size());
        self.scan_for_errors();

        let old_post_alloc_ptr = align_up(ptr.as_ptr().add(old_layout.size()));
        let new_post_alloc_ptr = align_up(ptr.as_ptr().add(new_size));

        if old_post_alloc_ptr == new_post_alloc_ptr {
            // this handles a rare short-circuit, but more helpfully
            // also guarantees that we'll never need to add padding to
            // reach minimum chunk size with new_tag_ptr later as
            // min alloc size (1) rounded up to (WORD) + post_alloc_ptr (WORD) + new_tag_ptr (WORD) >= MIN_CHUNK_SIZE

            #[cfg(feature = "counters")]
            self.counters.account_grow_in_place(old_layout.size(), new_size);

            return Ok(ptr);
        }

        let (tag_ptr, tag) = tag_from_alloc_ptr(ptr.as_ptr(), old_layout.size());

        // tag_ptr may be greater where extra free space needed to be reserved
        if new_post_alloc_ptr <= tag_ptr {
            if new_post_alloc_ptr < tag_ptr {
                new_post_alloc_ptr.cast::<*mut u8>().write(tag_ptr);
            }

            #[cfg(feature = "counters")]
            self.counters.account_grow_in_place(old_layout.size(), new_size);

            return Ok(ptr);
        }

        let new_tag_ptr = new_post_alloc_ptr;

        let base = tag.chunk_base();
        let acme = tag_ptr.add(TAG_SIZE);

        debug_assert!(tag.is_allocated());
        debug_assert!(is_chunk_size(base, acme));

        // otherwise, check if 1) is free 2) is large enough
        // because free chunks don't border free chunks, this needn't be recursive
        if tag.is_above_free() {
            let above_size = gap_base_to_size(acme).read();
            let above_tag_ptr = tag_ptr.add(above_size);

            if new_tag_ptr <= above_tag_ptr {
                self.deregister_gap(acme, bin_of_size(above_size));

                // finally, determine if the remainder of the free block is big enough
                // to be freed again, or if the entire region should be allocated
                if is_chunk_size(new_tag_ptr, above_tag_ptr) {
                    self.register_gap(new_tag_ptr.add(TAG_SIZE), above_tag_ptr.add(TAG_SIZE));
                    Tag::write(new_tag_ptr.cast(), base, true);
                } else {
                    Tag::write(above_tag_ptr.cast(), base, false);

                    if new_post_alloc_ptr != above_tag_ptr {
                        new_post_alloc_ptr.cast::<*mut u8>().write(above_tag_ptr);
                    }
                }

                #[cfg(feature = "counters")]
                self.counters.account_grow_in_place(old_layout.size(), new_size);

                return Ok(ptr);
            }
        }

        Err(())
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
        self.scan_for_errors();

        let (tag_ptr, tag) = tag_from_alloc_ptr(ptr.as_ptr(), layout.size());
        let chunk_base = tag.chunk_base();

        debug_assert!(tag.is_allocated());
        debug_assert!(is_chunk_size(chunk_base, tag_ptr.add(TAG_SIZE)));

        // the word immediately after the allocation
        let new_post_alloc_ptr = align_up(ptr.as_ptr().add(new_size));
        // the tag position, accounting for the minimum size of a chunk
        let mut new_tag_ptr = chunk_base.add(MIN_TAG_OFFSET).max(new_post_alloc_ptr);

        // if the remainder between the new required size and the originally allocated
        // size is large enough, free the remainder, otherwise leave it
        if is_chunk_size(new_tag_ptr, tag_ptr) {
            let mut acme = tag_ptr.add(TAG_SIZE);
            let new_acme = new_tag_ptr.add(TAG_SIZE);

            if tag.is_above_free() {
                let above_size = gap_base_to_size(acme).read();
                self.deregister_gap(acme, bin_of_size(above_size));

                acme = acme.add(above_size);
            }

            self.register_gap(new_acme, acme);
            Tag::write(new_tag_ptr.cast(), chunk_base, true);
        } else {
            new_tag_ptr = tag_ptr;
        }

        if new_tag_ptr != new_post_alloc_ptr {
            new_post_alloc_ptr.cast::<*mut u8>().write(new_tag_ptr);
        }

        #[cfg(feature = "counters")]
        self.counters.account_shrink_in_place(layout.size(), new_size);
    }

    /// Returns an uninitialized [`Talc`].
    ///
    /// If you don't want to handle OOM, use [`ErrOnOom`].
    ///
    /// In order to make this allocator useful, `claim` some memory.
    pub const fn new(oom_handler: O) -> Self {
        Self {
            oom_handler,
            availability_low: 0,
            availability_high: 0,
            bins: null_mut(),

            #[cfg(feature = "counters")]
            counters: counters::Counters::new(),
        }
    }

    /// Returns the minimum [`Span`] containing this heap's allocated memory.
    /// # Safety
    /// `heap` must be the return value of a heap manipulation function.
    pub unsafe fn get_allocated_span(&self, heap: Span) -> Span {
        assert!(heap.size() >= MIN_HEAP_SIZE);

        let (mut base, mut acme) = heap.get_base_acme().unwrap();

        // check for free space at the heap's top
        if is_gap_below(acme) {
            acme = gap_acme_to_base(acme);
        }

        // check for free memory at the bottom of the heap using the base tag
        if is_gap_above_heap_base(base) {
            base = gap_base_to_acme(base.add(TAG_SIZE)).sub(TAG_SIZE);
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
    /// and memory therein (when not allocated to the user) must not be mutated
    /// while the allocator is in use.
    /// - `memory` should not overlap with any other active heap.
    ///
    /// # Panics
    /// Panics if `memory` contains the null address.
    pub unsafe fn claim(&mut self, memory: Span) -> Result<Span, ()> {
        self.scan_for_errors();

        const BIN_ARRAY_SIZE: usize = core::mem::size_of::<Bin>() * BIN_COUNT;

        // create a new heap
        // if bins is null, we will need to try put the metadata in this heap
        // this metadata is allocated 'by hand' to be isomorphic with other chunks

        assert!(!memory.contains(null_mut()), "heap covers the null address!");

        let aligned_heap = memory.word_align_inward();

        // if this fails, there's no space to work with
        if let Some((base, acme)) = aligned_heap.get_base_acme() {
            // check if the allocator has already successfully placed its metadata
            if !self.bins.is_null() {
                // check if there's enough space to establish a free chunk
                if acme as usize - base as usize >= MIN_HEAP_SIZE {
                    // write in the base tag
                    Tag::write(base.cast(), null_mut(), true);

                    // register the free memory
                    let chunk_base = base.wrapping_add(TAG_SIZE);
                    self.register_gap(chunk_base, acme);
                    
                    self.scan_for_errors();

                    #[cfg(feature = "counters")]
                    self.counters.account_claim(aligned_heap.size());

                    return Ok(aligned_heap);
                }
            } else {
                // check if there's enough space to allocate metadata and establish a free chunk
                if acme as usize - base as usize >= TAG_SIZE + BIN_ARRAY_SIZE + TAG_SIZE {
                    Tag::write(base.cast(), null_mut(), false);

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
                        self.register_gap(metadata_chunk_acme, acme);
                        Tag::write(post_metadata_ptr.cast(), base, true);
                    } else {
                        let tag_ptr = acme.sub(TAG_SIZE).cast::<Tag>();

                        if tag_ptr != post_metadata_ptr.cast() {
                            post_metadata_ptr.cast::<*mut Tag>().write(tag_ptr);
                        }
                        Tag::write(tag_ptr, base, false);
                    }

                    self.scan_for_errors();

                    #[cfg(feature = "counters")]
                    self.counters.account_claim(aligned_heap.size());

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
    /// - The entire `req_heap` memory but be readable and writable
    /// and unmutated besides that which is allocated so long as the heap is in use.
    ///
    /// # Panics
    /// This function panics if:
    /// - `old_heap` is too small or heap metadata is not yet allocated
    /// - `req_heap` doesn't contain `old_heap`
    /// - `req_heap` contains the null address
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
    pub unsafe fn extend(&mut self, old_heap: Span, req_heap: Span) -> Span {
        assert!(!self.bins.is_null());
        assert!(old_heap.size() >= MIN_HEAP_SIZE);
        assert!(req_heap.contains_span(old_heap), "new_heap must contain old_heap");
        assert!(!req_heap.contains(null_mut()), "new_heap covers the null address!");

        self.scan_for_errors();

        let (old_base, old_acme) = old_heap.word_align_inward().get_base_acme().unwrap();
        let (new_base, new_acme) = req_heap.word_align_inward().get_base_acme().unwrap();
        let new_chunk_base = new_base.add(TAG_SIZE);
        let mut ret_base = new_base;
        let mut ret_acme = new_acme;

        // if the top chunk is free, extend the block to cover the new extra area
        // otherwise allocate above if possible
        if is_gap_below(old_acme) {
            let (top_base, top_size) = gap_acme_to_base_size(old_acme);
            self.deregister_gap(top_base, bin_of_size(top_size));
            self.register_gap(top_base, new_acme);
        } else if is_chunk_size(old_acme, new_acme) {
            self.register_gap(old_acme, new_acme);
            Tag::set_above_free(old_acme.sub(TAG_SIZE).cast());
        } else {
            ret_acme = old_acme;
        }

        // extend the bottom chunk if it's free, else add free chunk below if possible
        if is_gap_above_heap_base(old_base) {
            let bottom_base = old_base.add(TAG_SIZE);
            let bottom_size = gap_base_to_size(bottom_base).read();
            self.deregister_gap(bottom_base, bin_of_size(bottom_size));
            self.register_gap(new_chunk_base, bottom_base.add(bottom_size));
            Tag::write(new_base.cast(), null_mut(), true);
        } else if is_chunk_size(new_base, old_base) {
            self.register_gap(new_base.add(TAG_SIZE), old_base.add(TAG_SIZE));
            Tag::write(new_base.cast(), null_mut(), true);
        } else {
            ret_base = old_base;
        }

        let ret_heap = Span::new(ret_base, ret_acme);

        #[cfg(feature = "counters")]
        self.counters.account_extend(old_heap.size(), ret_heap.size());

        ret_heap
    }

    /// Reduce the extent of a heap.
    /// The new extent must encompass all current allocations. See below.
    ///
    /// The resultant heap is always equal to or slightly smaller than `req_heap`.
    ///
    /// Truncating to an empty [`Span`] is valid for heaps where no memory is
    /// currently allocated within it.
    ///
    /// In all cases where the return value is empty, the heap no longer exists.
    /// You may do what you like with the heap memory. The empty span should not be
    /// used as input to [`truncate`](Talc::truncate), [`extend`](Talc::extend),
    /// or [`get_allocated_span`](Talc::get_allocated_span).
    ///
    /// # Safety
    /// `old_heap` must be the return value of a heap-manipulation function
    /// of this allocator instance.
    ///
    /// # Panics:
    /// This function panics if:
    /// - `old_heap` doesn't contain `req_heap`
    /// - `req_heap` doesn't contain all the allocated memory in `old_heap`
    /// - the heap metadata is not yet allocated, see [`claim`](Talc::claim)
    ///
    /// # Usage
    ///
    /// A recommended pattern for satisfying these criteria is:
    /// ```rust
    /// # use talc::*;
    /// # let mut talc = Talc::new(ErrOnOom);
    /// let mut heap = [0u8; 2000];
    /// let old_heap = Span::from(&mut heap[300..1700]);
    /// let old_heap = unsafe { talc.claim(old_heap).unwrap() };
    ///
    /// // note: lock a `Talck` here otherwise a race condition may occur
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
    pub unsafe fn truncate(&mut self, old_heap: Span, req_heap: Span) -> Span {
        assert!(!self.bins.is_null(), "no heaps have been successfully established!");

        self.scan_for_errors();

        let new_heap = req_heap.word_align_inward();

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
            self.deregister_gap(
                old_chunk_base,
                bin_of_size(old_acme as usize - old_chunk_base as usize),
            );

            #[cfg(feature = "counters")]
            self.counters.account_truncate(old_heap.size(), 0);

            return Span::empty();
        }

        let (new_base, new_acme) = new_heap.get_base_acme().unwrap();
        let new_chunk_base = new_base.add(TAG_SIZE);
        let mut ret_base = new_base;
        let mut ret_acme = new_acme;

        // trim the top
        if new_acme < old_acme {
            let (top_base, top_size) = gap_acme_to_base_size(old_acme);
            self.deregister_gap(top_base, bin_of_size(top_size));

            if is_chunk_size(top_base, new_acme) {
                self.register_gap(top_base, new_acme);
            } else {
                ret_acme = top_base;
                Tag::clear_above_free(top_base.sub(TAG_SIZE).cast());
            }
        }

        // no need to check if the entire heap vanished;
        // we eliminated this possibility earlier

        // trim the bottom
        if old_base < new_base {
            debug_assert!(is_gap_above_heap_base(old_base));

            let (bottom_acme, bottom_size) = gap_base_to_acme_size(old_chunk_base);
            self.deregister_gap(old_chunk_base, bin_of_size(bottom_size));

            if is_chunk_size(new_chunk_base, bottom_acme) {
                self.register_gap(new_chunk_base, bottom_acme);
                Tag::write(new_base.cast(), null_mut(), true);
            } else {
                ret_base = bottom_acme.sub(TAG_SIZE);
                Tag::write(ret_base.cast(), null_mut(), false);
            }
        }

        let ret_heap = Span::new(ret_base, ret_acme);

        #[cfg(feature = "counters")]
        self.counters.account_truncate(old_heap.size(), ret_heap.size());

        ret_heap
    }

    #[cfg(not(debug_assertions))]
    fn scan_for_errors(&self) {}

    #[cfg(debug_assertions)]
    /// Debugging function for checking various assumptions.
    fn scan_for_errors(&self) {
        #[cfg(any(test, fuzzing))]
        let mut vec = std::vec::Vec::<Span>::new();

        if !self.bins.is_null() {
            for b in 0..BIN_COUNT {
                let mut any = false;
                unsafe {
                    for node in LlistNode::iter_mut(*self.get_bin_ptr(b)) {
                        any = true;
                        if b < WORD_BITS {
                            assert!(self.availability_low & 1 << b != 0);
                        } else {
                            assert!(self.availability_high & 1 << (b - WORD_BITS) != 0);
                        }

                        let base = gap_node_to_base(node);
                        let (acme, size) = gap_base_to_acme_size(base);
                        let low_size = gap_acme_to_size(acme).read();
                        assert!(low_size == size);

                        let lower_tag = base.sub(TAG_SIZE).cast::<Tag>().read();
                        assert!(lower_tag.is_allocated());
                        assert!(lower_tag.is_above_free());

                        #[cfg(any(test, fuzzing))]
                        {
                            let span = Span::new(base, acme);
                            //dbg!(span);
                            for other in &vec {
                                assert!(!span.overlaps(*other), "{} intersects {}", span, other);
                            }
                            vec.push(span);
                        }
                    }
                }

                if !any {
                    if b < WORD_BITS {
                        assert!(self.availability_low & 1 << b == 0);
                    } else {
                        assert!(self.availability_high & 1 << (b - WORD_BITS) == 0);
                    }
                }
            }
        } else {
            assert!(self.availability_low == 0);
            assert!(self.availability_high == 0);
        }
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

        let mut talc = Talc::new(crate::ErrOnOom);

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

        let mut talc = Talc::new(crate::ErrOnOom);

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
