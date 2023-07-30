//! Code that doesn't have a great place elsewhere at the moment.
//!
//! Nothing in here should be publicly exposed.

use crate::*;

/// `size` should be larger or equal to MIN_CHUNK_SIZE
#[inline]
pub(crate) unsafe fn bin_of_size(size: usize) -> usize {
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

/// Aligns `ptr` up to the next `align_mask + 1`.
///
/// `align_mask` must be a power of two minus one.
#[inline]
pub(crate) fn align_up_by(ptr: *mut u8, align_mask: usize) -> *mut u8 {
    debug_assert!((align_mask + 1).is_power_of_two());

    // this incantation maintains provenance of ptr
    // while allowing the compiler to see through the wrapping_add and optimize it
    ptr.wrapping_add(((ptr as usize + align_mask) & !align_mask) - ptr as usize)
    // equivalent to the following:
    // ((ptr as usize + align_mask) & !align_mask) as *mut u8
    // i.e. just align up to the next align_mask + 1
}

pub(crate) fn align_down(ptr: *mut u8) -> *mut u8 {
    ptr.wrapping_sub(ptr as usize % ALIGN)
}
pub(crate) fn align_up_overflows(ptr: *mut u8) -> bool {
    ALIGN - 1 > usize::MAX - ptr as usize
}
pub(crate) fn align_up(ptr: *mut u8) -> *mut u8 {
    debug_assert!(!align_up_overflows(ptr));

    let offset_ptr = ptr.wrapping_add(ALIGN - 1);
    offset_ptr.wrapping_sub(offset_ptr as usize % ALIGN)
}

/// Returns whether the two pointers are greater than `MIN_CHUNK_SIZE` apart.
pub(crate) fn is_chunk_size(base: *mut u8, acme: *mut u8) -> bool {
    debug_assert!(acme >= base, "!(acme {:p} > ptr {:p})", acme, base);
    acme as isize - base as isize >= MIN_CHUNK_SIZE as isize
}

/// Determines the chunk pointer and retrieves the tag, given the allocated pointer.
#[inline]
pub(crate) unsafe fn chunk_ptr_from_alloc_ptr(ptr: *mut u8) -> (*mut u8, Tag) {
    #[derive(Clone, Copy)]
    union PreAllocationData {
        tag: Tag,
        ptr: *mut Tag,
    }

    let mut low_ptr = ptr.sub(TAG_SIZE + ptr as usize % ALIGN);

    let data = low_ptr.cast::<PreAllocationData>().read();

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

/// Determine the required allocated chunk acme.
#[inline]
pub(crate) fn required_acme(alloc_base: *mut u8, size: usize, tag_ptr: *mut u8) -> *mut u8 {
    // choose the highest between...
    core::cmp::max(
        // the required chunk acme due to the allocation
        align_up(alloc_base.wrapping_add(size)),
        // the required chunk acme due to the minimum chunk size
        tag_ptr.wrapping_add(MIN_CHUNK_SIZE),
    )
}

/// Pointer wrapper to a free chunk. Provides convenience methods
/// for getting the LlistNode pointer and lower pointer to its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub(crate) struct FreeChunk(pub(crate) *mut u8);

impl FreeChunk {
    const NODE_OFFSET: usize = 0;
    const SIZE_OFFSET: usize = NODE_SIZE;

    pub(crate) fn base(self) -> *mut u8 {
        self.0
    }

    pub(crate) fn node_ptr(self) -> *mut LlistNode {
        unsafe { self.0.add(Self::NODE_OFFSET).cast() }
    }

    pub(crate) fn size_ptr(self) -> *mut usize {
        unsafe { self.0.add(Self::SIZE_OFFSET).cast() }
    }
}

/// An abstraction over the unknown state of the chunk above.
pub(crate) enum AboveChunk {
    Free(FreeChunk),
    Allocated(*mut Tag),
}

/// Distinguish the nature of the chunk above.
pub(crate) unsafe fn identify_above(chunk_acme: *mut u8) -> AboveChunk {
    if (*chunk_acme.cast::<Tag>()).is_allocated() {
        AboveChunk::Allocated(chunk_acme.cast())
    } else {
        AboveChunk::Free(FreeChunk(chunk_acme))
    }
}

/// Debugging function for checking various assumptions.
pub(crate) fn scan_for_errors<O: OomHandler>(_talc: &mut Talc<O>) {
    #[cfg(debug_assertions)]
    {
        assert!(_talc.allocatable_acme >= _talc.allocatable_base);
        let alloc_span = Span::new(_talc.allocatable_base as _, _talc.allocatable_acme as _);
        assert!(_talc.arena.contains_span(alloc_span));

        #[cfg(test)]
        let mut vec = Vec::<(*mut u8, *mut u8)>::new();

        if _talc.bins.as_mut_ptr() != null_mut() {
            assert!(_talc.allocatable_base != null_mut());
            assert!(_talc.allocatable_acme != null_mut());

            for b in 0..BIN_COUNT {
                let mut any = false;
                unsafe {
                    for node in LlistNode::iter_mut(*_talc.get_bin_ptr(b)) {
                        any = true;
                        if b < WORD_BITS {
                            assert!(_talc.availability_low & 1 << b != 0);
                        } else {
                            assert!(_talc.availability_high & 1 << (b - WORD_BITS) != 0);
                        }

                        let free_chunk = FreeChunk(node.as_ptr().cast());
                        let low_size = *free_chunk.size_ptr();
                        let high_size = *free_chunk.base().add(low_size - TAG_SIZE).cast::<usize>();
                        assert!(low_size == high_size);
                        assert!(free_chunk.base().add(low_size) <= _talc.allocatable_acme);

                        if free_chunk.base().add(low_size) < _talc.allocatable_acme {
                            let upper_tag = *free_chunk.base().add(low_size).cast::<Tag>();
                            assert!(upper_tag.is_allocated());
                            assert!(upper_tag.is_below_free());
                        } else {
                            assert!(_talc.is_top_free);
                        }

                        #[cfg(test)]
                        {
                            let low_ptr = free_chunk.base();
                            let high_ptr = low_ptr.add(low_size);

                            for &(other_low, other_high) in &vec {
                                assert!(other_high <= low_ptr || high_ptr <= other_low);
                            }
                            vec.push((low_ptr, high_ptr));
                        }
                    }
                }

                if !any {
                    if b < WORD_BITS {
                        assert!(_talc.availability_low & 1 << b == 0);
                    } else {
                        assert!(_talc.availability_high & 1 << (b - WORD_BITS) == 0);
                    }
                }
            }
        } else {
            assert!(_talc.allocatable_base == null_mut());
            assert!(_talc.allocatable_acme == null_mut());
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
