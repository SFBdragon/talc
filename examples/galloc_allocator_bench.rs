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
    ptr::NonNull,
    time::{Duration, Instant},
};

use average::Mean;
use spin::Barrier;

const CHUNKS_AMOUNT: usize = 1 << 23;
const CHUNK_SIZE: usize = 64;
const HEAP_SIZE: usize = CHUNKS_AMOUNT * CHUNK_SIZE;

static mut HEAP: simple_chunk_allocator::PageAligned<[u8; HEAP_SIZE]> =
    simple_chunk_allocator::heap!(chunks = CHUNKS_AMOUNT, chunksize = CHUNK_SIZE);
static mut HEAP_BITMAP: simple_chunk_allocator::PageAligned<[u8; CHUNKS_AMOUNT / 8]> =
    simple_chunk_allocator::heap_bitmap!(chunks = CHUNKS_AMOUNT);

const TIME_STEPS_AMOUNT: usize = 12;
const TIME_STEP_MILLIS: usize = 100;

const MIN_MILLIS_AMOUNT: usize = TIME_STEP_MILLIS;
const MAX_MILLIS_AMOUNT: usize = TIME_STEP_MILLIS * TIME_STEPS_AMOUNT;

// We need to create a wrapper over chunk allocator and implement
// `GlobalAllocator` manually for it, because the implementation provided by the
// `simple-chunk-allocator` crate just panics on memory exhasution instead of
// returning `null`.
#[derive(Debug)]
pub struct GlobalChunkAllocator<
    'a,
    const CHUNK_SIZE: usize = { simple_chunk_allocator::DEFAULT_CHUNK_SIZE },
>(spin::Mutex<simple_chunk_allocator::ChunkAllocator<'a, CHUNK_SIZE>>);

impl<'a, const CHUNK_SIZE: usize> GlobalChunkAllocator<'a, CHUNK_SIZE> {
    #[inline]
    pub const fn new(heap: &'a mut [u8], bitmap: &'a mut [u8]) -> Self {
        let inner_alloc =
            simple_chunk_allocator::ChunkAllocator::<CHUNK_SIZE>::new_const(heap, bitmap);
        Self(spin::Mutex::new(inner_alloc))
    }
}

unsafe impl<'a, const CHUNK_SIZE: usize> GlobalAlloc for GlobalChunkAllocator<'a, CHUNK_SIZE> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0.lock().allocate(layout).map(|p| p.as_mut_ptr()).unwrap_or(core::ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.lock().deallocate(NonNull::new(ptr).unwrap(), layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.0
            .lock()
            .realloc(NonNull::new(ptr).unwrap(), layout, new_size)
            .map(|p| p.as_mut_ptr())
            .unwrap_or(core::ptr::null_mut())
    }
}

struct NamedBenchmark {
    benchmark_fn: fn(Duration, &(dyn GlobalAlloc + Send + Sync), &Barrier) -> usize,
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
    init_fn: fn() -> &'static (dyn GlobalAlloc + Send + Sync),
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

static mut TALLOC_ALLOCATOR: talloc::Tallock = talloc::Talloc::new().spin_lock();
static mut GALLOC_ALLOCATOR: good_memory_allocator::SpinLockedAllocator =
    good_memory_allocator::SpinLockedAllocator::empty();
static LINKED_LIST_ALLOCATOR: linked_list_allocator::LockedHeap =
    linked_list_allocator::LockedHeap::empty();
static CHUNK_ALLOCATOR: GlobalChunkAllocator<'static, CHUNK_SIZE> =
    GlobalChunkAllocator::new(unsafe { HEAP.deref_mut_const() }, unsafe {
        HEAP_BITMAP.deref_mut_const()
    });

fn main() {
    const BENCHMARK_RESULTS_DIR: &str = "./benchmark_results";
    const TRIALS_AMOUNT: usize = 5;

    // create a directory for the benchmark results.
    let _ = std::fs::create_dir(BENCHMARK_RESULTS_DIR);

    let benchmarks = benchmark_list!(random_actions /* ,heap_exhaustion */);

    let allocators = allocator_list!(
        init_talloc,
        init_galloc,
        init_jemalloc,
        init_linux,
        init_linked_list_allocator,
        init_chunk_allocator
    );

    for benchmark in benchmarks {
        let mut csv = String::new();
        for allocator in allocators {
            let scores_as_strings = (MIN_MILLIS_AMOUNT..=MAX_MILLIS_AMOUNT)
                .step_by(TIME_STEP_MILLIS)
                .map(|i| {
                    let duration = Duration::from_millis(i as u64);
                    let mean: Mean = (0..TRIALS_AMOUNT)
                        .map(|_| {
                            let allocator_ref = (allocator.init_fn)();
                            if true {
                                let barrier = Barrier::new(1);
                                (benchmark.benchmark_fn)(duration, allocator_ref, &barrier) as f64
                            } else {
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
                            }
                        })
                        .collect();
                    println!("hi");
                    mean.mean()
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

fn init_talloc() -> &'static (dyn GlobalAlloc + Send + Sync) {
    unsafe {
        TALLOC_ALLOCATOR = talloc::Talloc::new().spin_lock();
        TALLOC_ALLOCATOR.0.lock().init(HEAP.as_mut_ptr_range().into());
    }
    unsafe { &TALLOC_ALLOCATOR }
}

fn init_linked_list_allocator() -> &'static (dyn GlobalAlloc + Send + Sync) {
    let mut a = LINKED_LIST_ALLOCATOR.lock();
    *a = linked_list_allocator::Heap::empty();
    unsafe { a.init(HEAP.as_mut_ptr(), HEAP_SIZE) }
    &LINKED_LIST_ALLOCATOR
}

fn init_galloc() -> &'static (dyn GlobalAlloc + Send + Sync) {
    unsafe {
        GALLOC_ALLOCATOR = good_memory_allocator::SpinLockedAllocator::empty();
    }
    unsafe { GALLOC_ALLOCATOR.init(HEAP.as_ptr() as usize, HEAP_SIZE) }
    unsafe { &GALLOC_ALLOCATOR }
}

fn init_chunk_allocator() -> &'static (dyn GlobalAlloc + Send + Sync) {
    let mut a = CHUNK_ALLOCATOR.0.lock();
    unsafe {
        *a = simple_chunk_allocator::ChunkAllocator::new(
            HEAP.deref_mut_const(),
            HEAP_BITMAP.deref_mut_const(),
        )
        .unwrap();
    }
    &CHUNK_ALLOCATOR
}

fn init_jemalloc() -> &'static (dyn GlobalAlloc + Send + Sync) {
    &tikv_jemallocator::Jemalloc
}

fn init_linux() -> &'static (dyn GlobalAlloc + Send + Sync) {
    &std::alloc::System
}

pub fn random_actions(
    duration: Duration,
    allocator: &(dyn GlobalAlloc + Send + Sync),
    barrier: &Barrier,
) -> usize {
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
                _ => unreachable!(),
            }
        }
    }

    score
}

pub fn heap_exhaustion(
    duration: Duration,
    allocator: &(dyn GlobalAlloc + Send + Sync),
    barrier: &Barrier,
) -> usize {
    let mut v = Vec::with_capacity(10000);
    let mut score = 0;

    barrier.wait();
    let start = Instant::now();

    while start.elapsed() < duration {
        for _ in 0..10 {
            let size = fastrand::usize(10000..=300000);
            let alignment = 4 << fastrand::u16(..).trailing_zeros() / 2;

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
