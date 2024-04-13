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

#![feature(iter_intersperse)]
#![feature(const_mut_refs)]

use std::{
    alloc::{GlobalAlloc, Layout}, 
    ptr::NonNull, 
    sync::{Arc, Barrier}, 
    time::{Duration, Instant}, 
    fmt::Write
};

use buddy_alloc::{BuddyAllocParam, FastAllocParam, NonThreadsafeAlloc};

const THREAD_COUNT: usize = 4;

const RA_TRIALS_AMOUNT: usize = 7;
const RA_TIME: Duration = Duration::from_millis(200);
const RA_MAX_ALLOC_SIZES: &[usize] = &[1000, 5000, 10000, 50000, 100000];
const RA_MAX_REALLOC_SIZE_MULTI: usize = 10;
const RA_TARGET_MIN_ALLOCATIONS: usize = 300;

const HE_MAX_ALLOC_SIZE: usize = 100000;
const HE_MAX_REALLOC_SIZE_MULTI: usize = 1000000;

const HEAP_SIZE: usize = 1 << 29;
static mut HEAP: [u8; HEAP_SIZE] = [0u8; HEAP_SIZE];

const BENCHMARK_RESULTS_DIR: &str = "./benchmark_results";

struct NamedAllocator {
    name: &'static str,
    init_fn: unsafe fn() -> Box<dyn GlobalAlloc + Sync>,
}

fn main() {
    // create a directory for the benchmark results.
    let _ = std::fs::create_dir(BENCHMARK_RESULTS_DIR);

    let allocators = &[
        NamedAllocator { name: "Talc", init_fn: init_talc },
        NamedAllocator { name: "RLSF", init_fn: init_rlsf },
        NamedAllocator { name: "Frusa", init_fn: init_frusa },
        NamedAllocator { name: "Dlmalloc", init_fn: init_dlmalloc },
        NamedAllocator { name: "System", init_fn: init_system },
        NamedAllocator { name: "Buddy Alloc", init_fn: init_buddy_alloc },
        NamedAllocator { name: "Linked List", init_fn: init_linked_list_allocator },
    ];

    print!("Run heap efficiency benchmarks? y/N: ");
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    if input.trim() == "y" {
        // heap efficiency benchmark

        println!("|             Allocator | Average Random Actions Heap Efficiency |");
        println!("| --------------------- | -------------------------------------- |");

        for allocator in allocators {
            // these request memory from the OS on-demand, instead of being arena-allocated
            if matches!(allocator.name, "Frusa" | "System") { continue; }

            let efficiency = heap_efficiency(unsafe {(allocator.init_fn)() }.as_ref());

            println!("|{:>22} | {:>38} |", allocator.name, format!("{:2.2}%", efficiency));
        }
    }

    let mut csv = String::new();

    write!(csv, ",").unwrap();
    csv.extend(RA_MAX_ALLOC_SIZES.iter().map(|i| i.to_string()).intersperse(",".to_owned()));
    writeln!(csv).unwrap();

    for &NamedAllocator { name, init_fn } in allocators {
        write!(csv, "{}", name).unwrap();
        
        for &max_alloc_size in RA_MAX_ALLOC_SIZES.iter() {
            eprintln!("benchmarking {} - max alloc size {}B ...", name, max_alloc_size);
    
            let score = (0..RA_TRIALS_AMOUNT)
                .map(|_| {
                    let allocator = unsafe { (init_fn)() };
                    let allocator_ref = allocator.as_ref();
    
                    std::thread::scope(|scope| {
                        let barrier = Arc::new(Barrier::new(THREAD_COUNT));
                        let mut handles = vec![];
    
                        for _ in 0..THREAD_COUNT {
                            let bi = barrier.clone();
                            handles.push(scope.spawn(move || random_actions( allocator_ref, max_alloc_size, bi)));
                        }
    
                        handles.into_iter().map(|h| h.join().unwrap()).sum::<usize>()
                    })
                }).sum::<usize>() / RA_TRIALS_AMOUNT;
    
            write!(csv, ",{}", score).unwrap();
        }

        writeln!(csv).unwrap();
    }
    // remove the last newline.
    csv.pop();

    std::fs::write(format!("{}/Random Actions Benchmark.csv", BENCHMARK_RESULTS_DIR), csv).unwrap();
}


pub fn random_actions(allocator: &dyn GlobalAlloc, max_alloc_size: usize, barrier: Arc<Barrier>) -> usize {
    let mut score = 0;
    let mut v: Vec<AllocationWrapper<'_>> = Vec::with_capacity(100000);
    let rng = fastrand::Rng::new();

    barrier.wait();
    let start = Instant::now();
    while start.elapsed() < RA_TIME {
        for _ in 0..100 {
            let action = rng.usize(0..5);

            // 20% reallocate
            // 40% if there are enough allocations, deallocate
            // 40% if enough allocations else 80%, allocate

            // this avoids staying close to zero allocations
            // while also avoiding growing the heap unboundedly
            // as benchmarking high heap contention isn't usually relavent
            // but having a very low number of allocations isn't realistic either

            if action == 4 {
                if !v.is_empty() {
                    let index = rng.usize(0..v.len());
                    if let Some(random_allocation) = v.get_mut(index) {
                        let size = rng.usize(1..(max_alloc_size * RA_MAX_REALLOC_SIZE_MULTI));
                        random_allocation.realloc(size);
                    } else {
                        eprintln!("Reallocation failure!");
                    }
                    score += 1;
                }
            } else if action < 2 || v.len() < RA_TARGET_MIN_ALLOCATIONS {
                let size = rng.usize(1..max_alloc_size);
                let alignment =  std::mem::align_of::<usize>() << rng.u16(..).trailing_zeros() / 2;
                if let Some(allocation) = AllocationWrapper::new(size, alignment, allocator) {
                    v.push(allocation);
                    score += 1;
                } else {
                    eprintln!("Allocation failure!");
                }
            } else {
                let index = rng.usize(0..v.len());
                v.swap_remove(index);
                score += 1;
            }
        }
    }

    score
}

pub fn heap_efficiency(allocator: &dyn GlobalAlloc) -> f64 {
    let mut v = Vec::with_capacity(100000);
    let mut used = 0;
    let mut total = 0;

    for _ in 0..300 {
        loop {
            let action = fastrand::usize(0..10);

            match action {
                0..=4 => {
                    let size = fastrand::usize(1..HE_MAX_ALLOC_SIZE);
                    let align = std::mem::align_of::<usize>() << fastrand::u16(..).trailing_zeros() / 2;

                    if let Some(allocation) = AllocationWrapper::new(size, align, allocator) {
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
                        v.swap_remove(index);
                    }
                }
                6..=9 => {
                    if !v.is_empty() {
                        let index = fastrand::usize(0..v.len());

                        if let Some(random_allocation) = v.get_mut(index) {
                            let new_size = fastrand::usize(1..(HE_MAX_ALLOC_SIZE*HE_MAX_REALLOC_SIZE_MULTI));
                            random_allocation.realloc(new_size);
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

        let ptr = unsafe { (*allocator).alloc(layout) };

        if ptr.is_null() {
            return None;
        }

        Some(Self { ptr, layout, allocator })
    }

    fn realloc(&mut self, new_size: usize) {
        let new_ptr = unsafe { (*self.allocator).realloc(self.ptr, self.layout, new_size) };
        if new_ptr.is_null() {
            return;
        }
        self.ptr = new_ptr;
        self.layout = Layout::from_size_align(new_size, self.layout.align()).unwrap();
    }
}

impl<'a> Drop for AllocationWrapper<'a> {
    fn drop(&mut self) {
        unsafe { (*self.allocator).dealloc(self.ptr, self.layout) }
    }
}



/// Memory must be available.
unsafe fn init_talc() -> Box<dyn GlobalAlloc + Sync> {
    unsafe {
        let talck: _ = talc::Talc::new(talc::ErrOnOom).lock::<spin::Mutex<()>>();
        talck.lock().claim(HEAP.as_mut_slice().into()).unwrap();
        Box::new(talck)
    }
}

unsafe fn init_linked_list_allocator() -> Box<dyn GlobalAlloc + Sync> {
    let lla = linked_list_allocator::LockedHeap::new(HEAP.as_mut_ptr(), HEAP_SIZE);
    lla.lock().init(HEAP.as_mut_ptr().cast(), HEAP_SIZE);
    Box::new(lla)
}

unsafe fn init_system() -> Box<dyn GlobalAlloc + Sync> {
    Box::new(std::alloc::System)
}

unsafe fn init_frusa() -> Box<dyn GlobalAlloc + Sync> {
    Box::new(frusa::Frusa2M::new(&std::alloc::System))
}

#[allow(unused)]
unsafe fn init_galloc() -> Box<dyn GlobalAlloc + Sync> {
    let galloc = good_memory_allocator::SpinLockedAllocator
        ::<{good_memory_allocator::DEFAULT_SMALLBINS_AMOUNT}, {good_memory_allocator::DEFAULT_ALIGNMENT_SUB_BINS_AMOUNT}>
        ::empty();
    galloc.init(HEAP.as_ptr() as usize, HEAP_SIZE);
    Box::new(galloc)
}

unsafe fn init_rlsf() -> Box<dyn GlobalAlloc + Sync> {
    let tlsf = GlobalRLSF(spin::Mutex::new(rlsf::Tlsf::new()));
    tlsf.0.lock().insert_free_block(unsafe { std::mem::transmute(&mut HEAP[..]) });
    Box::new(tlsf)
}

unsafe fn init_buddy_alloc() -> Box<dyn GlobalAlloc + Sync> {
    let ba = BuddyAllocWrapper(spin::Mutex::new(NonThreadsafeAlloc::new(
        FastAllocParam::new(HEAP.as_ptr().cast(), HEAP.len() / 8),
        BuddyAllocParam::new(
            HEAP.as_ptr().cast::<u8>().add(HEAP.len() / 8),
            HEAP.len() / 8 * 7,
            64,
        ),
    )));
    
    Box::new(ba)
}

unsafe fn init_dlmalloc() -> Box<dyn GlobalAlloc + Sync> {
    let dl = DlMallocator(spin::Mutex::new(
        dlmalloc::Dlmalloc::new_with_allocator(DlmallocArena(spin::Mutex::new(false))),
    ));
    Box::new(dl)
}

struct BuddyAllocWrapper(pub spin::Mutex<NonThreadsafeAlloc>);

unsafe impl Send for BuddyAllocWrapper {}
unsafe impl Sync for BuddyAllocWrapper {}

unsafe impl GlobalAlloc for BuddyAllocWrapper {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8  { self.0.lock().alloc(layout) }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout)  { self.0.lock().dealloc(ptr, layout) }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 { self.0.lock().alloc_zeroed(layout) }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.0.lock().realloc(ptr, layout, new_size)
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

// Turn DlMalloc into an arena allocator
struct DlmallocArena(spin::Mutex<bool>);

unsafe impl dlmalloc::Allocator for DlmallocArena {
    fn alloc(&self, _: usize) -> (*mut u8, usize, u32) {
        let mut lock = self.0.lock();

        if *lock {
            (core::ptr::null_mut(), 0, 0)
        } else {
            *lock = true;
            let align = std::mem::align_of::<usize>();
            let heap_align_offset = unsafe { HEAP.as_mut_ptr() }.align_offset(align);
            (unsafe { HEAP.as_mut_ptr().add(heap_align_offset) }, (HEAP_SIZE - heap_align_offset) / align * align, 1)
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

struct GlobalRLSF<'p>(spin::Mutex<rlsf::Tlsf<'p, usize, usize, {usize::BITS as usize - 12}, {usize::BITS as _}>>);
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
