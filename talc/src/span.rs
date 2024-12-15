use core::ops::Range;

use crate::ptr_utils::{align_down_by, align_up_by};

/// Represents an interval of memory `[base, acme)`
///
/// Use `get_base_acme` to retrieve `base` and `acme` directly.
///
/// # Empty Spans
/// Note that where `base >= acme`, the [`Span`] is empty, in which case
/// the specific values of `base` and `acme` are considered meaningless.
/// * Empty spans contain nothing and overlap with nothing.
/// * Empty spans are contained by any sized span.
#[derive(Clone, Copy, Hash)]
pub struct Span {
    base: *mut u8,
    acme: *mut u8,
}

unsafe impl Send for Span {}

impl Default for Span {
    fn default() -> Self {
        Self::empty()
    }
}

impl core::fmt::Debug for Span {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("{:p}..[{}]..{:p}", self.base, self.size(), self.acme))
    }
}

impl core::fmt::Display for Span {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.get_base_acme() {
            Some((base, acme)) => f.write_fmt(format_args!("{:p}..{:p}", base, acme)),
            None => f.write_str("Empty Span"),
        }
    }
}

impl<T> From<Range<*mut T>> for Span {
    fn from(value: Range<*mut T>) -> Self {
        Self { base: value.start.cast(), acme: value.end.cast() }
    }
}

impl<T> From<Range<*const T>> for Span {
    fn from(value: Range<*const T>) -> Self {
        Self { base: value.start.cast_mut().cast(), acme: value.end.cast_mut().cast() }
    }
}

impl<T> From<&mut [T]> for Span {
    fn from(value: &mut [T]) -> Self {
        Self::from(value.as_mut_ptr_range())
    }
}

impl<T, const N: usize> From<&mut [T; N]> for Span {
    fn from(value: &mut [T; N]) -> Self {
        Self::from(value as *mut [T; N])
    }
}

impl<T> From<*mut [T]> for Span {
    fn from(value: *mut [T]) -> Self {
        Self::from_slice(value)
    }
}

impl<T, const N: usize> From<*mut [T; N]> for Span {
    fn from(value: *mut [T; N]) -> Self {
        Self::from_array(value)
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
    pub fn is_empty(self) -> bool {
        self.acme <= self.base
    }

    /// Returns whether `base < acme`.
    #[inline]
    pub fn is_sized(self) -> bool {
        !self.is_empty()
    }

    /// Returns the size of the span, else zero if `base >= span`.
    #[inline]
    pub fn size(self) -> usize {
        if self.is_empty() { 0 } else { self.acme as usize - self.base as usize }
    }

    /// If `self` isn't empty, returns `(base, acme)`
    #[inline]
    pub fn get_base_acme(self) -> Option<(*mut u8, *mut u8)> {
        if self.is_empty() { None } else { Some((self.base, self.acme)) }
    }

    /// Create an empty span.
    #[inline]
    pub const fn empty() -> Self {
        Self { base: core::ptr::null_mut(), acme: core::ptr::null_mut() }
    }

    /// Create a new span.
    #[inline]
    pub const fn new(base: *mut u8, acme: *mut u8) -> Self {
        Self { base, acme }
    }

    /// Creates a [`Span`] given a `base` and a `size`.
    ///
    /// If `base + size` overflows, the result is empty.
    #[inline]
    pub const fn from_base_size(base: *mut u8, size: usize) -> Self {
        Self { base, acme: base.wrapping_add(size) }
    }

    #[inline]
    pub const fn from_slice<T>(slice: *mut [T]) -> Self {
        Self {
            base: slice as *mut T as *mut u8,
            // SAFETY: pointing directly after an object is considered
            // within the same object
            acme: unsafe { (slice as *mut T).add(slice.len()).cast() },
        }
    }

    #[inline]
    pub const fn from_array<T, const N: usize>(array: *mut [T; N]) -> Self {
        Self {
            base: array as *mut T as *mut u8,
            // SAFETY: pointing directly after an object is considered
            // within the same object
            acme: unsafe { (array as *mut T).add(N).cast() },
        }
    }

    /// Returns `None` if `self` is empty.
    #[inline]
    pub fn to_ptr_range(self) -> Option<Range<*mut u8>> {
        if self.is_empty() { None } else { Some(self.base..self.acme) }
    }

    /// Returns `None` if `self` is empty.
    #[inline]
    pub fn to_slice(self) -> Option<*mut [u8]> {
        if self.is_empty() {
            None
        } else {
            Some(core::ptr::slice_from_raw_parts_mut(self.base, self.size()))
        }
    }

    /// Returns whether `self` contains `addr`.
    ///
    /// Empty spans contain nothing.
    #[inline]
    pub fn contains(self, ptr: *mut u8) -> bool {
        // if self is empty, this always evaluates to false
        self.base <= ptr && ptr < self.acme
    }

    /// Returns whether `self` contains `other`.
    ///
    /// Empty spans are contained by any span, even empty ones.
    #[inline]
    pub fn contains_span(self, other: Span) -> bool {
        other.is_empty() || self.base <= other.base && other.acme <= self.acme
    }

    /// Returns whether some of `self` overlaps with `other`.
    ///
    /// Empty spans don't overlap with anything.
    #[inline]
    pub fn overlaps(self, other: Span) -> bool {
        self.is_sized() && other.is_sized() && !(other.base >= self.acme || self.base >= other.acme)
    }

    /// Raises `base` if `base` is smaller than `min`.
    #[inline]
    pub fn above(self, min: *mut u8) -> Self {
        Self { base: if min > self.base { min } else { self.base }, acme: self.acme }
    }
    /// Lowers `acme` if `acme` is greater than `max`.
    #[inline]
    pub fn below(self, max: *mut u8) -> Self {
        Self { base: self.base, acme: if max < self.acme { max } else { self.acme } }
    }

    /// Returns the [`Span`]s of `self` below and above the `exclude` span, respectively.
    /// Alternatively worded, the set difference `self`\\`exclude`.
    /// 
    /// If `exclude` is empty, `self` and an empty `Span` are returned.
    #[inline]
    pub fn except(self, exclude: Span) -> (Self, Self) {
        match exclude.get_base_acme() {
            Some((base, acme)) => (self.below(base), self.above(acme)),
            None => (self, Span::empty()),
        }
    }

    /// Returns a span that `other` contains by raising `base` or lowering `acme`.
    ///
    /// If `other` is empty, returns `other`.
    #[inline]
    pub fn fit_within(self, other: Span) -> Self {
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
    pub fn fit_over(self, other: Self) -> Self {
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
    pub fn extend(self, low: usize, high: usize) -> Self {
        if self.is_empty() {
            self
        } else {
            assert!((self.base as usize).checked_sub(low).is_some());
            assert!((self.acme as usize).checked_add(high).is_some());

            Self { base: self.base.wrapping_sub(low), acme: self.acme.wrapping_add(high) }
        }
    }

    /// Raise `base` by `low` and lower `acme` by `high`.
    ///
    /// If `self` is empty, `self` is returned.
    ///
    /// If either operation would wrap around the address space, an empty span is returned.
    #[inline]
    pub fn truncate(self, low: usize, high: usize) -> Span {
        if self.is_empty() {
            self
        } else if (self.base as usize).checked_add(low).is_none()
            || (self.acme as usize).checked_sub(high).is_none()
        {
            Span::empty()
        } else {
            Self {
                // if either boundary saturates, the span will be empty thereafter, as expected
                base: self.base.wrapping_add(low),
                acme: self.acme.wrapping_sub(high),
            }
        }
    }

    /// Aligns the boundaries of the `Span` inwards.
    #[inline]
    pub fn align_inwards(self, alignment: usize) -> Span {
        if (self.base as usize).checked_add(alignment - 1).is_none() {
            Span::empty()
        } else {
            Span {
                base: align_up_by(self.base, alignment - 1),
                acme: align_down_by(self.acme, alignment - 1),
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn ptr(addr: usize) -> *mut u8 {
        // don't ` as usize` to avoid upsetting miri too much
        core::ptr::null_mut::<u8>().wrapping_add(addr)
    }

    #[test]
    fn test_span() {
        let base = 1234usize;
        let acme = 5678usize;

        let bptr = ptr(base);
        let aptr = ptr(acme);

        let span = Span::from(bptr..aptr);
        assert!(!span.is_empty());
        assert!(span.size() == acme - base);

        assert_eq!(span.above(ptr(2345)), Span::new(ptr(2345), aptr));
        assert_eq!(span.below(ptr(7890)), Span::new(bptr, aptr));
        assert_eq!(span.below(ptr(3456)), Span::new(bptr, ptr(3456)));
        assert_eq!(span.below(ptr(0123)), Span::empty());
        assert_eq!(span.above(ptr(7890)), Span::empty());

        assert_eq!(
            span.except(Span::new(ptr(base + 1111), ptr(acme - 1111))),
            (Span::new(bptr, ptr(base + 1111)), Span::new(ptr(acme - 1111), aptr))
        );
        assert_eq!(
            span.except(Span::new(ptr(base + 1111), ptr(acme + 1111))),
            (Span::new(bptr, ptr(base + 1111)), Span::empty())
        );
        assert_eq!(
            span.except(Span::new(ptr(base - 1111), ptr(acme + 1111))),
            (Span::empty(), Span::empty())
        );
        assert_eq!(span.except(Span::empty()), (span, Span::empty()));

        assert!(span.fit_over(Span::empty()) == span);
        assert!(span.fit_within(Span::empty()).is_empty());
        assert!(span.fit_within(Span::new(ptr(0), ptr(10000))) == span);
        assert!(span.fit_over(Span::new(ptr(0), ptr(10000))) == Span::new(ptr(0), ptr(10000)));
        assert!(span.fit_within(Span::new(ptr(4000), ptr(10000))) == Span::new(ptr(4000), aptr));
        assert!(span.fit_over(Span::new(ptr(4000), ptr(10000))) == Span::new(bptr, ptr(10000)));

        assert!(span.extend(1234, 1010) == Span::new(ptr(0), ptr(5678 + 1010)));
        assert!(span.truncate(1234, 1010) == Span::new(ptr(1234 + 1234), ptr(5678 - 1010)));
        assert!(span.truncate(235623, 45235772).is_empty());
    }
}
