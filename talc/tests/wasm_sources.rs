//! Regression tests for the WebAssembly sources. Run with:
//! `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner WASM_BINDGEN_TEST_ONLY_NODE=1 cargo test --target wasm32-unknown-unknown --test wasm_sources`
#![cfg(all(target_family = "wasm", not(target_feature = "atomics")))]

use core::alloc::{GlobalAlloc, Layout};

use talc::base::Talc;
use talc::cell::TalcCell;
use talc::wasm::{WasmBinning, WasmGrowAndClaim, WasmGrowAndExtend};
use wasm_bindgen_test::wasm_bindgen_test;

fn memory_pages() -> usize {
    core::arch::wasm32::memory_size::<0>()
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn random_layout(rng: &mut Lcg) -> Layout {
    let size = match rng.below(100) {
        0..=64 => 1 + rng.below(4096) as usize,
        65..=84 => 4096 + rng.below(61440) as usize,
        85..=92 => (1 + rng.below(16) as usize) * 65536,
        _ => 65536 + rng.below(2 * 1024 * 1024) as usize,
    };
    let align = 1 << rng.below(5);
    Layout::from_size_align(size, align).unwrap()
}

fn fill(ptr: *mut u8, len: usize, pattern: u8) {
    unsafe { core::ptr::write_bytes(ptr, pattern, len) }
}

#[track_caller]
fn verify(ptr: *const u8, len: usize, pattern: u8) {
    let buf = unsafe { core::slice::from_raw_parts(ptr, len) };
    if let Some(pos) = buf.iter().position(|&b| b != pattern) {
        panic!(
            "heap corruption: {len}-byte allocation with pattern {pattern:#04x} \
            contains {:#04x} at offset {pos}",
            buf[pos]
        );
    }
}

/// `WasmGrowAndClaim` must not grow memory unboundedly around page-sized
/// allocation boundaries: sizes just below a page multiple used to make
/// `acquire` claim heaps that could never fit the allocation's required
/// chunk size, looping until the wasm memory limit.
#[wasm_bindgen_test]
fn grow_and_claim_terminates_on_page_boundary_sizes() {
    let mut talc = Talc::<_, WasmBinning>::new(WasmGrowAndClaim);

    // prime: the first claim hosts the allocator metadata, so the
    // allocations below go through subsequent claims
    let prime = Layout::new::<u128>();
    unsafe { talc.allocate(prime) }.unwrap();

    for pages in 1..=4usize {
        for size in [pages * 65536 - 16, pages * 65536] {
            let before = memory_pages();
            let layout = Layout::from_size_align(size, 1).unwrap();
            let alloc = unsafe { talc.allocate(layout) };
            let grown = memory_pages() - before;

            assert!(alloc.is_some(), "allocation of {size} failed");
            assert!(grown <= pages + 1, "grew {grown} pages for a {size}-byte allocation");

            unsafe { talc.deallocate(alloc.unwrap().as_ptr(), layout) };
        }
    }
}

/// Like the above, but with varied small live allocations first, so the
/// page-multiple request hits claims in assorted bin/gap states.
#[wasm_bindgen_test]
fn grow_and_claim_terminates_after_varied_priming() {
    for seed in 0..50u64 {
        let mut talc = Talc::<_, WasmBinning>::new(WasmGrowAndClaim);
        let mut rng = Lcg(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut live = Vec::new();
        for _ in 0..40 {
            let layout =
                Layout::from_size_align(1 + rng.below(4096) as usize, 1 << rng.below(4)).unwrap();
            let ptr = unsafe { talc.allocate(layout) }.unwrap();
            if rng.below(2) == 0 {
                unsafe { talc.deallocate(ptr.as_ptr(), layout) };
            } else {
                live.push((ptr, layout));
            }
        }

        let before = memory_pages();
        let layout = Layout::from_size_align(65536, 1).unwrap();
        let alloc = unsafe { talc.allocate(layout) };
        let grown = memory_pages() - before;

        assert!(alloc.is_some(), "seed {seed}: 1-page allocation failed");
        assert!(grown <= 3, "seed {seed}: grew {grown} pages for a 1-page allocation");
        // the abandoned talc leaks its heaps; fine for a test
    }
}

/// Allocation trace captured from a real workload (an authenticated
/// decryption of a 64 KiB buffer) that grew memory to 4 GiB under
/// `WasmGrowAndClaim`. Ops are (kind, a, b): 1 = alloc(size, align),
/// 2 = dealloc(size, align) of the most recent matching allocation,
/// 3 = realloc(old_size, new_size).
#[wasm_bindgen_test]
fn grow_and_claim_replayed_decrypt_trace_terminates() {
    const TRACE: &[(u8, usize, usize)] = &[
        (1, 65536, 1),
        (1, 12, 1),
        (1, 32, 1),
        (1, 1, 1),
        (1, 16, 1),
        (1, 16, 1),
        (1, 4, 4),
        (1, 12, 4),
        (1, 1, 1),
        (2, 4, 4),
        (1, 8, 1),
        (3, 8, 16),
        (1, 8, 1),
        (3, 8, 16),
        (1, 42, 1),
        (2, 42, 1),
        (1, 18, 1),
        (1, 65520, 1),
        (1, 65520, 1),
        (2, 65520, 1),
        (1, 12, 4),
        (2, 65520, 1),
        (2, 18, 1),
        (1, 33, 1),
        (2, 12, 4),
        (2, 16, 1),
        (2, 16, 1),
        (2, 1, 1),
        (2, 12, 4),
        (2, 16, 1),
        (2, 16, 1),
        (2, 1, 1),
        (2, 32, 1),
        (2, 12, 1),
        (2, 65536, 1),
        (1, 10, 1),
        (3, 10, 43),
        (2, 43, 1),
        (1, 60, 8),
        (2, 33, 1),
    ];

    let cell = TalcCell::<_, WasmBinning>::new(WasmGrowAndClaim);
    let mut live: Vec<(*mut u8, Layout)> = Vec::new();

    for (i, &(kind, a, b)) in TRACE.iter().enumerate() {
        let before = memory_pages();
        match kind {
            1 => {
                let layout = Layout::from_size_align(a, b).unwrap();
                let ptr = unsafe { cell.alloc(layout) };
                assert!(!ptr.is_null(), "op {i}: alloc({a},{b}) failed");
                live.push((ptr, layout));
            }
            2 => {
                let layout = Layout::from_size_align(a, b).unwrap();
                let at = live.iter().rposition(|&(_, l)| l == layout).unwrap();
                let (ptr, layout) = live.remove(at);
                unsafe { cell.dealloc(ptr, layout) };
            }
            _ => {
                let at = live.iter().rposition(|&(_, l)| l.size() == a).unwrap();
                let (ptr, layout) = live.remove(at);
                let new_ptr = unsafe { cell.realloc(ptr, layout, b) };
                assert!(!new_ptr.is_null(), "op {i}: realloc({a},{b}) failed");
                live.push((new_ptr, Layout::from_size_align(b, layout.align()).unwrap()));
            }
        }
        let grown = memory_pages() - before;
        let requested_pages = (a.max(b) + 65535) / 65536;
        assert!(grown <= requested_pages + 2, "op {i} ({kind},{a},{b}): grew {grown} pages");
    }
}

/// Extending a heap whose top gap is at least 16 MiB must fuse the gap
/// rather than corrupt it: `end_to_tag` used to read the most significant
/// byte of the gap's trailing size, misclassifying gaps with bit 24 set as
/// allocated chunks and corrupting their recorded size.
#[wasm_bindgen_test]
fn extend_fuses_large_top_gap() {
    let cell = TalcCell::<_, WasmBinning>::new(WasmGrowAndExtend::new());

    // leave a ~24 MiB top gap (bit 24 set in its size)
    let big = Layout::from_size_align(0x180_0000, 1).unwrap();
    let a = unsafe { cell.alloc(big) };
    assert!(!a.is_null());
    unsafe { cell.dealloc(a, big) };

    // doesn't fit: forces acquire → memory.grow → extend over the top gap
    let bigger = Layout::from_size_align(0x200_0000, 1).unwrap();
    let b = unsafe { cell.alloc(bigger) };
    assert!(!b.is_null());
    fill(b, bigger.size(), 0x5a);
    verify(b, bigger.size(), 0x5a);
    unsafe { cell.dealloc(b, bigger) };

    // exercises another scan over the (previously corrupted) gap lists
    let small = Layout::from_size_align(64, 1).unwrap();
    let c = unsafe { cell.alloc(small) };
    assert!(!c.is_null());
    unsafe { cell.dealloc(c, small) };
}

/// Random alloc/dealloc/realloc actions with content verification against
/// `WasmGrowAndExtend`, driven through `TalcCell`'s `GlobalAlloc` like a
/// global allocator would be.
#[wasm_bindgen_test]
fn grow_and_extend_random_actions_preserve_contents() {
    let cell = TalcCell::<_, WasmBinning>::new(WasmGrowAndExtend::new());
    let mut rng = Lcg(0x5eed_1234_abcd_ef01);
    let mut slots: Vec<(*mut u8, Layout, u8)> = Vec::with_capacity(2048);
    let mut live = 0usize;

    for _ in 0..30_000 {
        while live > 48 * 1024 * 1024 {
            let (ptr, layout, pattern) = slots.swap_remove(rng.below(slots.len() as u64) as usize);
            verify(ptr, layout.size(), pattern);
            unsafe { cell.dealloc(ptr, layout) };
            live -= layout.size();
        }

        match rng.below(100) {
            0..=44 => {
                let layout = random_layout(&mut rng);
                let pattern = (rng.next() & 0xff) as u8;
                let ptr = unsafe { cell.alloc(layout) };
                assert!(!ptr.is_null(), "allocation of {layout:?} failed");
                fill(ptr, layout.size(), pattern);
                live += layout.size();
                slots.push((ptr, layout, pattern));
            }
            45..=74 if !slots.is_empty() => {
                let (ptr, layout, pattern) =
                    slots.swap_remove(rng.below(slots.len() as u64) as usize);
                verify(ptr, layout.size(), pattern);
                unsafe { cell.dealloc(ptr, layout) };
                live -= layout.size();
            }
            75.. if !slots.is_empty() => {
                let i = rng.below(slots.len() as u64) as usize;
                let (ptr, layout, pattern) = slots[i];
                let new_size = (random_layout(&mut rng).size()).max(1);
                verify(ptr, layout.size(), pattern);
                let new_ptr = unsafe { cell.realloc(ptr, layout, new_size) };
                assert!(!new_ptr.is_null(), "realloc to {new_size} failed");
                verify(new_ptr, layout.size().min(new_size), pattern);
                fill(new_ptr, new_size, pattern);
                live = live - layout.size() + new_size;
                slots[i] =
                    (new_ptr, Layout::from_size_align(new_size, layout.align()).unwrap(), pattern);
            }
            _ => {}
        }
    }

    for (ptr, layout, pattern) in slots {
        verify(ptr, layout.size(), pattern);
        unsafe { cell.dealloc(ptr, layout) };
    }
}

/// Many buffers growing by repeated doubling reallocs, interleaved so that
/// in-place growth, copying reallocs, and gap coalescing all churn near the
/// heap top.
#[wasm_bindgen_test]
fn grow_and_extend_interleaved_growth_preserves_contents() {
    let cell = TalcCell::<_, WasmBinning>::new(WasmGrowAndExtend::new());
    let mut rng = Lcg(0x0ddc_0ffe_e0dd_f00d);
    let mut pinned: Vec<(*mut u8, Layout, u8)> = Vec::new();

    for _ in 0..48u64 {
        let mut slots: Vec<(*mut u8, Layout, u8)> = (0..64)
            .map(|i| {
                let layout = Layout::from_size_align(8 << (i % 4), 1 << (i % 4)).unwrap();
                let pattern = (rng.next() & 0xff) as u8;
                let ptr = unsafe { cell.alloc(layout) };
                assert!(!ptr.is_null());
                fill(ptr, layout.size(), pattern);
                (ptr, layout, pattern)
            })
            .collect();

        while slots.iter().any(|(_, layout, _)| layout.size() < 128 * 1024) {
            let i = rng.below(slots.len() as u64) as usize;
            let (ptr, layout, pattern) = slots[i];
            if layout.size() >= 128 * 1024 {
                continue;
            }
            let new_size = layout.size() * 2;
            verify(ptr, layout.size(), pattern);
            let new_ptr = unsafe { cell.realloc(ptr, layout, new_size) };
            assert!(!new_ptr.is_null());
            verify(new_ptr, layout.size(), pattern);
            fill(new_ptr, new_size, pattern);
            slots[i] =
                (new_ptr, Layout::from_size_align(new_size, layout.align()).unwrap(), pattern);
        }

        // free most, pin some across rounds to scatter live chunks
        for (i, (ptr, layout, pattern)) in slots.drain(..).enumerate() {
            verify(ptr, layout.size(), pattern);
            if i % 8 == 7 && pinned.len() < 64 {
                pinned.push((ptr, layout, pattern));
            } else {
                unsafe { cell.dealloc(ptr, layout) };
            }
        }
        for &(ptr, layout, pattern) in &pinned {
            verify(ptr, layout.size(), pattern);
        }
    }

    for (ptr, layout, pattern) in pinned {
        verify(ptr, layout.size(), pattern);
        unsafe { cell.dealloc(ptr, layout) };
    }
}
