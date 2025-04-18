#![no_main]
#![feature(allocator_api)]
#![feature(slice_ptr_get)]

use std::alloc::{GlobalAlloc, Layout};
use std::ptr::null_mut;

use talc::prelude::*;

use libfuzzer_sys::arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
enum Actions {
    /// Allocate memory with the given size and align of 1 << (align % 12)
    Alloc { size: u16, align_bit: u8 },
    /// Dealloc the ith allocation
    Dealloc { index: u8 },
    /// Realloc the ith allocation
    Realloc { index: u8, new_size: u16 },
}

fuzz_target!(|actions: Vec<Actions>| fuzz_talc(actions));

fn fuzz_talc(actions: Vec<Actions>) {
    let allocator = TalcCell::new(Os::new());

    let mut allocations: Vec<(*mut u8, Layout)> = vec![];

    for action in actions {
        match action {
            Actions::Alloc { size, align_bit } => {
                if size == 0 || align_bit > 12 {
                    continue;
                }

                let layout = Layout::from_size_align(size as usize, 1 << align_bit).unwrap();
                let ptr = unsafe { allocator.alloc(layout) };

                if null_mut() != ptr {
                    // eprintln!(
                    //     "ALLOC | size: {:x} align: {:x}, ptr: {:p}",
                    //     size,
                    //     1 << align_bit,
                    //     ptr
                    // );

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

                    // eprintln!(
                    //     "DEALLOC | ptr: {:p} size: {:x} align: {:x}",
                    //     ptr,
                    //     layout.size(),
                    //     layout.align()
                    // );

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

                    // eprintln!(
                    //     "REALLOC | ptr: {:p} old size: {:x} old align: {:x} new_size: {:x}",
                    //     ptr,
                    //     old_layout.size(),
                    //     old_layout.align(),
                    //     new_size
                    // );

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
        }
    }

    // Free any remaining allocations.
    for (ptr, layout) in allocations {
        // eprintln!(
        //     "DEALLOC FINAL | ptr: {:p} size: {:x} align: {:x}",
        //     ptr,
        //     layout.size(),
        //     layout.align()
        // );
        unsafe {
            allocator.dealloc(ptr, layout);
        }
    }
}
