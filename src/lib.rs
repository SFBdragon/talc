#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), no_std)]
#![feature(ptr_sub_ptr)]
#![feature(pointer_is_aligned)]
#![feature(offset_of)]
#![feature(alloc_layout_extra)]
#![feature(slice_ptr_get)]
#![feature(core_intrinsics)]
#![feature(const_mut_refs)]
#![feature(slice_ptr_len)]
#![feature(const_slice_from_raw_parts_mut)]
#![cfg_attr(feature = "allocator", feature(allocator_api))]
#![feature(maybe_uninit_uninit_array)]

#[cfg(feature = "spin")]
mod talck;

mod llist;
mod span;
mod tag;

#[cfg(feature = "spin")]
pub use talck::Talck;
#[cfg(all(feature = "spin", feature = "allocator"))]
pub use talck::TalckRef;

use llist::LlistNode;
pub use span::Span;
use tag::Tag;

use core::{alloc::Layout, ptr::NonNull};

// desciptive error for failures
// borrow allocator_api's if available, else define our own
#[cfg(feature = "allocator")]
pub use core::alloc::AllocError;

#[cfg(not(feature = "allocator"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocError;

#[cfg(not(feature = "allocator"))]
impl core::fmt::Display for AllocError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("memory allocation failed")
    }
}

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

/// `size` should be larger or equal to MIN_CHUNK_SIZE
#[inline]
unsafe fn bucket_of_size(size: usize) -> usize {
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

fn low_aligned_fit(ptr: *mut u8, align_mask: usize) -> *mut u8 {
    ((ptr as usize + align_mask) & !align_mask) as *mut u8
}

fn align_down(ptr: *mut u8) -> *mut u8 {
    (ptr as usize & !(ALIGN - 1)) as *mut u8
}
fn align_up(ptr: *mut u8) -> *mut u8 {
    ((ptr as usize + (ALIGN - 1)) & !(ALIGN - 1)) as *mut u8
}

/// Returns whether the two pointers are greater than `MIN_CHUNK_SIZE` apart.
fn ge_min_size_apart(ptr: *mut u8, acme: *mut u8) -> bool {
    debug_assert!(acme >= ptr, "!(acme {:p} > ptr {:p})", acme, ptr);
    acme as isize - ptr as isize >= MIN_CHUNK_SIZE as isize
}

/// Determines the chunk pointer and retrieves the tag, given the allocated pointer.
#[inline]
unsafe fn chunk_ptr_from_alloc_ptr(ptr: *mut u8) -> (*mut u8, Tag) {
    #[derive(Clone, Copy)]
    union PreAllocationData {
        tag: Tag,
        ptr: *mut Tag,
    }

    let mut low_ptr = ((ptr as usize - TAG_SIZE) & !(ALIGN - 1)) as *mut u8;

    let data = *low_ptr.cast::<PreAllocationData>();

    // if the chunk_ptr doesn't point to an allocated tag
    // it points to a pointer to the actual tag
    let tag = if !data.tag.is_allocated() {
        low_ptr = data.ptr.cast();
        *data.ptr
    } else {
        data.tag
    };

    (low_ptr, tag)
}

/// Pointer wrapper to a free chunk. Provides convenience methods
/// for getting the LlistNode pointer and lower pointer to its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
struct FreeChunk(*mut u8);

impl FreeChunk {
    const NODE_OFFSET: usize = 0;
    const SIZE_OFFSET: usize = NODE_SIZE;

    fn ptr(self) -> *mut u8 {
        self.0
    }

    fn node_ptr(self) -> *mut LlistNode {
        unsafe { self.0.add(Self::NODE_OFFSET).cast() }
    }

    fn size_ptr(self) -> *mut usize {
        unsafe { self.0.add(Self::SIZE_OFFSET).cast() }
    }
}

/// An abstraction over the unknown state of the chunk above.
enum HighChunk {
    Free(FreeChunk),
    Full(*mut Tag),
}

impl HighChunk {
    #[inline]
    unsafe fn from_ptr(ptr: *mut u8) -> Self {
        if *ptr.cast::<usize>() & Tag::ALLOCATED_FLAG != 0 {
            Self::Full(ptr.cast())
        } else {
            Self::Free(FreeChunk(ptr))
        }
    }
}

type OomHandler = fn(&mut Talc, Layout) -> Result<(), AllocError>;

pub fn alloc_error(_: &mut Talc, _: Layout) -> Result<(), AllocError> {
    Err(AllocError)
}

/// The Talc Allocator!
///
/// Call `spin_lock` on the struct before initialization to get 
/// a `Talck` which supports the `GlobalAlloc` and `Allocator` traits.
pub struct Talc {
    oom_handler: OomHandler,

    arena: Span,

    alloc_base: *mut u8,
    alloc_acme: *mut u8,

    is_top_free: bool,

    /// The low bits of the availability flags.
    availability_low: usize,
    /// The high bits of the availability flags.
    availability_high: usize,

    /// Linked list buckets.
    ///
    /// # Safety
    /// This field is not referenced and modified with respect to Rust's aliasing rules.
    /// This can result in undefined behaviour (resulting in real bugs, trust me).
    ///
    /// Therefore, do not read directly, instead use `read_llist` and `get_llist_ptr`.
    llists: [Option<NonNull<LlistNode>>; BIN_COUNT],
}

unsafe impl Send for Talc {}

impl core::fmt::Debug for Talc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Talc")
            .field("arena", &self.arena)
            .field("alloc_base", &self.alloc_base)
            .field("alloc_acme", &self.alloc_acme)
            .field("is_top_free", &self.is_top_free)
            .field("availability_low", &format_args!("{:x}", self.availability_low))
            .field("availability_high", &format_args!("{:x}", self.availability_high))
            .finish()
    }
}

impl Talc {
    /// # Safety:
    /// - Do not dereference the pointer. Use `read_llist` instead.
    /// - `b` must be lower than `BUCKET_COUNT`
    unsafe fn get_llist_ptr(&mut self, b: usize) -> *mut Option<NonNull<LlistNode>> {
        debug_assert!(b < BIN_COUNT);

        self.llists.as_mut_ptr().add(b)
    }

    /// Safely read from `llists`.
    /// # Safety:
    /// `b` must be lower than `BUCKET_COUNT`
    unsafe fn read_llist(&mut self, b: usize) -> Option<NonNull<LlistNode>> {
        // read volatile gets around the issue with violating Rust's aliasing rules
        // by preventing the compiler from eliding or messing with reads to the
        // linked list pointers. For example, Rust will not realize that removing
        // a node in the linked list might modify the llists while we hold an
        // &mut to the struct, and you get weird behaviour due to optimizations.
        // see llists's docs on safety for some more info.
        self.get_llist_ptr(b).read_volatile()
    }

    const fn required_chunk_size(size: usize) -> usize {
        if size <= MIN_CHUNK_SIZE - TAG_SIZE {
            MIN_CHUNK_SIZE
        } else {
            (size + TAG_SIZE + (ALIGN - 1)) & !(ALIGN - 1)
        }
    }

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

    #[inline]
    unsafe fn add_chunk_to_record(&mut self, base: *mut u8, acme: *mut u8) {
        debug_assert!(ge_min_size_apart(base, acme));
        let size = acme.sub_ptr(base);

        let b = bucket_of_size(size);
        let free_chunk = FreeChunk(base);

        if self.read_llist(b).is_none() {
            self.set_avails(b);
        }

        LlistNode::insert(free_chunk.node_ptr(), self.get_llist_ptr(b), self.read_llist(b));

        debug_assert!(self.read_llist(b).is_some());

        // write in low size tag above the node pointers
        *free_chunk.size_ptr() = size;
        // write in high size tag at the end of the free chunk
        *acme.cast::<usize>().sub(1) = size;
    }

    #[inline]
    unsafe fn remove_chunk_from_record(&mut self, node_ptr: *mut LlistNode, b: usize) {
        debug_assert!(self.read_llist(b).is_some());

        LlistNode::remove(node_ptr);

        if self.read_llist(b).is_none() {
            self.clear_avails(b);
        }
    }

    /// Allocate a contiguous region of memory according to `layout`, if possible.
    /// # Safety
    /// `layout.size()` must be nonzero.
    pub unsafe fn malloc(&mut self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        debug_assert!(layout.size() != 0);

        // no checks for initialization are performed, as it would be overhead.
        // this will return None here as the availability flags are initialized
        // to zero; all clear; no memory to allocate, call the OOM handler.
        let (free_chunk_ptr, free_chunk_acme, alloc_ptr) = loop {
            match self.get_sufficient_chunk(layout) {
                Some(payload) => break payload,
                None => (self.oom_handler)(self, layout)?,
            }
        };

        let pre_alloc_ptr = align_down(alloc_ptr.sub(TAG_SIZE));
        let mut tag_ptr = free_chunk_acme.sub(MIN_CHUNK_SIZE).min(pre_alloc_ptr);

        let mut is_low_free = false;
        if ge_min_size_apart(free_chunk_ptr, tag_ptr) {
            // add free block below the allocation
            self.add_chunk_to_record(free_chunk_ptr, tag_ptr);
            is_low_free = true;
        } else {
            tag_ptr = free_chunk_ptr;
        }

        if tag_ptr != pre_alloc_ptr {
            // write the real tag ptr where the tag is expected to be
            *pre_alloc_ptr.cast::<*mut u8>() = tag_ptr;
        }

        // choose the highest between...
        let req_acme = core::cmp::max(
            // the required chunk acme due to the allocation
            align_up(alloc_ptr.add(layout.size())),
            // the required chunk acme due to the minimum chunk size
            tag_ptr.add(MIN_CHUNK_SIZE),
        );

        if ge_min_size_apart(req_acme, free_chunk_acme) {
            // add free block above the allocation
            self.add_chunk_to_record(req_acme, free_chunk_acme);

            *tag_ptr.cast() = Tag::new(req_acme, is_low_free);
        } else {
            if free_chunk_acme != self.alloc_acme {
                Tag::clear_low_free(free_chunk_acme.cast());
            } else {
                debug_assert!(self.is_top_free);
                self.is_top_free = false;
            }

            *tag_ptr.cast() = Tag::new(free_chunk_acme, is_low_free);
        }

        self.scan_for_errors();
        Ok(NonNull::new_unchecked(alloc_ptr))
    }

    /// Returns `(chunk_ptr, chunk_size, alloc_ptr)`
    unsafe fn get_sufficient_chunk(
        &mut self,
        layout: Layout,
    ) -> Option<(*mut u8, *mut u8, *mut u8)> {
        let req_chunk_size = Self::required_chunk_size(layout.size());

        // we need to cast to isize to allow this to wrap to -1
        // and have comparisons (within larger_nonempty_bucket) work right
        // this will immediately correct after the first call to larger_nonempty_bucket
        let mut b = bucket_of_size(req_chunk_size) as isize - 1;

        if layout.align() <= ALIGN {
            // the required alignment is most often the machine word size (or less)
            // a faster loop without alignment checking is used in this case
            loop {
                b = self.larger_nonempty_bucket(b)?;

                for node_ptr in LlistNode::iter_mut(self.read_llist(b as usize)) {
                    let free_chunk = FreeChunk(node_ptr.as_ptr().cast());
                    let chunk_size = *free_chunk.size_ptr();

                    // if the chunk size is sufficient, remove from bookkeeping data structures and return
                    if chunk_size >= req_chunk_size {
                        self.remove_chunk_from_record(free_chunk.node_ptr(), b as usize);

                        return Some((
                            free_chunk.ptr(),
                            free_chunk.ptr().add(chunk_size),
                            free_chunk.ptr().add(TAG_SIZE),
                        ));
                    }
                }
            }
        } else {
            // a larger than word-size alignement is demanded
            // therefore each chunk is manually checked to be sufficient accordingly
            let align_mask = layout.align() - 1;

            loop {
                b = self.larger_nonempty_bucket(b)?;

                for node_ptr in LlistNode::iter_mut(self.read_llist(b as usize)) {
                    let free_chunk = FreeChunk(node_ptr.as_ptr().cast());
                    let chunk_size = *free_chunk.size_ptr();

                    if chunk_size >= req_chunk_size {
                        // calculate the lowest aligned pointer above the tag-offset free chunk pointer
                        let aligned_ptr =
                            low_aligned_fit(free_chunk.ptr().add(TAG_SIZE), align_mask);
                        let chunk_acme = free_chunk.ptr().add(chunk_size);

                        // if the remaining size is sufficient, remove the chunk from the books and return
                        if aligned_ptr.add(layout.size()) <= chunk_acme {
                            self.remove_chunk_from_record(free_chunk.node_ptr(), b as usize);
                            return Some((free_chunk.ptr(), chunk_acme, aligned_ptr));
                        }
                    }
                }
            }
        }
    }

    #[inline(always)]
    fn larger_nonempty_bucket(&self, mut b: isize) -> Option<isize> {
        // if b == 63, the next up are the high flags,
        // so only worry about the low flags for b < 63
        if b < 63 {
            // shift flags such that only flags for larger buckets are kept
            let shifted_avails = self.availability_low >> (b + 1);

            // find the next up, grab from the high flags, or quit
            if shifted_avails != 0 {
                b += 1 + shifted_avails.trailing_zeros() as isize;
            } else if self.availability_high != 0 {
                b = self.availability_high.trailing_zeros() as isize + 64;
            } else {
                return None;
            }
        } else {
            // similar process to the above, but the low flags are irrelevant
            let shifted_avails = self.availability_high >> (b - 63);

            if shifted_avails != 0 {
                b += 1 + shifted_avails.trailing_zeros() as isize;
            } else {
                return None;
            }
        }

        Some(b)
    }

    /// Free previously allocated/reallocated memory.
    /// # Safety
    /// `ptr` must have been previously allocated given `layout`.
    pub unsafe fn free(&mut self, ptr: NonNull<u8>, _: Layout) {
        let (mut chunk_ptr, tag) = chunk_ptr_from_alloc_ptr(ptr.as_ptr());
        let mut chunk_acme = tag.acme_ptr();

        debug_assert!(tag.is_allocated());
        debug_assert!(ge_min_size_apart(chunk_ptr, chunk_acme));

        if chunk_acme != self.alloc_acme {
            // a higher check exists, handle the freee and non-free cases
            match HighChunk::from_ptr(chunk_acme) {
                // if taken, just set the flag for the low chunk
                HighChunk::Full(tag_ptr) => Tag::set_low_free(tag_ptr),

                // if free, recombine the freed chunk and the high free chunk
                HighChunk::Free(high_chunk) => {
                    // get the size, remove the high free chunk from the books, widen the deallotation
                    let high_chunk_size = *high_chunk.size_ptr();
                    self.remove_chunk_from_record(
                        high_chunk.node_ptr(),
                        bucket_of_size(high_chunk_size),
                    );
                    chunk_acme = chunk_acme.add(high_chunk_size);
                }
            }
        } else {
            debug_assert!(!self.is_top_free);
            self.is_top_free = true;
        }

        if tag.is_low_free() {
            // low tag is free; recombine
            // grab the size off the top of the block first, then remove at the base
            let low_chunk_size = *chunk_ptr.cast::<usize>().sub(1);
            chunk_ptr = chunk_ptr.sub(low_chunk_size);

            self.remove_chunk_from_record(
                FreeChunk(chunk_ptr).node_ptr(),
                bucket_of_size(low_chunk_size),
            );
        }

        // add the full recombined free chunk back into the books
        self.add_chunk_to_record(chunk_ptr, chunk_acme);

        self.scan_for_errors();
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
    ) -> Result<NonNull<u8>, AllocError> {
        debug_assert!(new_size >= layout.size());

        let (chunk_ptr, tag) = chunk_ptr_from_alloc_ptr(ptr.as_ptr());
        let chunk_acme = tag.acme_ptr();

        debug_assert!(tag.is_allocated());
        debug_assert!(ge_min_size_apart(chunk_ptr, chunk_acme));

        // choose the highest between...
        let new_req_acme = core::cmp::max(
            // the required chunk acme due to the allocation
            align_up(ptr.as_ptr().add(new_size)),
            // the required chunk acme due to the minimum chunk size
            chunk_ptr.add(MIN_CHUNK_SIZE),
        );

        // short-circuit if the chunk is already large enough
        if new_req_acme <= chunk_acme {
            return Ok(ptr);
        }

        // otherwise, check if the chunk above 1) exists 2) is free 3) is large enough
        // because free chunks don't border free chunks, this needn't be recursive
        if chunk_acme != self.alloc_acme {
            // given there is a chunk above, is it free?
            if !(*chunk_acme.cast::<Tag>()).is_allocated() {
                let free_chunk = FreeChunk(chunk_acme);
                let high_chunk_size = *free_chunk.size_ptr();
                let high_chunk_acme = chunk_acme.add(high_chunk_size);

                // is the additional memeory sufficient?
                if high_chunk_acme >= new_req_acme {
                    self.remove_chunk_from_record(
                        free_chunk.node_ptr(),
                        bucket_of_size(high_chunk_size),
                    );

                    // finally, determine if the remainder of the free block is big enough
                    // to be freed again, or if the entire region should be allocated
                    if ge_min_size_apart(new_req_acme, high_chunk_acme) {
                        self.add_chunk_to_record(new_req_acme, high_chunk_acme);

                        Tag::set_acme(chunk_ptr.cast(), new_req_acme);
                    } else {
                        if high_chunk_acme != self.alloc_acme {
                            Tag::clear_low_free(high_chunk_acme.cast());
                        } else {
                            debug_assert!(self.is_top_free);
                            self.is_top_free = false;
                        }

                        Tag::set_acme(chunk_ptr.cast(), high_chunk_acme);
                    }

                    self.scan_for_errors();
                    return Ok(ptr);
                }
            }
        }

        // grow in-place failed; reallocate the slow way

        self.scan_for_errors();
        let allocation =
            self.malloc(Layout::from_size_align_unchecked(new_size, layout.align()))?;
        allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), layout.size());
        self.free(ptr, layout);
        self.scan_for_errors();
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
        debug_assert!(ge_min_size_apart(chunk_ptr, chunk_acme));

        // choose the highest between...
        let new_req_acme = core::cmp::max(
            // the required chunk acme due to the allocation
            align_up(ptr.as_ptr().add(layout.size())),
            // the required chunk acme due to the minimum chunk size
            chunk_ptr.add(MIN_CHUNK_SIZE),
        );

        // if the remainder between the new required size and the originally allocated
        // size is large enough, free the remainder, otherwise leave it
        if ge_min_size_apart(new_req_acme, chunk_acme) {
            // check if there's a chunk above, whether its taken or not, and
            // modify the taken is_low_free flag/recombine the free block
            if chunk_acme != self.alloc_acme {
                match HighChunk::from_ptr(chunk_acme) {
                    HighChunk::Full(tag_ptr) => Tag::set_low_free(tag_ptr),
                    HighChunk::Free(free_chunk) => {
                        let free_chunk_size = *free_chunk.size_ptr();
                        chunk_acme = free_chunk.ptr().add(free_chunk_size);
                        self.remove_chunk_from_record(
                            free_chunk.node_ptr(),
                            bucket_of_size(free_chunk_size),
                        );
                    }
                }
            } else {
                debug_assert!(!self.is_top_free);
                self.is_top_free = true;
            }

            self.add_chunk_to_record(new_req_acme, chunk_acme);

            Tag::set_acme(chunk_ptr.cast(), new_req_acme);
        }

        self.scan_for_errors();
    }

    pub const fn new() -> Self {
        Self {
            oom_handler: alloc_error,

            arena: Span::empty(),
            alloc_base: core::ptr::null_mut(),
            alloc_acme: core::ptr::null_mut(),
            is_top_free: true,

            availability_low: 0,
            availability_high: 0,
            llists: [None; BIN_COUNT],
        }
    }

    pub const fn with_oom_handler(oom_handler: OomHandler) -> Self {
        Self {
            oom_handler,

            arena: Span::empty(),
            alloc_base: core::ptr::null_mut(),
            alloc_acme: core::ptr::null_mut(),
            is_top_free: true,

            availability_low: 0,
            availability_high: 0,
            llists: [None; BIN_COUNT],
        }
    }

    /// Returns the [`Span`] which has been granted to this
    /// allocator as allocatable.
    pub const fn get_arena(&self) -> Span {
        self.arena
    }

    /// Returns the [`Span`] in which allocations may be placed.
    pub fn get_allocatable_span(&self) -> Span {
        Span::from(self.alloc_base..self.alloc_acme)
    }

    /// Returns the minimum [`Span`] containing all allocated memory.
    pub fn get_allocated_span(&self) -> Span {
        // check if the arena is nonexistant
        if unsafe { self.alloc_acme.sub_ptr(self.alloc_base) } < MIN_CHUNK_SIZE {
            return Span::empty();
        }

        let mut allocated_acme = self.alloc_acme as usize;
        let mut allocated_base = self.alloc_base as usize;

        // check for free space at the arena's top
        if self.is_top_free {
            let top_free_size = unsafe { *self.alloc_acme.cast::<usize>().sub(1) };

            allocated_acme -= top_free_size;
        }

        // check for free memory at the bottom of the arena
        if !(unsafe { *self.alloc_base.cast::<Tag>() }).is_allocated() {
            let free_bottom_chunk = FreeChunk(self.alloc_base);
            let free_bottom_size = unsafe { *free_bottom_chunk.size_ptr() };

            allocated_base += free_bottom_size;
        }

        // allocated_base might be greater or equal to allocated_acme
        // but that's fine, this'll just become a Span::Empty
        Span::new(allocated_base, allocated_acme)
    }

    /// Initialize the allocator heap.
    ///
    /// # Safety
    /// - After initialization, the allocator structure is invalidated if moved.
    /// This is because there are pointers on the heap to this struct.
    /// - Initialization restores validity, but erases all knowledge of previous allocations.
    ///
    /// Use the `mov` function to safely move the struct.
    pub unsafe fn init(&mut self, arena: Span) {
        assert!(!arena.contains(0), "Arena covers the null address!");

        self.arena = arena;

        self.llists = [None; BIN_COUNT];
        self.availability_low = 0;
        self.availability_high = 0;

        match arena.word_align_inward().get_base_acme() {
            Some((base, acme)) if acme - base >= MIN_CHUNK_SIZE => {
                self.alloc_base = base as *mut u8;
                self.alloc_acme = acme as *mut u8;

                self.add_chunk_to_record(self.alloc_base, self.alloc_acme);
                self.is_top_free = true;
            }
            _ => {
                self.alloc_acme = core::ptr::null_mut();
                self.alloc_base = core::ptr::null_mut();

                self.is_top_free = false;
            }
        }

        self.scan_for_errors();
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
    /// # use talc::{Span, Talc};
    /// # let mut talck = Talc::new().spin_lock();
    /// let mut talc = talck.talc();
    /// // compute the new arena as an extention of the old arena
    /// let new_arena = talc.get_arena().extend(1234, 5678).above(0x1000);
    /// // SAFETY: be sure not to extend into memory we can't use
    /// unsafe { talc.extend(new_arena); }
    /// ```
    pub unsafe fn extend(&mut self, new_arena: Span) {
        assert!(new_arena.contains_span(self.arena), "new_span must contain the current arena");
        assert!(!new_arena.contains(0), "Arena covers the null address!");

        if self.alloc_acme.sub_ptr(self.alloc_base) < MIN_CHUNK_SIZE {
            // there's no free or allocated memory, so just init instead
            self.init(new_arena);
            return;
        }

        self.arena = new_arena;

        let old_alloc_base = self.alloc_base;
        let old_alloc_acme = self.alloc_acme;

        match new_arena.word_align_inward().get_base_acme() {
            Some((base, acme)) if acme - base >= MIN_CHUNK_SIZE => {
                self.alloc_base = base as *mut u8;
                self.alloc_acme = acme as *mut u8;
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

            self.remove_chunk_from_record(top_chunk.node_ptr(), bucket_of_size(top_size));
            self.add_chunk_to_record(top_chunk.ptr(), self.alloc_acme);
        } else if self.alloc_acme.sub_ptr(old_alloc_acme) > MIN_CHUNK_SIZE {
            self.add_chunk_to_record(old_alloc_acme, self.alloc_acme);

            self.is_top_free = true;
        } else {
            self.alloc_acme = old_alloc_acme;
        }

        // if the lowest chunk is allocated, add free chunk below if possible
        // else extend the free chuk that's there
        if !(*old_alloc_base.cast::<Tag>()).is_allocated() {
            let bottom_chunk = FreeChunk(old_alloc_base);
            let bottom_size = *bottom_chunk.size_ptr();

            self.remove_chunk_from_record(bottom_chunk.node_ptr(), bucket_of_size(bottom_size));
            self.add_chunk_to_record(self.alloc_base, bottom_chunk.ptr().add(bottom_size));
        } else if old_alloc_base.sub_ptr(self.alloc_base) > MIN_CHUNK_SIZE {
            self.add_chunk_to_record(self.alloc_base, old_alloc_base);

            Tag::set_low_free(old_alloc_base.cast());
        } else {
            self.alloc_base = old_alloc_base;
        }

        self.scan_for_errors();
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
    /// # use talc::{Span, Talc};
    /// # let mut talck = Talc::new().spin_lock();
    /// // lock the allocator otherwise a race condition may occur
    /// // in between get_allocated_span and truncate
    /// let mut talc = talck.talc();
    /// // compute the new arena as a reduction of the old arena
    /// let new_arena = talc.get_arena().truncate(1234, 5678).fit_over(talc.get_allocated_span());
    /// // alternatively...
    /// let new_arena = Span::from(1234..5678).fit_within(talc.get_arena()).fit_over(talc.get_allocated_span());
    /// // truncate the arena
    /// talc.truncate(new_arena);
    /// ```
    pub fn truncate(&mut self, new_arena: Span) {
        let new_alloc_span = new_arena.word_align_inward();

        // check that the new_arena is correct
        assert!(self.arena.contains_span(new_arena), "the old arena must contain new_arena!");
        assert!(
            new_alloc_span.contains_span(self.get_allocated_span()),
            "the new_arena must contain the allocated span!"
        );

        // if the old allocatable arena is too small to contain anything, just reinit
        if (self.alloc_acme as isize - self.alloc_base as isize) < MIN_CHUNK_SIZE as isize {
            unsafe {
                self.init(new_arena);
            }
            return;
        }

        let new_alloc_base;
        let new_alloc_acme;

        // if it's decimating the entire arena, just reinit, else get the new allocatable extents
        match new_alloc_span.get_base_acme() {
            Some((base, acme)) if acme - base >= MIN_CHUNK_SIZE => {
                self.arena = new_arena;
                new_alloc_base = base as *mut u8;
                new_alloc_acme = acme as *mut u8;
            }

            _ => {
                unsafe {
                    self.init(new_arena);
                }
                return;
            }
        }

        // trim down the arena

        // trim the top
        if new_alloc_acme < self.alloc_acme {
            debug_assert!(self.is_top_free);

            let top_free_size = unsafe { *self.alloc_acme.cast::<usize>().sub(1) };

            let top_free_chunk = FreeChunk(self.alloc_acme.wrapping_sub(top_free_size));

            unsafe {
                self.remove_chunk_from_record(
                    top_free_chunk.node_ptr(),
                    bucket_of_size(top_free_size),
                );
            }

            if ge_min_size_apart(top_free_chunk.ptr(), new_alloc_acme) {
                self.alloc_acme = new_alloc_acme;

                unsafe {
                    self.add_chunk_to_record(top_free_chunk.ptr(), new_alloc_acme);
                }
            } else {
                self.alloc_acme = top_free_chunk.ptr();
                self.is_top_free = false;
            }
        }

        // no need to check if the entire arena vanished;
        // we checked against this possiblity earlier
        // i.e. that new_alloc_span is insignificantly sized

        // check for free memory at the bottom of the arena
        if new_alloc_base > self.alloc_base {
            let base_free_chunk = FreeChunk(self.alloc_base);
            let base_free_size = unsafe { *base_free_chunk.size_ptr() };
            let base_free_chunk_acme = base_free_chunk.ptr().wrapping_add(base_free_size);

            unsafe {
                self.remove_chunk_from_record(
                    base_free_chunk.node_ptr(),
                    bucket_of_size(base_free_size),
                );
            }

            if ge_min_size_apart(new_alloc_base, base_free_chunk_acme) {
                self.alloc_base = new_alloc_base;

                unsafe {
                    self.add_chunk_to_record(new_alloc_base, base_free_chunk_acme);
                }
            } else {
                self.alloc_base = base_free_chunk_acme;

                unsafe {
                    debug_assert!(base_free_chunk_acme != self.alloc_acme);
                    Tag::clear_low_free(base_free_chunk_acme.cast());
                }
            }
        }

        self.scan_for_errors();
    }

    /// Move the allocator structure to a new destination safely.
    pub fn mov(self, dest: &mut core::mem::MaybeUninit<Self>) -> &mut Self {
        let ref_mut = dest.write(self);

        for b in 0..ref_mut.llists.len() {
            if let Some(ptr) = unsafe { ref_mut.read_llist(b) } {
                unsafe {
                    (*ptr.as_ptr()).next_of_prev = ref_mut.get_llist_ptr(b);
                }
            }
        }

        ref_mut
    }

    /// Wrap in a spin mutex-locked wrapper struct.
    ///
    /// This implements the `GlobalAlloc` trait and provides
    /// access to the `Allocator` API.
    #[cfg(feature = "spin")]
    pub const fn spin_lock(self) -> Talck {
        Talck(spin::Mutex::new(self))
    }

    /// Debugging function for checking various assumptions.
    fn scan_for_errors(&mut self) {
        #[cfg(debug_assertions)]
        {
            assert!(self.alloc_acme as isize >= self.alloc_base as isize);
            let alloc_span = Span::new(self.alloc_base as _, self.alloc_acme as _);
            assert!(self.arena.contains_span(alloc_span));

            //let mut vec = Vec::<(*mut u8, *mut u8)>::new();

            for b in 0..BIN_COUNT {
                let mut any = false;
                unsafe {
                    for node in LlistNode::iter_mut(self.read_llist(b)) {
                        any = true;
                        if b < 64 {
                            assert!(self.availability_low & 1 << b != 0);
                        } else {
                            assert!(self.availability_high & 1 << (b - 64) != 0);
                        }

                        let free_chunk = FreeChunk(node.as_ptr().cast());
                        let low_size = *free_chunk.size_ptr();
                        let high_size = *free_chunk.ptr().add(low_size - TAG_SIZE).cast::<usize>();
                        assert!(low_size == high_size);
                        assert!(free_chunk.ptr().add(low_size) <= self.alloc_acme);

                        if free_chunk.ptr().add(low_size) < self.alloc_acme {
                            let upper_tag = *free_chunk.ptr().add(low_size).cast::<Tag>();
                            assert!(upper_tag.is_allocated());
                            assert!(upper_tag.is_low_free());
                        } else {
                            assert!(self.is_top_free);
                        }

                        /* let low_ptr = free_chunk.ptr();
                        let high_ptr = low_ptr.add(low_size);

                        for &(other_low, other_high) in &vec {
                            assert!(other_high <= low_ptr || high_ptr <= other_low);
                        }
                        vec.push((low_ptr, high_ptr)); */
                    }
                }

                if !any {
                    if b < 64 {
                        assert!(self.availability_low & 1 << b == 0);
                    } else {
                        assert!(self.availability_high & 1 << (b - 64) == 0);
                    }
                }
            }

            /* vec.sort_unstable_by(|&(x, _), &(y, _)| x.cmp(&y));
            eprintln!();
            for (low_ptr, high_ptr) in vec {
                eprintln!("{:p}..{:p} - {:x}", low_ptr, high_ptr, unsafe { high_ptr.sub_ptr(low_ptr) });
            }
            eprintln!("arena: {}", self.arena);
            eprintln!("alloc_base: {:p}", self.alloc_base);
            eprintln!("alloc_acme: {:p}", self.alloc_acme);
            eprintln!(); */
        }
    }
}

#[cfg(test)]
mod tests {

    use std;

    use super::*;

    #[test]
    fn it_works() {
        const ARENA_SIZE: usize = 10000000;

        let arena = vec![0u8; ARENA_SIZE].into_boxed_slice();
        let arena = Box::leak(arena);

        let mut talc = Talc::new();
        unsafe {
            talc.init(arena.into());
        }

        let layout = Layout::from_size_align(1243, 8).unwrap();

        let a = unsafe { talc.malloc(layout) };
        assert!(a.is_ok());
        unsafe {
            a.unwrap().as_ptr().write_bytes(255, layout.size());
        }

        let mut x = vec![NonNull::dangling(); 100];

        let t1 = std::time::Instant::now();
        for _ in 0..100 {
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
        let t2 = std::time::Instant::now();
        println!("duration: {:?}", (t2 - t1) / (1000 * 2000));

        unsafe {
            talc.free(a.unwrap(), layout);
        }
    }
}
