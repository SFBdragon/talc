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
    hint::unreachable_unchecked,
    sync::Barrier,
    time::{Duration, Instant},
};

use buddy_alloc::{BuddyAllocParam, FastAllocParam, NonThreadsafeAlloc};

const HEAP_SIZE: usize = 1 << 27;
static mut HEAP: [u8; HEAP_SIZE] = [0u8; HEAP_SIZE];

const TIME_STEPS_AMOUNT: usize = 12;
const TIME_STEP_MILLIS: usize = 100;

const MIN_MILLIS_AMOUNT: usize = TIME_STEP_MILLIS;
const MAX_MILLIS_AMOUNT: usize = TIME_STEP_MILLIS * TIME_STEPS_AMOUNT;

struct NamedBenchmark {
    benchmark_fn: fn(Duration, &dyn GlobalAlloc, &Barrier) -> usize,
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

static mut TALC_ALLOCATOR: talc::Talck = talc::Talc::new().spin_lock();
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

fn main() {
    const BENCHMARK_RESULTS_DIR: &str = "./benchmark_results";
    const TRIALS_AMOUNT: usize = 5;

    // create a directory for the benchmark results.
    let _ = std::fs::create_dir(BENCHMARK_RESULTS_DIR);

    let benchmarks = benchmark_list!(random_actions, heap_exhaustion, heap_efficiency);

    let allocators =
        allocator_list!(init_talc, init_galloc, init_buddy_alloc, init_linked_list_allocator);

    for benchmark in benchmarks {
        let mut csv = String::new();
        for allocator in allocators {
            let scores_as_strings = (MIN_MILLIS_AMOUNT..=MAX_MILLIS_AMOUNT)
                .step_by(TIME_STEP_MILLIS)
                .map(|i| {
                    eprintln!("benchmarking...");

                    let duration = Duration::from_millis(i as u64);

                    (0..TRIALS_AMOUNT)
                        .map(|_| {
                            let allocator_ref = (allocator.init_fn)();
                            /* if true { */
                            let barrier = Barrier::new(1);
                            (benchmark.benchmark_fn)(duration, allocator_ref, &barrier)
                            /* } else {
                                const THREADS: usize = 2;
                                let barrier = Barrier::new(THREADS);
                                std::thread::scope(|s| {
                                    let pts = [
                                        s.spawn(|| {
                                            (benchmark.benchmark_fn)(
                                                duration,
                                                allocator_ref,
                                                &barrier,
                                            )
                                        }),
                                        s.spawn(|| {
                                            (benchmark.benchmark_fn)(
                                                duration,
                                                allocator_ref,
                                                &barrier,
                                            )
                                        }),
                                    ];
                                    assert!(pts.len() == THREADS);

                                    pts.into_iter().map(|s| s.join().unwrap()).fold(0, |a, b| a + b)
                                        as f64
                                })
                            } */
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
        TALC_ALLOCATOR = talc::Talc::with_arena(HEAP.as_mut_slice().into()).spin_lock();
        &TALC_ALLOCATOR
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
    }
    unsafe { GALLOC_ALLOCATOR.init(HEAP.as_ptr() as usize, HEAP_SIZE) }
    unsafe { &GALLOC_ALLOCATOR }
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

pub fn random_actions(duration: Duration, allocator: &dyn GlobalAlloc, barrier: &Barrier) -> usize {
    let mut score = 0;
    let mut v = Vec::with_capacity(10000);

    barrier.wait();
    let start = Instant::now();
    while start.elapsed() < duration {
        for _ in 0..100 {
            let action = fastrand::usize(0..3);

            match action {
                0 => {
                    let size = fastrand::usize(100..=1000);
                    let alignment = 8 << fastrand::u16(..).trailing_zeros() / 2;
                    if let Some(allocation) = AllocationWrapper::new(size, alignment, allocator) {
                        v.push(allocation);
                        score += 1;
                    }
                }
                1 => {
                    if !v.is_empty() {
                        let index = fastrand::usize(0..v.len());
                        v.swap_remove(index);
                        score += 1;
                    }
                }
                2 => {
                    if !v.is_empty() {
                        let index = fastrand::usize(0..v.len());
                        if let Some(random_allocation) = v.get_mut(index) {
                            let size = fastrand::usize(100..=10000);
                            random_allocation.realloc(size);
                        }
                        score += 1;
                    }
                }
                _ => unsafe { unreachable_unchecked() },
            }
        }
    }

    score
}

pub fn heap_exhaustion(
    duration: Duration,
    allocator: &dyn GlobalAlloc,
    barrier: &Barrier,
) -> usize {
    let mut v = Vec::with_capacity(10000);
    let mut score = 0;

    barrier.wait();
    let start = Instant::now();

    while start.elapsed() < duration {
        for _ in 0..10 {
            let size = fastrand::usize(10000..=300000);
            let alignment = 8 << fastrand::u16(..).trailing_zeros() / 2;

            match AllocationWrapper::new(size, alignment, allocator) {
                Some(allocation) => {
                    v.push(allocation);
                    score += 1
                }
                None => {
                    // heap was exhausted, penalize the score by sleeping.
                    std::thread::sleep(Duration::from_millis(3));

                    // free all allocation to empty the heap.
                    v.clear();

                    break;
                }
            }
        }
    }

    score
}

pub fn heap_efficiency(
    duration: Duration,
    allocator: &dyn GlobalAlloc,
    barrier: &Barrier,
) -> usize {
    let mut v = Vec::with_capacity(10000);
    let mut used = 0;
    let mut total = HEAP_SIZE;

    barrier.wait();
    let start = Instant::now();

    while start.elapsed() < duration {
        for _ in 0..10 {
            let size = fastrand::usize(10000..=300000);
            let alignment = 8 << fastrand::u16(..).trailing_zeros() / 2;

            match AllocationWrapper::new(size, alignment, allocator) {
                Some(allocation) => {
                    used += allocation.layout.size();
                    v.push(allocation);
                }
                None => {
                    // heap was exhausted, penalize the score by sleeping.
                    std::thread::sleep(Duration::from_millis(3));

                    // free all allocation to empty the heap.
                    v.clear();

                    total += HEAP_SIZE;

                    break;
                }
            }
        }
    }

    used * 10000 / total
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
