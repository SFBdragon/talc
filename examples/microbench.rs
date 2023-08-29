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

/* Heavily modified by Shaun Beautement. All errors are probably my own. */

#![feature(allocator_api)]
#![feature(slice_ptr_get)]

use buddy_alloc::{BuddyAllocParam, FastAllocParam, NonThreadsafeAlloc};
use good_memory_allocator::DEFAULT_SMALLBINS_AMOUNT;
use talc::{ErrOnOom, Talc};

use std::alloc::{AllocError, Allocator, GlobalAlloc, Layout};
use std::time::Instant;

const BENCH_DURATION: f64 = 3.0;

const HEAP_SIZE: usize = 0x10000000;
static mut HEAP_MEMORY: [u8; HEAP_SIZE] = [0u8; HEAP_SIZE];

// NonThreadsafeAlloc doesn't implement Allocator: wrap it
struct BuddyAllocator(NonThreadsafeAlloc);

unsafe impl Allocator for BuddyAllocator {
    fn allocate(&self, layout: Layout) -> Result<std::ptr::NonNull<[u8]>, AllocError> {
        let ptr = unsafe { self.0.alloc(layout) };

        match std::ptr::NonNull::new(ptr) {
            Some(nn) => Ok(std::ptr::NonNull::slice_from_raw_parts(nn, layout.size())),
            None => Err(AllocError),
        }
    }

    unsafe fn deallocate(&self, ptr: std::ptr::NonNull<u8>, layout: Layout) {
        self.0.dealloc(ptr.as_ptr(), layout);
    }
}

// Dlmalloc doesn't implement Allocator
struct DlMallocator(spin::Mutex<dlmalloc::Dlmalloc<DlmallocArena>>);

unsafe impl Allocator for DlMallocator {
    fn allocate(&self, layout: Layout) -> Result<std::ptr::NonNull<[u8]>, AllocError> {
        let ptr = unsafe { self.0.lock().malloc(layout.size(), layout.align()) };

        match std::ptr::NonNull::new(ptr) {
            Some(nn) => Ok(std::ptr::NonNull::slice_from_raw_parts(nn, layout.size())),
            None => Err(AllocError),
        }
    }

    unsafe fn deallocate(&self, ptr: std::ptr::NonNull<u8>, layout: Layout) {
        self.0.lock().free(ptr.as_ptr(), layout.size(), layout.align());
    }
}

// Turn DlMalloc into an arena allocator
struct DlmallocArena(spin::Mutex<bool>);

unsafe impl dlmalloc::Allocator for DlmallocArena {
    fn alloc(&self, _size: usize) -> (*mut u8, usize, u32) {
        let mut lock = self.0.lock();

        if *lock {
            (core::ptr::null_mut(), 0, 0)
        } else {
            *lock = true;
            unsafe { (HEAP_MEMORY.as_mut_ptr(), HEAP_SIZE, 1) }
        }
    }

    fn remap(&self, _ptr: *mut u8, _oldsize: usize, _newsize: usize, _can_move: bool) -> *mut u8 {
        unimplemented!()
    }

    fn free_part(&self, _ptr: *mut u8, _oldsize: usize, _newsize: usize) -> bool {
        unimplemented!()
    }

    fn free(&self, _ptr: *mut u8, _size: usize) -> bool {
        true
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

fn main() {
    let linked_list_allocator =
        unsafe { linked_list_allocator::LockedHeap::new(HEAP_MEMORY.as_mut_ptr() as _, HEAP_SIZE) };
    let bench_linked = benchmark_allocator(&linked_list_allocator);

    let mut galloc_allocator =
        good_memory_allocator::SpinLockedAllocator::<DEFAULT_SMALLBINS_AMOUNT>::empty();
    unsafe {
        galloc_allocator.init(HEAP_MEMORY.as_ptr() as usize, HEAP_SIZE);
    }
    let bench_galloc = benchmark_allocator(&mut galloc_allocator);

    let buddy_alloc = unsafe {
        buddy_alloc::NonThreadsafeAlloc::new(
            FastAllocParam::new(HEAP_MEMORY.as_ptr(), HEAP_SIZE / 8),
            BuddyAllocParam::new(HEAP_MEMORY.as_ptr().add(HEAP_SIZE / 8), HEAP_SIZE / 8 * 7, 64),
        )
    };
    let bench_buddy = benchmark_allocator(&BuddyAllocator(buddy_alloc));

    let talc = Talc::new(ErrOnOom).lock::<spin::Mutex<()>>();
    unsafe { talc.0.lock().claim(HEAP_MEMORY.as_mut_slice().into()).unwrap(); }
    let bench_talc = benchmark_allocator(&talc.allocator());

    let dlmalloc = dlmalloc::Dlmalloc::new_with_allocator(DlmallocArena(spin::Mutex::new(false)));
    let bench_dlmalloc = benchmark_allocator(&DlMallocator(spin::Mutex::new(dlmalloc)));

    print_bench_results("Talc", &bench_talc);
    println!();
    print_bench_results("Buddy Allocator", &bench_buddy);
    println!();
    print_bench_results("Dlmalloc", &bench_dlmalloc);
    println!();
    print_bench_results("Galloc", &bench_galloc);
    println!();
    print_bench_results("Linked List Allocator", &bench_linked);
}

fn benchmark_allocator(allocator: &dyn Allocator) -> BenchRunResults {
    let now_fn = || unsafe {
        #[cfg(target_arch = "x86_64")]
        {
            let mut x = 0u32;
            return std::arch::x86_64::__rdtscp(&mut x);
        }

        #[cfg(target_arch = "aarch64")]
        {
            let mut timer: u64;
            std::arch::asm!("mrs {0}, cntvct_el0", out(reg) timer, options(nomem, nostack));
            return timer;
        }

        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        compile_error!(
            "Hardware-based counter is not implemented for this architecture. Supported: x86_64, aarch64"
        );
    };

    let mut active_allocations = Vec::new();

    let mut all_alloc_measurements = Vec::new();
    let mut nofail_alloc_measurements = Vec::new();
    let mut dealloc_measurements = Vec::new();

    let mut allocation_attempts = 0;
    let mut successful_allocations = 0;
    let mut pre_fail_allocations = 0;
    let mut deallocations = 0;

    let mut any_alloc_failed = false;

    let bench_begin_time = Instant::now();
    while bench_begin_time.elapsed().as_secs_f64() <= BENCH_DURATION {
        let size = fastrand::usize((1 << 6)..(1 << 16));
        let align = 8 << fastrand::u16(..).trailing_zeros() / 2;
        let layout = Layout::from_size_align(size, align).unwrap();

        let alloc_begin = now_fn();
        let res = allocator.allocate(layout);
        let alloc_ticks = now_fn() - alloc_begin;

        allocation_attempts += 1;
        if let Ok(ptr) = res {
            active_allocations.push((ptr.as_non_null_ptr(), layout));

            successful_allocations += 1;
            if !any_alloc_failed {
                pre_fail_allocations += 1;
            }
        } else {
            any_alloc_failed = true;
        }

        all_alloc_measurements.push(alloc_ticks);
        if !any_alloc_failed {
            nofail_alloc_measurements.push(alloc_ticks);
        }

        if active_allocations.len() > 10 && fastrand::usize(..10) == 0 {
            for _ in 0..7 {
                let index = fastrand::usize(..active_allocations.len());
                let allocation = active_allocations.swap_remove(index);

                let dealloc_begin = now_fn();
                unsafe {
                    allocator.deallocate(allocation.0, allocation.1);
                }
                let dealloc_ticks = now_fn() - dealloc_begin;

                deallocations += 1;
                dealloc_measurements.push(dealloc_ticks);
            }
        }
    }

    // sort
    all_alloc_measurements.sort();
    nofail_alloc_measurements.sort();
    dealloc_measurements.sort();

    BenchRunResults {
        allocation_attempts,
        successful_allocations,
        pre_fail_allocations,
        deallocations,

        all_alloc_measurements,
        nofail_alloc_measurements,
        dealloc_measurements,
    }
}

fn print_bench_results(bench_name: &str, res: &BenchRunResults) {
    println!("RESULTS OF BENCHMARK: {bench_name}");
    println!(
        " {:7} allocation attempts, {:7} successful allocations, {:7} pre-fail allocations, {:7} deallocations",
        res.allocation_attempts,
        res.successful_allocations,
        res.pre_fail_allocations,
        res.deallocations
    );

    println!(
        "            CATEGORY | OCTILE 0       1       2       3       4       5       6       7       8 | AVERAGE"
    );
    println!(
        "---------------------|--------------------------------------------------------------------------|---------"
    );
    print_measurement_set(&res.all_alloc_measurements, "All Allocations");
    print_measurement_set(&res.nofail_alloc_measurements, "Pre-Fail Allocations");
    print_measurement_set(&res.dealloc_measurements, "Deallocations");
}

fn print_measurement_set(measurements: &Vec<u64>, set_name: &str) {
    print!("{:>20} | ", set_name);
    for i in 0..=8 {
        print!("{:>8}", measurements[(measurements.len() / 8 * i).min(measurements.len() - 1)]);
    }

    print!(" | {:>7}   ticks\n", measurements.iter().sum::<u64>() / measurements.len() as u64);
}

/// Result of a bench run.
struct BenchRunResults {
    allocation_attempts: usize,
    successful_allocations: usize,
    pre_fail_allocations: usize,
    deallocations: usize,

    /// Sorted vector of the amount of clock ticks per successful allocation.
    all_alloc_measurements: Vec<u64>,
    /// Sorted vector of the amount of clock ticks per successful allocation under heap pressure.
    nofail_alloc_measurements: Vec<u64>,
    /// Sorted vector of the amount of clock ticks per deallocation.
    dealloc_measurements: Vec<u64>,
}
