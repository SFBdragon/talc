pub trait BitField: Copy + Clone + Sized {
    const BIT_COUNT: usize;

    const INIT: Self;

    fn lowest_set_bit(&self, b: usize) -> Option<usize>;
    fn set_bit(&mut self, b: usize);
    fn clear_bit(&mut self, b: usize);
    fn read_bit(&self, b: usize) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct OneUsizeBitField(pub usize);

impl BitField for OneUsizeBitField {
    const BIT_COUNT: usize = usize::BITS as usize;

    const INIT: Self = Self(0);

    fn lowest_set_bit(&self, b: usize) -> Option<usize> {
        debug_assert!(b < Self::BIT_COUNT);

        let shifted = self.0 >> b;
        if shifted != 0 {
            Some(b + shifted.trailing_zeros() as usize)
        } else {
            None
        }
    }

    fn set_bit(&mut self, b: usize) {
        debug_assert!(b < Self::BIT_COUNT);
        debug_assert!(b & 1 << b == 0);
        self.0 |= 1 << b;
    }

    fn clear_bit(&mut self, b: usize) {
        debug_assert!(b < Self::BIT_COUNT);
        debug_assert!(b & 1 << b != 0);
        self.0 ^= 1 << b;
    }

    fn read_bit(&self, b: usize) -> bool {
        debug_assert!(b < Self::BIT_COUNT);
        self.0 & 1 << b != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct U64BitField(pub u64);

impl BitField for U64BitField {
    const BIT_COUNT: usize = u64::BITS as usize;

    const INIT: Self = Self(0);

    fn lowest_set_bit(&self, b: usize) -> Option<usize> {
        debug_assert!(b < Self::BIT_COUNT);

        let shifted = self.0 >> b;
        if shifted != 0 {
            Some(b + shifted.trailing_zeros() as usize)
        } else {
            None
        }
    }

    fn set_bit(&mut self, b: usize) {
        debug_assert!(b < Self::BIT_COUNT);
        debug_assert!(b & 1 << b == 0);
        self.0 |= 1 << b;
    }

    fn clear_bit(&mut self, b: usize) {
        debug_assert!(b < Self::BIT_COUNT);
        debug_assert!(b & 1 << b != 0);
        self.0 ^= 1 << b;
    }

    fn read_bit(&self, b: usize) -> bool {
        debug_assert!(b < Self::BIT_COUNT);
        self.0 & 1 << b != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TwoUsizeBitField(pub [usize; 2]);

impl BitField for TwoUsizeBitField {
    const BIT_COUNT: usize = usize::BITS as usize * 2;

    const INIT: Self = Self([0; 2]);

    fn lowest_set_bit(&self, b: usize) -> Option<usize> {
        debug_assert!(b < Self::BIT_COUNT);

        if b < 64 {
            let shifted = self.0[0] >> b;
            if shifted != 0 {
                Some(b + shifted.trailing_zeros() as usize)
            } else {
                if self.0[1] != 0 {
                    Some(64 + self.0[1].trailing_zeros() as usize)
                } else {
                    None
                }
            }
        } else {
            let shifted = self.0[1] >> (b - 64);
            if shifted != 0 {
                Some(b + shifted.trailing_zeros() as usize)
            } else {
                None
            }
        }
    }

    fn set_bit(&mut self, b: usize) {
        debug_assert!(b < Self::BIT_COUNT);

        if b < 64 {
            debug_assert!(self.0[0] & 1 << b == 0);
            self.0[0] |= 1 << b;
        } else {
            debug_assert!(self.0[1] & 1 << (b - 64) == 0);
            self.0[1] |= 1 << (b - 64);
        }
    }

    fn clear_bit(&mut self, b: usize) {
        debug_assert!(b < Self::BIT_COUNT);

        if b < 64 {
            debug_assert!(self.0[0] & 1 << b != 0);
            self.0[0] ^= 1 << b;
        } else {
            debug_assert!(self.0[1] & 1 << (b - 64) != 0);
            self.0[1] ^= 1 << (b - 64);
        }
    }

    fn read_bit(&self, b: usize) -> bool {
        debug_assert!(b < Self::BIT_COUNT);

        if b < 64 {
            self.0[0] & 1 << b != 0
        } else {
            self.0[1] & 1 << (b - 64) != 0
        }
    }
}
