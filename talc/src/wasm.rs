#![doc = include_str!("../README_WASM.md")]

use crate::{
    Arena, Binning, ClaimOnOom,
    cell::{TalcCell, TalcCellAssumeSingleThreaded},
};

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
pub type WasmArenaTalc = TalcCellAssumeSingleThreaded<ClaimOnOom, WasmBinning>;

/// Yields a [`GlobalAlloc`](core::alloc::GlobalAlloc) implementation that
/// allocates out of a fixed-size region of memory.
///
/// See the [module docs](self) for more details.
/// 
/// # Safety
/// The target must be exclusively single-threaded.
pub const unsafe fn new_wasm_arena_allocator<T, const N: usize>(
    arena: *mut [T; N],
) -> WasmArenaTalc {
    TalcCellAssumeSingleThreaded::new(TalcCell::new(ClaimOnOom::array(arena)))
}

/// Type alias for the return value of [`new_wasm_dynamic_allocator`].
pub type WasmDynamicTalc = TalcCellAssumeSingleThreaded<ClaimWasmMemOnOom, WasmBinning>;

/// Yields a [`GlobalAlloc`](core::alloc::GlobalAlloc) implementation that
/// dynamically requests memory from the WebAssembly memory space as needed.
///
/// See the [module docs](self) for more details.
/// 
/// # Safety
/// The target must be exclusively single-threaded.
pub const unsafe fn new_wasm_dynamic_allocator() -> WasmDynamicTalc {
    TalcCellAssumeSingleThreaded::new(TalcCell::new(ClaimWasmMemOnOom))
}

/// This OOM handler requests memory from the WebAssembly memory subsystem as needed.
///
/// Unlike [`ExtendWasmMemOnOom`] it always creates new heaps; never extends the previous.
/// This increases fragmentation, decreasing memory efficiency somewhat,
/// but makes the compiled WebAssembly module smaller.
#[derive(Debug)]
pub struct ClaimWasmMemOnOom;

unsafe impl<B: Binning> crate::oom::OomHandler<B> for ClaimWasmMemOnOom {
    fn handle_oom(
        talc: &mut crate::base::Talc<Self, B>,
        layout: core::alloc::Layout,
    ) -> Result<(), ()> {
        // Growth strategy: just try to grow enough to avoid OOM again on this allocation
        // Performance testing shows that it works well even in random actions.
        let delta_pages = (layout.size() + crate::base::CHUNK_UNIT + (PAGE_SIZE - 1)) / PAGE_SIZE;

        let prev_memory_acme = match memory_grow::<0>(delta_pages) {
            usize::MAX => return Err(()),
            prev => prev,
        };

        let grown_base = (prev_memory_acme * PAGE_SIZE) as *mut u8;
        let grown_size = delta_pages * PAGE_SIZE;

        // This should always succeed. If it doesn't though, return Err(())
        match unsafe { talc.claim(grown_base, grown_size) } {
            Some(_) => Ok(()),
            None => Err(()),
        }
    }
}

/// This OOM handler requests memory from the WebAssembly memory subsystem as needed.
///
/// Unlike [`ClaimWasmMemOnOom`] it attempts to extend the heap instead of establishing
/// new heaps. This reduced fragmentation, increasing memory efficiency somewhat,
/// but makes the compiled WebAssembly module bigger.
#[derive(Debug, Default)]
pub struct ExtendWasmMemOnOom {
    top_arena: Option<Arena>,
}

impl ExtendWasmMemOnOom {
    /// Create a [`ExtendWasmMemOnOom`].
    ///
    /// This is an OOM handler that requests more memory
    /// from the WebAssembly memory subsystem as needed,
    /// extending the arena to encompass the additional memory.
    pub const fn new() -> Self {
        Self { top_arena: None }
    }
}

// SAFETY: does not invoke a Rust allocator or use allocated container types.
unsafe impl<B: Binning> crate::oom::OomHandler<B> for ExtendWasmMemOnOom {
    fn handle_oom(
        talc: &mut crate::base::Talc<Self, B>,
        layout: core::alloc::Layout,
    ) -> Result<(), ()> {

        // growth strategy: just try to grow enough to avoid OOM again on this allocation
        let delta_pages = (layout.size() + crate::base::CHUNK_UNIT + (PAGE_SIZE - 1)) / PAGE_SIZE;

        let prev_memory_acme = match memory_grow::<0>(delta_pages) {
            usize::MAX => return Err(()),
            prev => prev,
        };

        let grown_base = (prev_memory_acme * PAGE_SIZE) as *mut u8;
        let grown_size = delta_pages * PAGE_SIZE;

        // try to get base & acme, which will fail if prev_heap is empty
        // otherwise the allocator has been initialized previously
        if let Some(mut top_arena) = talc.oom_handler.top_arena.take() {
            if top_arena.end() == grown_base {
                unsafe {
                    talc.extend(&mut top_arena, grown_size);
                }
                talc.oom_handler.top_arena = Some(top_arena);

                return Ok(());
            }
        }

        talc.oom_handler.top_arena = unsafe { talc.claim(grown_base, grown_size) };

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