use std::alloc::Layout;

use wasm_bindgen::prelude::*;

/* #[cfg(all(feature = "talc", not(feature = "talc_arena")))]
#[global_allocator]
static TALC: talc::sync::Talck<spin::Mutex<()>, talc::wasm::ClaimWasmMemOnOom, talc::wasm::WasmBinning>
    = talc::sync::Talck::new(talc::wasm::ClaimWasmMemOnOom::new());

#[cfg(all(feature = "talc", feature = "talc_arena"))]
#[global_allocator]
static TALC_ARENA: talc::sync::Talck<spin::Mutex<()>, talc::ClaimOnOom, talc::wasm::WasmBinning> = {
    use core::mem::MaybeUninit;
    static mut MEMORY: [MaybeUninit<u8>; 128 * 1024 * 1024] = [MaybeUninit::uninit(); 128 * 1024 * 1024];

    unsafe {
        talc::sync::Talck::new(talc::ClaimOnOom::slice(&raw mut MEMORY))
    }
}; */

#[cfg(all(feature = "talc", not(feature = "talc_arena")))]
#[global_allocator]
static TALC: talc::wasm::WasmDynamicTalc = unsafe { talc::wasm::new_wasm_dynamic_allocator() };

#[cfg(all(feature = "talc", feature = "talc_arena"))]
#[global_allocator]
static TALC_ARENA: talc::wasm::WasmArenaTalc = {
    use core::mem::MaybeUninit;
    static mut MEMORY: [MaybeUninit<u8>; 128 * 1024 * 1024] =
        [MaybeUninit::uninit(); 128 * 1024 * 1024];

    unsafe { talc::wasm::new_wasm_arena_allocator(&raw mut MEMORY) }
};

#[cfg(all(feature = "rlsf", not(feature = "rlsf_small")))]
#[global_allocator]
static RLSF: rlsf::GlobalTlsf = rlsf::GlobalTlsf::new();

#[cfg(feature = "rlsf_small")]
#[global_allocator]
static RLSF: rlsf::SmallGlobalTlsf = rlsf::SmallGlobalTlsf::new();

#[cfg(feature = "lol_alloc")]
#[global_allocator]
static LOL_ALLOC: lol_alloc::AssumeSingleThreaded<lol_alloc::FreeListAllocator> =
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::FreeListAllocator::new()) };

#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = ["process", "stdout"])]
    fn write(data: &str);
}

const ACTIONS: usize = 100000;
const ITERATIONS: usize = 100;

const TARGET_MIN_ACTIVE_ALLOCATIONS: usize = 50;

#[wasm_bindgen]
pub fn bench() {
    console_error_panic_hook::set_once();

    let timer = web_sys::window().unwrap().performance().unwrap();

    for realloc in [false, true] {
        // warm up
        random_actions(realloc);

        // go!
        let start = timer.now();
        for _ in 0..ITERATIONS {
            random_actions(realloc);
        }
        let end = timer.now();

        // log durations
        let total_ms = end - start;
        let average_ms = total_ms / ITERATIONS as f64;
        let actions_per_microsecond = ACTIONS as f64 / average_ms / 1000.0;

        write(&format!(" {:.2}", actions_per_microsecond));
    }
}

fn random_actions(realloc: bool) {
    let mut score = 0;
    let mut v = Vec::with_capacity(10000);

    while score < ACTIONS {
        let action = fastrand::usize(0..(4 + realloc as usize));

        if v.len() < TARGET_MIN_ACTIVE_ALLOCATIONS || action < 2 {
            let size = fastrand::usize(1..=10000);
            let align = 8 << fastrand::u16(..).trailing_zeros() / 2;
            let layout = Layout::from_size_align(size, align).unwrap();

            let allocation = unsafe { std::alloc::alloc(layout) };

            if !allocation.is_null() {
                v.push((allocation, layout));
                score += 1;
            }
        } else if action < 4 {
            if !v.is_empty() {
                let index = fastrand::usize(0..v.len());
                let (ptr, layout) = v.swap_remove(index);

                unsafe {
                    std::alloc::dealloc(ptr, layout);
                }

                score += 1;
            }
        } else {
            if !v.is_empty() {
                let index = fastrand::usize(0..v.len());
                if let Some((ptr, layout)) = v.get_mut(index) {
                    let new_size = fastrand::usize(1..=10000);

                    unsafe {
                        let realloc = std::alloc::realloc(*ptr, *layout, new_size);

                        if !realloc.is_null() {
                            *ptr = realloc;
                            *layout = Layout::from_size_align_unchecked(new_size, layout.align());
                            score += 1;
                        }
                    }
                }
            }
        }
    }

    for (ptr, layout) in v {
        unsafe {
            std::alloc::dealloc(ptr, layout);
        }
    }
}
