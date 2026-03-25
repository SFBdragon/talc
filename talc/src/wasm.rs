#![doc = include_str!("../README_WASM.md")]

use core::ptr::NonNull;

use crate::{base::binning::Binning, cell::TalcSyncCell, ptr_utils, source::Claim};

/// A binning configuration optimized for WebAssembly.
///
/// Why does WebAssembly get its own bin config?
/// - `wasm32` has instructions for 64-bit numbers, which requires
///     less instructions than using two `usize`s for the availability bitfield.
pub struct WasmBinning;
impl Binning for WasmBinning {
    #[cfg(not(target_arch = "wasm64"))]
    type AvailabilityBitField = u64;

    #[cfg(target_arch = "wasm64")]
    type AvailabilityBitField = [usize; 2];

    fn size_to_bin(size: usize) -> u32 {
        #[cfg(not(target_arch = "wasm64"))]
        {
            crate::base::binning::linear_extent_then_linearly_divided_exponential_binning::<2, 8>(
                size,
            )
        }

        #[cfg(target_arch = "wasm64")]
        {
            crate::base::binning::linear_extent_then_linearly_divided_exponential_binning::<4, 8>(
                size,
            )
        }
    }
}

/// Type alias for the return value of [`new_wasm_arena_allocator`].
pub type WasmArenaTalc = TalcSyncCell<Claim, WasmBinning>;

/// Yields a [`GlobalAlloc`](core::alloc::GlobalAlloc) implementation that
/// allocates out of a fixed-size region of memory.
///
/// See the [module docs](self) for more details.
///
/// # Panics
///
/// Panics if the target is not single-threaded WebAssembly.
/// This is required to avoid creating a `TalcSyncCell` on
/// a platform where it is unsafe.
///
/// # Safety
///
/// The safety invariants required by [`Claim::array`] must be upheld for `arena`.
///
/// In short, you're handing full control of the memory to the allocator;
/// do not mutate it until the allocator is dropped or the memory arena is manually truncated.
///
/// # Example
///
/// ```
/// #[cfg(all(not(target_feature = "atomics"), target_family = "wasm"))]
/// #[global_allocator]
/// static TALC: talc::wasm::WasmArenaTalc = {
///     use core::mem::MaybeUninit;
///     static mut MEMORY: [MaybeUninit<u8>; 0x8000000] = [MaybeUninit::uninit(); 0x8000000];
///     // SAFETY: the memory for MEMORY is never modified externally. It's the allocator's.
///     unsafe { talc::wasm::new_wasm_arena_allocator(&raw mut MEMORY) }
/// };
/// ```
pub const unsafe fn new_wasm_arena_allocator<T, const N: usize>(
    arena: *mut [T; N],
) -> WasmArenaTalc {
    TalcSyncCell::new_wasm(Claim::array(arena))
}

/// Type alias for the return value of [`new_wasm_dynamic_allocator`].
pub type WasmDynamicTalc = TalcSyncCell<WasmGrowAndClaim, WasmBinning>;

/// Yields a [`GlobalAlloc`](core::alloc::GlobalAlloc) implementation that
/// dynamically requests memory from the WebAssembly memory space as needed.
///
/// See the [module docs](self) for more details.
///
/// # Panics
///
/// Panics if the target is not single-threaded WebAssembly.
/// This is required to avoid creating a `TalcSyncCell` on
/// a platform where it is unsafe.
///
/// # Examples
///
/// ```
/// use talc::wasm::*;
///
/// #[cfg(all(not(target_feature = "atomics"), target_family = "wasm"))]
/// #[global_allocator]
/// static TALC: WasmDynamicTalc = new_wasm_dynamic_allocator();
/// ```
pub const fn new_wasm_dynamic_allocator() -> WasmDynamicTalc {
    TalcSyncCell::new_wasm(WasmGrowAndClaim)
}

/// This source requests memory from the WebAssembly memory subsystem as needed.
///
/// Unlike [`WasmGrowAndExtend`] it always creates new heaps; never extends the previous.
/// This increases fragmentation, decreasing memory efficiency somewhat,
/// but makes the compiled WebAssembly module smaller.
///
/// # Heap management
///
/// Manual heap management (i.e. using [`Talc::claim`](crate::base::Talc::claim),
/// [`Talc::resize`](crate::base::Talc::resize), etc.) directly is not allowed, and will cause UB.
#[derive(Debug)]
pub struct WasmGrowAndClaim;

unsafe impl crate::source::Source for WasmGrowAndClaim {
    fn acquire<B: Binning>(
        talc: &mut crate::base::Talc<Self, B>,
        layout: core::alloc::Layout,
    ) -> Result<(), ()> {
        // Growth strategy: just try to grow enough to avoid OOM again on this allocation
        // Performance testing shows that it works well even in random actions.
        let delta_pages = (layout.size() + crate::base::CHUNK_UNIT + (PAGE_SIZE - 1)) / PAGE_SIZE;

        let prev_memory_end = match memory_grow::<0>(delta_pages) {
            usize::MAX => return Err(()),
            prev => prev,
        };

        let grown_base = (prev_memory_end * PAGE_SIZE) as *mut u8;
        let grown_size = delta_pages * PAGE_SIZE;

        // This should always succeed. If it doesn't though, return Err(())
        match unsafe { talc.claim(grown_base, grown_size) } {
            Some(_) => Ok(()),
            None => Err(()),
        }
    }
}

/// This source requests memory from the WebAssembly memory subsystem as needed.
///
/// Unlike [`WasmGrowAndClaim`] it attempts to extend the heap instead of establishing
/// new heaps. This reduced fragmentation, increasing memory efficiency somewhat,
/// but makes the compiled WebAssembly module a little bigger.
///
/// # Heap management
///
/// Manual heap management (i.e. using [`Talc::claim`](crate::base::Talc::claim),
/// [`Talc::resize`](crate::base::Talc::resize), etc.) directly is not allowed, and will cause UB.
#[derive(Debug, Default)]
pub struct WasmGrowAndExtend {
    end: Option<NonNull<u8>>,
}

impl WasmGrowAndExtend {
    /// Create a [`WasmGrowAndExtend`] source.
    ///
    /// This is an source that requests more memory
    /// from the WebAssembly memory subsystem as needed,
    /// extending the arena to encompass the additional memory.
    pub const fn new() -> Self {
        Self { end: None }
    }
}

// SAFETY: does not invoke a Rust allocator or use allocated container types.
unsafe impl crate::source::Source for WasmGrowAndExtend {
    fn acquire<B: Binning>(
        talc: &mut crate::base::Talc<Self, B>,
        layout: core::alloc::Layout,
    ) -> Result<(), ()> {
        // growth strategy: just try to grow enough to avoid OOM again on this allocation
        let delta_pages = (layout.size() + crate::base::CHUNK_UNIT + (PAGE_SIZE - 1)) / PAGE_SIZE;

        let prev_memory_end = match memory_grow::<0>(delta_pages) {
            usize::MAX => return Err(()),
            prev => prev,
        };

        let new_base = (prev_memory_end * PAGE_SIZE) as *mut u8;
        let new_bytes = delta_pages * PAGE_SIZE;
        let new_end = ptr_utils::saturating_ptr_add(new_base, new_bytes);

        // try to get base & end, which will fail if prev_heap is empty
        // otherwise the allocator has been initialized previously
        if let Some(old_end) = talc.source.end.take() {
            if old_end.as_ptr() == new_base {
                let new_end = unsafe { talc.extend(old_end, new_end) };
                talc.source.end = Some(new_end);

                return Ok(());
            }
        }

        talc.source.end = unsafe { talc.claim(new_base, new_bytes) };

        Ok(())
    }
}

/// WASM page size is 64KiB
const PAGE_SIZE: usize = 1024 * 64;

#[cfg(target_arch = "wasm32")]
use core::arch::wasm32::memory_grow;
#[cfg(target_arch = "wasm64")]
use core::arch::wasm64::memory_grow;
#[cfg(not(any(target_arch = "wasm32", target_arch = "wasm64")))]
fn memory_grow<const M: usize>(_pages: usize) -> usize {
    panic!("not running on wasm32 or wasm64")
}
