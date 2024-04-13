use core::alloc::Layout;

use crate::{Span, Talc};

pub trait OomHandler: Sized {
    /// Given the allocator and the `layout` of the allocation that caused
    /// OOM, resize or claim and return `Ok(())` or fail by returning `Err(())`.
    ///
    /// This function is called repeatedly if the allocator is still out of memory.
    /// Therefore an infinite loop will occur if `Ok(())` is repeatedly returned
    /// without extending or claiming new memory.
    fn handle_oom(talc: &mut Talc<Self>, layout: Layout) -> Result<(), ()>;
}

/// Doesn't handle out-of-memory conditions, immediate allocation error occurs.
pub struct ErrOnOom;

impl OomHandler for ErrOnOom {
    fn handle_oom(_: &mut Talc<Self>, _: Layout) -> Result<(), ()> {
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

impl OomHandler for ClaimOnOom {
    fn handle_oom(talc: &mut Talc<Self>, _: Layout) -> Result<(), ()> {
        if !talc.oom_handler.0.is_empty() {
            unsafe {
                talc.claim(talc.oom_handler.0)?;
            }

            talc.oom_handler.0 = Span::empty();

            Ok(())
        } else {
            Err(())
        }
    }
}

#[cfg(all(target_family = "wasm", feature = "lock_api"))]
pub struct WasmHandler {
    prev_heap: Span,
}

#[cfg(all(target_family = "wasm", feature = "lock_api"))]
unsafe impl Send for WasmHandler {}

#[cfg(all(target_family = "wasm", feature = "lock_api"))]
impl WasmHandler {
    /// Create a new WASM handler.
    /// # Safety
    /// [`WasmHandler`] expects to have full control over WASM memory
    /// and be running in a single-threaded environment.
    pub const unsafe fn new() -> Self {
        Self { prev_heap: Span::empty() }
    }
}

#[cfg(all(target_family = "wasm", feature = "lock_api"))]
impl OomHandler for WasmHandler {
    fn handle_oom(talc: &mut Talc<Self>, layout: Layout) -> Result<(), ()> {
        /// WASM page size is 64KiB
        const PAGE_SIZE: usize = 1024 * 64;

        // growth strategy: just try to grow enough to avoid OOM again on this allocation
        let required = (layout.size() + 8).max(layout.align() * 2);
        let mut delta_pages = (required + (PAGE_SIZE - 1)) / PAGE_SIZE;

        let prev = 'prev: {
            // This performs a scan, trying to find a smaller possible
            // growth if the previous one was unsuccessful. Return
            // any successful allocated to memory.
            // If not quite enough, talc will invoke handle_oom again.

            // if we're about to fail because of allocation failure
            // we may as well try as hard as we can to probe what's permissable
            // which can be done with a log2(n)-ish algorithm
            // (factoring in repeated called to handle_oom)
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

        let prev_heap_acme = (prev * PAGE_SIZE) as *mut u8;
        let new_heap_acme = prev_heap_acme.wrapping_add(delta_pages * PAGE_SIZE);

        // try to get base & acme, which will fail if prev_heap is empty
        // otherwise the allocator has been initialized previously
        if let Some((prev_base, prev_acme)) = talc.oom_handler.prev_heap.get_base_acme() {
            if prev_acme == prev_heap_acme {
                talc.oom_handler.prev_heap = unsafe {
                    talc.extend(talc.oom_handler.prev_heap, Span::new(prev_base, new_heap_acme))
                };

                return Ok(());
            }
        }

        talc.oom_handler.prev_heap = unsafe {
            // delta_pages is always greater than zero
            // thus one page is enough space for metadata
            // therefore we can unwrap the result
            talc.claim(Span::new(prev_heap_acme, new_heap_acme)).unwrap()
        };

        Ok(())
    }
}
