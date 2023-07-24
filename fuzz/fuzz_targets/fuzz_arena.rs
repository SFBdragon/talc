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
    // Extend the arena by the additional amount specified on the low and high side
    Extend { low: u16, high: u16 },
    // Truncate the arena by the additional amount specified on the low and high side
    Truncate { low: u16, high: u16 },
}
use Actions::*;

fuzz_target!(|data: (usize, Vec<Actions>)| {
    let (arena_size, actions) = data;

    let arena = Box::leak(vec![0u8; arena_size % (1 << 24)].into_boxed_slice());
    arena.fill(0x11);

    let allocator = Talc::new().lock::<spin::Mutex<()>>();
    unsafe { allocator.0.lock().init(arena.into()); }
    
    let mut allocations: Vec<(*mut u8, Layout)> = vec![];

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
            Extend { low, high } => {
                //eprintln!("EXTEND | low: {} high: {} old arena {}", low, high, allocator.0.lock().get_arena());

                let new_arena = allocator.0.lock().get_arena()
                    .extend(low as usize, high as usize)
                    .fit_within(arena.into());

                let _ = unsafe { allocator.0.lock().extend(new_arena) };
            },
            Truncate { low, high } => {
                //eprintln!("TRUNCATE | low: {} high: {} old arena {}", low, high, allocator.0.lock().get_arena());

                let mut talc = allocator.0.lock();
                let new_arena = talc.get_arena()
                    .truncate(low as usize, high as usize)
                    .fit_over(talc.get_allocated_span());

                talc.truncate(new_arena);
            }
        }
    }

    // Free any remaining allocations.
    for (ptr, layout) in allocations {
        //eprintln!("DEALLOC FINAL | ptr: {:p} size: {:x} align: {:x}", ptr, layout.size(), layout.align());
        unsafe { allocator.dealloc(ptr, layout); }
    }

    unsafe { drop(Box::from_raw(arena)); }
});
