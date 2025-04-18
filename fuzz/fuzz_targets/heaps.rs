#![no_main]
#![feature(allocator_api)]
#![feature(slice_ptr_get)]

use std::alloc::{GlobalAlloc, Layout, alloc, dealloc};
use std::ptr::null_mut;

use talc::prelude::*;

use libfuzzer_sys::arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
enum Actions {
    /// Allocate memory with the given size and align of 1 << (align % 12)
    Alloc {
        size: u16,
        align_bit: u8,
    },
    /// Dealloc the ith allocation
    Dealloc {
        index: u8,
    },
    /// Realloc the ith allocation
    Realloc {
        index: u8,
        new_size: u16,
    },
    /// Claim a new segment of memory
    Claim {
        offset: u8,
        size: u16,
        additional_capacity: u16,
    },
    // Extend the ith heap by the additional bytes specified
    Extend {
        index: u8,
        bytes: u16,
    },
    // Truncate the ith heap by the additional bytes specified
    Truncate {
        index: u8,
        bytes: u16,
    },
    // Query the reserved extent of the ith heap
    Reserved {
        index: u8,
    },
}

fuzz_target!(|actions: Vec<Actions>| fuzz_talc(actions));

struct FuzzBinning;
impl Binning for FuzzBinning {
    type AvailabilityBitField = u32;

    const BIN_COUNT: u32 = 25;

    fn size_to_bin(size: usize) -> u32 {
        talc::base::binning::linear_extent_then_linearly_divided_exponential_binning::<2, 8>(size)
    }
}

fn fuzz_talc(actions: Vec<Actions>) {
    let allocator: talc::cell::TalcCell<Manual, FuzzBinning> = talc::cell::TalcCell::new(Manual);

    let mut allocations: Vec<(*mut u8, Layout)> = vec![];
    let mut heaps: Vec<(*mut u8, Layout, *mut u8)> = vec![];

    for action in actions {
        match action {
            Actions::Alloc { size, align_bit } => {
                if size == 0 || align_bit > 12 {
                    continue;
                }

                let layout = Layout::from_size_align(size as usize, 1 << align_bit).unwrap();
                let ptr = unsafe { allocator.alloc(layout) };

                if null_mut() != ptr {
                    /* eprintln!(
                        "ALLOC | size: {:x} align: {:x}, ptr: {:p}",
                        size,
                        1 << align_bit,
                        ptr
                    ); */

                    allocations.push((ptr, layout));
                    unsafe {
                        ptr.write_bytes(0xab, layout.size());
                    }
                }
            }
            Actions::Dealloc { index } => {
                if allocations.len() > 0 {
                    let index = index as usize % allocations.len();
                    let (ptr, layout) = allocations.swap_remove(index);

                    /* eprintln!(
                        "DEALLOC | ptr: {:p} size: {:x} align: {:x}",
                        ptr,
                        layout.size(),
                        layout.align()
                    ); */

                    unsafe {
                        allocator.dealloc(ptr, layout);
                    }
                }
            }
            Actions::Realloc { index, new_size } => {
                if allocations.len() > 0 {
                    let index = index as usize % allocations.len();
                    if new_size == 0 {
                        continue;
                    }

                    let (ptr, old_layout) = allocations[index as usize];

                    /* eprintln!(
                        "REALLOC | ptr: {:p} old size: {:x} old align: {:x} new_size: {:x}",
                        ptr,
                        old_layout.size(),
                        old_layout.align(),
                        new_size as usize
                    ); */

                    let new_layout =
                        Layout::from_size_align(new_size as usize, old_layout.align()).unwrap();

                    let ptr = unsafe { allocator.realloc(ptr, old_layout, new_size as usize) };

                    if !ptr.is_null() {
                        allocations[index as usize] = (ptr, new_layout);
                        if old_layout.size() < new_size as usize {
                            unsafe {
                                ptr.add(old_layout.size())
                                    .write_bytes(0xcd, new_size as usize - old_layout.size());
                            }
                        }
                    }
                }
            }
            Actions::Claim { offset, size, additional_capacity } => {
                let offset = offset as usize;
                let size = size as usize;
                let capacity = offset + size + additional_capacity as usize;

                if capacity == 0 {
                    continue;
                }

                let mem_layout = Layout::from_size_align(capacity, 1).unwrap();
                let mem = unsafe { alloc(mem_layout) };
                assert!(!mem.is_null());

                if let Some(end) = unsafe { allocator.claim(mem.add(offset), size) } {
                    /* eprintln!("CLAIM | end {:p}", end); */

                    heaps.push((mem, mem_layout, end.as_ptr()));
                } else {
                    unsafe {
                        dealloc(mem, mem_layout);
                    }
                }
            }
            Actions::Extend { index, bytes } => {
                if heaps.len() > 0 {
                    let index = index as usize % heaps.len();
                    let (ptr, mem_layout, end) = &mut heaps[index];

                    let new_end =
                        end.wrapping_add(bytes as _).min((*ptr).wrapping_add(mem_layout.size()));
                    unsafe {
                        let new_end = allocator.extend(*end, new_end).as_ptr();

                        /* eprintln!("EXTEND | old end: {:p} new end {:p}", *end, new_end); */

                        *end = new_end;
                    }
                }
            }
            Actions::Truncate { index, bytes } => {
                if heaps.len() > 0 {
                    let index = index as usize % heaps.len();
                    let (mem, mem_layout, end) = heaps.swap_remove(index);

                    let new_end = unsafe { allocator.truncate(end, end.wrapping_sub(bytes as _)) };

                    /* eprintln!("TRUNCATE | old end: {:p} new end: {:?}", end, new_end); */

                    if let Some(new_end) = new_end {
                        heaps.push((mem, mem_layout, new_end.as_ptr()));
                    } else {
                        unsafe {
                            dealloc(mem, mem_layout);
                        }
                    }
                }
            }
            Actions::Reserved { index } => {
                if heaps.len() > 0 {
                    let index = index as usize % heaps.len();
                    let end = heaps[index].2;

                    unsafe {
                        allocator.reserved(end);
                    }
                }
            }
        }
    }

    // Free any remaining allocations.
    for (ptr, layout) in allocations {
        /* eprintln!(
            "DEALLOC FINAL | ptr: {:p} size: {:x} align: {:x}",
            ptr,
            layout.size(),
            layout.align()
        ); */

        unsafe {
            allocator.dealloc(ptr, layout);
        }
    }

    // Drop the remaining heaps.
    // Typically you wouldn't worry about truncating down each heap
    // before freeing the memory, but it helps catch bugs quicker here.
    // The heap with metadata can't be deleted using truncate, and must
    // be deleted last because `truncate` (an allocator operation) is otherwise used after.

    let mut undropped_heaps = vec![];
    for (mem, mem_layout, heap_end) in heaps {
        if let Some(heap_end) = unsafe { allocator.truncate(heap_end, null_mut()) } {
            undropped_heaps.push((mem, mem_layout, heap_end));
        } else {
            unsafe {
                dealloc(mem, mem_layout);
            }
        }
    }

    let counters = allocator.counters();
    assert_eq!(counters.allocated_bytes, 0);
    assert_eq!(counters.allocation_count, 0);
    assert!(counters.heap_count <= 1);

    drop(allocator);
    for (mem, mem_layout, _heap_end) in undropped_heaps {
        unsafe {
            dealloc(mem, mem_layout);
        }
    }
}
