use core::{
    alloc::{GlobalAlloc, Layout},
    fmt::Debug,
    mem::{align_of, size_of},
    ptr::NonNull,
};

use crate::{
    Binning,
    base::{CHUNK_UNIT, Talc},
    node::Node,
    ptr_utils,
};

use super::OomHandler;

/// TODO
///
/// Talc's arenas' addresses can't be moved. `GlobalAlloc`'s `realloc` implementation
/// cannot be used as it might change the allocation position.
/// Therefore [`AllocOnOom`] only allocated and deallocates as memory is needed or no longer.
#[derive(Debug)]
pub struct AllocOnOom<const BLOCK: usize, G: GlobalAlloc> {
    allocator: G,
    allocation_chain: Option<NonNull<Option<NonNull<Node>>>>,
}

/// 2 MiB, chosen pretty arbitrarily.
const DEFAULT_BLOCK_SIZE: usize = 2 << 20;

impl<G: GlobalAlloc> AllocOnOom<DEFAULT_BLOCK_SIZE, G> {
    pub const fn new(allocator: G) -> Self {
        Self { allocator, allocation_chain: None }
    }
}

impl<G: GlobalAlloc, const BLOCK: usize> AllocOnOom<BLOCK, G> {
    pub const fn with_block_size(allocator: G) -> Self {
        Self { allocator, allocation_chain: None }
    }
}

unsafe impl<G: GlobalAlloc + Debug, B: Binning, const BLOCK: usize> OomHandler<B>
    for AllocOnOom<BLOCK, G>
{
    fn handle_oom(talc: &mut Talc<Self, B>, layout: Layout) -> Result<(), ()> {
        // Account for the size and potential overhead from alignment.
        // Allocating extra space isn't a big deal; more space for future
        // allocations to make use of.
        let mut required_size = layout.size() + layout.align();

        // Extra space for Talc's internal arena alignment on either side.
        // This is more than absolutely necessary but whatever.
        required_size += CHUNK_UNIT + CHUNK_UNIT;
        // Extra space for the footer.
        required_size += size_of::<Footer>();

        if !talc.is_metadata_established() {
            //
            required_size += crate::min_first_arena_layout::<B>().size();
            // Ensure there's additional space to establish this OOM handler's metadata too.
            // The metadata is just a `Option<NonNull<Node>>` but we allocate it, so a
            // CHUNK_UNIT gets consumed.
            required_size += size_of::<Option<NonNull<Node>>>();
        }

        let required_blocks = (required_size + BLOCK - 1) & !(BLOCK - 1);

        debug_assert!(CHUNK_UNIT > align_of::<Footer>());
        let layout = unsafe { Layout::from_size_align_unchecked(required_blocks, BLOCK_ALIGN) };
        let allocation = unsafe { talc.oom_handler.allocator.alloc(layout) };

        if allocation.is_null() {
            return Err(());
        }

        let mut base_offset = 0;

        let meta = if let Some(meta) = talc.oom_handler.allocation_chain {
            meta.as_ptr()
        } else {
            let meta = ptr_utils::align_up_by(allocation, align_of::<Option<NonNull<Node>>>())
                .cast::<Option<NonNull<Node>>>();
            base_offset = size_of::<Option<NonNull<Node>>>() + meta as usize - allocation as usize;

            let allocation_chain = NonNull::new(meta);
            debug_assert!(allocation_chain.is_some());
            talc.oom_handler.allocation_chain = allocation_chain;

            meta
        };

        let arena_end = unsafe {
            talc.claim(
                allocation.wrapping_add(base_offset),
                required_blocks - base_offset - size_of::<Footer>(),
            )
            .unwrap_unchecked()
        };

        unsafe {
            let footer = arena_end.as_ptr().cast::<Footer>();
            Node::link_at(&raw mut (*footer).node, Node { next: *meta, next_of_prev: meta });
            (*footer).base = allocation;
            (*footer).size = required_blocks;
        }

        Ok(())
    }

    const TRACK_ARENA_END: bool = true;

    unsafe fn maybe_resize_arena(
        &mut self,
        chunk_base: *mut u8,
        arena_end: *mut u8,
        is_arena_base: bool,
    ) -> *mut u8 {
        if is_arena_base {
            let footer = arena_end.cast::<Footer>();
            Node::unlink((*footer).node);

            let layout = Layout::from_size_align_unchecked((*footer).size, BLOCK_ALIGN);
            self.allocator.dealloc((*footer).base, layout);

            chunk_base
        } else {
            arena_end
        }
    }
}

impl<G: GlobalAlloc, const BLOCK: usize> Drop for AllocOnOom<BLOCK, G> {
    fn drop(&mut self) {
        if let Some(chain) = self.allocation_chain {
            unsafe {
                for node_ptr in Node::iter_mut(chain.as_ptr().read()) {
                    let footer = node_ptr.cast::<Footer>().as_ptr();
                    let layout = Layout::from_size_align_unchecked((*footer).size, CHUNK_UNIT);
                    self.allocator.dealloc((*footer).base, layout);
                }
            }
        }
    }
}

#[repr(C)] // ensure the node ptr is the same as the footer ptr
struct Footer {
    node: Node,
    base: *mut u8,
    size: usize,
}

const BLOCK_ALIGN: usize = 1;
