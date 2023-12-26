#![no_main]

#![feature(allocator_api)]
#![feature(slice_ptr_get)]

use std::alloc::{Layout, GlobalAlloc};
use std::ptr;

use talc::*;

use libfuzzer_sys::fuzz_target;

use libfuzzer_sys::arbitrary::Arbitrary;

#[derive(Arbitrary, Debug)]
enum Actions {
    /// Allocate memory with the given size and align of 1 << (align % 12)
    Alloc { size: u16, align_bit: u8 },
    /// Dealloc the ith allocation
    Dealloc { index: u8 },
    /// Realloc the ith allocation
    Realloc { index: u8, new_size: u16 },
    /// Claim a new segment of memory
    Claim { offset: u16, size: u16, capacity: u16 },
    // Extend the ith heap by the additional amount specified on the low and high side
    Extend { index: u8, low: u16, high: u16 },
    // Truncate the ith heap by the additional amount specified on the low and high side
    Truncate { index: u8, low: u16, high: u16 },
}
use Actions::*;

fuzz_target!(|actions: Vec<Actions>| {

    let allocator = Talc::new(ErrOnOom).lock::<spin::Mutex<()>>();
    
    let mut allocations: Vec<(*mut u8, Layout)> = vec![];
    let mut heaps: Vec<(Span, Span)> = vec![];

    for action in actions {
        match action {
            Alloc { size, align_bit } => {
                if size == 0 || align_bit > 12 { continue; }
                //eprintln!("ALLOC | size: {:x} align: {:x}", size as usize, 1 << align_bit % 12);

                let layout = Layout::from_size_align(size as usize, 1 << align_bit).unwrap();
                let ptr = unsafe { allocator.alloc(layout) };

                if ptr::null_mut() != ptr {
                    allocations.push((ptr, layout));
                    unsafe { ptr.write_bytes(0xab, layout.size()); }
                }
            }
            Dealloc { index } => {
                if index as usize >= allocations.len() { continue; }
                
                let (ptr, layout) = allocations[index as usize];
                
                //eprintln!("DEALLOC | ptr: {:p} size: {:x} align: {:x}", ptr, layout.size(), layout.align());
                unsafe { allocator.dealloc(ptr, layout); }
                allocations.swap_remove(index as usize);
            }
            Realloc { index, new_size } => {
                if index as usize >= allocations.len() { continue; }
                if new_size == 0 { continue; }

                let (ptr, old_layout) = allocations[index as usize];
                
                //eprintln!("REALLOC | ptr: {:p} old size: {:x} old align: {:x} new_size: {:x}", ptr, old_layout.size(), old_layout.align(), new_size as usize);
                let new_layout = Layout::from_size_align(new_size as usize, old_layout.align()).unwrap();
                let ptr = unsafe { allocator.realloc(ptr, old_layout, new_size as usize) };

                if !ptr.is_null() {
                    allocations[index as usize] = (ptr, new_layout);
                    if old_layout.size() < new_size as usize {
                        unsafe { ptr.add(old_layout.size()).write_bytes(0xcd, new_size as usize - old_layout.size()); }
                    }
                }
            },
            Claim { offset, size, capacity } => {
                if capacity == 0 { continue; }

                let capacity = capacity as usize;
                let mem = Span::from(Box::leak(vec![0u8; capacity].into_boxed_slice()));

                let size = size as usize % capacity;
                let offset = if size == capacity { 0 } else { offset as usize % (capacity - size) };

                let heap = mem.truncate(offset, capacity - size + offset);
                let heap = unsafe { allocator.lock().claim(heap) };

                if let Ok(heap) = heap {
                    heaps.push((heap, mem));
                } else {
                    drop(unsafe { Vec::from_raw_parts(mem.get_base_acme().unwrap().0, 0, mem.size()) });
                }
            },
            Extend { index, low, high } => {
                //eprintln!("EXTEND | low: {} high: {} old arena {}", low, high, allocator.talc().get_arena());

                let index = index as usize;
                if index >= heaps.len() { continue; }

                let (old_heap, mem) = heaps[index];

                let new_heap = old_heap.extend(low as usize, high as usize).fit_within(mem);
                let new_heap = unsafe { allocator.lock().extend(old_heap, new_heap) };

                heaps[index].0 = new_heap;
            },
            Truncate { index, low, high } => {
                //eprintln!("TRUNCATE | low: {} high: {} old arena {}", low, high, allocator.talc().get_arena());

                let index = index as usize;
                if index >= heaps.len() { continue; }

                let (old_heap, _) = heaps[index];

                let mut talc = allocator.lock();

                let new_heap = old_heap
                    .truncate(low as usize, high as usize)
                    .fit_over(unsafe { talc.get_allocated_span(old_heap) });

                let new_heap = unsafe { talc.truncate(old_heap, new_heap) };

                if new_heap.is_empty() {
                    let (_, mem) = heaps.swap_remove(index);
                    drop(unsafe { Vec::from_raw_parts(mem.get_base_acme().unwrap().0, 0, mem.size()) });
                } else {
                    heaps[index].0 = new_heap;
                }
            }
        }
    }

    // Free any remaining allocations.
    for (ptr, layout) in allocations {
        //eprintln!("DEALLOC FINAL | ptr: {:p} size: {:x} align: {:x}", ptr, layout.size(), layout.align());
        unsafe { allocator.dealloc(ptr, layout); }
    }

    // drop the remaining heaps
    for (_, mem) in heaps {
        unsafe {
            drop(Vec::from_raw_parts(mem.get_base_acme().unwrap().0, 0, mem.size()));
        }
    }
});
