//! [`Talc`](crate::base::Talc)'s internal binning strategy is dictated by
//! the [`Binning`] implementation used. See [`Binning`]'s docs for more
//! information on those specifics.
//!
//! This is useful to change depending on how [`Talc`](crate::base::Talc)
//! is being used. For example, WebAssembly module size and performance
//! substantially benefits from [`WasmBinning`](crate::wasm::WasmBinning)
//! over [`DefaultBinning`] due to platform-specific quirks (64-bit
//! instructions on a 32-bit memory architecture).
//!
//! [`DefaultBinning`] serves as a very general-purpose binning strategy.
//! If you need to implement something more tailored, be aware of
//! - [`test_utils`] which provides methods for evaluating binning strategies.
//! - [`linear_extent_then_linearly_divided_exponential_binning`] which is a highly
//!     optimized `size_to_bin` implementation that you should consider using.

use super::bitfield::BitField;

/// Implementors describe a binning strategy for [`Talc`](super::Talc) to use.
///
/// Different binning strategies greatly affect the performance and efficiency
/// of the allocator, and different bit-fields are better depending on the
/// instruction set architecture.
///
/// A binning strategy determines:
/// - [`Binning::BIN_COUNT`]: how many free lists there are.
/// - [`Binning::size_to_bin`]: which chunks go into which lists based on size.
/// - [`Binning::AvailabilityBitField`]: what type is used to track the availability of chunks in the free lists.
pub trait Binning: Sized {
    /// The bit-field type to use for describing the availability of bins.
    ///
    /// Use one that yield sufficient bits for the binning strategy,
    /// but otherwise this should be chosen to be as fast as possible.
    ///
    /// [`BitField`] is implemented for `uint` types, `[uint; N]`, and
    /// [`TwoLevelBitField`](crate::base::bitfield::TwoLevelBitField)
    /// provides a good high-bit solution as well.
    type AvailabilityBitField: BitField;

    /// The number of bins [`Talc`](crate::base::Talc) needs to set up.
    ///
    /// This must not exceed `Self::AvailabilityBitField::BITS`.
    ///
    /// The size of [`Talc`](crate::base::Talc)'s metadata chunk is entirely
    /// determined by this value.
    /// The larger this is, the larger the metadata chunk.
    ///
    /// # What should I set this to?
    /// - `Self::AvailabilityBitField::BITS - 1` if you have a bin to spare to eliminate
    ///     an uncommon branch in the allocation routine. Not a big deal.
    /// - `Self::AvailabilityBitField::BITS` is otherwise a good default.
    ///
    /// (If you have way too many bits in your `AvailabilityBitField`,
    /// consider using a smaller bitfield instead of leaving many of those
    /// bits unused. Keeping the complexity of the bitfield minimal is good
    /// for performance, as long as you have enough bins.)
    const BIN_COUNT: u32 = Self::AvailabilityBitField::BITS;

    /// Given `size` of a chunk return which bin, or free-list, it should be in.
    ///
    /// # How a normal `size_to_bin` function works
    ///
    /// Any size within `0..chunk_unit::<A>()` results in `u8::MAX`.
    /// This is an edge case that Talc takes advantage of.
    ///
    /// Then ranges of sizes in increasing order are allocated increasingly larger bins.
    ///
    /// After a certain size, `Self::bin_count() - 1` is returned, the largest bin size.
    ///
    /// # What guarantees are associated with `size`
    ///
    /// When Talc ends up with a free chunk and needs to keep track of it, it'll
    /// call `size_to_bin` with its size, which is always a non-zero multiple
    /// of `chunk_unit::<A>()` to figure out which free list to put it in.
    ///
    /// When the user attempts to allocate using Talc, Talc tries to avoid having to
    /// search for a block if it can grab one directly.
    ///
    /// A free list contains a range of sizes. Let's say bin 3 holds chunk sizes from
    /// 96 to 128. If Talc needs a chunk that's 96 bytes, it should take a chunk from this
    /// list. But if Talc needs a 112 byte chunk, it could search this list for a chunk that's
    /// at least 112 chunks or it could instead go to bin 4, which holds 128..160 byte chunks
    /// and grab the first one.
    ///
    /// To achieve this, upon allocating, Talc asks what bin the `required_chunk_size - 1`
    /// would slot into, and then tries to take from the bin+1. It's much faster this way.
    /// Notice that if `chunk_unit::<A>()` is 32 and bin size ranges are 32..64, 64..96, 96..128, ...
    /// then this will always grab from the bin range with the exact size it needs.
    /// If, however, the ranges are larger, and
    ///
    ///
    ///
    /// # Requirements
    ///
    /// There's some rules to hold up, here. TODO
    /// - Do not return values equal to or greater than `Self::bin_count()`
    ///     - Exception: if `size < chuck_unit::<A>()`, then the function is _allowed_ to return `u8::MAX`
    /// - TODO
    ///
    /// . (NOTE: WRAPPING)
    ///
    /// This mapping must be deterministic for a given [`Binning`] implementation.
    fn size_to_bin(size: usize) -> u32;

    /// Maps from a chunk's size to which bin, or free-list, will definitely
    /// have a sufficient chunk size. This must match the `size_to_bin` implementation.
    ///
    /// If the return value is `BIN_COUNT-1` or higher, it's assumed that the last
    /// bin is the only choice, and that every chunk in the last bin might not necessarily
    /// be sufficiently large.
    #[inline]
    fn size_to_bin_ceil(size: usize) -> u32 {
        // Override this function if your `size_to_bin` has a faster `size_to_bin_ceil` implementation.
        // Override this function if your `size_to_bin` function has special requirements
        // e.g. `size` cannot be less than `CHUNK_UNIT` or `size` must be a multiple of `CHUNK_UNIT`
        // though it's recommended to instead not rely on this and leave this implementation as-is.

        // If we're at the bottom of a bin, then all chunks in this bin are grab-able.
        //  So subtract 1 so that size_to_bin yields bin-1, then add 1 to get back to the same bin.
        // If we're not at the bottom of a bin, then some chunks in this bin might not be big enough,
        //  subtracting one from the size doesn't change the return value of size_to_bin.
        //  Adding one achieves the "ceil".
        Self::size_to_bin(size - 1).wrapping_add(1)
    }
}

/// The default [`Binning`] implementation used by `Talc`.
///
/// Very competitive efficiency while still being very fast. Sticking with this is generally a good choice.
pub struct DefaultBinning;
impl Binning for DefaultBinning {
    type AvailabilityBitField = [usize; 3]; // super::bitfield::TwoLevelBitField<usize, usize, 3>;

    const BIN_COUNT: u32 = Self::AvailabilityBitField::BITS - 1;

    #[inline]
    fn size_to_bin(size: usize) -> u32 {
        if cfg!(target_pointer_width = "64") {
            linear_extent_then_linearly_divided_exponential_binning::<8, 4>(size)
        } else if cfg!(target_pointer_width = "32") {
            linear_extent_then_linearly_divided_exponential_binning::<4, 4>(size)
        } else {
            panic!("only 64-bit and 32-bit architectures are currently supported")
        }
    }
}

/// A fast binning algorithm with relatively even coverage and configurable behavior.
///
/// This is the default binning algorithm that `Talc` uses due to having a good
/// spread of bin intervals, being able to take advantage of many or few buckets
/// well, and being very fast (only a handful of instructions with one branch).
///
/// # Behavior by size
/// - `0..=(CHUNK_UNIT*LIN_DIVS*LIN_EXT_MULTI)` : Bins sizes into one-bin-per-chunk-size  
/// - `(CHUNK_UNIT*LIN_DIVS*LIN_EXT_MULTI)..`   : Binds sizes by linearly-subdivided exponential levels.
///
/// # Parameters
/// - `LIN_DIVS`: the number of linear regions per power of two in the exponential region.
///     - The higher this is, the more buckets are needed but the binning is more fine-grained.
///     - Must be a power of two.
///     - Typically 2 (few bins, subpar granularity), 4, or 8 (lots of bins, good granularity).
///     - This is the parameter you want to figure out first for a given number of bins.
///
/// - `LIN_EXT_MULTI`: the linear region extent multiplier.
///     - Scales the extent of the linear region.
///     - Must be a power of two.
///     - Set this to 1 by default.
///     - If there are too many bins being used on excessively-high size regions, this is useful
///         for spending those bins on more buckets for small sizes instead.
///
/// # Deciding on the parameters
/// Make use of [`test_utils::find_binning_boundaries`] to get a sense for
/// the mapping. `LIN_DIVS` has a much larger effect so tinker with that first
/// while keeping `LIN_EXT_MULTI` low, and then increase `LIN_EXT_MULTI` if
/// there is useless range at the top, given the number of bins you have.
///
/// Having a range up to around 128MiB~2GiB is generally good.
///
/// The main effects on the allocator will be the heap efficiency and the performance.
/// Scripts to test these can be found in the repository.
#[inline]
pub const fn linear_extent_then_linearly_divided_exponential_binning<
    const LIN_DIVS: usize,
    const LIN_EXT_MULTI: u32,
>(
    size: usize,
) -> u32 {
    assert!(LIN_DIVS.is_power_of_two());
    assert!(LIN_EXT_MULTI.is_power_of_two());

    let exponential_region = super::CHUNK_UNIT * LIN_DIVS * LIN_EXT_MULTI as usize;

    // If the size is small enough, just divide by the chunk size.
    // This is fast short-circuit that handles the case where Talc
    // might give us a `size` smaller than `super::CHUNK_UNIT`
    // and doesn't waste extra bins due to exponential subdivisions
    // being smaller than `super::CHUNK_UNIT` here.
    if size <= exponential_region {
        return (size >> ilog2(super::CHUNK_UNIT)) as u32;
    }

    // Let's say `sub_exponential` is 256, the chunk unit is 32, LIN_DIVS is 4
    //
    // Exponential level 0:  256 ;  (512 - 256)/LIN_DIVS = 256/LIN_DIVS = 64
    //  Subdiv 0: 256       ; bin 0 + LIN_DIVS * LIN_EXT_MULTI
    //  Subdiv 1: 256 +  64 ; bin 1 + LIN_DIVS * LIN_EXT_MULTI
    //  Subdiv 2: 256 + 128 ; bin 2 + LIN_DIVS * LIN_EXT_MULTI
    //  Subdiv 3: 256 + 196 ; bin 3 + LIN_DIVS * LIN_EXT_MULTI
    // Exponential level 1:  512 ;  512/LIN_DIVS = 128
    //  Subdiv 0: 512       ; bin 4 + LIN_DIVS * LIN_EXT_MULTI
    //  Subdiv 1: 512 + 128 ; bin 5 + LIN_DIVS * LIN_EXT_MULTI
    //  Subdiv 2: 512 + 256 ; bin 6 + LIN_DIVS * LIN_EXT_MULTI
    //  Subdiv 3: 512 + 384 ; bin 7 + LIN_DIVS * LIN_EXT_MULTI
    // Exponential level 2: 1024 ; 1024/LIN_DIVS = 256
    //  etc...
    //
    // Any size here is essentially broken up as follows:
    //
    // 00000000_1_01_010101010
    //               ^^^^^^^^^ dead bits; all of this is ignored, effectively rounding down these bits away
    //            ^^ linear division bits; LIN_DIVS.ilog2() bits long after the first set bit; tells us which linear subdivision we're in
    //          ^ first set bit; dictates size.ilog2(); tells us which "exponential level" this

    let size_ilog2 = ilog2(size);

    // Shift out the dead bits. This leaves the linear subdivision plus LIN_DIVS (due to the always-set bit at the top)
    let linear_subdivision_plus_lin_divs = size >> (size_ilog2 - ilog2(LIN_DIVS));
    // Extract the exponential level above the `exponential_region` limit
    // add LIN_EXT_MULTI here along with the other constants, it will get multiplied by LIN_DIVS next which gives us the exponential bins offset
    // subtract 1 along with the other constants, this is important later
    let unshifted_exponential_minus_one =
        size_ilog2 - ilog2(exponential_region) + LIN_EXT_MULTI - 1;
    // Multiply the exponential level by LIN_DIVS to shift it above the linear division bits
    // Multiply the LIN_EXT_MULTI by LIN_DIVS to add the offset due to the linearly-spaced buckets
    // Multiply (-1) to get (-LIN_DIVS)
    let exponential_plus_offset_minus_lin_divs =
        unshifted_exponential_minus_one << ilog2(LIN_DIVS);

    // This LIN_DIVS cancel out, yielding the expected exponential-region bin
    exponential_plus_offset_minus_lin_divs + linear_subdivision_plus_lin_divs as u32
}

#[inline]
const fn ilog2(i: usize) -> u32 {
    debug_assert!(i != 0);
    usize::BITS - 1 - i.leading_zeros()
}

#[cfg(test)]
mod tests {
    use super::test_utils::check_binning_properties;

    /* #[test]
    fn test_fast_linear_else_exponential_bin_to_size() {
        binning_implementation_test(None, &|size| super::fast_linear_else_exponential_bin_to_size::<super::DefaultBinning>(size));
    } */

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_linear_extent_then_linearly_divided_exponential_binning() {
        check_binning_properties(None, &|size| {
            super::linear_extent_then_linearly_divided_exponential_binning::<8, 4>(size)
        });
    }
}

/// Contains utilities for evaluating and testing [`Binning`](super::Binning)
/// implementation behavior.
pub mod test_utils {
    /// Scans for binning boundaries, returning where the `size_to_bin`
    /// starts allocating buckets to the next bin.
    ///
    /// Calls `bin_boundary_callback` each time the boundary is found.
    ///
    /// # Correctness
    /// This doesn't check every bin, so it's not guaranteed to find every
    /// boundary if the bins aren't monotonically increasing.
    ///
    /// For properly checking correctness, you need to check every bin.
    ///
    /// Still good enough for catching skipping bins, as well as
    /// catching various other issues, like panicking on certain inputs or doing
    /// a step-down for more than a few bins.
    pub fn find_binning_boundaries(
        start_from_size: usize,
        stop_at_bin: Option<u32>,
        size_to_bin: &dyn Fn(usize) -> u32,
        bin_boundary_callback: &mut dyn FnMut(u32, usize),
    ) {
        let mut prev_size = start_from_size;
        let mut size = start_from_size;
        let mut increment = 1;

        let mut prev_bin: Option<u32> = None;

        loop {
            let bin = size_to_bin(size);

            if prev_bin.is_none() || prev_bin.unwrap() != bin {
                if let Some(prev_bin) = prev_bin {
                    size = find_binning_boundary(
                        prev_bin.wrapping_add(1),
                        prev_size,
                        size,
                        size_to_bin,
                    );
                }

                bin_boundary_callback(bin, size);

                increment = ((size - prev_size) / 4).max(1);
                prev_size = size;

                prev_bin = Some(bin);
            }

            if size != usize::MAX {
                if let Some(next_size) = size.checked_add(increment) {
                    size = next_size;
                } else {
                    size = usize::MAX;
                }
            } else {
                break;
            }

            if let Some(max) = stop_at_bin {
                if max == bin {
                    break;
                }
            }
        }
    }

    /// Uses [`find_binning_boundaries`] to scan through various sizes and asserts
    /// that the mapping obeys a number of properties:
    /// - Bins should be monotonically increasing with size
    /// - Only the bin at `CHUNK_UNIT - 1` may return `u32::MAX`
    /// - Bins shouldn't be skipped (for efficiency, not correctness)
    pub fn check_binning_properties(stop_at_bin: Option<u32>, size_to_bin: &dyn Fn(usize) -> u32) {
        let mut prev_bin: Option<u32> = None;

        let mut callback = |bin: u32, size: usize| {
            if let Some(prev_bin) = prev_bin {
                assert_eq!(prev_bin.wrapping_add(1), bin);
            }
            prev_bin = Some(bin);

            assert!(
                stop_at_bin.is_none()
                    || bin <= stop_at_bin.unwrap()
                    || (bin == u32::MAX && size < crate::base::CHUNK_UNIT)
            );
        };

        find_binning_boundaries(
            crate::base::CHUNK_UNIT - 1,
            stop_at_bin,
            size_to_bin,
            &mut callback,
        );
    }

    /// Searches for the first size where `size_to_bin` yields `next_bin`
    /// in the inclusive range `base..=acme`.
    ///
    /// This assumes that `size_to_bin` is monotonically increasing.
    ///
    /// - If `size_to_bin` never yields `next_bin`, this will try to find the first size to yield a higher bin.
    /// - If all inputs to `size_to_bin` yield a bin equal or greater than `next_bin`, `base` is returned.
    /// - If all inputs to `size_to_bin` yield a bin less that `next_bin`, `acme` is returned.
    fn find_binning_boundary(
        next_bin: u32,
        mut base: usize,
        mut acme: usize,
        size_to_bin: &dyn Fn(usize) -> u32,
    ) -> usize {
        while base < acme {
            let mid = base + (acme - base) / 2;

            if size_to_bin(mid) >= next_bin {
                acme = mid;
            } else {
                base = mid + 1;
            }
        }

        base
    }

    #[cfg(test)]
    mod tests {
        use crate::base::binning::test_utils::find_binning_boundaries;

        use super::find_binning_boundary;

        #[test]
        fn check_find_binning_boundary() {
            let size_to_bin = [0, 1, 1, 1, 2, 2, 2, 2, 2, 2, 3, 5];
            assert_eq!(find_binning_boundary(2, 1, 3, &|s| size_to_bin[s]), 3);
            assert_eq!(find_binning_boundary(2, 3, 5, &|s| size_to_bin[s]), 4);
            assert_eq!(find_binning_boundary(2, 2, 4, &|s| size_to_bin[s]), 4);
            assert_eq!(find_binning_boundary(2, 4, 6, &|s| size_to_bin[s]), 4);
            assert_eq!(find_binning_boundary(2, 5, 7, &|s| size_to_bin[s]), 5);

            assert_eq!(find_binning_boundary(2, 2, 11, &|s| size_to_bin[s]), 4);
            assert_eq!(find_binning_boundary(2, 0, 7, &|s| size_to_bin[s]), 4);

            assert_eq!(find_binning_boundary(4, 0, 11, &|s| size_to_bin[s]), 11);
        }

        #[test]
        fn check_find_binning_boundaries() {
            let size_to_bin = [0, 1, 1, 1, 2, 2, 2, 2, 2, 2, 3, 3];
            let boundary_sizes = [0, 1, 4, 10];

            let mut i = 0;
            let mut verifier = |bin, size| {
                assert_eq!(size, boundary_sizes[i]);
                assert_eq!(bin, size_to_bin[size]);
                i += 1;
            };

            find_binning_boundaries(0, Some(3), &|s| size_to_bin[s], &mut verifier);
        }
    }
}

// The graveyard of some binning algorithms that I haven't bothered to fix up/finish/expose yet.
// `fast_linear_else_exponential_bin_to_size` is fast but rather heap inefficient and is unused.
// `bin_to_size_64_kib_max_with_63_bins` was part of an old idea that never came to fruition (MT+superblocks).
// The large two are relatively slow compared to `linear_extent_then_linearly_divided_exponential_binning`

/* /// A [`Binning::size_to_bin`] implementation that bins linearly, then exponentially.
///
/// This is a very simple but reasonably effective binning algorithm.
///
/// This works well on any platform and with any `chunk_unit::<A>()`,
/// but requires that `B::bin_count()` is at least 29.
/// This might be relaxed in the future.
#[inline]
pub fn fast_linear_else_exponential_bin_to_size<B: Binning>(size: usize) -> u32 {
    let linear_division_size = crate::base::CHUNK_UNIT;

    if B::BIN_COUNT < 29 {
        // This isn't a hard requirement, but I doubt people will commonly want
        // so few bins and still want to use this binning algorithm.
        // If you run into this, check if there's a misconfiguration with the
        // `Binning` impl being used. If you do really want so few bins,
        // consider writing your own `size_to_bin` implementation instead.
        panic!("`bin_to_size_lin_exp` expects at least 29 bins to be available.")
    }

    // This really doesn't need to be that high. Doesn't matter what
    // platform you're on, allocating over 100MiB at a time is rare enough
    // to not dedicate many bins to.
    //
    // At worst, the number of bins is 28 and so there will be 7 linear bins.
    // At worst, linear_divisions might be 16 on 32 bit platforms.
    // => `linear_extent` is 16 * (7 + 1) = 128
    // => The maximum bin category is 2^(ilog2(144) + 18) which is 32MiB
    // This is fine.
    // More commonly, `linear_extent` will be at least 512, resulting in a reach of
    // 2^(log2(512) + 18) which is 128MiB, which is plenty.
    let exponential_bins = 22;
    let linear_bins = B::BIN_COUNT - exponential_bins;

    // We map the sizes as follows:
    // Linear mapping:
    //    0..linear_divisions                                         : u8::MAX
    //    linear_divisions..(linear_divisions * (linear_bins + 1)) : 0..linear_bins
    // Exponential mapping:
    //    (linear_divisions * linear_bins)..                       : linear_bins..B::bin_count()

    let linear_extent = linear_division_size * (linear_bins as usize + 1);

    if size < linear_extent {
        ((size / linear_division_size) as u32).wrapping_sub(1)
    } else {
        let exp_bin = size.ilog2() - linear_extent.ilog2() + linear_bins;
        exp_bin
    }
} */

/* pub const fn bin_to_size_64_kib_max_with_63_bins(size: usize) -> usize {
    if size < 1024 {
        (size >> 5) - 1
    } else {
        let category = usize::BITS as usize - 10 - size.leading_zeros() as usize;
        let v = 1 << (6 - category);
        ((size >> (4 + category * 2)) ^ (v >> 1)) + (63 - v)
    }
} */

/*/// Calls [`auto_size_to_bin_with_soft_max`] with `SOFT_MAX` = 96 MiB on 64 bits systems and 20 MiB on 32 bit systems.
pub unsafe fn auto_size_to_bin<B: Binning>(size: usize) -> u8 {
    #[cfg(target_pointer_width = "32")]
    return auto_size_to_bin_with_soft_max::<B, {1 << 20}>(size);

    #[cfg(target_pointer_width = "64")]
    return auto_size_to_bin_with_soft_max::<B, {1 << 20}>(size);
    // return bin_of_size_l1_l2_pexp::<TwoUsizeBinCfg, A>(size);
    // return bin_of_size_lin_exp::<A>(size);
}

/// Linear region.
/// Transition (trans) region.
/// Pseudo-exponential (pexp) region.
pub unsafe fn auto_size_to_bin_with_soft_max<B: Binning, const SOFT_MAX: usize>(size: usize) -> u32 {
    // All of these `let` statements are done at compile time if optimizations are enabled.
    // They configure how we will do bin allocation across 3 regions:
    // - Linearly divided bins
    // - Exponential region with changing linear subdivisions (transition region)
    // - Exponential region with fixed linear subdivisions
    // These give nice coverage and spread.
    // The function is pretty fast too after any level of optimizations.

    let alloc_unit = crate::base::CHUNK_UNIT;
    let non_pexp_bins = 1 << (B::BIN_COUNT * 2 / 3).ilog2();
    let linear_bins = non_pexp_bins >> 1;

    let linear_max = alloc_unit * linear_bins;
    let trans_max = linear_max * linear_bins;
    let pexp_bin_offset = non_pexp_bins - 2;
    let pexp_bins = B::BIN_COUNT as usize - pexp_bin_offset;

    let linear_subdivs_per_exp_step = pexp_bins / (SOFT_MAX.ilog2() - trans_max.ilog2());
    let linear_subdivs_per_exp_step = 2.max(1usize << linear_subdivs_per_exp_step.ilog2());

    // Calculate the maximum extent of the pseudo-exponential region with the bins available.
    // Note that the divide loses some information here; the extra few bins will add linear subdivs.
    let pexp_max_without_linear_extra = trans_max * 1 << (pexp_bins / linear_subdivs_per_exp_step);
    // We add the extra extent due to the linear subdivs by finding the modulo.
    let pexp_max = pexp_max_without_linear_extra
        + (pexp_bins % linear_subdivs_per_exp_step) * pexp_max_without_linear_extra / linear_subdivs_per_exp_step;

    /* // This asserts in debug or "proves" in release to the compiler
    // that the size is at least `alloc_unit`.
    if size < alloc_unit {
        debug_assert!(false, "size is less than allocation unit!");
        unsafe { core::hint::unreachable_unchecked() };
    } */

    if alloc_unit * B::BIN_COUNT as usize >= SOFT_MAX {
        return ((size / alloc_unit) as u8).min(B::BIN_COUNT).wrapping_sub(1);
    }

    if size < linear_max {
        ((size / alloc_unit) as u8).wrapping_sub(1)
    } else if size < trans_max {
        let category = ilog2(size) - (ilog2(alloc_unit * linear_bins) - 1);
        let v = 1 << (ilog2(linear_bins) - category);
        (((size >> (alloc_unit.ilog2() as usize - 1 + category * 2)) ^ v) + (non_pexp_bins - 1 - v * 2)) as u8
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
        // the index into the pseudo-exponential bins.
        let bin_offset = (magnitude << pexp_div_bits) | division;

        // cap the max bin at the last bin
        let pexp_bin = bin_offset + pexp_bin_offset;

        pexp_bin as u8
    } else {
        B::BIN_COUNT - 1
    }
} */

/* pub trait L1L2PexpConfig {
    const L1_DIVS: u32;
    const L2_DIVS: u32;
    const PEXP_LIN_DIVS: u32;
}

pub const fn bin_of_size_l1_l2_pexp<B: Binning, Cfg: L1L2PexpConfig>(size: usize) -> u32 {
    // this mess determines the binning strategy used by the allocator
    // the default is to have a bin per multiple of word size from the minimum
    // chunk size up to WORD_BINED_SIZE and double word gap (sharing two sizes)
    // up to DOUBLE_BINED_SIZE, and from there on use pseudo-logarithmic sizes.

    // such sizes are as follows: begin at some power of two (DOUBLE_BINED_SIZE)
    // and increase by some power of two fraction (quarters, on 64 bit machines)
    // until reaching the next power of two, and repeat:
    // e.g. begin at 32, increase by quarters: 32, 40, 48, 56, 64, 80, 96, 112, 128, ...

    let l1_size = crate::base::CHUNK_UNIT;
    let l2_size = l1_size * 2;

    let l1_extent = l1_size * (Cfg::L1_DIVS as usize + 1);
    let l2_extent = l1_extent + l2_size * Cfg::L2_DIVS as usize;

    // Log 2 of (minimum pseudo-exponential chunk size)
    let min_exp_bits_less_1 = l2_extent.ilog2();

    // how many bits are used to determine the division
    let pexp_div_bits = Cfg::PEXP_LIN_DIVS.ilog2();

    /* // Calculate the maximum extent of the pseudo-exponential region with the bins available.
    let pexp_bins = Cfg::BIN_COUNT - Cfg::L1_DIVS - Cfg::L2_DIVS;
    // Note that the divide loses some information here; the extra few bins will add linear subdivs.
    let pexp_max_without_linear_extra = l2_extent * 1 << (pexp_bins / Cfg::PEXP_LIN_DIVS);
    // We add the extra extent due to the linear subdivs by finding the modulo.
    let pexp_max = pexp_max_without_linear_extra
        + (pexp_bins % Cfg::PEXP_LIN_DIVS) * pexp_max_without_linear_extra / Cfg::PEXP_LIN_DIVS; */

    if size < l1_extent {
        // single word separated bins

        ((size / l1_size) as u32).wrapping_sub(1)
    } else if size < l2_extent {
        // quad word separated bins

        // equiv to (size - WORD_BIN_LIMIT) / 2WORD_SIZE + DBL_BIN
        // but saves an instruction
        (size / l2_size - l1_extent / l2_size) as u32 + Cfg::L1_DIVS
    } else {
        // pseudo-exponentially separated bin

        // here's what a size is, bit by bit: 1_div_extra
        // e.g. with four divisions 1_01_00010011000
        // the bin is determined by the magnitude and the division
        // mag 0 div 0, mag 0 div 1, mag 0 div 2, mag 0 div 3, mag 1 div 0, ...

        // let shifted_size = (size)
        let bits_less_one = size.ilog2();

        // the magnitude the size belongs to.
        // calculate the difference in bit count i.e. difference in power
        let magnitude = bits_less_one - min_exp_bits_less_1;
        // the division of the magnitude the size belongs to.
        // slide the size to get the division bits at the bottom and remove the top bit
        let division = (size >> (bits_less_one - pexp_div_bits)) as u32 & !Cfg::PEXP_LIN_DIVS;
        // the index into the pseudo-exponential bins.
        let bin_offset = (magnitude << pexp_div_bits) | division;

        // cap the max bin at the last bin
        let pexp_bin = bin_offset + Cfg::L1_DIVS + Cfg::L2_DIVS;

        if pexp_bin >= B::BIN_COUNT {
            return B::BIN_COUNT - 1;
        }

        pexp_bin
    }
}

pub struct TwoUsizeBinCfg;

#[cfg(target_pointer_width = "64")]
impl L1L2PexpConfig for TwoUsizeBinCfg {
    const L1_DIVS: u32 = 55;
    const L2_DIVS: u32 = 36;
    const PEXP_LIN_DIVS: u32 = 4;
}

#[cfg(target_pointer_width = "32")]
impl L1L2PexpConfig for TwoUsizeBinCfg {
    const L1_DIVS: u32 = 27;
    const L2_DIVS: u32 = 18;
    const PEXP_LIN_DIVS: u32 = 2;
} */
