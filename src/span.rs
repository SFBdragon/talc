use core::ops::Range;

use crate::ALIGN;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Empty,
    Sized { base: isize, acme: isize },
}


impl Default for Span {
    fn default() -> Self {
        Self::Empty
    }
}

impl core::fmt::Debug for Span {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Span::Empty => f.write_str("Empty Span"),
            Span::Sized { base, acme } => 
                f.write_fmt(format_args!("{:#x}..{:#x}", base, acme)),
        }
    }
}

impl core::fmt::Display for Span {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Span::Empty => f.write_str("Empty Span"),
            Span::Sized { base, acme } => 
                f.write_fmt(format_args!("{:#x}..{:#x}", base, acme)),
        }
    }
}


impl From<Range<isize>> for Span {
    fn from(value: Range<isize>) -> Self {
        if value.end > value.start {
            Self::Sized { base: value.start, acme: value.end }
        } else {
            Self::Empty
        }
    }
}

impl From<Range<*mut u8>> for Span {
    fn from(value: Range<*mut u8>) -> Self {
        if value.end as isize > value.start as isize {
            Self::Sized { base: value.start as isize, acme: value.end as isize }
        } else {
            Self::Empty
        }
    }
}

impl From<*mut [u8]> for Span {
    #[inline]
    fn from(value: *mut [u8]) -> Self {
        if value.len() > 0 {
            Self::Sized { 
                base: value.as_mut_ptr() as isize, 
                acme: value.as_mut_ptr().wrapping_add(value.len()) as isize,
            }
        } else {
            Self::Empty
        }
    }
}

impl From<&mut [u8]> for Span {
    #[inline]
    fn from(value: &mut [u8]) -> Self {
        if value.len() > 0 {
            Self::Sized { 
                base: value.as_mut_ptr() as isize, 
                acme: value.as_mut_ptr().wrapping_add(value.len()) as isize,
            }
        } else {
            Self::Empty
        }
    }
}

impl Span {
    #[inline]
    pub const fn new(base: isize, acme: isize) -> Self {
        if acme - base > 0 {
            Self::Sized { base, acme }
        } else {
            Self::Empty
        }
    }

    /// If the `base` is greater than `acme`, returns a span with the given
    /// `base` and a size of zero. 
    #[inline]
    pub const fn from_base_size(base: isize, size: usize) -> Self {
        if size != 0 {
            Self::Sized { base, acme: base + size as isize }
        } else {
            Self::Empty
        }
    }

    #[inline]
    pub fn from_ptr_size(ptr: *mut u8, size: usize) -> Self {
        Self::from_base_size(ptr as isize, size)
    }

    #[inline]
    pub fn from_ptr_slice(slice: *mut [u8]) -> Self {
        slice.into()
    }
    #[inline]
    pub fn from_ptr_range(range: Range<*mut u8>) -> Self {
        range.into()
    }

    

    #[inline]
    pub const fn to_ptr_range(self) -> Option<Range<*mut u8>> {
        match self {
            Span::Empty => None,
            Span::Sized { base, acme } => Some((base as *mut u8)..(acme as *mut u8)),
        }
    }

    #[inline]
    pub const fn to_slice(self) -> Option<*mut [u8]> {
        match self {
            Span::Empty => None,
            Span::Sized { base, acme } => 
                Some(core::ptr::slice_from_raw_parts_mut(base as *mut u8, (acme - base) as usize)),
        }
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        matches!(self, Self::Empty)
    }

    #[inline]
    pub const fn size(self) -> usize {
        match self {
            Span::Empty => 0,
            Span::Sized { base, acme } => (acme - base) as usize,
        }
    }

    /// Returns whether `self` contains `other`.
    /// 
    /// Signed comparisons are used.
    #[inline]
    pub const fn contains(self, addr: isize) -> bool {
        match self {
            Span::Empty => false,
            Span::Sized { base, acme } => base <= addr && addr < acme,
        }
    }

    /// Returns whether `self` contains `other`.
    /// 
    /// Signed comparisons are used.
    #[inline]
    pub fn contains_ptr(self, ptr: *mut u8) -> bool {
        self.contains(ptr as isize)
    }

    /// Returns whether `self` contains `other`.
    /// 
    /// Empty spans are contained by any span, even empty ones.
    #[inline]
    pub const fn contains_span(self, span: Span) -> bool {
        match span {
            Span::Empty => true,
            Span::Sized { base: other_base, acme: other_acme } => {
                match self {
                    Span::Empty => false,
                    Span::Sized { base, acme } => {
                        base <= other_base && other_acme <= acme
                    },
                }
            },
        }
    }

    /// Returns whether some of `self` overlaps some of `other`.
    /// 
    /// Empty spans don't overlap with anything.
    #[inline]
    pub const fn overlaps(self, span: Span) -> bool {
        match span {
            Span::Empty => false,
            Span::Sized { base: other_base, acme: other_acme } => {
                match self {
                    Span::Empty => false,
                    Span::Sized { base, acme } => {
                        !(other_base >= acme || base >= other_acme)
                    },
                }
            },
        }
    }

    /// Aligns `base` upward and `acme()` downward by `align_of::<usize>()`.
    #[inline]
    pub const fn word_align_inward(self) -> Self {
        match self {
            Span::Empty => Self::Empty,
            Span::Sized { base, acme } => {
                let base =  ((base + (ALIGN as isize - 1)) as usize & !(ALIGN - 1)) as isize;
                let acme =  ( acme                         as usize & !(ALIGN - 1)) as isize;

                if acme > base {
                    Self::Sized { base, acme }
                } else {
                    Self::Empty
                }
            },
        }
    }
    /// Aligns `base` downward and `acme` upward by `align_of::<usize>()`.
    #[inline]
    pub const fn word_align_outward(self) -> Self {
        match self {
            Span::Empty => Self::Empty,
            Span::Sized { base, acme } => Self::Sized {
                base: ( base                         as usize & !(ALIGN - 1)) as isize,
                acme: ((acme + (ALIGN as isize - 1)) as usize & !(ALIGN - 1)) as isize,
            },
        }
    }

    /// Raises `base` if `base` is smaller than `min`.
    #[inline]
    pub const fn above(self, min: isize) -> Self {
        match self {
            Span::Sized { base, acme } if acme > min => Self::Sized {
                base: if base < min { min } else { base },
                acme,
            },
            _ => Self::Empty,
        }
    }
    /// Lowers `acme` if `acme` is greater than `max`.
    #[inline]
    pub const fn below(self, max: isize) -> Self {
        match self {
            Span::Sized { base, acme } if max > base => Self::Sized {
                base,
                acme: if acme > max { max } else { acme },
            },
            _ => Self::Empty,
        }
    }
    /// Returns a span that `other` contains by raising `base` or lowering `acme`.
    /// 
    /// If `other` is empty, returns `other`.
    #[inline]
    pub const fn fit_within(self, span: Span) -> Self {
        match span {
            Span::Empty => Self::Empty,
            Span::Sized { base: other_base, acme: other_acme } => {
                match self {
                    Span::Empty => Self::Empty,
                    Span::Sized { base, acme } => {
                        Self::Sized {
                            base: if other_base > base { other_base } else { base },
                            acme: if other_acme < acme { other_acme } else { acme },
                        }
                    },
                }
            },
        }
    }
    /// Returns a span that contains `other` by extending `self`.
    /// 
    /// If `other` is empty, returns `self`, as all spans contain any empty span.
    #[inline]
    pub const fn fit_over(self, span: Self) -> Self {
        match span {
            Span::Empty => self,
            Span::Sized { base: other_base, acme: other_acme } => {
                match self {
                    Span::Empty => span,
                    Span::Sized { base, acme } => {
                        Self::Sized {
                            base: if other_base < base { other_base } else { base },
                            acme: if other_acme > acme { other_acme } else { acme },
                        }
                    },
                }
            },
        }
    }

    /// Lower `base` by `low` and raise `acme` by `high`.
    #[inline]
    pub const fn extend(self, low: usize, high: usize) -> Self {
        match self {
            Span::Empty => self,
            Span::Sized { base, acme } => Self::Sized {
                base: base.wrapping_sub_unsigned(low), 
                acme: acme.wrapping_add_unsigned(high),
            },
        }
    }

    /// Raise `base` by `low` and lower `size` by `low + high` (zero, if this underflows).
    #[inline]
    pub const fn truncate(self, low: usize, high: usize) -> Span {
        match self {
            Span::Empty => self,
            Span::Sized { base, acme } => {
                if (acme - base) as usize > low + high {
                    Self::Sized {
                        base: base.wrapping_add_unsigned(low), 
                        acme: acme.wrapping_sub_unsigned(high),
                    }
                } else {
                    Self::Empty
                }
            },
        }
    }
}


#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_span() {
        let span = Span::from(1234..5678);
        assert!(!span.is_empty());
        assert!(span.size() == 5678 - 1234);

        assert!(span.word_align_inward() == Span::Sized { base: 1234 + 8 - 1234 % 8, acme: 5678 - 5678 % 8 });
        assert!(span.word_align_outward() == Span::Sized { base: 1234 - 1234 % 8, acme: 5678 + 8 - 5678 % 8 });

        assert!(span.above(2345) == Span::Sized { base: 2345, acme: 5678 });
        assert!(span.below(7890) == Span::Sized { base: 1234, acme: 5678 });
        assert!(span.below(3456) == Span::Sized { base: 1234, acme: 3456 });
        assert!(span.below(0123) == Span::Empty);
        assert!(span.above(7890) == Span::Empty);

        assert!(span.fit_over(Span::Empty) == span);
        assert!(span.fit_within(Span::Empty) == Span::Empty);
        assert!(span.fit_within(Span::Sized { base: 0, acme: 10000 }) == span);
        assert!(span.fit_over(Span::Sized { base: 0, acme: 10000 }) == Span::Sized { base: 0, acme: 10000 });
        assert!(span.fit_within(Span::Sized { base: 4000, acme: 10000 }) == Span::Sized { base: 4000, acme: 5678 });
        assert!(span.fit_over(Span::Sized { base: 4000, acme: 10000 }) == Span::Sized { base: 1234, acme: 10000 });

        assert!(span.extend(1234, 1010) == Span::Sized { base: 0, acme: 5678 + 1010 });
        assert!(span.truncate(1234, 1010) == Span::Sized { base: 1234 + 1234, acme: 5678 - 1010 });
        assert!(span.truncate(235623, 45235772) == Span::Empty);
        assert!(span.extend(235623, 45235772) == Span::Sized { base: 1234 - 235623, acme: 5678 + 45235772 });
    }
}
