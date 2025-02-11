#![no_main]
#![feature(allocator_api)]
#![feature(slice_ptr_get)]

use std::alloc::{GlobalAlloc, Layout, alloc, dealloc};
use std::ptr::null_mut;

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
use Actions::*;
use talc::{Arena, ErrOnOom};

fuzz_target!(|actions: Vec<Actions>| fuzz_talc(actions));

struct FuzzBinning;
impl talc::Binning for FuzzBinning {
    type AvailabilityBitField = u32;

    const BIN_COUNT: u32 = 25;

    fn size_to_bin(size: usize) -> u32 {
        talc::base::binning::linear_extent_then_linearly_divided_exponential_binning::<2, 8>(size)
    }
}

fn fuzz_talc(actions: Vec<Actions>) {
    let allocator: talc::cell::TalcCell<ErrOnOom, FuzzBinning> =
        talc::cell::TalcCell::new(ErrOnOom);

    let mut allocations: Vec<(*mut u8, Layout)> = vec![];
    let mut arenas: Vec<(*mut u8, Layout, Arena)> = vec![];

    for action in actions {
        match action {
            Alloc { size, align_bit } => {
                if size == 0 || align_bit > 12 {
                    continue;
                }

                // eprintln!("ALLOC | size: {:x} align: {:x}", size as usize, 1 << align_bit % 12);

                let layout = Layout::from_size_align(size as usize, 1 << align_bit).unwrap();
                let ptr = unsafe { allocator.alloc(layout) };

                if null_mut() != ptr {
                    allocations.push((ptr, layout));
                    unsafe {
                        ptr.write_bytes(0xab, layout.size());
                    }
                }
            }
            Dealloc { index } => {
                if allocations.len() > 0 {
                    let index = index as usize % allocations.len();
                    let (ptr, layout) = allocations.swap_remove(index);
                    // eprintln!("DEALLOC | ptr: {:p} size: {:x} align: {:x}", ptr, layout.size(), layout.align());
                    unsafe {
                        allocator.dealloc(ptr, layout);
                    }
                }
            }
            Realloc { index, new_size } => {
                if allocations.len() > 0 {
                    let index = index as usize % allocations.len();
                    if new_size == 0 {
                        continue;
                    }

                    let (ptr, old_layout) = allocations[index as usize];

                    // eprintln!("REALLOC | ptr: {:p} old size: {:x} old align: {:x} new_size: {:x}", ptr, old_layout.size(), old_layout.align(), new_size as usize);

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
            Claim { offset, size, additional_capacity } => {
                let offset = offset as usize;
                let size = size as usize;
                let capacity = offset + size + additional_capacity as usize;

                if capacity == 0 {
                    continue;
                }

                let mem_layout = Layout::from_size_align(capacity, 1).unwrap();
                let mem = unsafe { alloc(mem_layout) };
                assert!(!mem.is_null());

                if let Some(arena) = unsafe { allocator.claim(mem.add(offset), size) } {
                    // eprintln!("CLAIM | heap {}", heap);
                    arenas.push((mem, mem_layout, arena));
                } else {
                    unsafe {
                        dealloc(mem, mem_layout);
                    }
                }
            }
            Extend { index, bytes } => {
                if arenas.len() > 0 {
                    let index = index as usize % arenas.len();
                    let (ptr, mem_layout, arena) = &mut arenas[index];

                    // eprintln!("EXTEND | high: {} old heap {}", high, old_heap);

                    let new_size = (bytes as usize)
                        .min(mem_layout.size() - (arena.base() as usize - *ptr as usize));
                    unsafe {
                        allocator.extend(arena, new_size);
                    }
                }
            }
            Truncate { index, bytes } => {
                if arenas.len() > 0 {
                    let index = index as usize % arenas.len();
                    let (mem, mem_layout, arena) = arenas.swap_remove(index);

                    // eprintln!("TRUNCATE | high: {} old heap {}", high, old_heap);

                    let new_arena = unsafe {
                        let new_size = bytes as usize;
                        allocator.truncate(arena, new_size)
                    };

                    if let Some(new_arena) = new_arena {
                        arenas.push((mem, mem_layout, new_arena));
                    } else {
                        unsafe {
                            dealloc(mem, mem_layout);
                        }
                    }
                }
            }
            Reserved { index } => {
                if arenas.len() > 0 {
                    let index = index as usize % arenas.len();
                    let arena = &arenas[index].2;

                    unsafe {
                        allocator.reserved(arena);
                    }
                }
            }
        }
    }

    // Free any remaining allocations.
    for (ptr, layout) in allocations {
        //eprintln!("DEALLOC FINAL | ptr: {:p} size: {:x} align: {:x}", ptr, layout.size(), layout.align());
        unsafe {
            allocator.dealloc(ptr, layout);
        }
    }

    // drop the remaining heaps
    let mut undropped_arenas = vec![];
    for (mem, mem_layout, arena) in arenas {
        if let Some(arena) = unsafe { allocator.truncate(arena, 0) } {
            undropped_arenas.push((mem, mem_layout, arena));
        } else {
            unsafe {
                dealloc(mem, mem_layout);
            }
        }
    }

    let counters = allocator.counters();
    assert_eq!(counters.allocated_bytes, 0);
    assert_eq!(counters.allocation_count, 0);
    assert!(counters.arena_count <= 1);

    drop(allocator);
    for (mem, mem_layout, _arena) in undropped_arenas {
        unsafe {
            dealloc(mem, mem_layout);
        }
    }
}
