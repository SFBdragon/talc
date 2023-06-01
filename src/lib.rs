//#![doc = include_str!("../README.md")]

//#![cfg_attr(not(test), no_std)]

#![feature(alloc_layout_extra)]

#![feature(slice_ptr_get)]
#![feature(slice_ptr_len)]
#![feature(const_mut_refs)]
#![feature(const_slice_ptr_len)]
#![feature(const_slice_from_raw_parts_mut)]

#![feature(core_intrinsics)]
#![feature(const_assume)]

#![feature(maybe_uninit_uninit_array)]
#![feature(maybe_uninit_array_assume_init)]
#![feature(const_maybe_uninit_uninit_array)]
#![feature(const_maybe_uninit_array_assume_init)]

#![cfg_attr(feature = "allocator", feature(allocator_api))]

mod span;

mod utils;

#[allow(dead_code)]
mod llist;

//#[cfg(feature = "spin")]
//mod tallock;


//pub use utils::copy_slice_bits; // for fuzzing

use spin::Mutex;
pub use span::Span;
use llist::LlistNode;

//#[cfg(feature = "spin")]
//pub use tallock::Tallock;

use core::{
    ptr,
    alloc::{Layout, GlobalAlloc},
    intrinsics::{unlikely, assume, likely},
};


// desciptive error for failures
// borrow allocator_api's if available, else define our own
#[cfg(feature = "allocator")]
pub use core::alloc::AllocError;
use std::ptr::NonNull;

#[cfg(not(feature = "allocator"))]
#[derive(Debug)]
pub struct AllocError();
#[cfg(not(feature = "allocator"))]
impl core::fmt::Display for AllocError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("memory allocation failed")
    }
}


const NULL: isize = 0;
const ZERO_PAGE: isize = 1 << 12;
const NODE_SIZE: usize = core::mem::size_of::<LlistNode>();
const NODE_ALIGN: usize = core::mem::align_of::<LlistNode>();

/// The minimum value of `min_size`. Lower values are clamped to this value.
pub const MIN_MIN_SIZE: usize = NODE_SIZE.next_power_of_two();
/// The minimum arena size eligible for `extend` using `MemMode::Automatic`.
/// Smaller values yield `Err(AllocError)`.
pub const MIN_ARENA_SIZE: usize = 1 << 6;

/// Fastest, waste a quarter of memory on average (or more, if size < align).
pub const MAX_SPEED_BIAS: usize = 0;
/// Fast, waste an eighth of memory on average.
pub const SPEED_BIAS: usize = 2;
/// Not quite as fast, waste a sixteenth of memory on average.
pub const EFFICIENCY_BIAS: usize = 3;
/// Slower, waste half a `min_size` on average.
pub const MAX_EFFICIENCY_BIAS: usize = usize::MAX;


/// Simple `OomHandler` function that immediately returns `Err(AllocError)`.
pub const fn alloc_error<const BIAS: usize>(_: &mut TallocCore<BIAS>, _: Layout) -> Result<(), AllocError> {
    Err(AllocError)
}


/// Returns the base pointer to the buddy of this block of `size`.
#[inline]
fn buddy(base: *mut u8, size: usize) -> *mut u8 {
    (base as usize ^ size) as *mut u8
}

/// Returns the base pointer to the pair this block of `size` belongs.
#[inline]
fn pair(base: *mut u8, size: usize) -> *mut u8 {
    (base as usize & !size) as *mut u8
}


#[inline]
const fn align_up(addr: isize, align: usize) -> isize {
    debug_assert!(align.count_ones() == 1);

    let min_size_m1 = align - 1;
    ((addr as usize).wrapping_add(min_size_m1) & !min_size_m1) as isize
}
#[inline]
const fn align_down(addr: isize, align: usize) -> isize {
    debug_assert!(align.count_ones() == 1);

    (addr as usize & !(align - 1)) as isize
}

#[inline]
fn align_ptr_down(ptr: *mut u8, align: usize) -> *mut u8 {
    align_down(ptr as isize, align) as *mut u8
}

#[inline]
fn align_ptr_up(ptr: *mut u8, align: usize) -> *mut u8 {
    align_up(ptr as isize, align) as *mut u8
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemMode {
    /// The memory in the arena is released for allocation 
    /// automatically and is used for `metadata_memory` too.
    Automatic,
    /// Specify memory management parameters.
    Manual {
        /// Whether new memeory is automatically released.
        auto_release: bool, 
        /// Must be in accordance with [`req_meta_mem`].
        /// 
        /// [`req_meta_mem`]: method.Talloc.req_meta_mem.html
        metadata_memory: Option<*mut u8> 
    },
}

impl MemMode {
    #[inline]
    pub const fn auto_release(self) -> bool {
        matches!(self, MemMode::Automatic | MemMode::Manual { auto_release: true, metadata_memory: _ })
    }
}

pub struct TallocCore<const BIAS: usize> {
    oom_handler: Option<OomHandler<BIAS>>,

    /// The power-of-two size of the smallest allocatable block in bytes.
    min_size: usize,

    /// The span of the arena, aligned to `min_size`.
    arena: Span,

    /// The next power-of-two size of the arena in bytes.
    size_npow2: usize,
    /// The log base 2 of the next power-of-two of the arena size plus one.
    /// 
    /// `log2(pair size) = l2_sz_2np2 - g`
    l2_sz_2np2: usize,
    /// The leading zero count of the next power-of-two of the arena size.
    lzcnt_sz_np2: u32,

    // Blocks are powers of two, aligned on power of two addresses, of granularity G
    // Blocks are halved into buddies of G(n+1), with half the size and align
    // G0 corresponds to a size and align of the arena size to the next power of two
    // G1 corresponds to half the size and align of G0, etc. until G of the smallest block size

    /// Tracks memory block availability in the linked lists.
    /// 
    ///  Bit index `i` corresponds to granularity `Gi`.
    avails: usize,

    /// The sentinels of the linked lists that each hold available fixed-size 
    /// memory blocks per granularity at an index.
    /// 
    /// `llists[i]` contains blocks of size and align corresponding to `Gi`
    llists: *mut [LlistNode],


    /// Describes occupation of memory blocks in the arena.
    /// 
    /// Bitfield of length `1 << llists.len()` in bits, where each bitfield subset per granularity 
    /// has a bit for each buddy, offset from the base by that width in bits. Where digits 
    /// represent each bit for a certain granularity: `01223333_44444444_55555555_55555555_6..`. 
    /// Buddies are represented from low addresses to high addresses.
    /// * Clear bit indicates homogeneity: both or neither are allocated.
    /// * Set bit indicated heterogeneity: one buddy is allocated.
    bitmap: *mut [u8],

    /// The low-flags of the bitmap, as each bit field needs an extra for pair alignment.
    /// Bit index corresponds to granularity.
    /// 
    /// `bmp_idx` will return `None` where using this is necessary. 
    lflags: usize,
}

// TODO
impl<const BIAS: usize> core::fmt::Debug for TallocCore<BIAS> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Talloc")
        .field("arena", &format_args!("{:?}", self.arena))
        .field("size_npow2", &format_args!("{:#x}", self.size_npow2))
        .field("l2_sz_2np2", &format_args!("{}", self.l2_sz_2np2))
        .field("lzcnt_sz_np2", &format_args!("{}", self.lzcnt_sz_np2))
        .field("min_size", &format_args!("{:#x}", self.min_size))
        .field("avails", &format_args!("{:#b}", self.avails))
        .field("llists", &format_args!("{:?}", self.llists))
        .field("bitmap", &format_args!("{:?}", self.bitmap))
        .field("lflags", &format_args!("{:#b}", self.lflags))
        .finish()
    }
}

impl<const BIAS: usize> TallocCore<BIAS> {
    /* /// Utility function to read the bitmap at the offset in bits.
    /// 
    /// `base` is expected to be aligned to `g`'s corresponding block size.
    /// 
    /// # Safety
    /// In order to safely do this, `locks[g]` must be held.
    #[inline]
    fn read_bitflag(&self, base: *mut u8, g: usize) -> bool {
        if let Some(bmp_idx) = self.bmp_idx(base, g) {
            let index = bmp_idx >> u8::BITS.trailing_zeros();
            // SAFETY: bmp_idx should be valid (as checked in debug builds)
            let data = unsafe { *self.bitmap.get_unchecked_mut(index) };
            let bit_mask = 1 << (bmp_idx & u8::BITS as usize - 1);

            data & bit_mask != 0
        } else {
            self.lflags.load(Ordering::SeqCst) & (1 << g) != 0
        }
    } */

    /// Utility function to toggle the bitmap at the offset in bits.
    /// 
    /// `base` is expected to be aligned to `g`'s corresponding block size.
    /// 
    /// # Safety:
    /// `avails` and `llists` must be updated accordingly. 
    #[inline]
    unsafe fn toggle_bitflag(&mut self, base: *mut u8, g: usize) -> bool {
        if let Some(bmp_idx) = self.bmp_idx(base, g) {
            // bmp_idx is in bits; get the byte and sub-byte indecies seperately
            let index = bmp_idx / 8;
            // SAFETY: bmp_idx should be valid (as checked in debug builds)
            let data = unsafe { self.bitmap.get_unchecked_mut(index) };
            let bit_mask = 1 << (bmp_idx % 8);

            let bmp_byte = *data;
            unsafe { *data = bmp_byte ^ bit_mask; }
            bmp_byte & bit_mask != 0
        } else {
            let bit_mask = 1 << g;
            let lflags = self.lflags;
            self.lflags ^= bit_mask;
            lflags & bit_mask != 0
        }
    }

    /// Registers a block into the books, making it available for allocation,
    /// but does not toggle the bitflag, this is left up to the user.
    /// 
    /// ### SAFETY:
    /// * The block at `node` of `g` must be allocatable and not allocated.
    /// * `node` is expected to be aligned to `g`'s corresponding block size.
    /// * Caller must toggle the bitflag before invocation.
    #[inline]
    unsafe fn add_block_noflip(&mut self, node: *mut LlistNode, g: usize) {
        debug_assert!(g < self.llists.len());
        debug_assert!(node as usize % (self.size_npow2 >> g) == 0);
        //eprintln!("add block {}", Span::from_ptr_size(node.cast(), self.size_npow2 >> g));

        // populating llist
        self.avails |= 1 << g;

        // add node to llist
        // SAFETY: caller guaranteed and by the above assertions
        let sentinel = self.llists.get_unchecked_mut(g);
        LlistNode::insert(node, sentinel, (*sentinel).next);

        self.scan_books_for_errors();
    }
    
    /// Unregisters a known block from the free list, reserving it against allocation,
    /// but does not toggle the bitflag, this is left up to the user.
    /// 
    /// ### SAFETY:
    /// * The block at `node` of `g` must be allocatable and not allocated.
    /// * `node` is expected to be aligned to `g`'s corresponding block size.
    /// * Caller must toggle the bitflag before invocation.
    #[inline]
    unsafe fn remove_block_noflip(&mut self, node: *mut LlistNode, g: usize) {
        debug_assert!(g < self.llists.len());
        debug_assert!(node as usize % (self.size_npow2 >> g) == 0);
        //eprintln!("rem block {}", Span::from_ptr_size(node.cast(), self.size_npow2 >> g));

        if (*node).prev == (*node).next {
            // last nonsentinel block in llist, toggle off avails flag
            self.avails &= !(1 << g);
        }

        // remove node from llist
        // SAFETY: caller guaranteed
        LlistNode::remove(node);

        self.scan_books_for_errors();
    }

    /// Registers a block into the books, making it available for allocation.
    /// 
    /// ### SAFETY:
    /// * The block at `node` of `g` must be allocatable and not allocated.
    /// * `node` is expected to be aligned to `g`'s corresponding block size.
    #[inline]
    unsafe fn add_block(&mut self, node: *mut LlistNode, g: usize) {
        let x = self.toggle_bitflag(node.cast(), g);
        debug_assert!(!x);
        self.add_block_noflip(node, g);
    }

    /// Unregisters a known block from the free list, reserving it against allocation.
    /// 
    /// ### Safety:
    /// `node` must be a valid `LlistNode` at an allocatable, but unallocated block of `g`.
    #[inline]
    unsafe fn remove_block(&mut self, node: *mut LlistNode, g: usize) {
        self.toggle_bitflag(node.cast(), g);
        self.remove_block_noflip(node, g);
    }

    /// If this block's buddy is available, it is removed and the base pointer
    /// of the pair is returned. Otherwise, the block is made available and
    /// `None` is returned.
    /// 
    /// This is useful for recombination during deallocation.
    /// 
    /// ### Safety:
    /// The block at `node` of `g` must be allocatable and not allocated.
    #[inline]
    unsafe fn remove_buddy_else_add_base(&mut self, base: *mut u8, size: usize, g: usize) -> bool {
        if self.toggle_bitflag(base, g) {
            // bitflag was positive, thus buddy is available, so remove
            self.remove_block_noflip(buddy(base, size).cast(), g);
            return true;
        } else {
            // bitflag was negative, thus buddy is unavailabe, so add base
            self.add_block_noflip(base.cast(), g);
            return false;
        }
    }

    /// Unregisters the next block in the free list, reserving it against 
    /// allocation, and returning the base.
    /// 
    /// # Safety:
    /// There must be a free block. Accordingly `avails[g]` must be 
    /// set and `llists[g]` should have a nonsentinel node.
    #[inline]
    unsafe fn remove_block_next(&mut self, g: usize) -> *mut u8 {
        let sentinel = unsafe { self.llists.get_unchecked_mut(g) };
        let next_node = unsafe { (*sentinel).next };
        self.remove_block(next_node, g);
        next_node.cast()
    }


    // ---------- the line of no more bookkeeping data access through imm ref ----------- //


    /// Returns the corresponding granularity for a given block size.
    /// 
    /// `size` should not be larger than `self.arena_size_pow2`.
    /// 
    /// # Safety:
    /// `size` must be nonzero.
    #[inline]
    const unsafe fn g_of_size(&self, size: usize) -> usize {
        debug_assert!(self.min_size <= size && size <= self.size_npow2);

        // this prevents a bunch of extra instructions being emmitted when
        // lzcnt isn't available
        assume(size != 0);

        // effectively computing: self.size_npow2.log2() - size.log2()
        (size.leading_zeros() - self.lzcnt_sz_np2) as usize
    }

    /// Returns the offset in bits into the bitmap that indicates the block's buddy status.
    /// 
    /// `base` is expected to be aligned to `g`'s corresponding block size.
    #[inline]
    fn bmp_idx(&self, base: *mut u8, g: usize) -> Option<usize> {
        // get the log base 2 of the pair size
        let pair_log2 = self.l2_sz_2np2 - g;
        // round up base to the next multiple of pair size
        let aligned_base = ((self.arena.base - 1) as usize >> pair_log2).wrapping_add(1) << pair_log2;
        
        debug_assert!(g < self.llists.len());
        debug_assert!(base as usize % (self.size_npow2 >> g) == 0);
        debug_assert!(base as usize >= aligned_base || base as usize + (1 << pair_log2 >> 1) == aligned_base);
        debug_assert!((self.size_npow2 >> g).ilog2() as usize == pair_log2 - 1);
        debug_assert!((base as usize) >= aligned_base || 
            align_up(self.arena.base, self.size_npow2 >> g) as usize & self.size_npow2 >> g != 0);

        // base might be `align - size` while still being valid, handle with lflags
        let arena_offset = (base as usize).checked_sub(aligned_base)?;
        
        debug_assert!(self.size_npow2 > arena_offset);
        
        // self.l2_sz_2np2 - g = (2 * size).ilog2() = (pair size).ilog2()
        // the shift 'crushes' the field offset (size_npow2) and the
        // field index (arena_offset) by the (pair size).ilog2()
        Some(self.size_npow2 + arena_offset >> pair_log2)
    }

    /// Returns the current arena.
    pub const fn get_arena(&self) -> Span {
        self.arena
    }

    /// Returns the metadata memory pointer and requested `Layout`.
    pub const fn get_meta_mem(&self) -> (*mut u8, Layout) {
        (
            self.llists.as_mut_ptr().cast(),

            if let Ok(layout) = Layout::from_size_align(
                self.llists.len() * NODE_SIZE + self.bitmap.len(), 
                NODE_ALIGN
            ) {
                layout
            } else {
                unreachable!()
            },
        )
    }

    /// Returns (llist size, bitmap size, total size).
    const fn slice_bytes(&self, new_size_npow2: usize) -> (usize, usize, usize) {
        assert!(new_size_npow2 >= self.min_size);

        // llists_len is equal to the number of granularities/allocatable sizes
        // let s, m = ceil(log2(new_size)), log2(min_size)
        // for 2^s to 2^m, the number of block sizes is s - m + 1 
        let llists_len = new_size_npow2.ilog2() + 1 - self.min_size.ilog2();
            
        // bitmap size is 2^llists_len bits, convert to bytes, and clamp
        // this allows 1 bit for G0, 1 for G1, 2 for G3, etc.
        let bitmap_len = 1usize << llists_len >> u8::BITS.ilog2();

        let ll_bytes = llists_len as usize * NODE_SIZE;
        let bm_bytes = if bitmap_len != 0 { bitmap_len } else { 1 };

        (ll_bytes, bm_bytes, ll_bytes + bm_bytes)
    }

    /// Returns the requirement for `metadata_memory` as required by `Talloc::extend`.
    /// 
    /// Note that while `MemMode::Automatic` does not explicitly demand `metadata_memory`,
    /// `extend` will fail if there isn't enough memory strictly within the memory in the new
    /// arena to allocate.
    /// 
    /// ### Panics
    /// Panics if `new_arena` doesn't cover the current arena. Empty current arenas are exempt.
    pub const fn req_meta_mem(&self, new_arena: Span) -> Option<Layout> {
        assert!(new_arena.contains_span(self.arena), "new_arena does not contain current arena");

        let new_arena = new_arena.align_inward(self.min_size);

        if new_arena.size() < MIN_ARENA_SIZE { return None; }

        let new_size_npow2 = new_arena.size().next_power_of_two();
        
        if new_size_npow2 == self.size_npow2 && self.arena.base == new_arena.base {
            None
        } else {
            let size = self.slice_bytes(new_size_npow2).2;
            
            if let Ok(layout) = Layout::from_size_align(size, NODE_ALIGN) {
                return Some(layout);
            } else {
                unreachable!()
            }
        }
    }

    /// Extend the arena.
    /// 
    /// This returns `AllocError` when insufficient memory for metadata is available.
    /// 
    /// ### Panics
    /// Panics if `new_arena` does not contain the current arena. Empty current arenas are exempt.
    /// 
    /// ### Safety
    /// - If `MemMode::Automatic` is used or `auto_release` is set to `true`, 
    /// you must guarantee that all of the memory within `new_arena` 
    /// (excluding everything in the current arena) is valid for reads and writes and won't be
    /// corrupted by external modification while the allocator is in use.
    /// - If `MemMode::Manual` is used, you must ensure that the `metadata_memory` pointer
    /// points to sufficient memory as specified by `req_meta_mem` and that it's valid for
    /// reads and writes and isn't corrupted while the allocator is in use.
    pub unsafe fn extend(&mut self, new_arena: Span, mem_mode: MemMode) -> Result<(), AllocError> {
        let new_arena = new_arena.align_inward(self.min_size);
        let new_size_npow2 = new_arena.size().next_power_of_two();

        // ensure arena_base, arena_size covers the old arena
        assert!(new_arena.contains_span(self.arena), "New arena doesn't cover the old arena.");

        // arena is probably too small to comfortably hold the metadata memory
        if matches!(mem_mode, MemMode::Automatic) && new_arena.size() < MIN_ARENA_SIZE {
            return Err(AllocError);
        }

        if new_size_npow2 == self.size_npow2 && self.arena.base == new_arena.base {
            // only arena needs to be updated

            let old_arena = self.arena;
            self.arena = new_arena;

            if mem_mode.auto_release() {
                self.release(new_arena.below(old_arena.base));
                self.release(new_arena.above(old_arena.acme));
            }

            Ok(())
        } else {
            // get metadata memory size
            let (ll_bytes, bm_bytes, meta_size) = self.slice_bytes(new_size_npow2);

            let meta_ptr = match mem_mode {
                MemMode::Automatic => {
                    // given the extention may have occured over null, we need to avoid it
                    // so check the hi span above null, the high spam below null, the low span above ...
                    // always allocate at the high or low edge of the arena for maximum memory continuity
                    // a lot of these checks assume that NODE_ALIGN <= min_size
                    assert!(NODE_ALIGN <= MIN_MIN_SIZE);

                    let meta_base;

                    if new_arena.above(self.arena.acme).above(NULL).size() >= meta_size {
                        meta_base = align_down(new_arena.acme - meta_size as isize, NODE_ALIGN);
                    } else if new_arena.above(self.arena.acme).below(NULL).size() >= meta_size {
                        meta_base = align_down(new_arena.acme.min(NULL) - meta_size as isize, NODE_ALIGN);
                    } else if new_arena.below(self.arena.base).above(ZERO_PAGE).size() >= meta_size {
                        meta_base = new_arena.base.max(ZERO_PAGE);
                    } else if new_arena.below(self.arena.base).below(NULL).size() >= meta_size {
                        meta_base = new_arena.base;
                    } else {
                        return Err(AllocError);
                    }

                    meta_base as *mut u8
                },
                MemMode::Manual { auto_release: _, metadata_memory } => {
                    if let Some(mm) = metadata_memory {
                        assert!(mm.align_offset(NODE_ALIGN) == 0);

                        mm
                    } else {
                        return Err(AllocError);
                    }
                },
            };

            let meta_mem = Span::from_ptr_size(meta_ptr, meta_size);
                
            // new talloc instance
            let mut new_core = TallocCore {
                min_size: self.min_size,
                oom_handler: self.oom_handler,
  
                arena: new_arena,
                size_npow2: new_size_npow2,
                l2_sz_2np2: new_size_npow2.ilog2() as usize + 1,
                lzcnt_sz_np2: new_size_npow2.leading_zeros(),

                // initialized shortly
                avails: 0,
                lflags: 0,

                llists: ptr::slice_from_raw_parts_mut(
                    meta_ptr.cast(), 
                    ll_bytes / NODE_SIZE
                ),
                bitmap: ptr::slice_from_raw_parts_mut(
                    meta_ptr.wrapping_add(ll_bytes), 
                    bm_bytes
                ),
            };
            
            let delta_g = new_core.llists.len() - self.llists.len();
    
            self.scan_books_for_errors();

            // copy/init llists
            for i in 0..new_core.llists.len() {
                if i < delta_g {
                    LlistNode::new(new_core.llists.get_unchecked_mut(i));
                } else {
                    LlistNode::mov(
                        self.llists.get_unchecked_mut(i - delta_g),
                        new_core.llists.get_unchecked_mut(i),
                    );
                }
            }
    
            // shift avails
            new_core.avails <<= delta_g;
            
            // init/copy bitmap
            new_core.bitmap.as_mut_ptr().write_bytes(0, new_core.bitmap.len());
            if self.bitmap.len() != 0 {
                for old_g in 0..self.llists.len() {
                    let new_g = old_g + delta_g;
                    let size = self.size_npow2 >> old_g;
                    
                    // the pair up might not even be in the arena, in which case it's totes blank~
                    let base_pair = align_up(self.arena.base, size << 1);
                    if base_pair < self.arena.acme {
                        let old_bmp_field = 1 << old_g >> 1;

                        // if this panics, there's a serious bug in bmp_idx or base_pair or something
                        let new_bmp_idx = new_core.bmp_idx(base_pair as *mut _, new_g).unwrap();
        
                        utils::copy_slice_bits(
                            new_core.bitmap,
                            self.bitmap,
                            new_bmp_idx,
                            old_bmp_field,
                            old_bmp_field.max(1),
                        );
                        // todo optimize the above using word-size iteration where possible?
                    }
                    
                    if self.lflags & 1 << old_g != 0 {
                        debug_assert!(align_up(self.arena.base, size) as usize & size != 0);
                        new_core.toggle_bitflag(align_ptr_up(self.arena.base_ptr(), size), new_g);
                    }
                }
            }

            new_core.scan_books_for_errors();
    
            let old_ctrl_size = self.llists.len() * NODE_SIZE + self.bitmap.len();
            let old_ctrl_mem = Span::from_ptr_size(self.llists.cast(), old_ctrl_size);
            let contained_ctrl_mem = old_ctrl_mem.align_outward(self.min_size).within(self.arena);
            new_core.release(contained_ctrl_mem);

            new_core.scan_books_for_errors();

            if mem_mode.auto_release() {
                new_core.release(new_core.arena.below(self.arena.base).below(meta_mem.base));
                new_core.release(new_core.arena.below(self.arena.base).above(meta_mem.acme));
                new_core.release(new_core.arena.above(self.arena.acme).below(meta_mem.base));
                new_core.release(new_core.arena.above(self.arena.acme).above(meta_mem.acme));
            }

            //eprintln!("EXTEND");
    
            *self = new_core;

            Ok(())
        }
    }

    fn scan_books_for_errors(&self) {
        #[cfg(debug_assertions)]
        let mut vec = Vec::<Span>::new();
        #[cfg(debug_assertions)]
        for g in 0..self.llists.len() {
            unsafe {
                let sentinel = self.llists.get_unchecked_mut(g);
                assert!(sentinel as usize > 0x1000);
                assert!(self.arena.contains_ptr((*sentinel).next.cast()));
                assert!(self.arena.contains_ptr((*sentinel).prev.cast()));

                for node in LlistNode::iter_mut(sentinel) {
                    assert!(node as usize % (self.size_npow2 >> g) == 0);

                    if let Some(idx) = self.bmp_idx(node.cast(), g) {
                        assert!(*self.bitmap.get_unchecked_mut(idx/8) & 1 << (idx & 0b111) != 0);
                    } else {
                        assert!(self.lflags & 1 << g != 0);
                    }

                    assert!(self.arena.contains_ptr((*node).next.cast()), 
                        "node: {:p} next: {:p} prev: {:p}", node, (*node).next, (*node).prev);
                    assert!(self.arena.contains_ptr((*node).prev.cast()), 
                        "node: {:p} next: {:p} prev: {:p}", node, (*node).next, (*node).prev);

                    let span = Span::from_ptr_size(node.cast(), self.size_npow2 >> g);
                    for &s in &vec {
                        assert!(!span.overlaps(s), "{} {}", span, s);
                    }
                    vec.push(span);
                }
            }
        }
    }

    
    /// Release memory for allocation.
    /// Address-space wraparound is allowed, but the zero page will not be released.
    /// 
    /// Note that this will clamp the memory range to within the arena, and round toward the
    /// alignment of `min_size` interior to the range. 
    /// 
    /// It is recommended to account for this to avoid holes.
    /// 
    /// ### Safety:
    /// * `span` must be readable and writable.
    /// * `span` must not have been previously released by `release` or `extend`.
    /// * `span` must not overlap the current metadata memory, `get_meta_mem`.
    /// * Unallocated memory in `span` must not be modified.
    pub unsafe fn release(&mut self, span: Span) {
        let span = span.within(self.arena).align_inward(self.min_size);
        
        // nothing to release; return early
        if span.is_empty() { return; }
        
        // avoid releasing null, instead release either side of it
        if span.overlaps((NULL..ZERO_PAGE).into()) {
            self.release(span.below(NULL));
            self.release(span.above(ZERO_PAGE));
            return;
        }

        // Strategy:
        // - Start address at the base of the bounds
        // - Repeatedly allocate as large a block as possible for the given alignment, bump address
        // -    Do so until adding a larger block would overflow the top bound
        // - Allocate the previous power of two of the delta between current address and top + smlst, bump address
        // - When the delta is zero, the bounds have been entirely filled
        
        let mut block_base = span.base;
        let mut asc_block_sizes = true;
        loop {
            let block_size = if asc_block_sizes {
                let block_size = 1 << block_base.trailing_zeros();

                if block_base + block_size as isize <= span.acme {
                    block_size
                } else {
                    asc_block_sizes = false;
                    continue;
                }
            } else {
                let delta = (span.acme - block_base) as usize;
                if delta >= self.min_size {
                    // SAFETY: min_size is never zero thus neither is delta
                    utils::prev_pow2_nonzero(delta)
                } else {
                    break;
                }
            };
            
            // SAFETY: deallocating reserved memory is valid and memory safe
            // and block_size is not smaller than self.smlst_block
            // and null has already been avoided from being released
            self.dealloc(
                block_base as *mut u8, 
                Layout::from_size_align_unchecked(block_size, 1)
            );

            self.scan_books_for_errors();
            
            block_base += block_size as isize;
        }
    }
    
    
    /// Takes a `Layout` and outputs a block size that is:
    /// * Nonzero
    /// * A power of two
    /// * Not smaller than smlst_block
    /// * Not smaller than `layout.size`
    /// * Sufficiently aligned
    /// 
    /// ### Safety:
    /// `layout.size` must be nonzero.
    #[inline]
    const unsafe fn layout_to_size(&self, layout: Layout) -> usize {
        // Get the maximum between the required size as a power of two, the smallest allocatable,
        // and the alignment. The alignment being larger than the size is a rather esoteric case,
        // which is handled by simply allocating a larger size with the required alignment. This
        // may be highly memory inefficient for very bizarre scenarios.
        utils::prev_pow2_nonzero( // SAFETY: caller guaranteed
            utils::next_pow2_nonzero(layout.size())
            // there is code that relies on this behaviour of allocating align-sized blocks
            | layout.align()
            | self.min_size
        )
    }


    // TODO: the trim functions need internal documentation
    // for now, we just try to subdivide the allocated block and free some of the 
    // high blocks in a deterministic way, and reverse that process for deallocation
    // this is what BIAS actually does, it controls how finely we try to divvy up allocated blocks

    #[inline]
    unsafe fn trim(&mut self, ptr: *mut u8, size: usize, g: usize, layout_size: usize) {
        if BIAS == 0 { return; }

        let delta_g = BIAS.min(self.llists.len() - g - 1);

        let acme: *mut u8 = ptr.wrapping_add(size);
        
        let mut cursor = align_up(
            ptr as isize + layout_size as isize, 
            size >> delta_g
        );
    
        while cursor < acme as isize {
            let new_acme_less_size = cursor & (cursor - 1);
            let sub_size = cursor - new_acme_less_size;
            self.add_block(cursor as _, self.g_of_size(sub_size as usize));
            cursor += sub_size;
        }

        self.scan_books_for_errors();
    }

    #[inline]
    unsafe fn dealloc_trimmed<const HALF: bool, const GROW: bool, const TRIMUP: bool>(
        &mut self, 
        ptr: *mut u8, 
        size: usize, 
        g: usize, 
        layout_size: usize,
        new_layout: Layout,
    ) -> Result<(), NonNull<u8>> {
        // god is dead, and this function killed 'em
        //
        // This function has four seperate callers and each one needs the same
        // thing done but in slightly different ways and I hate code duplication
        // - dealloc wants to receive an empty size block or we need to deallocate the rest
        // - shrink wants the same but only the top half of the block (HALF)
        // - GROW wants dealloc's but we reallocate before we touch the memory (GROW)
        // - GROW wants grow's where we stop detrimming and retrim to new_size (GROW & TRIMUP)
        //
        // I also dislike unintelligible code, and this function is pretty awful, so eh....

        if BIAS == 0 { return Ok(()); }

        let delta_g = BIAS.min(self.llists.len() - g - 1);
        let mut sub_size = size >> delta_g;

        let offset = align_up(layout_size as isize, sub_size) as usize;
        if offset == size { return Ok(()); }
        let mut cursor = ptr.add(offset);

        let min_g = g + HALF as usize;
        let mut sub_g = g + delta_g;

        while sub_g > min_g {
            if TRIMUP && cursor as isize >= ptr.wrapping_add(new_layout.size()) as isize {
                let mut new_cursor = align_up(
                    ptr as isize + new_layout.size() as isize, 
                    size >> delta_g
                );
                
                while new_cursor < cursor as isize {
                    let new_acme_less_size = new_cursor & (new_cursor - 1);
                    let sub_size = new_cursor - new_acme_less_size;
                    self.add_block(new_cursor as _, self.g_of_size(sub_size as usize));
                    new_cursor += sub_size;
                }
                
                return Ok(());
            }

            if cursor as usize & sub_size != 0 {
                if self.toggle_bitflag(cursor, sub_g) {
                    self.remove_block_noflip(cursor.cast(), sub_g);
                    cursor = cursor.wrapping_add(sub_size);
                } else {
                    let allocation = if GROW {
                        // we got called by grow, so we need to move the data out of
                        // the block before we touch the memory with the add_block
                        // we don't know the aligns, but we require the caller to pass
                        // us the outputs of layout_to_size, which are repreoduced as follows

                        // if this fails, we're fucked, so it's on grow to make sure that never happens
                        // I wrote a whole sonnet about it down there
                        let allocation = self.alloc(new_layout).unwrap();
                        allocation.as_ptr().copy_from_nonoverlapping(ptr, layout_size);
                        allocation
                    } else {
                        NonNull::dangling()
                    };

                    self.add_block_noflip(buddy(cursor, sub_size).cast(), sub_g);

                    cursor = cursor.sub(sub_size);

                    loop {
                        
                        sub_g -= 1;
                        sub_size <<= 1;

                        if sub_g <= min_g { break; }
            
                        if cursor as usize & sub_size != 0 {
                            cursor = cursor.sub(sub_size);
                            self.add_block(cursor.cast(), sub_g);
                        }
                    }

                    return Err(allocation);
                }
            }

            sub_g -= 1;
            sub_size <<= 1;
        }

        self.scan_books_for_errors();

        return Ok(());
    }

    #[inline]
    unsafe fn trim_down(&mut self, base: *mut u8, size: usize, old_layout_size: usize, new_layout_size: usize) {
        if BIAS == 0 { return;}

        // trim from old_size to new_size

        let mut hi = align_ptr_up(base.wrapping_add(old_layout_size), size);
        let mut lo = align_ptr_up(base.wrapping_add(new_layout_size), size);
        if lo == hi { return; }

        let mut recombine = true;

        loop {
            // reset the LSB, dropping it down
            let hi_less_sub_size = hi as usize & (hi as usize - 1);

            if hi_less_sub_size < lo as usize {
                while lo < hi {
                    let sub_size = 1 << (lo as usize).trailing_zeros();
                    self.remove_block(lo.cast(), self.g_of_size(sub_size));
                    lo = lo.wrapping_add(sub_size);
                }

                break;
            }

            // get the size of the block of the pair we're at the base of
            let sub_size = hi as usize - hi_less_sub_size;
            hi = hi_less_sub_size as *mut u8;

            let sub_g = self.g_of_size(sub_size);

            if recombine {
                recombine = self.remove_buddy_else_add_base(hi, sub_size, sub_g);
            } else {
                self.add_block(hi.cast(), sub_g);
            }
        }

        debug_assert!(hi == lo);
    }



    unsafe fn alloc(&mut self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        // SAFETY: caller guaranteed
        let size = self.layout_to_size(layout);

        // signal OOM until either AllocError or arena_size is large enough
        // otherwise g_of_size may crash/give underflowed results
        while unlikely(size > self.size_npow2) {
            self.oom_handler.map_or(Err(AllocError), |oh: _| oh(self, layout))?;
        }
        
        let req_g = self.g_of_size(size);
        
        let ptr = 'block: {
            // try to allocate immediately if a block of the correct size is available
            if self.avails & 1 << req_g != 0 {
                break 'block self.remove_block_next(req_g);
            }

            // find free memory in a loop, avails might change under our noses
            // and OOM might occur multiple times
            let (big_block, big_g) = loop {
                let ge_avails = self.avails & !((usize::MAX-1) << req_g);

                if likely(ge_avails != 0) {
                    let g_big = utils::ilog2_nonzero(ge_avails);
                    break (self.remove_block_next(g_big), g_big);
                } else {
                    self.oom_handler.map_or(Err(AllocError), |oh: _| oh(self, layout))?;
                }
            };
    
            // 'deallocate' the high half of the 'allocated' block repeatedly
            // until only an appropriately sized block is allocated
            let mut size_hi = self.size_npow2 >> big_g+1;
            for g_hi in (big_g+1)..=req_g {
                let base_hi = big_block.wrapping_add(size_hi);
                self.add_block(base_hi.cast(), g_hi);
                size_hi >>= 1;
            }

            
            big_block
        };
        
        // trim down the block
        self.trim(ptr, size, req_g, layout.size());
        
        //eprintln!("ALLOC | ptr: {:p} size: {:x} lsize: {:x} align {:x} g: {}", ptr, size, layout.size(), layout.align(), req_g);

        Ok(NonNull::new_unchecked(ptr))
    }

    unsafe fn dealloc(&mut self, mut ptr: *mut u8, layout: Layout) {
        let mut size = self.layout_to_size(layout);
        let mut g = self.g_of_size(size);

        //eprintln!("DEALLOC | ptr: {:p} size: {:x} lsize: {:x} align {:x} g: {}", ptr, size, layout.size(), layout.align(), g);
        debug_assert!(ptr as usize & (size-1) == 0);

        if self.dealloc_trimmed::<false, false, false>(ptr, size, g, layout.size(), Layout::new::<()>()).is_ok() {
            while self.remove_buddy_else_add_base(ptr, size, g) {
                ptr = pair(ptr, size);
                size <<= 1;
                g -= 1;
            }
        }
    }

    // old size must be <= new_size otherwise grow
    // old align must be <= new align otherwise grow?
    unsafe fn shrink(&mut self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) {
        let ptr = ptr.as_ptr();
        let old_size = self.layout_to_size(old_layout);
        let new_size = self.layout_to_size(new_layout);

        //eprintln!("SHRINK | ptr: {:p} size: {:x} lsize: {:x} size: {:x} lsize: {:x}, align {:x}", ptr, old_size, old_layout.size(), new_size, new_layout.size(), new_layout.align());

        if old_size == new_size {
            // increase the trim of the block as necessary
            self.trim_down(
                ptr, 
                new_size,
                old_layout.size(), 
                new_layout.size(),
            );
        } else {
            let old_g = self.g_of_size(old_size);
            let new_g = self.g_of_size(new_size);

            // deallocate (release) the high half of the old block -
            // doesn't occur when BIAS is zero or dealloc_trimmed successfully frees the sub blocks
            let dont_free_top = (BIAS != 0 && self.dealloc_trimmed::<true, false, false>(
                ptr, old_size, old_g, old_layout.size(), Layout::new::<()>()
            ).is_err()) as usize;

            // break up the block until the required size is reached
            // release high-halves while not overlapping new_layout
            // re-combining is not possible when shrinking
            // this procedure is identical to that in alloc
            let mut hi_size = old_size >> 1 + dont_free_top;
            for hi_g in (old_g + 1 + dont_free_top)..=new_g {
                let hi_base = ptr.wrapping_add(hi_size);
                self.add_block(hi_base.cast(), hi_g);
                hi_size >>= 1;
            }
    
            // trim down the new block
            self.trim(ptr, new_size, new_g, new_layout.size());
        }
    }

    // grow tends to avoid reallocation only about 1/30 of the time or so, hardly worth the complexity
    /// Grow the block of memory provided.
    /// 
    /// Allocations are guaranteed to be a power of two in size, *align-sized*,
    /// not smaller than `new_layout.size()`.
    /// 
    /// Returns `Err` upon memory exhaustion.
    /// ### Safety:
    /// * `ptr` must have been previously acquired, given `old_layout`.
    /// * `new_layout`'s required size must be greater or equal to `old_layout`'s.
    pub unsafe fn grow(
        &mut self, 
        ptr: NonNull<u8>, 
        old_layout: Layout, 
        new_layout: Layout
    ) -> Result<NonNull<u8>, AllocError> {

        // SAFETY: caller guaranteed
        let new_size = self.layout_to_size(new_layout);

        // check if the pointer is sufficiently aligned, otherwise bail;
        // we can never grow in-place to a better-aligned region.
        // why new_size and not new_layout.align()? because all blocks must be size-aligned
        if ptr.as_ptr() as usize & (new_size - 1) != 0 {
            let allocation = self.alloc(new_layout)?;
            allocation.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), old_layout.size());
            self.dealloc(ptr.as_ptr(), old_layout);
            return Ok(allocation);
        }

        // make sure the arena is big enough for the granularity values to be valid
        while unlikely(new_size > self.size_npow2) { 
            self.oom_handler.map_or(Err(AllocError), |oh: _| oh(self, new_layout))?;
        }

        let mut new_g = self.g_of_size(new_size);

        // this function relies on being able to go through the deallocation
        // process. If deallocation ceases before we have a big enough region to
        // fit new_size, we need to be able to allocate a new block before continuing,
        // otherwise we get caught with our pants down.
        // We would need to roll back the entire process, then try to reallocate 
        // which would fail anyway, as the blocks claimed to grow the block won't
        // infringe on the ability to allocate a new block of sufficient size.
        // ((why? because blocks are size-aligned and the data in this block is 
        // occupying whatever bigger block this data is in, and growth isn't going to 
        // extend beyond bigger block))
        while self.avails & !((usize::MAX-1) << new_g) == 0 {
            self.oom_handler.map_or(Err(AllocError), |oh: _| oh(self, new_layout))?;
            new_g = self.g_of_size(new_size);
        }

        let old_size = self.layout_to_size(old_layout);
        let old_g = self.g_of_size(old_size);
        //eprintln!("GROW | ptr: {:p} size: {:x} lsize: {:x} size: {:x} lsize: {:x} align {:x}", ptr, old_size, old_layout.size(), new_size, new_layout.size(), old_layout.align());

        
        if old_size == new_size {
            // we need to GROW TRIMUP variant, where we retrim from the old
            // trim to the new trim. dealloc_trimmed will return the allocation
            // it makes if it isn't able to reclaim enough memory for in-place grow
            return if let Err(allocation) = self.dealloc_trimmed::<false, true, true>(
                ptr.as_ptr(), 
                old_size, 
                old_g, 
                old_layout.size(), 
                new_layout,
            ) {
                Ok(allocation)
            } else {
                Ok(ptr)
            }
        }
        
        // Check high buddies recursively, if available, reserve them, else realloc.
        // This satisfies the requirement on Allocator::grow that the memory
        // must not be modified or reclaimed if Err is returned.

        if let Err(allocation) = self.dealloc_trimmed::<false, true, false>(
            ptr.as_ptr(), old_size, old_g, old_layout.size(), new_layout
        ) {
            Ok(allocation)
        } else {
            let mut base = ptr.as_ptr();
            let mut size = old_size;
            let mut g = old_g;

            while self.toggle_bitflag(base, g) {
                self.remove_block_noflip(buddy(base, size).cast(), g);
                
                base = pair(base, size);
                size <<= 1;
                g -= 1;

                if g == new_g {
                    self.trim(ptr.as_ptr(), size, g, new_layout.size());
                    return Ok(ptr);
                }
            }

            // reallocate before we touch the memory with add_block
            let allocation = self.alloc(new_layout);

            if let Ok(nn) = allocation {
                nn.as_ptr().copy_from_nonoverlapping(ptr.as_ptr(), old_layout.size());
            }
            
            self.add_block_noflip(base.cast(), g);

            allocation
        }
    }


}



/// A Talloc Out-Of-Memory handler header.
/// 
/// This gives the user an opportunity to recover from a failed allocation by releasing more memory.
/// 
/// Note that Talloc isn't in an invalid state. 
/// `extend` and `release` can be used as per usual to make room for the new allocation.
/// 
/// Returning `Err(AllocError)` will cause the allocation to fail. 
/// `Ok(())` will result in another attempt.
/// 
/// Recovering may involve the following steps:
/// - `talloc.get_arena()` to fetch the current arena.
/// - [`extend`] (extend) the arena to get a `new_arena`.
/// - `talloc.get_meta_mem()` to fetch the old metadata memory.
/// - `talloc.req_meta_mem(new_arena)` to get the new metadata memory requirement.
/// - `talloc.extend(new_arena, mem_mode)` to extend the allocator's arena.
/// - `talloc.release(mem)` to release some memory for allocation.
/// 
/// See the example in the README of this project. TODO LINK
/// 
/// Use `talloc::alloc_error` if you don't wish to implement customized error handling.
pub type OomHandler<const BIAS: usize> = fn(&mut TallocCore<BIAS>, Layout) -> Result<(), AllocError>;


/// ## Talloc, the TauOS Allocator
/// 
/// ### Features:
/// * Low time complexity and maximizing performance at the cost of memory usage
/// * Minimization of external fragmentation at the cost of internal fragmentation
/// * Arena can wrap around the address space
/// 
/// ### Allocator design:
/// * **O(log n)** worst case allocation and deallocation performance.
/// * **O(2^n)** amortized memory usage, at most `arena size / 64 + k`.
/// * **buddy allocation** + **linked free-lists** + **bitmap**
/// 
/// Note that the extra slices can be stored within the arena, 
/// as long as they remain reserved.
/// 
/// ### Allocator usage:
/// 
/// ```rust
/// const MIN_SIZE: usize = ...;
/// 
/// #[global_allocator]
/// static ALLOCATOR: Tallock<{talloc::SPEED_BIAS}> = 
///     talloc::Talloc::<{talloc::SPEED_BIAS}>::new_empty(MIN_SIZE, talloc::alloc_error)
///     .wrap_spin_lock();
/// 
/// // initialize it later...
/// let arena = talloc::Span::from(0x0..0x100000);
/// unsafe { ALLOCATOR.lock().extend(arena, MemMode::Automatic, talloc::alloc_error); }
/// ```
/// 
/// Use it as an arena allocator via the `Allocator` API like so:
/// ```rust
/// let min_block_size = ...;
/// let arena = Box::leak(vec![0u8; SIZE].into_boxed_slice());
/// 
/// let tallock = Talloc::<{talloc::SPEED_BIAS}>::new_arena(arena, min_block_size)
///     .wrap_spin_lock();
/// 
/// tallock.allocate(...);
/// ```
/// 
/// The `Talloc::new`, `Talloc::extend`, and `Talloc::release` functions 
/// give plenty of flexibility for more niche applications.
pub struct Talloc<const BIAS: usize> {
    // todo figure out if this should be pub
    pub core: Mutex<TallocCore<BIAS>>,
}

unsafe impl<const BIAS: usize> Send for Talloc<BIAS> {}
unsafe impl<const BIAS: usize> Sync for Talloc<BIAS> {}

impl<const BIAS: usize> Talloc<BIAS> {
    const fn clamp_min_size(min_size: usize) -> usize {
        if min_size > MIN_MIN_SIZE {
            min_size.next_power_of_two()
        } else {
            MIN_MIN_SIZE
        }
    }

    /// Returns a new Talloc with no arena. Allocations will signal OOM.
    /// 
    /// Use `extend` to establish the arena and `release` to free up memory for allocation.
    /// 
    /// Alternatively, use `new_arena`.
    /// 
    /// ### Arguments:
    /// * `min_size` determines the minimum block size used for allocation. 
    ///     * It will be clamped to above `MIN_MIN_SIZE` and rounded up to the next power of two.
    /// * `oom_handler` is called when the allocator is short on memory. 
    /// See `OomHandler` for more details.
    pub const fn new(min_size: usize) -> Self {

        // given all constructors call this function, min_size is always valid unless modified
        let min_size = Self::clamp_min_size(min_size);

        Self {
            core: Mutex::new(TallocCore {
                oom_handler: None,

                min_size,
    
                arena: Span::empty(),
                size_npow2: 0,
                l2_sz_2np2: 0,
                lzcnt_sz_np2: 0,
                avails: 0,
                lflags: 0,
                llists: ptr::slice_from_raw_parts_mut(ptr::null_mut(), 0),
                bitmap: ptr::slice_from_raw_parts_mut(ptr::null_mut(), 0),
            })
        }
    }

    /// Create a new Talloc for allocating memory in `arena`.
    /// 
    /// Metadata is automatically stored in `arena`.
    pub fn new_arena(arena: &mut [u8], min_size: usize) -> Talloc<BIAS> {
        let talloc = Self::new(min_size);
        unsafe { let _ = talloc.core.lock().extend(arena.into(), MemMode::Automatic); }
        talloc
    }
}

unsafe impl<const BIAS: usize> GlobalAlloc for Talloc<BIAS> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.core.lock().alloc(layout)
            .map_or(ptr::null_mut(), |nn| nn.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.core.lock().dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        let new_layout = Layout::from_size_align_unchecked(new_size, old_layout.align());

        if new_layout.size() > old_layout.size() {
            self.core.lock().grow(NonNull::new_unchecked(ptr), old_layout, new_layout)
                .map_or(ptr::null_mut(), |nn| nn.as_ptr())
        } else {
            self.core.lock().shrink(NonNull::new_unchecked(ptr), old_layout, new_layout);
            ptr
        }
    }
}



#[cfg(test)]
mod tests {

    use std;

    use super::*;


    #[test]
    fn it_works() {
        const ARENA_SIZE: usize = 100000000;
        const SMALL_SIZE: usize = 1 << 6;

        let arena = vec![0u8; ARENA_SIZE].into_boxed_slice();
        let arena = Box::leak(arena);

        
        let talloc = Talloc::<SPEED_BIAS>::new_arena(arena.into(), SMALL_SIZE);

        let layout = Layout::from_size_align(1243, 8).unwrap();
 
        let a = unsafe { talloc.alloc(layout) };
        assert!(!a.is_null());
        unsafe { a.write_bytes(255, layout.size()); }

        let mut x =  vec![ptr::null_mut(); 1000];

        let t1 = std::time::Instant::now();
        for _ in 0..1000 {
            for i in 0..1000 {
                let allocation = unsafe { talloc.alloc(layout) };
                x[i] = allocation;
            }

            for i in (0..1000).rev() {
                unsafe { talloc.dealloc(x[i], layout); }
            }
        }
        let t2 = std::time::Instant::now();
        //e/println!("duration: {:?}", (t2 - t1) / (1000 * 2000));

        unsafe {
            talloc.dealloc(a, layout);
        }
    }
}

