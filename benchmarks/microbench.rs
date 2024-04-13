/*
MIT License

Copyright (c) 2022 Philipp Schuster

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/

// Heavily modified by Shaun Beautement.

#![feature(iter_intersperse)]

use buddy_alloc::{BuddyAllocParam, FastAllocParam};
use good_memory_allocator::DEFAULT_SMALLBINS_AMOUNT;
use talc::{ErrOnOom, Talc};

use std::alloc::{GlobalAlloc, Layout};
use std::fs::File;
use std::ptr::NonNull;
use std::time::Instant;

const BENCH_DURATION: f64 = 1.0;

const HEAP_SIZE: usize = 0x10000000;
static mut HEAP_MEMORY: [u8; HEAP_SIZE] = [0u8; HEAP_SIZE];


// Turn DlMalloc into an arena allocator
struct DlmallocArena(std::sync::atomic::AtomicBool);

unsafe impl dlmalloc::Allocator for DlmallocArena {
    fn alloc(&self, _size: usize) -> (*mut u8, usize, u32) {
        let has_data = self.0.fetch_and(false, core::sync::atomic::Ordering::SeqCst);

        if has_data {
            let align = std::mem::align_of::<usize>();
            let heap_align_offset = unsafe { HEAP_MEMORY.as_mut_ptr() }.align_offset(align);
            unsafe { (HEAP_MEMORY.as_mut_ptr().add(heap_align_offset), (HEAP_SIZE - heap_align_offset) / align * align, 1) }
        } else {
            (core::ptr::null_mut(), 0, 0)
        }
    }

    fn remap(&self, _ptr: *mut u8, _oldsize: usize, _newsize: usize, _can_move: bool) -> *mut u8 {
        unimplemented!()
    }

    fn free_part(&self, _ptr: *mut u8, _oldsize: usize, _newsize: usize) -> bool {
        unimplemented!()
    }

    fn free(&self, _ptr: *mut u8, _size: usize) -> bool {
        unimplemented!()
    }

    fn can_release_part(&self, _flags: u32) -> bool {
        false
    }

    fn allocates_zeros(&self) -> bool {
        false
    }

    fn page_size(&self) -> usize {
        4 * 1024
    }
}

struct DlMallocator(spin::Mutex<dlmalloc::Dlmalloc<DlmallocArena>>);

unsafe impl GlobalAlloc for DlMallocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0.lock().malloc(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.lock().free(ptr, layout.size(), layout.align());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.0.lock().realloc(ptr, layout.size(), layout.align(), new_size)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        self.0.lock().calloc(layout.size(), layout.align())
    }
}

struct GlobalRLSF<'p>(spin::Mutex<rlsf::Tlsf<'p, usize, usize, {usize::BITS as usize - 4}, {usize::BITS as usize}>>);
unsafe impl<'a> GlobalAlloc for GlobalRLSF<'a> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0.lock().allocate(layout).map_or(std::ptr::null_mut(), |nn| nn.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.lock().deallocate(NonNull::new_unchecked(ptr), layout.align());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.0.lock().reallocate(NonNull::new_unchecked(ptr), Layout::from_size_align_unchecked(new_size, layout.align()))
            .map_or(std::ptr::null_mut(), |nn| nn.as_ptr())
    }
}



fn main() {
    const BENCHMARK_RESULTS_DIR: &str = "./benchmark_results/micro/";
    // create a directory for the benchmark results.
    let _ = std::fs::create_dir_all(BENCHMARK_RESULTS_DIR).unwrap();

    let sum_file = File::create(BENCHMARK_RESULTS_DIR.to_owned() + "Alloc Plus Dealloc.csv").unwrap();
    let mut csvs = Csvs { sum_file };

    // warm up the memory caches, avoid demand paging issues, etc.
    for i in 0..HEAP_SIZE {
        unsafe {
            HEAP_MEMORY.as_mut_ptr().add(i).write(0xAE);
        }
    }

    /* let linked_list_allocator =
        unsafe { linked_list_allocator::LockedHeap::new(HEAP_MEMORY.as_mut_ptr() as _, HEAP_SIZE) };
    
    benchmark_allocator(&linked_list_allocator, "Linked List Allocator", &mut csvs); */

    let mut galloc_allocator =
        good_memory_allocator::SpinLockedAllocator::<DEFAULT_SMALLBINS_AMOUNT>::empty();
    unsafe {
        galloc_allocator.init(HEAP_MEMORY.as_ptr() as usize, HEAP_SIZE);
    }

    benchmark_allocator(&mut galloc_allocator, "Galloc", &mut csvs);

    let buddy_alloc = unsafe {
        buddy_alloc::NonThreadsafeAlloc::new(
            FastAllocParam::new(HEAP_MEMORY.as_ptr(), HEAP_SIZE / 8),
            BuddyAllocParam::new(HEAP_MEMORY.as_ptr().add(HEAP_SIZE / 8), HEAP_SIZE / 8 * 7, 64),
        )
    };
    benchmark_allocator(&buddy_alloc, "Buddy Allocator", &mut csvs);

    let dlmalloc = DlMallocator(spin::Mutex::new(
        dlmalloc::Dlmalloc::new_with_allocator(DlmallocArena(true.into()))
    ));
    benchmark_allocator(&dlmalloc, "Dlmalloc", &mut csvs);

    let talc = Talc::new(ErrOnOom).lock::<talc::locking::AssumeUnlockable>();
    unsafe { talc.lock().claim(HEAP_MEMORY.as_mut().into()) }.unwrap();

    benchmark_allocator(&talc, "Talc", &mut csvs);

    let tlsf = GlobalRLSF(spin::Mutex::new(rlsf::Tlsf::new()));
    tlsf.0.lock().insert_free_block(unsafe { std::mem::transmute(&mut HEAP_MEMORY[..]) });
    benchmark_allocator(&tlsf, "RLSF", &mut csvs);
    
    // benchmark_allocator(&std::alloc::System, "System", &mut csvs);
    // benchmark_allocator(&frusa::Frusa2M::new(&std::alloc::System), "Frusa", &mut csvs);
}

fn now() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        let mut x = 0u32;
        unsafe { std::arch::x86_64::__rdtscp(&mut x) }
    }

    #[cfg(target_arch = "aarch64")]
    {
        let mut timer: u64;
        unsafe { std::arch::asm!("mrs {0}, cntvct_el0", out(reg) timer, options(nomem, nostack)); }
        return timer;
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    compile_error!(
        "Hardware-based counter is not implemented for this architecture. Supported: x86_64, aarch64"
    );
}

struct Csvs {
    pub sum_file: File, 
}

fn benchmark_allocator(allocator: &dyn GlobalAlloc, name: &str, csvs: &mut Csvs) {
    eprintln!("Benchmarking: {name}...");

    let mut active_allocations = Vec::new();

    let mut alloc_ticks_vec = Vec::new();
    let mut dealloc_ticks_vec = Vec::new();

    // warm up
    for i in 1..10000 {
        let layout = Layout::from_size_align(i * 8, 8).unwrap();
        let ptr = unsafe { allocator.alloc(layout) };
        assert!(!ptr.is_null());
        unsafe { let _ = ptr.read_volatile(); }
        unsafe { allocator.dealloc(ptr, layout); }
    }

    let bench_timer = Instant::now();
    for i in 0.. {
        if i % 0x10000 == 0 && (Instant::now() - bench_timer).as_secs_f64() > BENCH_DURATION { break; }

        let size = fastrand::usize(1..0x10000);
        let align = 8 << fastrand::u16(..).trailing_zeros() / 2;
        let layout = Layout::from_size_align(size, align).unwrap();

        let alloc_begin = now();
        let alloc = unsafe { allocator.alloc(layout) };
        let alloc_ticks = now().wrapping_sub(alloc_begin);

        if std::ptr::null_mut() != alloc {
            alloc_ticks_vec.push(alloc_ticks);
            active_allocations.push((alloc, layout));
        } else {
            for (ptr, layout) in active_allocations.drain(..) {
                let dealloc_begin = now();
                unsafe { allocator.dealloc(ptr, layout); }
                let dealloc_ticks = now().wrapping_sub(dealloc_begin);
                dealloc_ticks_vec.push(dealloc_ticks);
            }
            continue;
        }

        if active_allocations.len() > 10 && fastrand::usize(..10) == 0 {
            for _ in 0..8 {
                let index = fastrand::usize(..active_allocations.len());
                let allocation = active_allocations.swap_remove(index);

                let dealloc_begin = now();
                unsafe {
                    allocator.dealloc(allocation.0, allocation.1);
                }
                let dealloc_ticks = now().wrapping_sub(dealloc_begin);
                dealloc_ticks_vec.push(dealloc_ticks);
            }
        }
    }

    alloc_ticks_vec.sort_unstable();
    dealloc_ticks_vec.sort_unstable();
    let alloc_ticks = alloc_ticks_vec.into_iter().map(|x| x as f64).collect::<Vec<_>>();
    let dealloc_ticks = dealloc_ticks_vec.into_iter().map(|x| x as f64).collect::<Vec<_>>();
    let filtered_alloc_ticks = filter_sorted_outliers(&alloc_ticks);
    let filtered_dealloc_ticks = filter_sorted_outliers(&dealloc_ticks);

    let alloc_quartiles = quartiles(filtered_alloc_ticks);
    let dealloc_quartiles = quartiles(filtered_dealloc_ticks);
    let mut sum_quartiles = [0.0; 5];
    for i in 0..sum_quartiles.len() { sum_quartiles[i] = alloc_quartiles[i] + dealloc_quartiles[i] }

    let data_to_string = |data: &[f64]|
        String::from_iter(data.into_iter().map(|x| x.to_string()).intersperse(",".to_owned()));

    use std::io::Write;
    writeln!(csvs.sum_file, "{name},{}", data_to_string(&sum_quartiles[..])).unwrap();

}

fn filter_sorted_outliers(samples: &[f64]) -> &[f64] {
    // filter extreme outliers
    // these might occur due to system interrupts or rescheduling

    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let var = samples.iter().map(|&x| (x - mean) * (x - mean)).sum::<f64>() / samples.len() as f64;
    let stddev = var.sqrt();
    let filter_limit = mean + stddev * 50.0;

    let mut i = samples.len();
    while i > 0 {
        i -= 1;

        if samples[i] < filter_limit {
            return &samples[..=i];
        }
    }

    unreachable!()
}

fn quartiles(data: &[f64]) -> [f64; 5] {
    let len = data.len();
    [data[0], data[len/4], data[len/2], data[3*len/4], data[len-1]]
}
