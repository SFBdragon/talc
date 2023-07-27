use core::alloc::Layout;

use crate::{Span, Talc};

pub trait OomHandler: Sized {
    /// Given the allocator and the `layout` of the allocation that caused
    /// OOM, resize the arena and return `Ok(())` or fail by returning `Err(())`.
    ///
    /// This function is called repeatedly if the arena was insufficiently extended.
    /// Therefore an infinite loop will occur if `Ok(())` is repeatedly returned
    /// without extending the arena.
    fn handle_oom(talc: &mut Talc<Self>, layout: Layout) -> Result<(), ()>;
}

/// An out-of-memory handler that simply returns [`Err`].
pub struct ErrOnOom;

impl OomHandler for ErrOnOom {
    fn handle_oom(_: &mut Talc<Self>, _: Layout) -> Result<(), ()> {
        Err(())
    }
}

/// An out-of-memory handler that initializes the [`Talc`]'s arena
/// to the given [`Span`] on OOM if it has not been initialized already.
///
/// Otherwise, this returns [`Err`].
pub struct InitOnOom(Span);

impl InitOnOom {
    /// # Safety
    /// The memory within the given [`Span`] must conform to
    /// the requirements laid out by [`Talc::init`].
    pub const unsafe fn new(span: Span) -> Self {
        InitOnOom(span)
    }
}

impl OomHandler for InitOnOom {
    fn handle_oom(talc: &mut Talc<Self>, _: Layout) -> Result<(), ()> {
        if talc.get_allocatable_span().is_empty() {
            unsafe {
                talc.init(talc.oom_handler.0);
            }

            Ok(())
        } else {
            Err(())
        }
    }
}
