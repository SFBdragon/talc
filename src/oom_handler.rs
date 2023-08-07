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
        if talc.get_arena().is_empty() && !talc.oom_handler.0.is_empty() {
            unsafe {
                talc.init(talc.oom_handler.0);
            }

            Ok(())
        } else {
            Err(())
        }
    }
}


#[cfg(target_family = "wasm")]
pub struct WasmHandler;

#[cfg(target_family = "wasm")]
impl OomHandler for WasmHandler {
    fn handle_oom(talc: &mut Talc<Self>, layout: Layout) -> Result<(), ()> {

        /// WASM page size is 64KiB
        const PAGE_SIZE: usize = 1024 * 64;

        // growth strategy: just try to grow enough to avoid OOM again on this allocation
        let required = (layout.size() + 8).max(layout.align() * 2);
        let mut delta_pages = (required + (PAGE_SIZE - 1)) / PAGE_SIZE;
        
        let prev = 'prev: { 
            // this performs a scan, trying to find a smaller possible
            // growth if the previous one was unsuccessful. return
            // any successful allocated to memory, and try again.
            
            // if we're about to fail because of allocation failure
            // we may as well try as hard as we can to probe what's permissable
            // which can be done with a log2(n)-ish algorithm
            while delta_pages != 0 {
                // use `core::arch::wasm` instead once it doesn't 
                // require the unstable feature wasm_simd64?
                let result = core::arch::wasm32::memory_grow::<0>(delta_pages);

                if result != usize::MAX {
                    break 'prev result;
                } else {
                    delta_pages >>= 1;
                    continue;
                }
            }

            return Err(());
        };

        // taking ownership from the bottom seems to cause problems
        // so only cover grown memory

        unsafe {
            talc.extend(Span::new(
                talc
                    .get_arena()
                    .get_base_acme()
                    .map_or((prev * PAGE_SIZE) as _, |(base, _)| base), 

                ((prev + delta_pages) * PAGE_SIZE) as *mut u8,
            ));
        }

        Ok(())
    }
}
