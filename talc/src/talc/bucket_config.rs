use super::{alignment::{alloc_unit, ChunkAlign}, bitfield::{BitField, TwoUsizeBitField}};

pub trait BucketConfig {
    type Availability: BitField;

    unsafe fn size_to_bucket<A: ChunkAlign>(size: usize) -> usize {
        auto_size_to_bucket::<A, Self::Availability>(size)
    }

    const INIT: Self;
}

pub struct TwoUsizeBucketConfig;

impl BucketConfig for TwoUsizeBucketConfig {
    type Availability = TwoUsizeBitField;
    const INIT: Self = Self;
}

/* pub const fn bucket_to_size_64_kib_max_with_63_buckets(size: usize) -> usize {
    if size < 1024 {
        (size >> 5) - 1
    } else {
        let category = usize::BITS as usize - 10 - size.leading_zeros() as usize;
        let v = 1 << (6 - category);
        ((size >> (4 + category * 2)) ^ (v >> 1)) + (63 - v)
    }
} */

/// Calls [`auto_size_to_bucket_with_soft_max`] with `SOFT_MAX` = 96 MiB on 64 bits systems and 20 MiB on 32 bit systems.
/// 
/// These numbers are somewhat arbitrary, but result in TODO
pub unsafe fn auto_size_to_bucket<A: ChunkAlign, B: BitField>(size: usize) -> usize {
    #[cfg(target_pointer_width = "32")]
    return auto_size_to_bucket_with_soft_max::<A, B, {1 << 20}>(size);

    #[cfg(target_pointer_width = "64")]
    // return auto_size_to_bucket_with_soft_max::<A, B, {1 << 20}>(size);
    return bucket_of_size_l1_l2_pexp::<TwoUsizeBinCfg, A>(size);
}

pub fn calc_level(size: usize) -> u8 {
    unsafe {
        core::hint::assert_unchecked(size != 0);
    }
    let log = size.ilog2() as u8;

    if log > 24 {
        63
    } else if log < 5 {
        0
    } else {
        let level = log - 4;
        level * 4 + (size >> level) as u8 - 4
    }
}

/// Linear region.
/// Transition (trans) region.
/// Pseudo-exponential (pexp) region.
pub const unsafe fn auto_size_to_bucket_with_soft_max<A: ChunkAlign, B: BitField, const SOFT_MAX: usize>(size: usize) -> usize {
    #[inline]
    const fn ilog2(i: usize) -> usize { i.ilog2() as usize }
    #[inline]
    const fn at_least_2(v: usize) -> usize { if v > 2 { v } else { 2 } }

    // All of these `let` statements are done at compile time if optimizations are enabled.
    // They configure how we will do bucket allocation across 3 regions:
    // - Linearly divided buckets
    // - Exponential region with changing linear subdivisions (transition region)
    // - Exponential region with fixed linear subdivisions
    // These give nice coverage and spread.
    // The function is pretty fast too after any level of optimizations.

    let alloc_unit = alloc_unit::<A>();
    let non_pexp_buckets = 1 << (B::BIT_COUNT * 2 / 3).ilog2();
    let linear_buckets = non_pexp_buckets >> 1;

    let linear_max = alloc_unit * linear_buckets;
    let trans_max = linear_max * linear_buckets;
    let pexp_bucket_offset = non_pexp_buckets - 2;
    let pexp_buckets = B::BIT_COUNT - pexp_bucket_offset;

    let linear_subdivs_per_exp_step = pexp_buckets / (ilog2(SOFT_MAX) - ilog2(trans_max));
    let linear_subdivs_per_exp_step = at_least_2(1usize << linear_subdivs_per_exp_step.ilog2());

    // Calculate the maximum extent of the pseudo-exponential region with the bins available.
    // Note that the divide loses some information here; the extra few bins will add linear subdivs.
    let pexp_max_without_linear_extra = trans_max * 1 << (pexp_buckets / linear_subdivs_per_exp_step);
    // We add the extra extent due to the linear subdivs by finding the modulo.
    let pexp_max = pexp_max_without_linear_extra 
        + (pexp_buckets % linear_subdivs_per_exp_step) * pexp_max_without_linear_extra / linear_subdivs_per_exp_step;

    /* // This asserts in debug or "proves" in release to the compiler
    // that the size is at least `alloc_unit`.
    if size < alloc_unit {
        debug_assert!(false, "size is less than allocation unit!");
        unsafe { core::hint::unreachable_unchecked() };
    } */

    let linear_subdivisions_are_enough = alloc_unit * B::BIT_COUNT >= SOFT_MAX;

    if linear_subdivisions_are_enough || size < linear_max {
        (size / alloc_unit).wrapping_sub(1)
    } else if size < trans_max {
        let category = ilog2(size) - (ilog2(alloc_unit * linear_buckets) - 1);
        let v = 1 << (ilog2(linear_buckets) - category);
        ((size >> (alloc_unit.ilog2() as usize - 1 + category * 2)) ^ v) + (non_pexp_buckets - 1 - v * 2)
    } else if size < pexp_max {
        // Log 2 of (minimum pseudo-exponential chunk size)
        let min_exp_bits_less_1 = ilog2(trans_max);

        // how many bits are used to determine the division
        let pexp_div_bits = ilog2(linear_subdivs_per_exp_step);

        let bits_less_one = ilog2(size);

        // the magnitude the size belongs to.
        // calculate the difference in bit count i.e. difference in power
        let magnitude = bits_less_one - min_exp_bits_less_1;
        // the division of the magnitude the size belongs to.
        // slide the size to get the division bits at the bottom and remove the top bit
        let division = (size >> (bits_less_one - pexp_div_bits)) & !linear_subdivs_per_exp_step;
        // the index into the pseudo-exponential buckets.
        let bin_offset = (magnitude << pexp_div_bits) | division;

        // cap the max bucket at the last bucket
        let pexp_bin = bin_offset + pexp_bucket_offset;
        
        pexp_bin
    } else {
        B::BIT_COUNT - 1
    }
}


pub trait L1L2PexpConfig {
    const L1_DIVS: usize;
    const L2_DIVS: usize;
    const PEXP_LIN_DIVS: usize;
    const BIN_COUNT: usize;
}

pub const unsafe fn bucket_of_size_l1_l2_pexp<Cfg: L1L2PexpConfig, A: ChunkAlign>(size: usize) -> usize {
    // this mess determines the bucketing strategy used by the allocator
    // the default is to have a bucket per multiple of word size from the minimum
    // chunk size up to WORD_BUCKETED_SIZE and double word gap (sharing two sizes)
    // up to DOUBLE_BUCKETED_SIZE, and from there on use pseudo-logarithmic sizes.

    // such sizes are as follows: begin at some power of two (DOUBLE_BUCKETED_SIZE)
    // and increase by some power of two fraction (quarters, on 64 bit machines)
    // until reaching the next power of two, and repeat:
    // e.g. begin at 32, increase by quarters: 32, 40, 48, 56, 64, 80, 96, 112, 128, ...

    let l1_size = alloc_unit::<A>();
    let l2_size = l1_size * 2;

    let l1_extent = l1_size + l1_size * Cfg::L1_DIVS;
    let l2_extent = l1_extent + l2_size * Cfg::L2_DIVS;

    // Log 2 of (minimum pseudo-exponential chunk size)
    let min_exp_bits_less_1 = l2_extent.ilog2() as usize;

    // how many bits are used to determine the division
    let pexp_div_bits = Cfg::PEXP_LIN_DIVS.ilog2() as usize;

    if size < l1_extent {
        // single word separated buckets

        (size / l1_size).wrapping_sub(1)
    } else if size < l2_extent {
        // quad word separated buckets

        // equiv to (size - WORD_BIN_LIMIT) / 2WORD_SIZE + DBL_BUCKET
        // but saves an instruction
        size / l2_size - l1_extent / l2_size + Cfg::L1_DIVS
    } else {
        // pseudo-exponentially separated bucket

        // here's what a size is, bit by bit: 1_div_extra
        // e.g. with four divisions 1_01_00010011000
        // the bucket is determined by the magnitude and the division
        // mag 0 div 0, mag 0 div 1, mag 0 div 2, mag 0 div 3, mag 1 div 0, ...

        // let shifted_size = (size)
        let bits_less_one = size.ilog2() as usize;

        // the magnitude the size belongs to.
        // calculate the difference in bit count i.e. difference in power
        let magnitude = bits_less_one - min_exp_bits_less_1;
        // the division of the magnitude the size belongs to.
        // slide the size to get the division bits at the bottom and remove the top bit
        let division = (size >> (bits_less_one - pexp_div_bits)) & !Cfg::PEXP_LIN_DIVS;
        // the index into the pseudo-exponential buckets.
        let bin_offset = (magnitude << pexp_div_bits) | division;

        // cap the max bucket at the last bucket
        let pexp_bin = bin_offset + Cfg::L1_DIVS + Cfg::L2_DIVS;

        if pexp_bin < Cfg::BIN_COUNT {
            pexp_bin
        } else {
            Cfg::BIN_COUNT - 1
        }
    }
} 

pub struct TwoUsizeBinCfg;

#[cfg(target_pointer_width = "64")]
impl L1L2PexpConfig for TwoUsizeBinCfg {
    const L1_DIVS: usize = 55;
    const L2_DIVS: usize = 36;
    const PEXP_LIN_DIVS: usize = 2;
    const BIN_COUNT: usize = usize::BITS as usize * 2;
}

#[cfg(target_pointer_width = "32")]
impl L1L2PexpConfig for TwoUsizeBinCfg {
    const L2_SIZE: usize = 256;
    const PEXP_SIZE: usize = 1024;
    const PEXP_LIN_DIVS: usize = 2;
    const MAX_BIN: usize = 63;
}