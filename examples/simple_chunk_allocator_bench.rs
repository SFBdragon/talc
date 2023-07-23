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

use good_memory_allocator::DEFAULT_SMALLBINS_AMOUNT;
use simple_chunk_allocator::{GlobalChunkAllocator, DEFAULT_CHUNK_SIZE};
use talc::{Talc, Talck};

use std::alloc::{Allocator, Layout};
use std::time::Instant;

const BENCH_DURATION: f64 = 3.0;

// 256 MiB
const HEAP_SIZE: usize = 0x10000000;
/// Backing memory for heap management.
static mut HEAP_MEMORY: PageAlignedBytes<HEAP_SIZE> = PageAlignedBytes([0; HEAP_SIZE]);

/// ChunkAllocator specific stuff.
const CHUNK_COUNT: usize = HEAP_SIZE / DEFAULT_CHUNK_SIZE;
const BITMAP_SIZE: usize = CHUNK_COUNT / 8;
static mut HEAP_BITMAP_MEMORY: PageAlignedBytes<BITMAP_SIZE> = PageAlignedBytes([0; BITMAP_SIZE]);

#[repr(align(4096))]
struct PageAlignedBytes<const N: usize>([u8; N]);

fn main() {
    let chunk_allocator = unsafe {
        GlobalChunkAllocator::<DEFAULT_CHUNK_SIZE>::new(
            HEAP_MEMORY.0.as_mut_slice(),
            HEAP_BITMAP_MEMORY.0.as_mut_slice(),
        )
    };
    let bench_chunk = benchmark_allocator(&chunk_allocator.allocator_api_glue());

    let linked_list_allocator = unsafe {
        linked_list_allocator::LockedHeap::new(HEAP_MEMORY.0.as_mut_ptr() as _, HEAP_SIZE)
    };
    let bench_linked = benchmark_allocator(&linked_list_allocator);

    let mut galloc_allocator =
        good_memory_allocator::SpinLockedAllocator::<DEFAULT_SMALLBINS_AMOUNT>::empty();
    unsafe {
        galloc_allocator.init(HEAP_MEMORY.0.as_ptr() as usize, HEAP_SIZE);
    }
    let bench_galloc = benchmark_allocator(&mut galloc_allocator);

    let talc: Talck<spin_crate::Mutex<()>> = Talc::new().spin_lock();
    unsafe {
        talc.0.lock().init(HEAP_MEMORY.0.as_mut_ptr_range().into());
    }
    let bench_talc = benchmark_allocator(&talc.allocator_api_ref());

    print_bench_results("Chunk Allocator", &bench_chunk);
    println!();
    print_bench_results("Linked List Allocator", &bench_linked);
    println!();
    print_bench_results("Galloc", &bench_galloc);
    println!();
    print_bench_results("Talc", &bench_talc);
}

fn benchmark_allocator(allocator: &dyn Allocator) -> BenchRunResults {
    let mut x = 0u32;
    let mut now_fn = || unsafe { std::arch::x86_64::__rdtscp(std::ptr::addr_of_mut!(x)) };

    let mut active_allocations = Vec::new();

    let mut all_alloc_measurements = Vec::new();
    let mut nofail_alloc_measurements = Vec::new();
    let mut dealloc_measurements = Vec::new();

    let mut allocation_attempts = 0;
    let mut successful_allocations = 0;
    let mut pre_fail_allocations = 0;
    let mut deallocations = 0;

    let mut any_alloc_failed = false;

    // run for 10s
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
