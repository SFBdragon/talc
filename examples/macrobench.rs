/* The MIT License (MIT)

Copyright © 2023 Roee Shoshani, Guy Nir

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the “Software”), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included
in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
*/

// Modified by Shaun Beautement

#![feature(allocator_api)]
#![feature(slice_ptr_get)]
#![feature(iter_intersperse)]
#![feature(const_mut_refs)]

use std::{
    alloc::{GlobalAlloc, Layout},
    io::Write,
    time::{Duration, Instant},
};

use buddy_alloc::{BuddyAllocParam, FastAllocParam, NonThreadsafeAlloc};

const HEAP_SIZE: usize = 1 << 27;
static mut HEAP: [u8; HEAP_SIZE] = [0u8; HEAP_SIZE];

const TIME_STEPS_AMOUNT: usize = 5;
const TIME_STEP_MILLIS: usize = 200;

const MIN_MILLIS_AMOUNT: usize = TIME_STEP_MILLIS;
const MAX_MILLIS_AMOUNT: usize = TIME_STEP_MILLIS * TIME_STEPS_AMOUNT;

const BENCHMARK_RESULTS_DIR: &str = "./benchmark_results";
const TRIALS_AMOUNT: usize = 15;

struct NamedBenchmark {
    benchmark_fn: fn(Duration, &dyn GlobalAlloc) -> usize,
    name: &'static str,
}

macro_rules! benchmark_list {
    ($($benchmark_fn: path),+) => {
        &[
            $(
                NamedBenchmark {
                    benchmark_fn: $benchmark_fn,
                    name: stringify!($benchmark_fn),
                }
            ),+
        ]
    }
}

struct NamedAllocator {
    name: &'static str,
    init_fn: fn() -> &'static (dyn GlobalAlloc),
}

macro_rules! allocator_list {
    ($($init_fn: path),+) => {
        &[
            $(
                NamedAllocator {
                    init_fn: $init_fn,
                    name: {
                        const INIT_FN_NAME:&'static str = stringify!($init_fn);
                        &INIT_FN_NAME["init_".len()..]
                    },
                }
            ),+
        ]
    }
}

static mut TALC_ALLOCATOR: talc::Talck<spin::Mutex<()>, talc::ErrOnOom> =
    talc::Talc::new(talc::ErrOnOom).lock();
static mut PREV_TALC_ALLOCATOR: prev_talc::Talck<spin::Mutex<()>, prev_talc::ErrOnOom> =
    prev_talc::Talc::new(prev_talc::ErrOnOom).lock();
static mut BUDDY_ALLOCATOR: buddy_alloc::NonThreadsafeAlloc = unsafe {
    NonThreadsafeAlloc::new(
        FastAllocParam::new(HEAP.as_ptr(), HEAP_SIZE / 8),
        BuddyAllocParam::new(HEAP.as_ptr().add(HEAP_SIZE / 8), HEAP_SIZE / 8 * 7, 64),
    )
};
static mut GALLOC_ALLOCATOR: good_memory_allocator::SpinLockedAllocator =
    good_memory_allocator::SpinLockedAllocator::empty();
static LINKED_LIST_ALLOCATOR: linked_list_allocator::LockedHeap =
    linked_list_allocator::LockedHeap::empty();
static mut DLMALLOC_ALLOCATOR: DlMallocator = DlMallocator(lock_api::Mutex::new(
    dlmalloc::Dlmalloc::new_with_allocator(DlmallocArena(spin::Mutex::new(false))),
));

struct DlMallocator(
    lock_api::Mutex<talc::locking::AssumeUnlockable, dlmalloc::Dlmalloc<DlmallocArena>>,
);

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

// Turn DlMalloc into an arena allocator
struct DlmallocArena(spin::Mutex<bool>);

unsafe impl dlmalloc::Allocator for DlmallocArena {
    fn alloc(&self, _: usize) -> (*mut u8, usize, u32) {
        let mut lock = self.0.lock();

        if *lock {
            (core::ptr::null_mut(), 0, 0)
        } else {
            *lock = true;
            unsafe { (HEAP.as_mut_ptr(), HEAP_SIZE, 1) }
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
    // create a directory for the benchmark results.
    let _ = std::fs::create_dir(BENCHMARK_RESULTS_DIR);

    let benchmarks = benchmark_list!(random_actions);

    let allocators = allocator_list!(
        init_talc,
        init_prev_talc,
        init_dlmalloc,
        init_buddy_alloc,
        init_galloc,
        init_linked_list_allocator
    );

    print!("Run heap efficiency microbenchmarks? y/N: ");
    std::io::stdout().flush().unwrap();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    if input.trim() == "y" {
        // heap efficiency benchmark

        println!("|             Allocator | Average Random Actions Heap Efficiency |");
        println!("| --------------------- | -------------------------------------- |");

        for allocator in allocators {
            let efficiency = heap_efficiency((allocator.init_fn)());

            println!("|{:>22} | {:>38} |", allocator.name, format!("{:2.2}%", efficiency));
        }
    }

    for benchmark in benchmarks {
        let mut csv = String::new();
        for allocator in allocators {
            let scores_as_strings = (MIN_MILLIS_AMOUNT..=MAX_MILLIS_AMOUNT)
                .step_by(TIME_STEP_MILLIS)
                .map(|i| {
                    eprintln!("benchmarking...");

                    let duration = Duration::from_millis(i as _);

                    (0..TRIALS_AMOUNT)
                        .map(|_| {
                            let allocator_ref = (allocator.init_fn)();
                            (benchmark.benchmark_fn)(duration, allocator_ref)
                        })
                        .sum::<usize>()
                        / TRIALS_AMOUNT
                })
                .map(|score| score.to_string());

            let csv_line = std::iter::once(allocator.name.to_owned())
                .chain(scores_as_strings)
                .intersperse(",".to_owned())
                .chain(std::iter::once("\n".to_owned()));
            csv.extend(csv_line);
        }
        // remove the last newline.
        csv.pop();

        std::fs::write(format!("{}/{}.csv", BENCHMARK_RESULTS_DIR, benchmark.name), csv).unwrap();
    }
}

fn init_talc() -> &'static (dyn GlobalAlloc) {
    unsafe {
        TALC_ALLOCATOR = talc::Talc::new(talc::ErrOnOom).lock();
        TALC_ALLOCATOR.lock().claim(HEAP.as_mut_slice().into()).unwrap();
        &TALC_ALLOCATOR
    }
}

#[allow(dead_code)]
fn init_prev_talc() -> &'static (dyn GlobalAlloc) {
    unsafe {
        PREV_TALC_ALLOCATOR =
            prev_talc::Talc::with_arena(prev_talc::ErrOnOom, HEAP.as_mut_slice().into()).lock();
        &PREV_TALC_ALLOCATOR
    }
}

fn init_linked_list_allocator() -> &'static (dyn GlobalAlloc) {
    let mut a = LINKED_LIST_ALLOCATOR.lock();
    *a = linked_list_allocator::Heap::empty();
    unsafe { a.init(HEAP.as_mut_ptr().cast(), HEAP_SIZE) }
    &LINKED_LIST_ALLOCATOR
}

fn init_galloc() -> &'static (dyn GlobalAlloc) {
    unsafe {
        GALLOC_ALLOCATOR = good_memory_allocator::SpinLockedAllocator::empty();
        GALLOC_ALLOCATOR.init(HEAP.as_ptr() as usize, HEAP_SIZE);
        &GALLOC_ALLOCATOR
    }
}

fn init_buddy_alloc() -> &'static (dyn GlobalAlloc) {
    unsafe {
        BUDDY_ALLOCATOR = NonThreadsafeAlloc::new(
            FastAllocParam::new(HEAP.as_ptr().cast(), HEAP.len() / 8),
            BuddyAllocParam::new(
                HEAP.as_ptr().cast::<u8>().add(HEAP.len() / 8),
                HEAP.len() / 8 * 7,
                64,
            ),
        );

        &BUDDY_ALLOCATOR
    }
}

fn init_dlmalloc() -> &'static dyn GlobalAlloc {
    unsafe {
        DLMALLOC_ALLOCATOR = DlMallocator(lock_api::Mutex::new(
            dlmalloc::Dlmalloc::new_with_allocator(DlmallocArena(spin::Mutex::new(false))),
        ));
        &DLMALLOC_ALLOCATOR
    }
}

pub fn random_actions(duration: Duration, allocator: &dyn GlobalAlloc) -> usize {
    let mut score = 0;
    let mut v = Vec::with_capacity(100000);

    let rng = fastrand::Rng::new();

    let start = Instant::now();
    while start.elapsed() < duration {
        for _ in 0..100 {
            let action = rng.usize(0..3);

            match action {
                0 => {
                    let size = rng.usize(1..=2000);
                    let alignment = 8 << rng.u16(..).trailing_zeros() / 2;
                    if let Some(allocation) = AllocationWrapper::new(size, alignment, allocator) {
                        v.push(allocation);
                        score += 1;
                    }
                }
                1 => {
                    if !v.is_empty() {
                        let index = rng.usize(0..v.len());
                        v.swap_remove(index);
                        score += 1;
                    }
                }
                2 => {
                    if !v.is_empty() {
                        let index = rng.usize(0..v.len());
                        if let Some(random_allocation) = v.get_mut(index) {
                            let size = rng.usize(1..=20000);
                            random_allocation.realloc(size);
                        }
                        score += 1;
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    score
}

pub fn heap_efficiency(allocator: &dyn GlobalAlloc) -> f64 {
    let mut v = Vec::with_capacity(100000);
    let mut used = 0;
    let mut total = HEAP_SIZE;

    for _ in 0..500 {
        loop {
            let action = fastrand::usize(0..10);

            match action {
                0..=4 => {
                    let size = fastrand::usize(1..=20000);
                    let align = 8 << fastrand::u16(..).trailing_zeros() / 2;

                    if let Some(allocation) = AllocationWrapper::new(size, align, allocator) {
                        //used += allocation.layout.size();
                        v.push(allocation);
                    } else {
                        used += v.iter().map(|a| a.layout.size()).sum::<usize>();
                        total += HEAP_SIZE;
                        v.clear();
                        break;
                    }
                }
                5 => {
                    if !v.is_empty() {
                        let index = fastrand::usize(0..v.len());
                        //used -= v[index].layout.size();
                        v.swap_remove(index);
                    }
                }
                6..=9 => {
                    if !v.is_empty() {
                        let index = fastrand::usize(0..v.len());

                        if let Some(random_allocation) = v.get_mut(index) {
                            //let old_size = random_allocation.layout.size();
                            let new_size = fastrand::usize(1..=200000);
                            random_allocation.realloc(new_size);
                            //used = used + new_size - old_size;
                        } else {
                            used += v.iter().map(|a| a.layout.size()).sum::<usize>();
                            total += HEAP_SIZE;
                            v.clear();
                            break;
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    used as f64 / total as f64 * 100.0
}

struct AllocationWrapper<'a> {
    ptr: *mut u8,
    layout: Layout,
    allocator: &'a dyn GlobalAlloc,
}
impl<'a> AllocationWrapper<'a> {
    fn new(size: usize, align: usize, allocator: &'a dyn GlobalAlloc) -> Option<Self> {
        let layout = Layout::from_size_align(size, align).unwrap();

        let ptr = unsafe { allocator.alloc(layout) };

        if ptr.is_null() {
            return None;
        }

        Some(Self { ptr, layout, allocator })
    }

    fn realloc(&mut self, new_size: usize) {
        let new_ptr = unsafe { self.allocator.realloc(self.ptr, self.layout, new_size) };
        if new_ptr.is_null() {
            return;
        }
        self.ptr = new_ptr;
        self.layout = Layout::from_size_align(new_size, self.layout.align()).unwrap();
    }
}

impl<'a> Drop for AllocationWrapper<'a> {
    fn drop(&mut self) {
        unsafe { self.allocator.dealloc(self.ptr, self.layout) }
    }
}
