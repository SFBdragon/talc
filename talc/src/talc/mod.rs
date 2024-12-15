use core::{alloc::Layout, fmt::Debug, mem::offset_of, ptr::NonNull};
use bucket_config::BucketConfig;
use alignment::{alloc_unit, ChunkAlign};
use bitfield::BitField;
use node::Node;
use oom_handler::OomHandler;
use tag::Tag;

use crate::{ptr_utils::{align_down_by, align_up_by}, Span};

mod tag;
mod node;
pub mod alignment;
pub mod bitfield;
pub mod bucket_config;
pub mod oom_handler;

#[cfg(feature = "counters")]
mod counters;
#[cfg(feature = "counters")]
pub use counters::Counters;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotEnoughMemory;

#[repr(C)]
pub struct GapData {
    pub(crate) bin: u16,
    pub(crate) is_arena_base: bool,
    pub(crate) size: usize,
}

pub type GapNode = Node<GapData>;

const GAP_NODE_OFFSET: usize = 0;
const GAP_LOW_SIZE_OFFSET: usize = offset_of!(GapNode, payload) + offset_of!(GapData, size);
const GAP_HIGH_SIZE_OFFSET: usize = size_of::<usize>();

// WASM perf tanks if these #[inline]'s are not present
#[inline]
unsafe fn gap_base_to_node(base: *mut u8) -> *mut GapNode {
    base.add(GAP_NODE_OFFSET).cast()
}
#[inline]
unsafe fn gap_base_to_size(base: *mut u8) -> *mut usize {
    base.add(GAP_LOW_SIZE_OFFSET).cast()
}
#[inline]
unsafe fn gap_base_to_acme_size(base: *mut u8) -> (*mut u8, usize) {
    let size = gap_base_to_size(base).read();
    (base.add(size), size)
}
#[inline]
unsafe fn gap_acme_to_size(acme: *mut u8) -> *mut usize {
    acme.sub(GAP_HIGH_SIZE_OFFSET).cast()
}
#[inline]
unsafe fn gap_acme_to_base(acme: *mut u8) -> *mut u8 {
    gap_acme_to_base_size(acme).0
}
#[inline]
unsafe fn gap_acme_to_base_size(acme: *mut u8) -> (*mut u8, usize) {
    let size = gap_acme_to_size(acme).read();
    (acme.sub(size), size)
}
#[inline]
unsafe fn gap_node_to_base(node: NonNull<GapNode>) -> *mut u8 {
    node.as_ptr().cast::<u8>().sub(GAP_NODE_OFFSET).cast()
}
#[inline]
unsafe fn gap_node_to_size(node: NonNull<GapNode>) -> *mut usize {
    node.as_ptr().cast::<u8>().sub(GAP_NODE_OFFSET).add(GAP_LOW_SIZE_OFFSET).cast()
}
#[inline]
unsafe fn is_gap_below(acme: *mut u8) -> bool {
    // gap size will never have bit 1 set, but a tag will
    acme.byte_sub(size_of::<Tag>()).read() & Tag::ALLOCATED_FLAG == 0
}
#[inline]
unsafe fn acme_to_tag(acme: *mut u8) -> *mut Tag {
    acme.byte_sub(size_of::<Tag>()).cast()
}

pub struct Talc<O: OomHandler<Cfg, A>, Cfg: BucketConfig, A: ChunkAlign> {
    #[cfg(feature = "counters")]
    /// Allocation statistics for this arena.
    counters: Counters,

    avails: Cfg::Availability,
    free_lists: *mut Option<NonNull<GapNode>>,

    pub bucket_config: Cfg,
    pub oom_handler: O,

    _phantom: core::marker::PhantomData<A>,
}

unsafe impl<O: OomHandler<Cfg, A> + Send, Cfg: BucketConfig + Send, A: ChunkAlign> Send for Talc<O, Cfg, A> {}

impl<O: OomHandler<Cfg, A>, Cfg: BucketConfig, A: ChunkAlign> Debug for Talc<O, Cfg, A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // TODO
        f
            .debug_struct("Talc")
            .finish()
    }
}

impl<O: OomHandler<Cfg, A>, Cfg: BucketConfig, A: ChunkAlign> Talc<O, Cfg, A> {
    /// Returns whether the two pointers are greater than `MIN_CHUNK_SIZE` apart.
    #[inline]
    fn is_chunk_size(base: *mut u8, acme: *mut u8) -> bool {
        acme as usize - base as usize >= alloc_unit::<A>()
    }

    #[inline]
    const fn required_chunk_size(size: usize) -> usize {
        (size + size_of::<Tag>() + (alloc_unit::<A>() - 1)) & !(alloc_unit::<A>() - 1)
    }
    #[inline]
    unsafe fn alloc_to_acme(base: *mut u8, size: usize) -> *mut u8 {
        align_up_by(base.byte_add(size + size_of::<Tag>()), alloc_unit::<A>() - 1)
    }

    #[inline]
    unsafe fn bin_ptr(&self, bin: usize) -> *mut Option<NonNull<GapNode>> {
        debug_assert!(bin < Cfg::Availability::BIT_COUNT);
        self.free_lists.add(bin)
    }

    /// Registers a gap in memory which is allocatable.
    #[inline]
    pub unsafe fn register_gap(&mut self, base: *mut u8, acme: *mut u8, is_arena_base: bool) {
        debug_assert!(Self::is_chunk_size(base, acme));

        let size = acme as usize - base as usize;
        let bin = Cfg::size_to_bucket::<A>(size);

        let bin_ptr = self.bin_ptr(bin);

        if (*bin_ptr).is_none() {
            self.avails.set_bit(bin);
        }

        GapNode::insert(
            gap_base_to_node(base), 
            GapNode { next: *bin_ptr, next_of_prev: bin_ptr, payload: GapData { bin: bin as _, is_arena_base, size } }
        );

        debug_assert!((*bin_ptr).is_some());

        gap_acme_to_size(acme).write(size);

        #[cfg(feature = "counters")]
        self.counters.account_register_gap(size);
    }

    /// De-registers memory, not allowing it to be allocated.
    #[inline]
    pub unsafe fn deregister_gap(&mut self, base: *mut u8) -> bool {
        debug_assert!((*self.bin_ptr(Cfg::size_to_bucket::<A>(gap_base_to_size(base).read()))).is_some());

        #[cfg(feature = "counters")]
        self.counters.account_deregister_gap(gap_base_to_size(base).read());

        let GapData { bin, is_arena_base, .. } = GapNode::remove(gap_base_to_node(base));
        let bin = bin as usize;

        if (*self.bin_ptr(bin)).is_none() {
            self.avails.clear_bit(bin);
        }

        is_arena_base
    }

    /// Allocate a contiguous region of memory according to `layout`, if possible.
    /// # Safety
    /// `layout.size()` must be nonzero.
    pub unsafe fn alloc(&mut self, layout: Layout) -> Result<NonNull<u8>, NotEnoughMemory> {
        debug_assert!(layout.size() != 0);
        self.scan_for_errors();

        let required_chunk_size = Self::required_chunk_size(layout.size());

        // if there are no valid heaps, availability is zero, and next_available_bin returns None
        let mut bin = loop {
            match self.avails.lowest_set_bit(Cfg::size_to_bucket::<A>(required_chunk_size - 1).wrapping_add(1)) {
                Some(bin) => break bin,
                None => O::handle_oom(self, layout).map_err(|_| NotEnoughMemory)?,
            }
        };

        let mut tag = Tag::ALLOCATED;
        let (base, chunk_acme) = 'find_gap: {
            if layout.align() <= alloc_unit::<A>() {
                let node_ptr = self.bin_ptr(bin).read().unwrap_unchecked();
                let size = gap_node_to_size(node_ptr).read();

                debug_assert!(size >= required_chunk_size);

                let base = gap_node_to_base(node_ptr);
                let is_arena_base =  self.deregister_gap(base);

                if !is_arena_base {
                    Tag::clear_above_free(acme_to_tag(base));
                } else {
                    tag |= Tag::ARENA_BASE;
                }

                break 'find_gap (base, base.add(size));
            } else {
                // a larger than word-size alignment is demanded
                // therefore each chunk is manually checked to be sufficient accordingly
                let align_mask = layout.align() - 1;
    
                loop {
                    for node_ptr in GapNode::iter_mut(*self.bin_ptr(bin)) {
                        let size = gap_node_to_size(node_ptr).read();
    
                        if size >= required_chunk_size {
                            let base = gap_node_to_base(node_ptr);
                            let acme = base.add(size);
                            // calculate the lowest aligned pointer above the tag-offset free chunk pointer
                            let aligned_base = align_up_by(base, align_mask);
    
                            // if the remaining size is sufficient, remove the chunk from the books and return
                            if aligned_base.add(required_chunk_size) <= acme {
                                let is_arena_base = self.deregister_gap(base);
    
                                // determine the base of the allocated chunk
                                // if the amount of memory below the chunk is too small, subsume it, else free it
                                if base != aligned_base {
                                    self.register_gap(base, aligned_base, is_arena_base);
                                } else if !is_arena_base {
                                    Tag::clear_above_free(acme_to_tag(base));
                                } else {
                                    tag |= Tag::ARENA_BASE;
                                }
    
                                break 'find_gap (aligned_base, acme);
                            }
                        }
                    }
    
                    if bin + 1 < Cfg::Availability::BIT_COUNT {
                        bin = self.avails.lowest_set_bit(bin + 1).ok_or(NotEnoughMemory)?;
                    }
                }
            }
        };

        let acme = Self::alloc_to_acme(base, layout.size());
        let tag_ptr = acme_to_tag(acme);

        // handle the space above the required allocation span
        if acme != chunk_acme {
            self.register_gap(acme, chunk_acme, false);
            tag |= Tag::ABOVE_FREE;
        }

        tag_ptr.write(tag);

        #[cfg(feature = "counters")]
        self.counters.account_alloc(layout.size());

        Ok(NonNull::new_unchecked(base))
    }

    /// Free previously allocated/reallocated memory.
    /// # Safety
    /// `ptr` must have been previously allocated given `layout`.
    pub unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        self.scan_for_errors();

        #[cfg(feature = "counters")]
        self.counters.account_dealloc(layout.size());

        let mut chunk_base = ptr;
        let mut chunk_acme = Self::alloc_to_acme(ptr, layout.size());
        let tag_ptr = acme_to_tag(chunk_acme);
        let tag = *tag_ptr;
        let mut is_arena_base = tag.is_arena_base();

        debug_assert!(tag.is_allocated());
        debug_assert!(Self::is_chunk_size(chunk_base, chunk_acme));

        // try to recombine the chunk below
        if !is_arena_base {
            if is_gap_below(chunk_base) {
                let below_base = gap_acme_to_base(chunk_base);
                is_arena_base = self.deregister_gap(below_base);
                chunk_base = below_base;
            } else {
                Tag::set_above_free(acme_to_tag(chunk_base))
            }
        }

        // try to recombine the chunk above
        if tag.is_above_free() {
            let above_is_arena_base = self.deregister_gap(chunk_acme);
            debug_assert!(!above_is_arena_base);
            let above_size = gap_base_to_size(chunk_acme).read();
            chunk_acme = chunk_acme.add(above_size);
        }

        // add the full recombined free chunk back into the books
        self.register_gap(chunk_base, chunk_acme, is_arena_base);
    }

    /* /// Grow a previously allocated/reallocated region of memory to `new_size`.
    /// # Safety
    /// `ptr` must have been previously allocated or reallocated given `layout`.
    /// `new_size` must be larger or equal to `layout.size()`.
    pub unsafe fn grow(
        &mut self,
        base: *mut u8,
        old_layout: Layout,
        new_size: usize,
    ) -> Result<NonNull<u8>, ArenaFullError> {
        match self.grow_in_place(base, old_layout, new_size) {
            Err(_) => {
                // grow in-place failed; reallocate the slow way
                let new_layout = Layout::from_size_align_unchecked(new_size, old_layout.align());
                let allocation = self.alloc(new_layout)?;
                allocation.as_ptr().copy_from_nonoverlapping(base, old_layout.size());
                self.dealloc(base, old_layout);

                Ok(allocation)
            }
            res => res,
        }
    } */

    /// Attempt to grow a previously allocated/reallocated region of memory to `new_size`.
    ///
    /// Returns `Err` if reallocation could not occur in-place.
    /// Ownership of the original memory remains with the caller.
    /// # Safety
    /// `ptr` must have been previously allocated or reallocated given `layout`.
    /// `new_size` must be larger or equal to `layout.size()`.
    pub unsafe fn grow_in_place(
        &mut self,
        base: *mut u8,
        old_layout: Layout,
        new_size: usize,
    ) -> Result<NonNull<u8>, NotEnoughMemory> {
        debug_assert!(new_size >= old_layout.size());
        self.scan_for_errors();

        let old_acme = Self::alloc_to_acme(base, old_layout.size());
        let new_acme = Self::alloc_to_acme(base, new_size);

        debug_assert!(Self::is_chunk_size(base, old_acme));

        if old_acme == new_acme {
            #[cfg(feature = "counters")]
            self.counters.account_grow_in_place(old_layout.size(), new_size);

            return Ok(NonNull::new_unchecked(base));
        }

        let old_tag = acme_to_tag(old_acme).read();

        debug_assert!(old_tag.is_allocated());

        // otherwise, check if 1) is free 2) is large enough
        // because free chunks don't border free chunks, this needn't be recursive
        if old_tag.is_above_free() {
            let above_size = gap_base_to_size(old_acme).read();
            let above_acme = old_acme.add(above_size);

            if new_acme <= above_acme {
                let is_arena_base = self.deregister_gap(old_acme);
                debug_assert!(!is_arena_base);

                // finally, determine if the remainder of the free block is big enough
                // to be freed again, or if the entire region should be allocated
                if new_acme != above_acme {
                    self.register_gap(new_acme, above_acme, false);
                    acme_to_tag(new_acme).write(Tag::ABOVE_FREE);
                } else {
                    acme_to_tag(new_acme).write(Tag::ALLOCATED);
                }

                #[cfg(feature = "counters")]
                self.counters.account_grow_in_place(old_layout.size(), new_size);

                return Ok(NonNull::new_unchecked(base));
            }
        }

        Err(NotEnoughMemory)
    }

    /// Shrink a previously allocated/reallocated region of memory to `new_size`.
    ///
    /// This function is infallible given valid inputs, and the reallocation will always be
    /// done in-place, maintaining the validity of the pointer.
    ///
    /// # Safety
    /// - `ptr` must have been previously allocated or reallocated given `layout`.
    /// - `new_size` must be smaller or equal to `layout.size()`.
    /// - `new_size` should be nonzero.
    pub unsafe fn shrink(&mut self, base: *mut u8, layout: Layout, new_size: usize) {
        debug_assert!(new_size != 0);
        debug_assert!(new_size <= layout.size());
        self.scan_for_errors();

        let mut chunk_acme = Self::alloc_to_acme(base, layout.size());
        let new_acme = Self::alloc_to_acme(base, new_size);

        debug_assert!(acme_to_tag(chunk_acme).read().is_allocated());
        debug_assert!(Self::is_chunk_size(base, chunk_acme));

        // if the remainder between the new required size and the originally allocated
        // size is large enough, free the remainder, otherwise leave it
        if new_acme != chunk_acme {
            let old_tag = acme_to_tag(chunk_acme).read();
            if old_tag.is_above_free() {
                let above_is_arena_base = self.deregister_gap(chunk_acme);
                debug_assert!(!above_is_arena_base);
                let above_size = gap_base_to_size(chunk_acme).read();
                chunk_acme = chunk_acme.add(above_size);
            }

            self.register_gap(new_acme, chunk_acme, false);
            acme_to_tag(new_acme).write(Tag::ABOVE_FREE);
        }

        #[cfg(feature = "counters")]
        self.counters.account_shrink_in_place(layout.size(), new_size);
    }


    pub const fn new(oom_handler: O) -> Self {
        Self {
            #[cfg(feature = "counters")]
            counters: Counters::new(),

            avails: Cfg::Availability::INIT,
            free_lists: core::ptr::null_mut(),
            bucket_config: Cfg::INIT,
            oom_handler,

            _phantom: core::marker::PhantomData,
        }
    }

    pub unsafe fn claim(&mut self, memory: Span) -> Result<Span, NotEnoughMemory> {
        let aligned = memory.align_inwards(alloc_unit::<A>());
        let Some((aligned_base, aligned_acme)) = aligned.get_base_acme() else {
            return Err(NotEnoughMemory);
        };
        
        let mut free_base = aligned_base;
        let mut gap_is_arena_base = true;

        if self.free_lists.is_null() {
            let free_lists_size = size_of::<Option<NonNull<GapData>>>() * Cfg::Availability::BIT_COUNT;
            let free_lists_chunk_size = Self::required_chunk_size(free_lists_size);
            free_base = aligned_base.add(free_lists_chunk_size);

            if free_base > aligned_acme {
                return Err(NotEnoughMemory);
            }

            let mut tag = Tag::ARENA_BASE;
            if free_base < aligned_acme { tag |= Tag::ABOVE_FREE };

            acme_to_tag(free_base).write(tag);
            gap_is_arena_base = false;

            self.free_lists = aligned_base.cast();
            for b in 0..Cfg::Availability::BIT_COUNT {
                self.bin_ptr(b).write(None);
            }
        }

        if free_base < aligned_acme {
            self.register_gap(free_base, aligned_acme, gap_is_arena_base);
        }

        let claimed_span = Span::new(aligned_base, aligned_acme);

        #[cfg(feature = "counters")]
        self.counters.account_claim(claimed_span.size());
        
        Ok(claimed_span)
    }

    pub unsafe fn inspect_available(&self, acme: *mut u8) -> *mut u8 {
        assert_eq!(acme as usize & (alloc_unit::<A>() - 1), 0, "acme is not correctly aligned");

        if is_gap_below(acme) {
            gap_acme_to_base(acme)
        } else {
            acme
        }
    }

    /// If you don't want recombination, use CLAIM
    pub unsafe fn extend(&mut self, acme: *mut u8, new_acme: *mut u8) -> *mut u8 {
        assert_eq!(acme as usize & (alloc_unit::<A>() - 1), 0, "acme is not correctly aligned");
        assert!(new_acme >= acme);

        let aligned_new_acme = align_down_by(new_acme, alloc_unit::<A>() - 1);

        let mut gap_base = acme;
        let mut is_arena_base = false;

        if is_gap_below(acme) {
            gap_base = gap_acme_to_base(acme);
            is_arena_base = self.deregister_gap(gap_base);
        }

        self.register_gap(gap_base, aligned_new_acme, is_arena_base);


        #[cfg(feature = "counters")]
        self.counters.account_append(acme, aligned_new_acme);

        aligned_new_acme
    }

    pub unsafe fn truncate(&mut self, acme: *mut u8, new_acme: *mut u8) -> Option<*mut u8> {
        assert_eq!(acme as usize & (alloc_unit::<A>() - 1), 0, "acme is not correctly aligned");
        assert!(new_acme <= acme);

        debug_assert!(self.inspect_available(acme) <= new_acme);
        
        let aligned_new_acme = align_down_by(new_acme, alloc_unit::<A>() - 1);

        if aligned_new_acme == acme {
            return Some(acme);
        }

        assert!(is_gap_below(acme));

        let gap_base = gap_acme_to_base(acme);
        let is_arena_base = self.deregister_gap(gap_base);

        assert!(gap_base <= aligned_new_acme);

        if gap_base < aligned_new_acme {
            self.register_gap(gap_base, aligned_new_acme, is_arena_base);

            #[cfg(feature = "counters")]
            self.counters.account_truncate(acme, aligned_new_acme, false);

            Some(aligned_new_acme)
        } else if is_arena_base {
            #[cfg(feature = "counters")]
            self.counters.account_truncate(acme, aligned_new_acme, true);

            None
        } else {
            #[cfg(feature = "counters")]
            self.counters.account_truncate(acme, aligned_new_acme, false);

            Some(aligned_new_acme)
        }

    }


    #[cfg(not(debug_assertions))]
    fn scan_for_errors(&self) {}

    #[cfg(debug_assertions)]
    /// Debugging function for checking various assumptions.
    fn scan_for_errors(&self) {
        #[cfg(any(test, feature = "fuzzing"))]
        let mut vec = std::vec::Vec::<Span>::new();

        if !self.free_lists.is_null() {
            for b in 0..Cfg::Availability::BIT_COUNT {
                let mut any = false;
                unsafe {
                    for node in GapNode::iter_mut(*self.bin_ptr(b)) {
                        any = true;
                        assert!(self.avails.read_bit(b));

                        let base = gap_node_to_base(node);
                        let (acme, size) = gap_base_to_acme_size(base);
                        let low_size = gap_acme_to_size(acme).read();
                        assert_eq!(low_size, size);

                        let lower_tag = acme_to_tag(base).read();
                        assert!(lower_tag.is_allocated());
                        assert!(lower_tag.is_above_free());

                        #[cfg(any(test, feature = "fuzzing"))]
                        {
                            let span = Span::new(base, acme);
                            // dbg!(span);
                            for other in &vec {
                                assert!(!span.overlaps(*other), "{} intersects {}", span, other);
                            }
                            vec.push(span);
                        }
                    }
                }

                if !any {
                    assert!(!self.avails.read_bit(b));
                }
            }
        } else {
            assert!(self.avails.lowest_set_bit(0).is_none());
        }
    }
}



/* #[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_assumptions_hold() {
        // claim assumes this
        assert!(ALIGN == std::mem::align_of::<Bin>() && ALIGN == std::mem::size_of::<Bin>());
    }

    #[test]
    fn alloc_dealloc_test() {
        const ARENA_SIZE: usize = 10000000;

        let arena = Box::leak(vec![0u8; ARENA_SIZE].into_boxed_slice()) as *mut [_];

        let mut talc = Talc::new(crate::ErrOnOom);

        unsafe {
            talc.claim(arena.as_mut().unwrap().into()).unwrap();
        }

        let layout = Layout::from_size_align(1243, 8).unwrap();

        let a = unsafe { talc.malloc(layout) };
        assert!(a.is_ok());
        unsafe {
            a.unwrap().as_ptr().write_bytes(255, layout.size());
        }

        let mut x = vec![NonNull::dangling(); 100];

        for _ in 0..1 {
            for i in 0..100 {
                let allocation = unsafe { talc.malloc(layout) };
                assert!(allocation.is_ok());
                unsafe {
                    allocation.unwrap().as_ptr().write_bytes(0xab, layout.size());
                }
                x[i] = allocation.unwrap();
            }

            for i in 0..50 {
                unsafe {
                    talc.free(x[i], layout);
                }
            }
            for i in (50..100).rev() {
                unsafe {
                    talc.free(x[i], layout);
                }
            }
        }

        unsafe {
            talc.free(a.unwrap(), layout);
        }

        unsafe {
            drop(Box::from_raw(arena));
        }
    }

    #[test]
    fn claim_truncate_extend_test() {
        // not big enough to fit the metadata
        let mut tiny_heap = [0u8; BIN_COUNT * WORD_SIZE / 2];
        let tiny_heap_span: Span = Span::from(&mut tiny_heap);

        // big enough with plenty of extra
        let big_heap = Box::leak(vec![0u8; BIN_COUNT * WORD_SIZE + 100000].into_boxed_slice());
        let big_heap_span = Span::from(big_heap.as_mut());

        let mut talc = Talc::new(crate::ErrOnOom);

        unsafe {
            talc.claim(tiny_heap_span).unwrap_err();
        }

        assert!(talc.bins.is_null());
        assert!(talc.availability_low == 0 && talc.availability_high == 0);

        let alloc_big_heap = unsafe { talc.claim(big_heap_span).unwrap() };

        assert!(!talc.bins.is_null());

        let alloc_big_heap = unsafe {
            talc.truncate(
                alloc_big_heap,
                alloc_big_heap.truncate(500, 500).fit_over(talc.get_allocated_span(alloc_big_heap)),
            )
        };

        let _alloc_tiny_heap = unsafe { talc.claim(tiny_heap_span).unwrap() };

        let allocation = unsafe {
            let allocation = talc.malloc(Layout::new::<u128>()).unwrap();
            allocation.as_ptr().write_bytes(0, Layout::new::<u128>().size());
            allocation
        };

        let alloc_big_heap = unsafe {
            talc.truncate(
                alloc_big_heap,
                alloc_big_heap
                    .truncate(100000, 100000)
                    .fit_over(talc.get_allocated_span(alloc_big_heap)),
            )
        };

        unsafe {
            talc.extend(
                alloc_big_heap,
                alloc_big_heap.extend(10000, 10000).fit_within(big_heap_span),
            );
        }

        unsafe {
            talc.free(allocation, Layout::new::<u128>());
        }

        unsafe {
            drop(Box::from_raw(big_heap));
        }
    }
}
 */