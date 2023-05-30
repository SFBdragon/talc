
/// Describes a linked list node.
/// 
/// The linked list is:
///  * **Intrusive** to minimize indirection
///  * **Circular** to minimize branches
///  * Contains a **'sentinel' node** which is homogenous, but isn't iterated over
///  * **Implicitly non-zero in length** by virtue of the lack of a heterogenous component
///  * **Doubly linked** to allow bidirectional traversal and single ref removal
///  * **Very unsafe** due to pointer usage, inter-referenciality, and self-referenciality
/// 
/// ### Safety:
/// `LlistNode`s are inherently unsafe due to the referencial dependency between nodes,
/// as well as the self-referencial configuration with linked lists of length 1. This requires
/// that `LlistNode`s are never moved manually, otherwise using the list becomes memory
/// unsafe and may lead to undefined behaviour.
/// 
/// This data structure is not thread-safe, use mutexes/locks to mutually exclude data access.
#[derive(Debug)]
pub(crate) struct LlistNode {
    pub next: *mut LlistNode,
    pub prev: *mut LlistNode,
}

impl LlistNode {
    /// Create a new independent node in place.
    /// 
    /// Warning: This will not call `remove` on `node`, regardless of initialization. 
    /// It is your responsibility to make sure `node` gets `remove`d if necessary.
    /// Failing to do so is not undefined behaviour or memory unsafe, but 
    /// may cause complex and unexpected linkages.
    /// 
    /// ### Safety:
    /// * `node` must be valid for writes and properly aligned.
    #[inline]
    pub unsafe fn new(node: *mut Self) {
        debug_assert!(node > 0x10000 as _);
        node.write(Self { prev: node, next: node });
    }

    /// Create a new node as a member of an existing linked list in place of `node`.
    /// 
    /// `prev` and `next` may belong to different linked lists,
    /// doing do may however cause complex and unexpected linkages.
    /// 
    /// Warning: This will not call `remove` on `node`, regardless of initialization. 
    /// It is your responsibility to make sure `node` gets `remove`d if necessary.
    /// Failing to do so when is not undefined behaviour or memory unsafe, but 
    /// may cause complex and unexpected linkages.
    /// 
    /// ### Safety:
    /// * `node` must be `ptr::write`-able.
    /// * `prev` and `next` must be dereferencable and valid.
    pub unsafe fn insert(node: *mut Self, prev: *mut Self, next: *mut Self) {
        debug_assert!(node > 0x10000 as _);
        node.write(Self { prev, next });

        (*next).prev = node;
        (*prev).next = node;
    }
    
    /// Move `self` into a new location, leaving `self` as an isolated node.
    /// ### Safety:
    /// * `dest` must be `ptr::write`-able.
    /// * `self` must be dereferencable and valid.
    pub unsafe fn mov(src: *mut Self, dst: *mut Self) {
        debug_assert!(src > 0x10000 as _);
        debug_assert!(dst > 0x10000 as _);
        // src.prev and src.next can be src, so order of ops is important
        let src_prev = (*src).prev;
        let src_next = (*src).next;
        (*src_prev).next = dst;
        (*src_next).prev = dst;

        let src_node = src.read();

        // src can be dst, so write in the canaries after reading but before writing
        (*src).prev = 0xabc as *mut _;
        (*src).next = 0xabc as *mut _;

        dst.write(src_node);
    }

    /// Remove `self` from it's linked list, leaving `self` as an isolated node.
    /// If `self` is linked only to itself, this is effectively a no-op.
    /// ### Safety:
    /// * `self` must be dereferencable and valid.
    pub unsafe fn remove(node: *mut Self) {
        debug_assert!(node > 0x10000 as _);
        let prev = (*node).prev;
        let next = (*node).next;
        debug_assert!(prev > 0x10000 as _, "{:p} {:p} {:p}", node, prev, next);
        debug_assert!(next > 0x10000 as _);
        (*prev).next = next;
        (*next).prev = prev;

        (*node).prev = node;
        (*node).next = node;
    }


    /// Removes a chain of nodes from `start` to `end` inclusive from it's current list and inserts the
    /// chain between `prev` and `next`. This can also be used to move a chain within a single list.
    /// 
    /// # Arguments
    /// * `start` and `end` can be identical (relink 1 node).
    /// * `prev` and `next` can be identical (relink around a 'head').
    /// * All 4 arguments can be identical (orphans a single node as its own linked list).
    /// 
    /// While `start`/`end` and `prev`/`next` should belong to the same lists respectively, this is not required.
    /// Not doing so may cause complex and unexpected linkages.
    /// ### Safety:
    /// * All arguments must be dereferencable and valid.
    pub unsafe fn relink(start: *mut Self, end: *mut Self, prev: *mut Self, next: *mut Self) {
        // link up old list
        let start_prev = (*start).prev;
        let end_next   = (*end)  .next;
        (*start_prev).next = end_next;
        (*end_next)  .prev = start_prev;

        // link up new list
        (*start).prev = prev;
        (*end)  .next = next;
        (*prev) .next = start;
        (*next) .prev = end;
    }


    /// Creates an iterator over the circular linked list, exclusive of
    /// the sentinel.
    /// ### Safety:
    /// `start`'s linked list must remain in a valid state during iteration.
    /// Modifying `LlistNode`s already returned by the iterator is okay.
    pub unsafe fn iter_mut(sentinel: *mut Self) -> IterMut {
        IterMut::new(sentinel)
    }
}


/// An iterator over the circular linked list `LlistNode`s, excluding the 'head'.
///
/// This `struct` is created by `LlistNode::iter_mut`. See its documentation for more.
#[derive(Debug, Clone, Copy)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub(crate) struct IterMut {
    forward: *mut LlistNode,
    backward: *mut LlistNode,
    ongoing: bool,
}
impl IterMut {
    /// Create a new iterator over a linked list, *except* `sentinel`.
    pub unsafe fn new(sentinel: *mut LlistNode) -> Self {
        Self {
            forward: (*sentinel).next,
            backward: (*sentinel).prev,
            ongoing: sentinel != (*sentinel).next,
        }
    }
}

impl Iterator for IterMut {
    type Item = *mut LlistNode;

    fn next(&mut self) -> Option<Self::Item> {
        if self.ongoing {
            let ret = self.forward;
            if self.forward == self.backward {
                self.ongoing = false;
            }
            self.forward = unsafe { (*self.forward).next };
            Some(ret)
        } else {
            None
        }
    }
}
