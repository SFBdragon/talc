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

// Heavily modified by Shaun Beautement. All errors are probably my own.

#![feature(allocator_api)]
#![feature(iter_intersperse)]
#![feature(slice_ptr_get)]

use buddy_alloc::{BuddyAllocParam, FastAllocParam, NonThreadsafeAlloc};
use good_memory_allocator::DEFAULT_SMALLBINS_AMOUNT;
use talc::{ErrOnOom, Talc};

use std::alloc::{AllocError, Allocator, GlobalAlloc, Layout};
use std::fs::File;
use std::time::Instant;

const BENCH_DURATION: f64 = 1.0;

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

fn main() {
    const BENCHMARK_RESULTS_DIR: &str = "./benchmark_results/micro/";
    // create a directory for the benchmark results.
    let _ = std::fs::create_dir(BENCHMARK_RESULTS_DIR);

    let deallocs_file = File::create(BENCHMARK_RESULTS_DIR.to_owned() + "deallocs.csv").unwrap();
    //let reallocs_file = File::create(BENCHMARK_RESULTS_DIR.to_owned() + "reallocs.csv").unwrap();
    let allocs_file = File::create(BENCHMARK_RESULTS_DIR.to_owned() + "allocs.csv").unwrap();
    let mut csvs = Csvs { allocs_file, deallocs_file };

    // warm up the memory caches, avoid demand paging issues, etc.
    for i in 0..HEAP_SIZE {
        unsafe {
            HEAP_MEMORY.as_mut_ptr().add(i).write(0xAE);
        }
    }

    /* let linked_list_allocator =ptr.read_volatile()
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
    benchmark_allocator(&BuddyAllocator(buddy_alloc), "Buddy Allocator", &mut csvs);

    let dlmalloc = dlmalloc::Dlmalloc::new_with_allocator(DlmallocArena(true.into()));
    
    benchmark_allocator(&DlMallocator(spin::Mutex::new(dlmalloc)), "Dlmalloc", &mut csvs);

    let talc = Talc::new(ErrOnOom).lock::<talc::locking::AssumeUnlockable/* spin::Mutex<()> */>();
    unsafe { talc.lock().claim(HEAP_MEMORY.as_mut().into()) }.unwrap();
    
    benchmark_allocator(&talc, "Talc", &mut csvs);
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
    pub allocs_file: File, 
    //pub reallocs_file: File,
    pub deallocs_file: File,
}

fn benchmark_allocator(allocator: &dyn Allocator, name: &str, csvs: &mut Csvs) {
    eprintln!("Benchmarking: {name}...");

    let mut active_allocations = Vec::new();

    let mut alloc_ticks_vec = Vec::new();
    // let mut realloc_ticks_vec = Vec::new();
    let mut dealloc_ticks_vec = Vec::new();

    // warm up
    for i in 1..10000 {
        let layout = Layout::from_size_align(i * 8, 8).unwrap();
        let ptr = allocator.allocate(layout).unwrap().as_non_null_ptr();
        unsafe { let _ = ptr.as_ptr().read_volatile(); }
        unsafe { allocator.deallocate(ptr, layout); }
    }

    let bench_timer = Instant::now();
    for i in 0.. {
        if i % 0x10000 == 0 && (Instant::now() - bench_timer).as_secs_f64() > BENCH_DURATION { break; }

        let size = fastrand::usize((1 << 6)..(1 << 18));
        let align = 8 << fastrand::u16(..).trailing_zeros() / 2;
        let layout = Layout::from_size_align(size, align).unwrap();

        let alloc_begin = now();
        let alloc_res = allocator.allocate(layout);
        let alloc_ticks = now().wrapping_sub(alloc_begin);

        if let Ok(ptr) = alloc_res {
            alloc_ticks_vec.push(alloc_ticks);
            active_allocations.push((ptr.as_non_null_ptr(), layout));
        } else {
            for (ptr, layout) in active_allocations.drain(..) {
                let dealloc_begin = now();
                unsafe { allocator.deallocate(ptr, layout); }
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
                    allocator.deallocate(allocation.0, allocation.1);
                }
                let dealloc_ticks = now().wrapping_sub(dealloc_begin);
                dealloc_ticks_vec.push(dealloc_ticks);
            }
        }
    }

    let data_to_string = |data: &[u64]|
        String::from_iter(data.into_iter().map(|x| x.to_string()).intersperse(",".to_owned()));

    use std::io::Write;
    writeln!(csvs.allocs_file, "{name},{}", data_to_string(&alloc_ticks_vec)).unwrap();
    writeln!(csvs.deallocs_file, "{name},{}", data_to_string(&dealloc_ticks_vec)).unwrap();
}

/* fn print_bench_results(bench_name: &str, res: &BenchRunResults) {
    println!("RESULTS OF BENCHMARK: {bench_name}");
    println!(
        " {:7} allocation attempts, {:7} successful allocations, {:7} pre-fail allocations, {:7} deallocations",
        res.allocation_attempts,
        res.successful_allocations,
        res.pre_fail_allocations,
        res.deallocations
    );

    println!(
        "| {:>20} | Average | Minimum | 1st Quartile | Median | 3rd Quartile | ", "CATEGORY"
    );
    println!("|-|-|{}", "-|".repeat(4));
    print_measurement_set(&res.nofail_alloc_measurements, "Normal Allocs");
    print_measurement_set(&res.high_pressure_alloc_measurements, "High-Pressure Allocs");
    print_measurement_set(&res.dealloc_measurements, "Deallocs");
}

fn print_measurement_set(measurements: &Vec<u64>, set_name: &str) {
    print!("| {:>20} | {:>7} | ", set_name, measurements.iter().sum::<u64>() / measurements.len() as u64);
    for i in 0..=8 {
        print!("{:>8}", measurements[(measurements.len() / 8 * i).min(measurements.len() - 1)]);
    }
    print!("  (ticks)\n", );
}

/// Result of a bench run.
struct BenchRunResults {
    allocation_attempts: usize,
    successful_allocations: usize,
    pre_fail_allocations: usize,
    deallocations: usize,

    /// Sorted vector of the amount of clock ticks per successful allocation under heap pressure.
    high_pressure_alloc_measurements: Vec<u64>,
    /// Sorted vector of the amount of clock ticks per successful allocation.
    nofail_alloc_measurements: Vec<u64>,
    /// Sorted vector of the amount of clock ticks per deallocation.
    dealloc_measurements: Vec<u64>,
}
 */
