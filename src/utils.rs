//! Code that doesn't have a great place elsewhere at the moment.
//!
//! Nothing in here should be exported.

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
    debug_assert!(acme >= base, "!(acme {:p} >= base {:p})", acme, base);
    acme as usize - base as usize >= MIN_CHUNK_SIZE
}

/// Determines the acme pointer and retrieves the tag, given the allocated pointer.
#[inline]
pub(crate) unsafe fn tag_from_alloc_ptr(ptr: *mut u8, size: usize) -> (*mut u8, Tag) {
    let post_alloc_ptr = align_up(ptr.add(size));
    // we're either reading a tag_ptr or a Tag with the base pointer + metadata in the low bits
    let base_or_tag_ptr = post_alloc_ptr.cast::<*mut u8>().read();

    // if the pointer is greater, it's a tag_ptr
    // if it's less, it's a Tag with the base pointer
    // the low bits of metadata don't effect the inequality
    if base_or_tag_ptr > post_alloc_ptr {
        (base_or_tag_ptr, base_or_tag_ptr.cast::<Tag>().read())
    } else {
        (post_alloc_ptr, Tag(base_or_tag_ptr))
    }
}

/// Pointer wrapper to a free chunk. Provides convenience methods
/// for getting the LlistNode pointer and upper pointer to its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub(crate) struct FreeChunk(pub(crate) *mut u8);

impl FreeChunk {
    const NODE_OFFSET: usize = 0;
    const SIZE_OFFSET: usize = NODE_SIZE;

    #[inline]
    pub(crate) fn base(self) -> *mut u8 {
        self.0
    }

    #[inline]
    pub(crate) fn node_ptr(self) -> *mut LlistNode {
        self.0.wrapping_add(Self::NODE_OFFSET).cast()
    }

    #[inline]
    pub(crate) fn size_ptr(self) -> *mut usize {
        self.0.wrapping_add(Self::SIZE_OFFSET).cast()
    }
}

#[cfg(not(debug_assertions))]
pub(crate) fn scan_for_errors<O: OomHandler>(_: &Talc<O>) {}

#[cfg(debug_assertions)]
/// Debugging function for checking various assumptions.
pub(crate) fn scan_for_errors<O: OomHandler>(talc: &Talc<O>) {
    #[cfg(any(test, fuzzing))]
    let mut vec = std::vec::Vec::<Span>::new();

    if !talc.bins.is_null() {
        for b in 0..BIN_COUNT {
            let mut any = false;
            unsafe {
                for node in LlistNode::iter_mut(*talc.get_bin_ptr(b)) {
                    any = true;
                    if b < WORD_BITS {
                        assert!(talc.availability_low & 1 << b != 0);
                    } else {
                        assert!(talc.availability_high & 1 << (b - WORD_BITS) != 0);
                    }

                    let free_chunk = FreeChunk(node.as_ptr().cast());
                    let size = *free_chunk.size_ptr();
                    let base_ptr = free_chunk.base();
                    let acme_ptr = free_chunk.base().add(size);
                    let low_size = acme_ptr.sub(WORD_SIZE).cast::<usize>().read();
                    assert!(low_size == size);

                    let lower_tag = base_ptr.sub(TAG_SIZE).cast::<Tag>().read();
                    assert!(lower_tag.is_allocated());
                    assert!(lower_tag.is_above_free());

                    #[cfg(any(test, fuzzing))]
                    {
                        let span = Span::new(base_ptr, acme_ptr);
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
                    assert!(talc.availability_low & 1 << b == 0);
                } else {
                    assert!(talc.availability_high & 1 << (b - WORD_BITS) == 0);
                }
            }
        }
    } else {
        assert!(talc.availability_low == 0);
        assert!(talc.availability_high == 0);
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
}
