use core::cell::Cell;


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
pub struct LlistNode<T> {
    pub data: T, // this is superfluous in this project, maybe remove it
    pub next: Cell<*mut LlistNode<T>>,
    pub prev: Cell<*mut LlistNode<T>>,
}

impl<T> LlistNode<T> {
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
    pub unsafe fn new_llist(node: *mut Self, data: T) {
        node.write(Self {
            data,
            prev: Cell::new(node),
            next: Cell::new(node),
        });
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
    pub unsafe fn new(node: *mut Self, prev: *mut Self, next: *mut Self, data: T) {
        node.write(Self { 
            data,
            prev: Cell::new(prev),
            next: Cell::new(next),
        });

        (*next).prev.set(node);
        (*prev).next.set(node);
    }
    
    /// Move `self` into a new location, leaving `self` as an isolated node.
    /// ### Safety:
    /// * `dest` must be `ptr::write`-able.
    /// * `self` must be dereferencable and valid.
    pub unsafe fn mov(src: *mut Self, dst: *mut Self) {
        // src.prev and src.next can be src, so order of ops is important
        let src_prev = (*src).prev.get();
        let src_next = (*src).next.get();
        (*src_prev).next.set(dst);
        (*src_next).prev.set(dst);

        let src_node = src.read();

        // src can be dst, so write in the canaries after reading but before writing
        (*src).prev.set(0xabc as *mut _);
        (*src).next.set(0xabc as *mut _);

        dst.write(src_node);
    }

    /// Remove `self` from it's linked list, leaving `self` as an isolated node.
    /// If `self` is linked only to itself, this is effectively a no-op.
    /// ### Safety:
    /// * `self` must be dereferencable and valid.
    pub unsafe fn remove(node: *mut Self) -> T {
        let prev = (*node).prev.get();
        let next = (*node).next.get();
        (*prev).next.set(next);
        (*next).prev.set(prev);

        (*node).prev.set(node);
        (*node).next.set(node);
        node.read().data
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
        let start_prev = (*start).prev.get();
        let end_next   = (*end)  .next.get();
        (*start_prev).next.set(end_next);
        (*end_next)  .prev.set(start_prev);

        // link up new list
        (*start).prev.set(prev);
        (*end)  .next.set(next);
        (*prev) .next.set(start);
        (*next) .prev.set(end);
    }


    /// Creates an iterator over the circular linked list, exclusive of
    /// the sentinel.
    /// ### Safety:
    /// `start`'s linked list must remain in a valid state during iteration.
    /// Modifying `LlistNode`s already returned by the iterator is okay.
    pub unsafe fn iter_mut(sentinel: *mut Self) -> IterMut<T> {
        IterMut::new(sentinel)
    }
}


/// An iterator over the circular linked list `LlistNode`s, excluding the 'head'.
///
/// This `struct` is created by `LlistNode::iter_mut`. See its documentation for more.
#[derive(Debug, Clone, Copy)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct IterMut<T> {
    forward: *mut LlistNode<T>,
    backward: *mut LlistNode<T>,
    ongoing: bool,
}
impl<T> IterMut<T> {
    /// Create a new iterator over a linked list, *except* `sentinel`.
    pub unsafe fn new(sentinel: *mut LlistNode<T>) -> Self {
        Self {
            forward: (*sentinel).next.get(),
            backward: (*sentinel).prev.get(),
            ongoing: sentinel != (*sentinel).next.get(),
        }
    }
}

impl<T> Iterator for IterMut<T> {
    type Item = *mut LlistNode<T>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.ongoing {
            let ret = self.forward;
            if self.forward == self.backward {
                self.ongoing = false;
            }
            self.forward = unsafe { (*self.forward).next.get() };
            Some(ret)
        } else {
            None
        }
    }
}

impl<T> DoubleEndedIterator for IterMut<T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.ongoing {
            let ret = self.backward;
            if self.forward == self.backward {
                self.ongoing = false;
            }
            self.backward = unsafe { (*self.backward).prev.get() };
            Some(ret)
        } else {
            None
        }
    }
}
