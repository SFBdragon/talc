use core::ptr::NonNull;

/// Describes a linked list node.
///
/// # Safety:
/// `LlistNode`s are inherently unsafe due to the referencial dependency between nodes. This requires
/// that `LlistNode`s are never moved manually, otherwise using the list becomes memory
/// unsafe and may lead to undefined behaviour.
///
/// This data structure is not thread-safe, use mutexes/locks to mutually exclude data access.
#[derive(Debug)]
#[repr(C)]
pub struct LlistNode {
    pub next: Option<NonNull<LlistNode>>,
    pub next_of_prev: *mut Option<NonNull<LlistNode>>,
}

impl LlistNode {
    #[inline]
    pub fn next_ptr(ptr: *mut Self) -> *mut Option<NonNull<LlistNode>> {
        ptr.cast() /* .cast::<u8>().wrapping_add(core::mem::offset_of!(LlistNode, next)) */
    }

    /// Create a new node as a member of an existing linked list at `node`.
    ///
    /// Warning: This will not call `remove` on `node`, regardless of initialization.
    /// It is your responsibility to make sure `node` gets `remove`d if necessary.
    /// Failing to do so when is not undefined behaviour or memory unsafe, but
    /// may cause unexpected linkages.
    ///
    /// # Safety
    /// * `node` must be `ptr::write`-able.
    /// * `next_of_prev` must be dereferencable and valid.
    pub unsafe fn insert(
        node: *mut Self,
        next_of_prev: *mut Option<NonNull<LlistNode>>,
        next: Option<NonNull<LlistNode>>,
    ) {
        debug_assert!(!node.is_null());
        debug_assert!(!next_of_prev.is_null());

        node.write(Self { next_of_prev, next });

        *next_of_prev = Some(NonNull::new_unchecked(node));

        if let Some(next) = next {
            (*next.as_ptr()).next_of_prev = Self::next_ptr(node);
        }
    }

    /// Remove `node` from it's linked list.
    ///
    /// Note that this does not modify `node`; it should be considered invalid.
    ///
    /// # Safety
    /// * `self` must be dereferencable and valid.
    pub unsafe fn remove(node: *mut Self) {
        debug_assert!(!node.is_null());
        let LlistNode { next, next_of_prev } = node.read();

        debug_assert!(!next_of_prev.is_null());
        *next_of_prev = next;

        if let Some(next) = next {
            (*next.as_ptr()).next_of_prev = next_of_prev;
        }
    }

    /// Creates an iterator over the circular linked list, exclusive of
    /// the sentinel.
    /// # Safety
    /// `start`'s linked list must remain in a valid state during iteration.
    /// Modifying `LlistNode`s already returned by the iterator is okay.
    pub unsafe fn iter_mut(first: Option<NonNull<Self>>) -> IterMut {
        IterMut::new(first)
    }
}

/// An iterator over the circular linked list `LlistNode`s, excluding the 'head'.
///
/// This `struct` is created by `LlistNode::iter_mut`. See its documentation for more.
#[derive(Debug, Clone, Copy)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct IterMut(Option<NonNull<LlistNode>>);

impl IterMut {
    /// Create a new iterator over the linked list from `first`.
    pub unsafe fn new(first: Option<NonNull<LlistNode>>) -> Self {
        Self(first)
    }
}

impl Iterator for IterMut {
    type Item = NonNull<LlistNode>;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.0?;
        self.0 = unsafe { (*current.as_ptr()).next };
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::null_mut;

    use super::*;

    #[test]
    fn test_llist() {
        unsafe {
            let x = Box::into_raw(Box::new(LlistNode { next: None, next_of_prev: null_mut() }));
            let y = Box::into_raw(Box::new(LlistNode { next: None, next_of_prev: null_mut() }));
            let z = Box::into_raw(Box::new(LlistNode { next: None, next_of_prev: null_mut() }));

            LlistNode::insert(y, LlistNode::next_ptr(x), None);
            LlistNode::insert(z, LlistNode::next_ptr(x), Some(NonNull::new(y).unwrap()));

            let mut iter = LlistNode::iter_mut(Some(NonNull::new(x)).unwrap());
            assert!(iter.next().is_some_and(|n| n.as_ptr() == x));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == z));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == y));
            assert!(iter.next().is_none());

            let mut iter = LlistNode::iter_mut(Some(NonNull::new(y).unwrap()));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == y));
            assert!(iter.next().is_none());

            LlistNode::remove(z);

            let mut iter = LlistNode::iter_mut(Some(NonNull::new(x).unwrap()));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == x));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == y));
            assert!(iter.next().is_none());

            LlistNode::insert(z, LlistNode::next_ptr(x), Some(NonNull::new(y).unwrap()));

            let mut iter = LlistNode::iter_mut(Some(NonNull::new(x).unwrap()));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == x));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == z));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == y));
            assert!(iter.next().is_none());

            LlistNode::remove(z);
            LlistNode::remove(y);

            let mut iter = LlistNode::iter_mut(Some(NonNull::new(x).unwrap()));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == x));
            assert!(iter.next().is_none());

            drop(Box::from_raw(x));
            drop(Box::from_raw(y));
            drop(Box::from_raw(z));
        }
    }
}
