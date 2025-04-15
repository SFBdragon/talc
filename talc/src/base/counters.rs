//! Track allocation statistics for Talc.

/// Allocation statistics struct for [`Talc`](crate::base::Talc).
///
/// # Example
///
/// ```
/// # use ::talc::{TalcCell, ErrOnOom};
/// let talc = TalcCell::new(ErrOnOom);
/// let counters = talc.counters();
/// assert_eq!(counters.total_freed_bytes(), 0);
/// eprintln!("{}", counters);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Counters {
    /// Number of active allocations.
    pub allocation_count: usize,
    /// Total number of allocations.
    pub total_allocation_count: u64,

    /// Sum of active allocations' layouts' size.
    pub allocated_bytes: usize,
    /// Sum of all allocations' layouts' maximum size.
    ///
    /// In-place reallocations' unchanged bytes are not recounted.
    pub total_allocated_bytes: u64,

    /// Number of bytes available for allocation.
    pub available_bytes: usize,
    /// Number of holes/gaps between allocations.
    pub fragment_count: usize,

    /// Number of active established arenas.
    pub arena_count: usize,
    /// Total number of established arenas.
    pub total_arena_count: u64,

    /// Sum of bytes actively claimed.
    pub claimed_bytes: usize,
    /// Sum of bytes ever claimed. Reclaimed bytes included.
    pub total_claimed_bytes: u64,
}

impl Counters {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            allocation_count: 0,
            total_allocation_count: 0,
            allocated_bytes: 0,
            total_allocated_bytes: 0,
            available_bytes: 0,
            fragment_count: 0,
            arena_count: 0,
            total_arena_count: 0,
            claimed_bytes: 0,
            total_claimed_bytes: 0,
        }
    }

    /// Returns the number of bytes unavailable due to metadata and alignment overhead.
    #[inline]
    pub const fn overhead_bytes(&self) -> usize {
        self.claimed_bytes - self.available_bytes - self.allocated_bytes
    }

    /// Returns the total number of allocated bytes that have been freed.
    #[inline]
    pub const fn total_freed_bytes(&self) -> u64 {
        self.total_allocated_bytes - self.allocated_bytes as u64
    }

    /// Returns the total number of claimed bytes have been released.
    #[inline]
    pub const fn total_released_bytes(&self) -> u64 {
        self.total_claimed_bytes - self.claimed_bytes as u64
    }

    #[inline]
    pub(crate) fn account_register_gap(&mut self, size: usize) {
        self.available_bytes += size;
        self.fragment_count += 1;
    }
    #[inline]
    pub(crate) fn account_deregister_gap(&mut self, size: usize) {
        self.available_bytes -= size;
        self.fragment_count -= 1;
    }

    #[inline]
    pub(crate) fn account_alloc(&mut self, alloc_size: usize) {
        self.allocation_count += 1;
        self.allocated_bytes += alloc_size;

        self.total_allocation_count += 1;
        self.total_allocated_bytes += alloc_size as u64;
    }

    #[inline]
    pub(crate) fn account_dealloc(&mut self, alloc_size: usize) {
        self.allocation_count -= 1;
        self.allocated_bytes -= alloc_size;
    }

    #[inline]
    pub(crate) fn account_grow_in_place(&mut self, old_alloc_size: usize, new_alloc_size: usize) {
        self.allocated_bytes += new_alloc_size - old_alloc_size;
        self.total_allocated_bytes += (new_alloc_size - old_alloc_size) as u64;
    }

    #[inline]
    pub(crate) fn account_shrink_in_place(&mut self, old_alloc_size: usize, new_alloc_size: usize) {
        self.allocated_bytes -= old_alloc_size - new_alloc_size;
        self.total_allocated_bytes -= (old_alloc_size - new_alloc_size) as u64;
    }

    #[inline]
    pub(crate) fn account_claim(&mut self, claimed_size: usize) {
        self.arena_count += 1;
        self.claimed_bytes += claimed_size;

        self.total_arena_count += 1;
        self.total_claimed_bytes += claimed_size as u64;
    }

    #[inline]
    pub(crate) fn account_append(&mut self, old_acme: *mut u8, new_acme: *mut u8) {
        self.claimed_bytes += new_acme as usize - old_acme as usize;
        self.total_claimed_bytes += (new_acme as usize - old_acme as usize) as u64;
    }

    #[inline]
    pub(crate) fn account_truncate(
        &mut self,
        old_acme: *mut u8,
        new_acme: *mut u8,
        deleted_arena: bool,
    ) {
        if deleted_arena {
            self.arena_count -= 1;
            self.claimed_bytes -= std::mem::size_of::<super::tag::Tag>();
        }

        self.claimed_bytes -= old_acme as usize - new_acme as usize;
    }
}

impl core::fmt::Display for Counters {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            r#"Stat                 | Current Total       | Accumulative Total
---------------------|---------------------|--------------------
# of Allocations     | {:>19} | {:>19}
# of Allocated Bytes | {:>19} | {:>19}
# of Available Bytes | {:>19} |                 N/A
# of Overhead Bytes  | {:>19} |                 N/A
# of Claimed Bytes   | {:>19} | {:>19}
# of Heaps           | {:>19} | {:>19}
# of Fragments       | {:>19} |                 N/A"#,
            self.allocation_count,
            self.total_allocation_count,
            self.allocated_bytes,
            self.total_allocated_bytes,
            self.available_bytes,
            self.overhead_bytes(),
            self.claimed_bytes,
            self.total_claimed_bytes,
            self.arena_count,
            self.total_arena_count,
            self.fragment_count,
        )
    }
}

impl<O: crate::oom::OomHandler<B>, B: super::Binning> super::Talc<O, B> {
    /// Obtain a reference to the internal allocation statistics.
    ///
    /// Avoid holding onto the reference as this will block allocations
    /// (as you're effectively holding the lock on the allocator,
    /// or preventing a mutable reference being created to the allocator).
    /// Reading immediately or cloning the struct is recommended.
    pub fn counters(&self) -> &Counters {
        &self.counters
    }
}

#[cfg(test)]
mod tests {
    use ::core::alloc::Layout;
    use core::ptr::null_mut;

    use crate::Binning;
    use crate::base::CHUNK_UNIT;

    use crate::*;

    #[test]
    fn test_claim_alloc_free_truncate() {
        fn test_claim_alloc_free_truncate_inner<B: Binning>() {
            let mut arena = [0u8; 1000000];

            let mut talc = crate::base::Talc::<_, B>::new(crate::ErrOnOom);

            let mem_base = arena.as_mut_ptr().wrapping_add(99);
            let mem_size = 10001;
            let end = unsafe { talc.claim(mem_base, mem_size) }.unwrap().as_ptr();

            let pre_alloc_claimed_bytes = talc.counters().claimed_bytes;
            let pre_alloc_avl_bytes = talc.counters().available_bytes;

            assert!(pre_alloc_claimed_bytes <= mem_size);
            assert!(pre_alloc_claimed_bytes > mem_size - CHUNK_UNIT * 2);
            assert_eq!(pre_alloc_claimed_bytes, talc.counters().total_claimed_bytes as _);

            let max_meta_overhead = crate::min_first_arena_size::<B>();
            assert!(pre_alloc_claimed_bytes - max_meta_overhead <= pre_alloc_avl_bytes);
            assert_eq!(talc.counters().allocated_bytes, 0);
            assert_eq!(talc.counters().total_allocated_bytes, 0);
            assert_eq!(talc.counters().allocation_count, 0);
            assert_eq!(talc.counters().total_allocation_count, 0);
            assert_eq!(talc.counters().fragment_count, 1);
            assert_eq!(
                talc.counters().overhead_bytes(),
                pre_alloc_claimed_bytes - pre_alloc_avl_bytes
            );

            let alloc_layout = Layout::new::<[u8; 3]>();
            let alloc = unsafe { talc.allocate(alloc_layout).unwrap() };

            let allocation_chunk_bytes =
                crate::base::Talc::<ErrOnOom, B>::required_chunk_size(alloc_layout.size());

            assert_eq!(talc.counters().claimed_bytes, pre_alloc_claimed_bytes);
            assert_eq!(
                talc.counters().available_bytes,
                pre_alloc_avl_bytes - allocation_chunk_bytes
            );
            assert_eq!(talc.counters().allocated_bytes, alloc_layout.size());
            assert_eq!(talc.counters().total_allocated_bytes, alloc_layout.size() as _);
            assert_eq!(talc.counters().allocation_count, 1);
            assert_eq!(talc.counters().total_allocation_count, 1);
            assert_eq!(talc.counters().fragment_count, 1);

            unsafe {
                talc.deallocate(alloc.as_ptr(), alloc_layout);
            }

            assert_eq!(talc.counters().claimed_bytes, pre_alloc_claimed_bytes);
            assert_eq!(talc.counters().total_claimed_bytes, pre_alloc_claimed_bytes as _);
            assert_eq!(talc.counters().available_bytes, pre_alloc_avl_bytes);
            assert_eq!(talc.counters().allocated_bytes, 0);
            assert_eq!(talc.counters().total_allocated_bytes, alloc_layout.size() as _);
            assert_eq!(talc.counters().allocation_count, 0);
            assert_eq!(talc.counters().total_allocation_count, 1);
            assert_eq!(talc.counters().fragment_count, 1);

            let end = unsafe { talc.truncate(end, null_mut()) }.unwrap();
            let extent = end.as_ptr() as usize - mem_base as usize;
            assert!(extent <= max_meta_overhead);

            assert_eq!(talc.counters().claimed_bytes, extent);
            assert_eq!(talc.counters().overhead_bytes(), extent);
            assert_eq!(talc.counters().total_claimed_bytes, pre_alloc_claimed_bytes as _);
            assert_eq!(talc.counters().available_bytes, 0);
            assert_eq!(talc.counters().allocated_bytes, 0);
            assert_eq!(talc.counters().total_allocated_bytes, alloc_layout.size() as _);
            assert_eq!(talc.counters().allocation_count, 0);
            assert_eq!(talc.counters().total_allocation_count, 1);
            assert_eq!(talc.counters().fragment_count, 0);
            assert_eq!(talc.counters().arena_count, 1);
            assert_eq!(talc.counters().total_arena_count, 1);
        }

        for_many_talc_configurations!(test_claim_alloc_free_truncate_inner);
    }
}
