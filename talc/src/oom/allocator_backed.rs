use core::{
    alloc::{GlobalAlloc, Layout},
    fmt::Debug,
};

use crate::{
    Binning,
    base::{CHUNK_UNIT, Talc},
};

use super::OomHandler;

#[derive(Debug)]
pub struct AllocOnOom<G: GlobalAlloc>(G);

impl<G: GlobalAlloc> AllocOnOom<G> {
    pub const fn new(allocator: G) -> Self {
        Self(allocator)
    }
}

unsafe impl<G: GlobalAlloc + Debug, B: Binning> OomHandler<B> for AllocOnOom<G> {
    fn handle_oom(talc: &mut Talc<Self, B>, layout: Layout) -> Result<(), ()> {
        let mut required_size = layout.size() + CHUNK_UNIT;

        // TODO roundup/?/MINSIZE

        if !talc.is_metadata_established() {
            required_size += crate::min_first_arena_layout::<B>().size();
        }

        let layout = unsafe {
            Layout::from_size_align_unchecked(
                required_size,
                crate::min_first_arena_layout::<B>().align(),
            )
        };

        let allocation = unsafe { talc.oom_handler.0.alloc(layout) };

        if allocation.is_null() {
            return Err(());
        }

        let arena = unsafe { talc.claim(allocation, layout.size()).unwrap_unchecked() };

        // TODO check _arena

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
            // todo
            todo!()
        } else {
            arena_end
        }
    }
}
