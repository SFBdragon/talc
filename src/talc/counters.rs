//! Track allocation counters for Talc.

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
    /// In-place reallocations's unchanged bytes are not recounted.
    pub total_allocated_bytes: u64,

    /// Number of bytes available for allocation.
    pub available_bytes: usize,
    /// Number of holes/gaps between allocations.
    pub fragment_count: usize,

    /// Number of active established heaps.
    pub heap_count: usize,
    /// Total number of established heaps.
    pub total_heap_count: u64,

    /// Sum of bytes actively claimed.
    pub claimed_bytes: usize,
    /// Sum of bytes ever claimed. Reclaimed bytes included.
    pub total_claimed_bytes: u64,
}

impl Counters {
    pub const fn new() -> Self {
        Self {
            allocation_count: 0,
            total_allocation_count: 0,
            allocated_bytes: 0,
            total_allocated_bytes: 0,
            available_bytes: 0,
            fragment_count: 0,
            heap_count: 0,
            total_heap_count: 0,
            claimed_bytes: 0,
            total_claimed_bytes: 0,
        }
    }

    /// Returns the number of bytes unavailable due to padding/metadata/etc.
    pub const fn overhead_bytes(&self) -> usize {
        self.claimed_bytes - self.available_bytes - self.allocated_bytes
    }

    /// Returns the total number of allocated bytes freed.
    pub const fn total_freed_bytes(&self) -> u64 {
        self.total_allocated_bytes - self.allocated_bytes as u64
    }

    /// Returns the total number of claimed bytes released.
    pub const fn total_released_bytes(&self) -> u64 {
        self.total_claimed_bytes - self.claimed_bytes as u64
    }

    pub(crate) fn account_register_gap(&mut self, size: usize) {
        self.available_bytes += size;
        self.fragment_count += 1;
    }
    pub(crate) fn account_deregister_gap(&mut self, size: usize) {
        self.available_bytes -= size;
        self.fragment_count -= 1;
    }

    pub(crate) fn account_alloc(&mut self, alloc_size: usize) {
        self.allocation_count += 1;
        self.allocated_bytes += alloc_size;

        self.total_allocation_count += 1;
        self.total_allocated_bytes += alloc_size as u64;
    }

    pub(crate) fn account_dealloc(&mut self, alloc_size: usize) {
        self.allocation_count -= 1;
        self.allocated_bytes -= alloc_size;
    }

    pub(crate) fn account_grow_in_place(&mut self, old_alloc_size: usize, new_alloc_size: usize) {
        self.allocated_bytes += new_alloc_size - old_alloc_size;
        self.total_allocated_bytes += (new_alloc_size - old_alloc_size) as u64;
    }

    pub(crate) fn account_shrink_in_place(&mut self, old_alloc_size: usize, new_alloc_size: usize) {
        self.allocated_bytes -= old_alloc_size - new_alloc_size;
        self.total_allocated_bytes -= (old_alloc_size - new_alloc_size) as u64;
    }

    pub(crate) fn account_claim(&mut self, claimed_size: usize) {
        self.heap_count += 1;
        self.claimed_bytes += claimed_size;

        self.total_heap_count += 1;
        self.total_claimed_bytes += claimed_size as u64;
    }

    pub(crate) fn account_extend(&mut self, old_claimed_size: usize, new_claimed_size: usize) {
        self.claimed_bytes += new_claimed_size - old_claimed_size;
        self.total_claimed_bytes += (new_claimed_size - old_claimed_size) as u64;
    }

    pub(crate) fn account_truncate(&mut self, old_claimed_size: usize, new_claimed_size: usize) {
        if old_claimed_size != 0 && new_claimed_size == 0 {
            self.heap_count -= 1;
        }

        self.claimed_bytes -= old_claimed_size - new_claimed_size;
    }
}

impl<O: super::OomHandler> super::Talc<O> {
    pub fn get_counters(&self) -> &Counters {
        &self.counters
    }
}

#[cfg(test)]
mod tests {
    use core::alloc::Layout;

    use ptr_utils::{WORD_BITS, WORD_SIZE};

    use crate::{*, talc::TAG_SIZE};

    #[test]
    fn test_claim_alloc_free_truncate() {
        let mut arena = [0u8; 1000000];

        let mut talc = Talc::new(ErrOnOom);

        let low = 99;
        let high = 10001;
        let heap1 = unsafe {
            talc.claim(arena.get_mut(low..high).unwrap().into()).unwrap()
        };

        let pre_alloc_claimed_bytes = talc.get_counters().claimed_bytes;
        assert!(talc.get_counters().claimed_bytes == heap1.size());
        assert!(talc.get_counters().claimed_bytes <= high - low);
        assert!(talc.get_counters().claimed_bytes >= high - low - 16);
        assert!(talc.get_counters().claimed_bytes == talc.get_counters().total_claimed_bytes as _);

        let pre_alloc_avl_bytes = talc.get_counters().available_bytes;
        dbg!(pre_alloc_avl_bytes);
        assert!(talc.get_counters().available_bytes < high - low - WORD_SIZE * WORD_BITS * 2);
        assert!(talc.get_counters().available_bytes >= high - low - WORD_SIZE * WORD_BITS * 2 - 64);

        assert!(talc.get_counters().allocated_bytes == 0);
        assert!(talc.get_counters().total_allocated_bytes == 0);

        assert!(talc.get_counters().allocation_count == 0);
        assert!(talc.get_counters().total_allocation_count == 0);
        assert!(talc.get_counters().fragment_count == 1);
        assert!(talc.get_counters().overhead_bytes() >= TAG_SIZE + WORD_SIZE * WORD_BITS * 2);
        assert!(talc.get_counters().overhead_bytes() <= TAG_SIZE + WORD_SIZE * WORD_BITS * 2 + 64);

        let alloc_layout = Layout::new::<[u128; 3]>();
        let alloc = unsafe {
            talc.malloc(alloc_layout).unwrap()
        };

        assert!(talc.get_counters().claimed_bytes == pre_alloc_claimed_bytes);
        assert!(talc.get_counters().available_bytes < pre_alloc_avl_bytes - alloc_layout.size());
        assert!(talc.get_counters().available_bytes < pre_alloc_avl_bytes - alloc_layout.size());
        assert!(talc.get_counters().allocated_bytes == alloc_layout.size());
        assert!(talc.get_counters().total_allocated_bytes == alloc_layout.size() as _);
        assert!(talc.get_counters().allocation_count == 1);
        assert!(talc.get_counters().total_allocation_count == 1);
        dbg!(talc.get_counters().fragment_count);
        assert!(matches!(talc.get_counters().fragment_count, 1..=2));

        assert!(talc.get_counters().overhead_bytes() >= 2 * TAG_SIZE);
        
        unsafe {
            talc.free(alloc, alloc_layout);
        }

        assert!(talc.get_counters().claimed_bytes == pre_alloc_claimed_bytes);
        assert!(talc.get_counters().total_claimed_bytes == pre_alloc_claimed_bytes as _);
        assert!(talc.get_counters().available_bytes == pre_alloc_avl_bytes);
        assert!(talc.get_counters().allocated_bytes == 0);
        assert!(talc.get_counters().total_allocated_bytes == alloc_layout.size() as _);
        assert!(talc.get_counters().allocation_count == 0);
        assert!(talc.get_counters().total_allocation_count == 1);
        assert!(talc.get_counters().fragment_count == 1);

        let heap1 = unsafe {
            talc.truncate(heap1, talc.get_allocated_span(heap1))
        };

        assert!(heap1.size() <= TAG_SIZE + WORD_SIZE * WORD_BITS * 2 + 64);

        assert!(talc.get_counters().claimed_bytes == heap1.size());
        assert!(talc.get_counters().overhead_bytes() == talc.get_counters().claimed_bytes);
        assert!(talc.get_counters().total_claimed_bytes == pre_alloc_claimed_bytes as _);
        assert!(talc.get_counters().available_bytes == 0);
        assert!(talc.get_counters().allocated_bytes == 0);
        assert!(talc.get_counters().total_allocated_bytes == alloc_layout.size() as _);
        assert!(talc.get_counters().allocation_count == 0);
        assert!(talc.get_counters().total_allocation_count == 1);
        assert!(talc.get_counters().fragment_count == 0);
    }
}
