use std::alloc::Layout;

use wasm_bindgen::prelude::*;


#[cfg(all(feature = "talc", not(feature = "talc_arena")))]
#[global_allocator]
static TALCK: talc::TalckWasm = unsafe { talc::TalckWasm::new_global() };


#[cfg(feature = "talc_arena")]
#[global_allocator]
static ALLOCATOR: talc::Talck<talc::locking::AssumeUnlockable, talc::ClaimOnOom> = {
    static mut MEMORY: [std::mem::MaybeUninit<u8>; 32 * 1024 * 1024]
        = [std::mem::MaybeUninit::uninit(); 32 * 1024 * 1024];
    let span = talc::Span::from_const_array(unsafe { std::ptr::addr_of!(MEMORY) });
    talc::Talc::new(unsafe { talc::ClaimOnOom::new(span) }).lock()
};

#[cfg(feature = "lol_alloc")]
#[global_allocator] static ALLOC: lol_alloc::AssumeSingleThreaded<lol_alloc::FreeListAllocator> = 
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::FreeListAllocator::new()) };


#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

const ACTIONS: usize = 100000;
const ITERATIONS: usize = 100;

#[wasm_bindgen]
pub fn bench() {
    console_error_panic_hook::set_once();

    let timer = web_sys::window().unwrap().performance().unwrap();

    // warm up
    random_actions();

    // go!
    let start = timer.now();
    for _ in 0..ITERATIONS { random_actions(); }
    let end = timer.now();

    // log durations
    let total_ms = end - start;
    let average_ms = total_ms / ITERATIONS as f64;
    let apms = ACTIONS as f64 / average_ms / 1000.0;
    log(format!("  total time: {} ms", total_ms).as_str());
    log(format!("  average time: {} ms", average_ms).as_str());
    log(format!("  average actions/s: {:.1}", apms).as_str());
}

fn random_actions() {
    let mut score = 0;
    let mut v = Vec::with_capacity(10000);

    while score < 100000 {
        let action = fastrand::usize(0..3);

        match action {
            0 => {
                let size = fastrand::usize(100..=1000);
                let align = 8 << fastrand::u16(..).trailing_zeros() / 2;
                let layout = Layout::from_size_align(size, align).unwrap();

                let allocation = unsafe { std::alloc::alloc(layout) };

                if !allocation.is_null() {
                    v.push((allocation, layout));
                    score += 1;
                }
            }
            1 => {
                if !v.is_empty() {
                    let index = fastrand::usize(0..v.len());
                    let (ptr, layout) = v.swap_remove(index);

                    unsafe {
                        std::alloc::dealloc(ptr, layout);
                    }

                    score += 1;
                }
            }
            2 => {
                if !v.is_empty() {
                    let index = fastrand::usize(0..v.len());
                    if let Some((ptr, layout)) = v.get_mut(index) {
                        let new_size = fastrand::usize(100..=10000);

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
            _ => unreachable!(),
        }
    }

    for (ptr, layout) in v {
        unsafe { std::alloc::dealloc(ptr, layout); }
    }
}
