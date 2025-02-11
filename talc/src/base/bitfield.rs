//! Different [`Binning`](crate::Binning) strategies require different numbers
//! of bins to keep track of free-list availability. Such availability is tracked
//! by a bit-field type implementing [`BitField`].
//!
//! Picking the simplest bit-field type for your needs is best for performance.
//!
//! Provided implementations:
//! - Most `uint` types; `u8`, `u16`, `u32`, `u64`, `usize` (use `[u64; 2]` instead of `u128`)
//! - `[uint; N]` arrays (intended for N = 2 or N = 3, maybe N = 4)
//! - [`TwoLevelBitField`] is intended for large numbers of bits, e.g. over 200.

mod tzcnt;

/// Trait allowing [`Talc`](crate::base::Talc) to use an arbitrary bitfield to keep
/// track of which free lists have available chunks.
pub trait BitField:
    core::fmt::Debug + Copy + Clone + Sized + Send + Sync + 'static + PartialEq + Eq
{
    /// Number of bits available in this bitfield.
    const BITS: u32;

    /// A constant initial value where all bits are unset.
    const ZEROES: Self;

    /// Find the lowest set bit at index b or greater.
    ///
    /// This is usually accomplished with a shift and a trailing-zeros-count.
    ///
    /// `b` will be less than [`BITS`](BitField::BITS).
    fn bit_scan_after(&self, b: u32) -> u32;

    /// Set the bit at index b.
    ///
    /// `b` will be less than [`BITS`](BitField::BITS).
    fn set_bit(&mut self, b: u32);

    /// Clear the bit at index b.
    ///
    /// `b` will be less than [`BITS`](BitField::BITS).
    fn clear_bit(&mut self, b: u32);

    /// Read the bit at index b.
    ///
    /// `b` will be less than [`BITS`](BitField::BITS).
    fn read_bit(&self, b: u32) -> bool;
}

macro_rules! impl_bitfield_for_integer {
    ($num:ty, $tzcnt_fn: path) => {
        /* impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let hex_digits = size_of::<$num>() * 2;
                write!(f, "{:#0z$X}", self.0, z=hex_digits)
            }
        } */

        impl BitField for $num {
            const BITS: u32 = <$num>::BITS;
            const ZEROES: Self = 0;

            #[inline(always)]
            fn bit_scan_after(&self, b: u32) -> u32 {
                $tzcnt_fn(*self >> b << b)
            }

            #[inline(always)]
            fn set_bit(&mut self, b: u32) {
                *self |= 1 << b;
            }

            #[inline(always)]
            fn clear_bit(&mut self, b: u32) {
                *self ^= 1 << b;
            }

            #[inline(always)]
            fn read_bit(&self, b: u32) -> bool {
                *self & 1 << b != 0
            }
        }
    };
}

// unsigned integer
impl_bitfield_for_integer!(u8, u8::trailing_zeros); // bsf and tzcnt don't take 8-bit registers
impl_bitfield_for_integer!(u16, u16::trailing_zeros); // LLVM already optimizes llvm.cttz.i16 well
impl_bitfield_for_integer!(u32, tzcnt::tzcnt_u32);
impl_bitfield_for_integer!(u64, tzcnt::tzcnt_u64);
impl_bitfield_for_integer!(usize, tzcnt::tzcnt_usize);

impl<const N: usize, B: BitField> BitField for [B; N] {
    const BITS: u32 = B::BITS * N as u32;
    const ZEROES: Self = [B::ZEROES; N];

    fn bit_scan_after(&self, b: u32) -> u32 {
        if N == 0 {
            0
        } else if N == 1 {
            self[0].bit_scan_after(b)
        } else if N == 2 {
            if b < B::BITS {
                let s = self[0].bit_scan_after(b);
                if s < B::BITS { s } else { self[1].bit_scan_after(0) + B::BITS }
            } else {
                self[1].bit_scan_after(b - B::BITS) + B::BITS
            }
        } else {
            let array_index = b / B::BITS;
            let bit_index =
                unsafe { self.get_unchecked(array_index as usize) }.bit_scan_after(b & (B::BITS - 1));

            if bit_index < B::BITS {
                return array_index * B::BITS + bit_index;
            }

            for array_index in (array_index + 1)..(N as u32) {
                let bit_index =
                    unsafe { self.get_unchecked(array_index as usize) }.bit_scan_after(0);

                if bit_index < B::BITS {
                    return array_index * B::BITS + bit_index;
                }
            }

            Self::BITS
        }
    }

    fn set_bit(&mut self, b: u32) {
        let array_index = b / B::BITS;
        let bit_index = b & (B::BITS - 1);

        unsafe {
            self.get_unchecked_mut(array_index as usize).set_bit(bit_index);
        }
    }

    fn clear_bit(&mut self, b: u32) {
        let array_index = b / B::BITS;
        let bit_index = b & (B::BITS - 1);

        unsafe {
            self.get_unchecked_mut(array_index as usize).clear_bit(bit_index);
        }
    }

    fn read_bit(&self, b: u32) -> bool {
        let array_index = b / B::BITS;
        let bit_index = b & (B::BITS - 1);

        unsafe { self.get_unchecked(array_index as usize).read_bit(bit_index) }
    }
}

/// A [`BitField`] implementation that uses one [`BitField`] to track the occupancy of
/// an array of [`BitField`]s.
///
/// This is faster than using an array of [`BitField`]s for `N` at least 4 or 5.
/// (Depends on bucketing strategy though.)
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TwoLevelBitField<L1B: BitField, L2B: BitField + core::fmt::UpperHex, const L1LEN: usize>
{
    l1: L1B,
    l2: [L2B; L1LEN],
}

impl<L1B: BitField, L2B: BitField + core::fmt::UpperHex, const L1LEN: usize> core::fmt::Debug
    for TwoLevelBitField<L1B, L2B, L1LEN>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::UpperHex::fmt(&self, f)
    }
}

impl<L1B: BitField, L2B: BitField + core::fmt::UpperHex, const L1LEN: usize> core::fmt::UpperHex
    for TwoLevelBitField<L1B, L2B, L1LEN>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for i in 0..L1LEN {
            if i != 0 {
                write!(f, ", ")?;
            }
            let hex_digits = core::mem::size_of::<L2B>() * 2;
            write!(
                f,
                "{}: [{}] {:#0z$X}",
                i,
                self.l1.read_bit(i as _) as usize,
                self.l2[i],
                z = hex_digits
            )?;
        }

        Ok(())
    }
}

impl<L1B: BitField, L2B: BitField + core::fmt::UpperHex, const L1LEN: usize> BitField
    for TwoLevelBitField<L1B, L2B, L1LEN>
{
    const BITS: u32 = L2B::BITS * L1LEN as u32;

    const ZEROES: Self = Self { l1: L1B::ZEROES, l2: [L2B::ZEROES; L1LEN] };

    fn bit_scan_after(&self, b: u32) -> u32 {
        if L1LEN as u32 == L1B::BITS {
            panic!(
                "To avoid an extra branch in `bit_scan_after` here, we require that L1LEN is less than L1B::BITS."
            )
        }

        let array_index = b / L2B::BITS;
        let bit_index = unsafe { self.l2.get_unchecked(array_index as usize) }
            .bit_scan_after(b & (L2B::BITS - 1));

        if bit_index < L2B::BITS {
            return array_index * L2B::BITS + bit_index;
        }

        let set_array_index = self.l1.bit_scan_after(array_index + 1);
        if set_array_index < L1B::BITS {
            let bit_index =
                unsafe { self.l2.get_unchecked(set_array_index as usize) }.bit_scan_after(0);

            debug_assert!(bit_index <= L2B::BITS);

            return set_array_index * L2B::BITS + bit_index;
        }

        Self::BITS
    }

    fn set_bit(&mut self, b: u32) {
        let array_index = b / L2B::BITS;
        let bit_index = b & (L2B::BITS - 1);

        self.l1.set_bit(array_index);
        unsafe {
            self.l2.get_unchecked_mut(array_index as usize).set_bit(bit_index);
        }
    }

    fn clear_bit(&mut self, b: u32) {
        let array_index = b / L2B::BITS;
        let bit_index = b & (L2B::BITS - 1);

        unsafe {
            self.l2.get_unchecked_mut(array_index as usize).clear_bit(bit_index);
            // Guaranteed by caller.
            if *self.l2.get_unchecked(array_index as usize) == L2B::ZEROES {
                self.l1.clear_bit(array_index);
            }
        }
    }

    fn read_bit(&self, b: u32) -> bool {
        let array_index = b / L2B::BITS;
        let bit_index = b & (L2B::BITS - 1);

        unsafe { self.l2.get_unchecked(array_index as usize).read_bit(bit_index) }
    }
}

#[cfg(test)]
mod tests {
    use super::TwoLevelBitField;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_bitfields() {
        super::test_utils::check_bitfield_properties::<u32>();
        super::test_utils::check_bitfield_properties::<u64>();
        super::test_utils::check_bitfield_properties::<usize>();

        super::test_utils::check_bitfield_properties::<[u8; 1]>();
        super::test_utils::check_bitfield_properties::<[u32; 1]>();
        super::test_utils::check_bitfield_properties::<[u64; 1]>();

        super::test_utils::check_bitfield_properties::<[u8; 2]>();
        super::test_utils::check_bitfield_properties::<[u32; 2]>();
        super::test_utils::check_bitfield_properties::<[u64; 2]>();

        super::test_utils::check_bitfield_properties::<[u8; 4]>();
        super::test_utils::check_bitfield_properties::<[u16; 5]>();

        super::test_utils::check_bitfield_properties::<TwoLevelBitField<u8, u8, 5>>();
        super::test_utils::check_bitfield_properties::<TwoLevelBitField<u32, u32, 2>>();
        super::test_utils::check_bitfield_properties::<TwoLevelBitField<u32, u64, 1>>();
    }
}

/// Contains utilities for testing [`BitField`] implementations
/// for soundness.
///
/// See [`test_utils::bitfield_tests`](crate::base::bitfield::test_utils::bitfield_tests).
pub mod test_utils {
    use super::BitField;

    /// Run a suite of tests on a [`BitField`] implementation.
    ///
    /// This doesn't guarantee correctness but it helps catch a number of possible
    /// bugs that might occur in more complicated implementations.
    pub fn check_bitfield_properties<F: BitField>() {
        set_unset::<F>();
        set_eq_set_all_unset_rest::<F>();
        bsf_zero::<F>();
        bsf::<F>();
        bsf_from_index::<F>();
        bsf_from_index_1::<F>();
        bsf_first_last::<F>();
        bsf_one_behind::<F>();
        bsf_one_behind_one_forward::<F>();
        bsf_one_behind_one_forward_one_on_point::<F>();
        bsf_one_forward::<F>();
        bsf_ones::<F>();
        bsf_ones_below::<F>();
    }

    fn set_unset<F: BitField>() {
        for i in 0..F::BITS {
            let mut bf = F::ZEROES;
            bf.set_bit(i);
            bf.clear_bit(i);
            assert_eq!(bf, F::ZEROES);
        }
    }

    fn set_eq_set_all_unset_rest<F: BitField>() {
        for i in 0..F::BITS {
            let mut bf = F::ZEROES;
            for j in 0..i {
                bf.set_bit(j);
            }

            let mut bf2 = F::ZEROES;
            for j in 0..F::BITS {
                bf2.set_bit(j);
            }
            for j in i..F::BITS {
                bf2.clear_bit(j);
            }

            assert_eq!(bf, bf2)
        }
    }

    fn bsf_zero<F: BitField>() {
        let bf = F::ZEROES;
        for i in 0..F::BITS {
            assert_eq!(bf.bit_scan_after(i), F::BITS);
        }
    }

    fn bsf<F: BitField>() {
        for i in 0..F::BITS {
            let mut bf = F::ZEROES;
            bf.set_bit(i);
            assert_eq!(bf.bit_scan_after(0), i);
        }
    }

    fn bsf_first_last<F: BitField>() {
        let mut bf = F::ZEROES;
        bf.set_bit(0);
        bf.set_bit(F::BITS - 1);

        assert_eq!(bf.bit_scan_after(0), 0);
        for i in 1..F::BITS {
            assert_eq!(bf.bit_scan_after(i), F::BITS - 1);
        }
    }

    fn bsf_one_behind<F: BitField>() {
        for i in 1..F::BITS {
            let mut bf = F::ZEROES;
            bf.set_bit(i - 1);
            assert_eq!(bf.bit_scan_after(i), F::BITS);
        }
    }
    fn bsf_one_behind_one_forward<F: BitField>() {
        for i in 1..(F::BITS - 1) {
            let mut bf = F::ZEROES;
            bf.set_bit(i - 1);
            bf.set_bit(i + 1);
            assert_eq!(bf.bit_scan_after(i), i + 1);
        }
    }
    fn bsf_one_forward<F: BitField>() {
        for i in 0..(F::BITS - 1) {
            let mut bf = F::ZEROES;
            bf.set_bit(i + 1);
            assert_eq!(bf.bit_scan_after(i), i + 1);
        }
    }
    fn bsf_one_behind_one_forward_one_on_point<F: BitField>() {
        for i in 1..(F::BITS - 1) {
            let mut bf = F::ZEROES;
            bf.set_bit(i - 1);
            bf.set_bit(i);
            bf.set_bit(i + 1);
            assert_eq!(bf.bit_scan_after(i), i);
        }
    }

    fn bsf_ones<F: BitField>() {
        for i in 0..F::BITS {
            let mut bf = F::ZEROES;

            for j in i..F::BITS {
                bf.set_bit(j);
            }
            assert_eq!(bf.bit_scan_after(0), i);
        }
    }

    fn bsf_from_index_1<F: BitField>() {
        let mut bf = F::ZEROES;
        bf.set_bit(0);
        for i in 1..F::BITS {
            assert_eq!(bf.bit_scan_after(i), F::BITS);
        }
    }

    fn bsf_ones_below<F: BitField>() {
        for i in 0..F::BITS {
            let mut bf = F::ZEROES;
            for j in 0..i {
                bf.set_bit(j);
            }

            assert_eq!(bf.bit_scan_after(i), F::BITS);
        }
    }

    fn bsf_from_index<F: BitField>() {
        for i in 0..F::BITS {
            let mut bf = F::ZEROES;

            for j in i..F::BITS {
                bf.set_bit(j);
            }

            for j in 0..F::BITS {
                assert_eq!(bf.bit_scan_after(j), i.max(j));
            }
        }
    }
}
