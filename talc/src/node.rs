use core::ptr::NonNull;

/// Describes a linked list node.
///
/// # Safety:
/// `LlistNode`s are inherently unsafe due to the referential dependency between nodes. This requires
/// that `LlistNode`s are never moved manually, otherwise using the list becomes memory
/// unsafe and may lead to undefined behavior.
///
/// This data structure is not thread-safe, use mutexes/locks to mutually exclude data access.
#[derive(Debug)]
#[repr(C)]
pub(crate) struct Node {
    pub next: Option<NonNull<Node>>,
    pub next_of_prev: *mut Option<NonNull<Node>>,
}

impl Node {
    #[inline]
    pub fn addr_of_next(ptr: *mut Self) -> *mut Option<NonNull<Self>> {
        ptr.cast() /* .cast::<u8>().wrapping_add(core::mem::offset_of!(LlistNode, next)) */
    }

    /// Create a new node as a member of an existing linked list at `node`.
    #[inline]
    pub unsafe fn link_at(ptr: *mut Self, data: Self) {
        debug_assert!(!ptr.is_null());
        debug_assert!(!data.next_of_prev.is_null());

        *data.next_of_prev = Some(NonNull::new_unchecked(ptr));

        if let Some(next) = data.next {
            (*next.as_ptr()).next_of_prev = Self::addr_of_next(ptr);
        }

        ptr.write(data);
    }

    /// Remove `node` from it's linked list.
    #[inline]
    pub unsafe fn unlink(self) {
        let Node { next, next_of_prev } = self;
        debug_assert!(!next_of_prev.is_null());

        *next_of_prev = next;

        if let Some(next) = next {
            (*next.as_ptr()).next_of_prev = next_of_prev;
        }
    }

    /// Creates an iterator over the linked list from the specified node.
    #[inline]
    pub unsafe fn iter_mut(first: Option<NonNull<Self>>) -> IterMut {
        IterMut::new(first)
    }
}

/// An iterator over the circular linked list `LlistNode`s, excluding the 'head'.
///
/// This `struct` is created by `LlistNode::iter_mut`. See its documentation for more.
#[derive(Debug)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub(crate) struct IterMut(Option<NonNull<Node>>);

impl IterMut {
    /// Create a new iterator over the linked list from `first`.
    pub unsafe fn new(first: Option<NonNull<Node>>) -> Self {
        Self(first)
    }
}

impl Iterator for IterMut {
    type Item = NonNull<Node>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.0?;
        self.0 = unsafe { (*current.as_ptr()).next };
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use core::mem::MaybeUninit;

    use super::*;

    #[test]
    fn test_node() {
        unsafe {
            let x = Box::into_raw(Box::new(MaybeUninit::<Node>::uninit())).cast::<Node>();
            let y = Box::into_raw(Box::new(MaybeUninit::<Node>::uninit())).cast::<Node>();
            let z = Box::into_raw(Box::new(MaybeUninit::<Node>::uninit())).cast::<Node>();

            Node::link_at(y, Node { next: None, next_of_prev: Node::addr_of_next(x) });
            Node::link_at(z, Node {
                next: Some(NonNull::new(y).unwrap()),
                next_of_prev: Node::addr_of_next(x),
            });

            let mut iter = Node::iter_mut(Some(NonNull::new(x)).unwrap());
            assert!(iter.next().is_some_and(|n| n.as_ptr() == x));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == z));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == y));
            assert!(iter.next().is_none());

            let mut iter = Node::iter_mut(Some(NonNull::new(y).unwrap()));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == y));
            assert!(iter.next().is_none());

            Node::unlink(z.read());

            let mut iter = Node::iter_mut(Some(NonNull::new(x).unwrap()));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == x));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == y));
            assert!(iter.next().is_none());

            Node::link_at(z, Node {
                next: Some(NonNull::new(y).unwrap()),
                next_of_prev: Node::addr_of_next(x),
            });

            let mut iter = Node::iter_mut(Some(NonNull::new(x).unwrap()));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == x));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == z));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == y));
            assert!(iter.next().is_none());

            Node::unlink(z.read());
            Node::unlink(y.read());

            let mut iter = Node::iter_mut(Some(NonNull::new(x).unwrap()));
            assert!(iter.next().is_some_and(|n| n.as_ptr() == x));
            assert!(iter.next().is_none());

            drop(Box::from_raw(x));
            drop(Box::from_raw(y));
            drop(Box::from_raw(z));
        }
    }
}
