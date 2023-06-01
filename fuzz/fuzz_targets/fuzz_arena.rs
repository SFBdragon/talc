#![no_main]

#![feature(allocator_api)]
#![feature(slice_ptr_get)]

use std::alloc::{Layout, GlobalAlloc};
use std::ptr;

use talloc::*;

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
    // Extend the arena by the additional amount specifie on the low and high side
    Extend { low: u16, high: u16 },
}
use Actions::*;

fuzz_target!(|data: (u8, u8, u8, Vec<Actions>)| {
    let (lo, hi, min_size, actions) = data;

    // fuzzed code goes here
    let arena = Box::leak(vec![0u8; 1 << 25].into_boxed_slice());
    
    let arena_len = arena.len();
    let halfway = arena.len() >> 1;
    let start_arena = &mut arena[(halfway.saturating_sub(lo as usize))..(halfway + hi as usize).min(arena_len)];

    let allocator = Talloc::<{talloc::SPEED_BIAS}>::new_arena(start_arena, min_size as usize);
    
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
                    //unsafe { ptr.as_ptr().as_mut_ptr().add(old_layout.size()).write_bytes(0xcd, layout.size()); }
                }
            }
            Extend { low, high } => {
                //eprintln!("EXTEND | low: {} high: {} old arena {}", low, high, allocator.core.lock().get_arena());

                let new_arena = allocator.core.lock().get_arena()
                    .extend(low as usize, high as usize)
                    .above(arena.as_mut_ptr() as isize)
                    .below(arena.as_mut_ptr().wrapping_add(arena.len()) as isize);

                let _ = unsafe { allocator.core.lock().extend(new_arena, MemMode::Automatic) };
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
