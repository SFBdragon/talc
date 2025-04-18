use core::{
    fmt::Debug,
    mem::{align_of, size_of},
    ptr::NonNull,
    ptr::addr_of_mut,
};

use allocator_api2::alloc::{Allocator, Layout};

use crate::{
    base::binning::Binning,
    base::{CHUNK_UNIT, Talc},
    node::Node,
    ptr_utils,
};

use super::Source;

/// Source memory from a backing allocator on-demand.
///
/// This will also release memory back to the allocator when memory blocks are freed up.
///
/// # Example
///
/// ```
/// # extern crate talc;
/// use talc::prelude::*;
/// use allocator_api2::alloc::{Allocator, System};
///
/// let talc = TalcCell::new(AllocatorSource::new(System));
/// let allocation = talc.allocate(Layout::new::<[usize; 500]>());
/// ```
#[derive(Debug)]
pub struct AllocatorSource<A: Allocator> {
    block_size: usize,
    allocator: A,
    allocation_chain: Option<NonNull<Option<NonNull<Node>>>>,
}

/// 4 MiB, chosen pretty arbitrarily.
const DEFAULT_BLOCK_SIZE: usize = 4 << 20;

impl<G: Allocator> AllocatorSource<G> {
    /// Create a new [`AllocatorSource`] with the given allocator.
    ///
    /// A default minimum block size per allocation is used.
    /// This is subject to change. If you need a specific value,
    /// use [`AllocatorSource::with_block_size`] instead.
    pub const fn new(allocator: G) -> Self {
        Self { block_size: DEFAULT_BLOCK_SIZE, allocator, allocation_chain: None }
    }

    /// Create a new [`AllocatorSource`] with the given allocator and power-of-two block size.
    ///
    /// # Panics
    ///
    /// Panics if `block_size` is not a power of two. This might be relaxed in the future.
    pub const fn with_block_size(allocator: G, block_size: usize) -> Self {
        assert!(block_size.is_power_of_two());

        Self { block_size, allocator, allocation_chain: None }
    }
}

unsafe impl<G: Allocator + Debug> Source for AllocatorSource<G> {
    fn acquire<B: Binning>(talc: &mut Talc<Self, B>, layout: Layout) -> Result<(), ()> {
        // Account for the size and potential overhead from alignment.
        // Allocating extra space isn't a big deal; more space for future
        // allocations to make use of.
        let mut required_size = layout.size() + layout.align();

        // Extra space for Talc's internal heap alignment on either side.
        // This is more than absolutely necessary but whatever.
        required_size += CHUNK_UNIT + CHUNK_UNIT;
        // Extra space for the footer.
        required_size += size_of::<Footer>();

        if !talc.is_metadata_established() {
            required_size += crate::min_first_heap_layout::<B>().size();
            // Ensure there's additional space to establish the chain pointer too.
            // The metadata is just a `Option<NonNull<Node>>` but we allocate it, so a
            // CHUNK_UNIT gets consumed.
            required_size += size_of::<Option<NonNull<Node>>>();
        }

        let required_blocks =
            (required_size + talc.source.block_size - 1) & !(talc.source.block_size - 1);

        debug_assert!(CHUNK_UNIT > align_of::<Footer>());
        let layout = unsafe { Layout::from_size_align_unchecked(required_blocks, BLOCK_ALIGN) };

        let base = match talc.source.allocator.allocate(layout) {
            Ok(allocation) => allocation.cast::<u8>(),
            Err(_) => return Err(()),
        };

        let mut base_offset = 0;

        let meta = if let Some(meta) = talc.source.allocation_chain {
            meta.as_ptr()
        } else {
            let meta = ptr_utils::align_up_by(base.as_ptr(), align_of::<Option<NonNull<Node>>>())
                .cast::<Option<NonNull<Node>>>();
            base_offset =
                size_of::<Option<NonNull<Node>>>() + meta as usize - base.as_ptr() as usize;

            let allocation_chain = NonNull::new(meta);
            debug_assert!(allocation_chain.is_some());
            talc.source.allocation_chain = allocation_chain;

            meta
        };

        let heap_end = unsafe {
            talc.claim(
                base.as_ptr().wrapping_add(base_offset),
                required_blocks - base_offset - size_of::<Footer>(),
            )
            .unwrap_unchecked()
        };

        unsafe {
            let footer = heap_end.as_ptr().cast::<Footer>();
            Node::link_at(addr_of_mut!((*footer).node), Node { next: *meta, next_of_prev: meta });
            (*footer).base = base;
            (*footer).size = required_blocks;
        }

        Ok(())
    }

    const TRACK_HEAP_END: bool = true;

    unsafe fn resize(
        &mut self,
        chunk_base: *mut u8,
        heap_end: *mut u8,
        is_heap_base: bool,
    ) -> *mut u8 {
        if is_heap_base {
            let footer = heap_end.cast::<Footer>();
            Node::unlink((*footer).node);

            let layout = Layout::from_size_align_unchecked((*footer).size, BLOCK_ALIGN);
            self.allocator.deallocate((*footer).base, layout);

            chunk_base
        } else {
            heap_end
        }
    }
}

impl<G: Allocator> Drop for AllocatorSource<G> {
    fn drop(&mut self) {
        if let Some(chain) = self.allocation_chain {
            unsafe {
                for node_ptr in Node::iter_mut(chain.as_ptr().read()) {
                    let footer = node_ptr.cast::<Footer>().as_ptr();
                    let layout = Layout::from_size_align_unchecked((*footer).size, CHUNK_UNIT);
                    self.allocator.deallocate((*footer).base, layout);
                }
            }
        }
    }
}

#[repr(C)] // ensure the node ptr is the same as the footer ptr
struct Footer {
    node: Node,
    base: NonNull<u8>,
    size: usize,
}

const BLOCK_ALIGN: usize = 1;
