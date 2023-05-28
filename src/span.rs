use core::ops::Range;

use crate::{align_down, align_up};

#[derive(Debug, Clone, Copy, Default)]
pub struct Span {
    pub base: isize,
    pub acme: isize,
}

impl From<Range<*mut u8>> for Span {
    fn from(value: Range<*mut u8>) -> Self {
        Self { base: value.start as isize, acme: value.end as isize }
    }
}

impl From<Range<isize>> for Span {
    fn from(value: Range<isize>) -> Self {
        Self { base: value.start, acme: value.end }
    }
}

impl From<*mut [u8]> for Span {
    #[inline]
    fn from(value: *mut [u8]) -> Self {
        Self { 
            base: value.as_mut_ptr() as isize, 
            acme: value.as_mut_ptr().wrapping_add(value.len())  as isize
        }
    }
}

impl From<&mut [u8]> for Span {
    #[inline]
    fn from(value: &mut [u8]) -> Self {
        Self { 
            base: value.as_mut_ptr() as isize, 
            acme: value.as_mut_ptr().wrapping_add(value.len()) as isize
        }
    }
}

impl PartialEq for Span {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        (self.is_empty() && other.is_empty()) 
        || (self.base == other.base && self.acme == other.acme)
    }
}
impl Eq for Span {}

impl Span {
    pub const fn empty() -> Self {
        Self { base: 0, acme: 0 }
    }

    pub const fn new(base: isize, acme: isize) -> Self {
        Self { base, acme }
    }

    pub const fn from_base_size(base: isize, size: usize) -> Self {
        Self {
            base,
            acme: base + size as isize,
        }
    }

    pub fn from_ptr_size(ptr: *mut u8, size: usize) -> Self {
        Self {
            base: ptr as isize,
            acme: ptr.wrapping_add(size) as isize,
        }
    }

    pub fn from_ptr_slice(slice: *mut [u8]) -> Self {
        slice.into()
    }
    pub fn from_ptr_range(range: Range<*mut u8>) -> Self {
        range.into()
    }

    pub const fn base_ptr(&self) -> *mut u8 {
        self.base as *mut u8
    }
    pub const fn acme_ptr(&self) -> *mut u8 {
        self.acme as *mut u8
    }

    pub const fn to_ptr_range(&self) -> Range<*mut u8> {
        Range { start: self.base_ptr(), end: self.acme_ptr() }
    }

    pub const fn to_slice(&self) -> *mut [u8] {
        core::ptr::slice_from_raw_parts_mut(self.base_ptr(), self.size())
    }

    
    pub const fn size(&self) -> usize {
        if self.acme > self.base {
            (self.acme - self.base) as usize
        } else {
            0
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.base >= self.acme
    }

    /// Returns whether `self` contains `other`.
    /// 
    /// Signed comparisons are used on the addresses.
    pub const fn contains(&self, other: isize) -> bool {
        self.base <= other as isize && (other as isize) < self.acme
    }

    /// Returns whether `self` contains `other`.
    /// 
    /// Signed comparisons are used on the addresses.
    pub fn contains_ptr(&self, other: *mut u8) -> bool {
        self.base <= (other as isize) && (other as isize) < self.acme
    }

    /// Returns whether `self` contains `other`.
    /// 
    /// Empty spans are contained by any span.
    pub const fn contains_span(&self, other: Span) -> bool {
        other.is_empty() || (self.base <= other.base && other.acme <= self.acme)
    }

    /// Returns whether some of `self` overlaps some of `other`.
    /// 
    /// Empty spans don't overlap with anything.
    pub const fn overlaps(&self, other: Span) -> bool {
        if self.is_empty() || other.is_empty() {
            false
        } else {
            !(self.base >= other.acme || other.base >= self.acme)
        }
    }

    /// Aligns `base` upward and `acme` downward.
    /// 
    /// ### Panics
    /// Panics if `align` is not a power of two.
    pub const fn align_inward(self, align: usize) -> Self {
        assert!(align.count_ones() == 1, "align is not a power of two.");

        Self {
            base: align_up  (self.base, align),
            acme: align_down(self.acme, align),
        }
    }
    /// Aligns `base` downward and `acme` upward.
    /// 
    /// ### Panics
    /// Panics if `align` is not a power of two.
    pub const fn align_outward(self, align: usize) -> Self {
        assert!(align.count_ones() == 1, "align is not a power of two.");

        Self {
            base: align_down(self.base, align),
            acme: align_up  (self.acme, align),
        }
    }

    #[inline]
    pub const fn above(self, min: isize) -> Self {
        Self {
            base: if min > self.base { min } else { self.base },
            acme: self.acme,
        }
    }
    #[inline]
    pub const fn below(self, max: isize) -> Self {
        Self {
            base: self.base,
            acme: if max < self.acme { max } else { self.acme },
        }
    }
    #[inline]
    pub const fn within(self, other: Span) -> Self {
        Self {
            base: if other.base > self.base { other.base } else { self.base },
            acme: if other.acme < self.acme { other.acme } else { self.acme },
        }
    }

    #[inline]
    pub const fn extend(self, low: usize, high: usize) -> Self {
        Self {
            base: self.base - low as isize,
            acme: self.acme + high as isize,
        }
    }
}

impl core::fmt::Display for Span {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("{:p}..{:p}", self.base_ptr(), self.acme_ptr()))
    }
}
