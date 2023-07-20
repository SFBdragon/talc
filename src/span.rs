use core::ops::Range;

use crate::ALIGN;

/// Represents an interval of memory `[base, acme)`
///
/// Use `get_base_acme` to retrieve `base` and `acme` directly.
///
/// # Empty Spans
/// Note that where `base >= acme`, the [`Span`] is considered empty, in which case
/// the specific values of `base` and `acme` are considered meaningless.
/// * Most functions will no-op or return `None` for empty spans (check the docs).
/// * Empty spans contain nothing and overlap with nothing.
/// * Empty spans are contained by all sized spans.
#[derive(Clone, Copy)]
pub struct Span {
    base: usize,
    acme: usize,
}

impl Default for Span {
    fn default() -> Self {
        Self::empty()
    }
}

impl core::fmt::Debug for Span {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.get_base_acme() {
            Some((base, acme)) => f.write_fmt(format_args!("{:#x}..{:#x}", base, acme)),
            None => f.write_str("Empty Span"),
        }
    }
}

impl core::fmt::Display for Span {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.get_base_acme() {
            Some((base, acme)) => f.write_fmt(format_args!("{:#x}..{:#x}", base, acme)),
            None => f.write_str("Empty Span"),
        }
    }
}

impl From<Range<usize>> for Span {
    fn from(value: Range<usize>) -> Self {
        Self { base: value.start, acme: value.end }
    }
}

impl From<Range<*mut u8>> for Span {
    fn from(value: Range<*mut u8>) -> Self {
        Self { base: value.start as usize, acme: value.end as usize }
    }
}

impl From<*mut [u8]> for Span {
    #[inline]
    fn from(value: *mut [u8]) -> Self {
        Self {
            base: value.as_mut_ptr() as usize,
            acme: value.as_mut_ptr().wrapping_add(value.len()) as usize,
        }
    }
}

impl From<&mut [u8]> for Span {
    #[inline]
    fn from(value: &mut [u8]) -> Self {
        Self {
            base: value.as_mut_ptr() as usize,
            acme: value.as_mut_ptr().wrapping_add(value.len()) as usize,
        }
    }
}

impl PartialEq for Span {
    fn eq(&self, other: &Self) -> bool {
        self.is_empty() && other.is_empty() || self.base == other.base && self.acme == other.acme
    }
}
impl Eq for Span {}

impl Span {
    /// Returns whether `base >= acme`.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.acme <= self.base
    }

    /// Returns whether `base < acme`.
    #[inline]
    pub const fn is_sized(self) -> bool {
        !self.is_empty()
    }

    /// Returns the size of the span, else zero if `base >= span`.
    #[inline]
    pub const fn size(self) -> usize {
        if self.is_empty() { 0 } else { self.acme - self.base }
    }

    /// If `self` isn't empty, returns `(base, acme)`
    #[inline]
    pub const fn get_base_acme(self) -> Option<(usize, usize)> {
        if self.is_empty() { None } else { Some((self.base, self.acme)) }
    }

    /// Create an empty span.
    #[inline]
    pub const fn empty() -> Self {
        Self { base: 0, acme: 0 }
    }

    /// Create a new span.
    #[inline]
    pub const fn new(base: usize, acme: usize) -> Self {
        Self { base, acme }
    }

    /// Creates a `Span` given a `base` and a `size`.
    /// # Panics
    /// Panics if `base + size` overflows.
    #[inline]
    pub const fn from_base_size(base: usize, size: usize) -> Self {
        match base.checked_add(size) {
            Some(acme) => Self { base, acme },
            None => panic!("base + size overflows!"),
        }
    }

    #[inline]
    pub fn from_ptr_size(ptr: *mut u8, size: usize) -> Self {
        Self::from_base_size(ptr as usize, size)
    }

    /// Returns `None` if `self` is empty.
    #[inline]
    pub const fn to_ptr_range(self) -> Option<Range<*mut u8>> {
        if self.is_empty() { None } else { Some((self.base as *mut u8)..(self.acme as *mut u8)) }
    }

    /// Returns `None` if `self` is empty.
    #[inline]
    pub const fn to_slice(self) -> Option<*mut [u8]> {
        if self.is_empty() {
            None
        } else {
            Some(core::ptr::slice_from_raw_parts_mut(self.base as *mut u8, self.acme - self.base))
        }
    }

    /// Returns whether `self` contains `addr`.
    ///
    /// Empty spans contain nothing.
    #[inline]
    pub const fn contains(self, addr: usize) -> bool {
        // if self is empty, this always evaluates to false
        self.base <= addr && addr < self.acme
    }

    /// Returns whether `self` contains `ptr`.
    ///
    /// Empty spans contain nothing.
    #[inline]
    pub fn contains_ptr(self, ptr: *mut u8) -> bool {
        self.contains(ptr as usize)
    }

    /// Returns whether `self` contains `other`.
    ///
    /// Empty spans are contained by any span, even empty ones.
    #[inline]
    pub const fn contains_span(self, other: Span) -> bool {
        other.is_empty() || self.base <= other.base && other.acme <= self.acme
    }

    /// Returns whether some of `self` overlaps with `other`.
    ///
    /// Empty spans don't overlap with anything.
    #[inline]
    pub const fn overlaps(self, other: Span) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && !(other.base >= self.acme || self.base >= other.acme)
    }

    /// Aligns `base` upward and `acme` downward by `align_of::<usize>()`.
    #[inline]
    pub const fn word_align_inward(self) -> Self {
        if usize::MAX - self.base < ALIGN {
            Self { base: usize::MAX & !(ALIGN - 1), acme: self.acme & !(ALIGN - 1) }
        } else {
            Self { base: (self.base + (ALIGN - 1)) & !(ALIGN - 1), acme: self.acme & !(ALIGN - 1) }
        }
    }
    /// Aligns `base` downward and `acme` upward by `align_of::<usize>()`.
    #[inline]
    pub const fn word_align_outward(self) -> Self {
        if self.acme > usize::MAX - (ALIGN - 1) {
            panic!("aligning acme upward would overflow!");
        }

        Self { base: self.base & !(ALIGN - 1), acme: (self.acme + (ALIGN - 1)) & !(ALIGN - 1) }
    }

    /// Raises `base` if `base` is smaller than `min`.
    #[inline]
    pub const fn above(self, min: usize) -> Self {
        Self { base: if min > self.base { min } else { self.base }, acme: self.acme }
    }
    /// Lowers `acme` if `acme` is greater than `max`.
    #[inline]
    pub const fn below(self, max: usize) -> Self {
        Self { base: self.base, acme: if max < self.acme { max } else { self.acme } }
    }
    /// Returns a span that `other` contains by raising `base` or lowering `acme`.
    ///
    /// If `other` is empty, returns `other`.
    #[inline]
    pub const fn fit_within(self, other: Span) -> Self {
        if other.is_empty() {
            other
        } else {
            Self {
                base: if other.base > self.base { other.base } else { self.base },
                acme: if other.acme < self.acme { other.acme } else { self.acme },
            }
        }
    }
    /// Returns a span that contains `other` by extending `self`.
    ///
    /// If `other` is empty, returns `self`, as all spans contain any empty span.
    #[inline]
    pub const fn fit_over(self, other: Self) -> Self {
        if other.is_empty() {
            self
        } else {
            Self {
                base: if other.base < self.base { other.base } else { self.base },
                acme: if other.acme > self.acme { other.acme } else { self.acme },
            }
        }
    }

    /// Lower `base` by `low` and raise `acme` by `high`.
    ///
    /// Does nothing if `self` is empty.
    ///
    /// # Panics
    /// Panics if lowering `base` by `low` or raising `acme` by `high` under/overflows.
    #[inline]
    pub const fn extend(self, low: usize, high: usize) -> Self {
        if self.is_empty() {
            self
        } else {
            assert!(self.base.checked_sub(low).is_some());
            assert!(self.acme.checked_add(high).is_some());

            Self { base: self.base - low, acme: self.acme + high }
        }
    }

    /// Raise `base` by `low` and lower `acme` by `high`.
    ///
    /// If `self` is empty, `self` is returned.
    #[inline]
    pub const fn truncate(self, low: usize, high: usize) -> Span {
        if self.is_empty() {
            self
        } else {
            Self {
                // if either boundary saturates, the span will be empty thereafter, as expected
                base: self.base.saturating_add(low),
                acme: self.acme.saturating_sub(high),
            }
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

        assert!(span.word_align_inward() == Span::new(1234 + 8 - 1234 % 8, 5678 - 5678 % 8));
        assert!(span.word_align_outward() == Span::new(1234 - 1234 % 8, 5678 + 8 - 5678 % 8));

        assert!(span.above(2345) == Span::new(2345, 5678));
        assert!(span.below(7890) == Span::new(1234, 5678));
        assert!(span.below(3456) == Span::new(1234, 3456));
        assert!(span.below(0123).is_empty());
        assert!(span.above(7890).is_empty());

        assert!(span.fit_over(Span::empty()) == span);
        assert!(span.fit_within(Span::empty()).is_empty());
        assert!(span.fit_within(Span::new(0, 10000)) == span);
        assert!(span.fit_over(Span::new(0, 10000)) == Span::new(0, 10000));
        assert!(span.fit_within(Span::new(4000, 10000)) == Span::new(4000, 5678));
        assert!(span.fit_over(Span::new(4000, 10000)) == Span::new(1234, 10000));

        assert!(span.extend(1234, 1010) == Span::new(0, 5678 + 1010));
        assert!(span.truncate(1234, 1010) == Span::new(1234 + 1234, 5678 - 1010));
        assert!(span.truncate(235623, 45235772).is_empty());
    }
}
