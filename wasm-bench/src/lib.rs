use std::alloc::Layout;

use wasm_bindgen::prelude::*;


#[cfg(all(feature = "talc", not(feature = "talc_static")))]
#[global_allocator]
static TALCK: talc::TalckWasm = unsafe { talc::TalckWasm::new_global() };

#[cfg(feature = "talc_static")]
#[global_allocator]
static ALLOCATOR: talc::Talck<talc::locking::AssumeUnlockable, talc::ClaimOnOom> = {
    static mut MEMORY: [std::mem::MaybeUninit<u8>; 64 * 1024 * 1024] =
        [std::mem::MaybeUninit::uninit(); 64 * 1024 * 1024];
    let span = talc::Span::from_base_size(unsafe { MEMORY.as_ptr() as *mut _ }, 64 * 1024 * 1024);
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

#[wasm_bindgen]
pub fn bench() {
    console_error_panic_hook::set_once();

    let iterations = 100;
    let timer = web_sys::window().unwrap().performance().unwrap();

    // warm up
    random_actions_10_000_000();

    // go!
    let start = timer.now();
    for _ in 0..iterations { random_actions_10_000_000(); }
    let end = timer.now();

    // log durations
    log(format!("total time: {} ms", end - start).as_str());
    log(format!("average time: {} ms", (end - start) / iterations as f64).as_str());
}

fn random_actions_10_000_000() {
    let mut score = 0;
    let mut v = Vec::with_capacity(10000);

    while score < 100_000 {
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
