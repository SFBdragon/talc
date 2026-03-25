//! This is a test that guards again a pathological case reported by mkroening here:
//! https://github.com/SFBdragon/talc/issues/44
//!
//! This fix was to add `.max(layout.align())` in the allocation routine to avoid
//! searching in bins unlikely to have a well-aligned gap chunk.
//! Removing this should cause the assertion in this binary to fail.
//!
//! Run with:
//! `cargo run -p benches --bin high_alignment_test --release`
//!
//! This benchmark needs to be run in release, so it's not suitable to be a normal test.
//! This is run with `just check` and in the GitHub CI instead to avoid a silent regression.

use std::{
    alloc::{GlobalAlloc, Layout},
    time::{Duration, Instant},
};

use talc::{TalcCell, source::Manual};

fn main() {
    let arena_size = 0x1000_0000; // 256 MiB
    let mut arena = vec![0u8; arena_size].into_boxed_slice();
    // Warm up caches
    arena.fill(1);
    let arena = Box::into_raw(arena);

    let low_align_layout = Layout::from_size_align(4096, 1).unwrap();
    let high_align_layout = Layout::from_size_align(1, 4096).unwrap();

    let mut low_align_elapsed: Option<Duration> = None;

    for layout in [low_align_layout, high_align_layout] {
        let count = 0x4000;
        let mut v = Vec::with_capacity(count);

        let talc = TalcCell::new(Manual);

        unsafe {
            talc.claim(arena.cast(), arena_size);
        }

        let now = Instant::now();

        for _ in 0..count {
            let ptr = unsafe { talc.alloc(layout) };
            assert!(!ptr.is_null());
            v.push(ptr);
        }

        for ptr in v.drain(..) {
            unsafe {
                talc.dealloc(ptr, layout);
            }
        }

        let elapsed = now.elapsed();

        if let Some(low_align_elapsed) = low_align_elapsed {
            eprintln!("high align elapsed: {:?}", elapsed);

            // Ensure that the performance hit for high alignments was not large.
            // 5x provides a large margin of error. Usually the penalty is closer to 1.1-2x
            // as a result of the extra work needed to handle high-alignment allocation requests on average.
            assert!(low_align_elapsed.as_secs_f64() * 5.0 > elapsed.as_secs_f64());
        } else {
            eprintln!("low align elapsed: {:?}", elapsed);
            low_align_elapsed = Some(elapsed)
        }
    }
}
