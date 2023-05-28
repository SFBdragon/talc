#![no_main]

#![feature(allocator_api)]
#![feature(slice_ptr_get)]

use std::alloc::{Layout, Allocator};
use std::ptr::NonNull;

use talloc::*;

use libfuzzer_sys::fuzz_target;

use libfuzzer_sys::arbitrary::Arbitrary;

#[derive(Arbitrary, Debug)]
enum Actions {
    /// Allocate memory with the given size and align of 1 << (align % 12)
    Alloc { size: u16, align_bit: u8 },
    /// Dealloc the ith allocation
    Dealloc { index: u16 },
    /// Realloc the ith allocation
    Realloc { index: u16, new_size: u16, new_align_bit: u8 },
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

    let allocator = Talloc::<{talloc::SPEED_BIAS}>::new_arena(start_arena, min_size as usize).wrap_spin_lock();
    
    let mut allocations: Vec<Option<(NonNull<u8>, Layout)>> = vec![];

    for action in actions {
        match action {
            Alloc { size, align_bit } => {
                //println!("ALLOC | size: {:x} align: {:x}", size as usize, 1 << align_bit % 12);

                let layout = Layout::from_size_align(size as usize, 1 << align_bit % 12).unwrap();
                let result = allocator.allocate(layout);

                if let Ok(ptr) = result {
                    allocations.push(Some((unsafe { ptr.get_unchecked_mut(0) }, layout)));
                    unsafe { ptr.as_ptr().as_mut_ptr().write_bytes(0xab, layout.size()); }
                }
            }
            Dealloc { index } => {
                
                if allocations.len() == 0 { continue; }
                let index = index as usize % (allocations.len() * 6);
                
                match allocations.get(index) {
                    Some(&Some((ptr, layout))) => {
                        //println!("DEALLOC | ptr: {:p} size: {:x} align: {:x}", ptr, layout.size(), layout.align());
                        unsafe { allocator.deallocate(ptr, layout); }
                        allocations[index as usize] = None;
                    }
                    _ => {}
                }
            }
            Realloc { index, new_size, new_align_bit } => {
                if allocations.len() == 0 { continue; }
                let index = index as usize % (allocations.len() * 6);

                match allocations.get(index) {
                    Some(&Some((ptr, old_layout))) => {
                        //println!("REALLOC | ptr: {:p} old size: {:x} old align: {:x} new_size: {:x} new_align: {:x}", ptr, old_layout.size(), old_layout.align(), new_size as usize, 1 << new_align_bit % 12);
                        let new_layout = Layout::from_size_align(new_size as usize, 1 << new_align_bit % 12).unwrap();

                        let result = if new_layout.size() > old_layout.size() {
                            unsafe { allocator.grow(ptr, old_layout, new_layout) }
                        } else {
                            unsafe { allocator.shrink(ptr, old_layout, new_layout) }
                        };

                        if let Ok(new_ptr) = result {
                            allocations[index as usize] = Some((unsafe { new_ptr.get_unchecked_mut(0) }, new_layout));
                        }
                    }
                    _ => {}
                }
            }
            Extend { low, high } => {
                let mut talloc = allocator.lock();
                let new_arena = talloc.get_arena()
                    .extend(low as usize, high as usize)
                    .above(arena.as_mut_ptr())
                    .below(arena.as_mut_ptr().wrapping_add(arena.len()));

                let _ = unsafe { talloc.extend(new_arena, MemMode::Automatic) };
            }
        }
    }

    // Free any remaining allocations.
    for alloc in allocations {
        if let Some((ptr, layout)) = alloc {
            unsafe { allocator.deallocate(ptr, layout); }
        }
    }

    unsafe { drop(Box::from_raw(arena)); }
});
