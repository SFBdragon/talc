use core::alloc::Layout;

use crate::Span;
use super::{alignment::ChunkAlign, bucket_config::BucketConfig, Talc};

pub trait OomHandler<B: BucketConfig, A: ChunkAlign>: Sized {
    /// Given the allocator and the `layout` of the allocation that caused
    /// OOM, resize or claim and return `Ok(())` or fail by returning `Err(())`.
    ///
    /// This function is called repeatedly if the allocator is still out of memory.
    /// Therefore an infinite loop will occur if `Ok(())` is repeatedly returned
    /// without extending or claiming new memory.
    fn handle_oom(talc: &mut Talc<Self, B, A>, layout: Layout) -> Result<(), ()>;
}

/// Doesn't handle out-of-memory conditions, immediate allocation error occurs.
pub struct ErrOnOom;

impl<B: BucketConfig, A: ChunkAlign> OomHandler<B, A> for ErrOnOom {
    fn handle_oom(_: &mut Talc<Self, B, A>, _: Layout) -> Result<(), ()> {
        Err(())
    }
}

/// An out-of-memory handler that attempts to claim the
/// memory within the given [`Span`] upon OOM.
///
/// The contained span is then overwritten with an empty span.
///
/// If the span is empty or `claim` fails, allocation failure occurs.
pub struct ClaimOnOom(Span);

impl ClaimOnOom {
    /// # Safety
    /// The memory within the given [`Span`] must conform to
    /// the requirements laid out by [`claim`](Talc::claim).
    pub const unsafe fn new(span: Span) -> Self {
        ClaimOnOom(span)
    }
}

impl<B: BucketConfig, A: ChunkAlign> OomHandler<B, A> for ClaimOnOom {
    fn handle_oom(talc: &mut Talc<Self, B, A>, _: Layout) -> Result<(), ()> {
        if !talc.oom_handler.0.is_empty() {
            unsafe {
                talc.claim(talc.oom_handler.0).map_err(|_| ())?;
            }

            talc.oom_handler.0 = Span::empty();

            Ok(())
        } else {
            Err(())
        }
    }
}
