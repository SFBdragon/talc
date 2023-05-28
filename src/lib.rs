#![cfg_attr(not(test), no_std)]

#![feature(alloc_layout_extra)]

#![feature(slice_ptr_get)]
#![feature(slice_ptr_len)]
#![feature(const_mut_refs)]
#![feature(const_slice_ptr_len)]
#![feature(const_slice_from_raw_parts_mut)]

#![feature(core_intrinsics)]
#![feature(const_assume)]

#![cfg_attr(feature = "allocator", feature(allocator_api))]

mod span;
mod utils;
#[allow(dead_code)]
mod llist;
#[cfg(feature = "spin")]
mod tallock;


//pub use utils::copy_slice_bits; // for fuzzing
#[cfg(feature = "spin")]
pub use tallock::Tallock;
pub use span::Span;
use llist::LlistNode;

use core::{
    ptr::{self, NonNull},
    alloc::Layout,
    intrinsics::{unlikely, assume}, 
};


#[cfg(feature = "allocator")]
pub use core::alloc::AllocError;

#[cfg(not(feature = "allocator"))]
pub struct AllocError();



const NULL: isize = 0;
const ZERO_PAGE: isize = 1 << 12;
const NODE_SIZE: usize = core::mem::size_of::<LlistNode<()>>();
const NODE_ALIGN: usize = core::mem::align_of::<LlistNode<()>>();

/// The minimum value of `min_size`. Lower values are clamped to this value.
pub const MIN_MIN_SIZE: usize = NODE_SIZE.next_power_of_two();
/// The minimum arena size eligible for `extend` using`MemMode::Automatic`.
pub const MIN_ARENA_SIZE: usize = 1 << 6;


pub const MAX_SPEED_BIAS: usize = 0;
pub const SPEED_BIAS: usize = 2;
pub const EFFICIENCY_BIAS: usize = 3;


/// Simple `OomHandler` function that immediately returns `Err(AllocError)`.
pub const fn alloc_error<const BIAS: usize>(_: &mut Talloc<BIAS>, _: Layout) -> Result<(), AllocError> {
    Err(AllocError)
}


/// Returns whether the block of the given base is the lower of its buddy pair.
#[inline]
fn is_lower_buddy(base: *mut u8, size: usize) -> bool {
    base as usize & size == 0
}

#[inline]
const fn align_up(base: isize, align: usize) -> isize {
    debug_assert!(align.count_ones() == 1);

    let min_size_m1 = align - 1;
    ((base as usize).wrapping_add(min_size_m1) & !min_size_m1) as isize
}
#[inline]
const fn align_down(base: isize, align: usize) -> isize {
    debug_assert!(align.count_ones() == 1);

    (base as usize & !(align - 1)) as isize
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
        /// Must be in accordance with `req_meta_mem`.
        metadata_memory: Option<*mut u8> 
    },
}

impl MemMode {
    #[inline]
    pub const fn auto_release(self) -> bool {
        matches!(self, MemMode::Automatic | MemMode::Manual { auto_release: true, metadata_memory: _ })
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
/// - Call `talloc.get_arena()` to fetch the current arena.
/// - Call `extend` on the arena to get a `new_arena`.
/// - Call `talloc.get_meta_mem()` to fetch the old metadata memory.
/// - Call `talloc.req_meta_mem(new_arena)` to get the new metadata memory requirement.
/// - Call `talloc.extend(new_arena, mem_mode)` to extend the allocator's arena.
/// - Call `talloc.release(mem)` to release some memory for allocation.
/// 
/// See the example in the README of this project. TODO LINK
/// 
/// Use `talloc::alloc_error` if you don't wish to implement customized error handling.
pub type OomHandler<const BIAS: usize> = fn(&mut Talloc<BIAS>, Layout) -> Result<(), AllocError>;


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
    /// The power-of-two size of the smallest allocatable block in bytes.
    min_size: usize,

    oom_handler: OomHandler<BIAS>,

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
    llists: *mut [LlistNode<()>],


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

unsafe impl<const BIAS: usize> Send for Talloc<BIAS> {}
unsafe impl<const BIAS: usize> Sync for Talloc<BIAS> {}

impl<const BIAS: usize> core::fmt::Debug for Talloc<BIAS> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Talloc")
        .field("arena", &format_args!("{:?}", self.arena))
        .field("size_npow2", &format_args!("{:#x}", self.size_npow2))
        .field("min_size", &format_args!("{:#x}", self.min_size))
        .field("avails", &format_args!("{:#b}", self.avails))
        .field("llists", &format_args!("{:?}", self.llists))
        .field("bitmap", &format_args!("{:?}", self.bitmap))
        .field("oom_handler", &format_args!("{:#p}", self.oom_handler as *mut u8))
        .finish()
    }
}

impl<const BIAS: usize> Talloc<BIAS> {
    /// Returns the corresponding granularity for a given block size.
    /// 
    /// `size` should not be larger than `self.arena_size_pow2`.
    /// 
    /// `size` must be nonzero, otherwise UB occurs.
    #[inline]
    const unsafe fn g_of_size(&self, size: usize) -> usize {
        // this prevents a bunch of extra instructions being emmitted when
        // lzcnt isn't available
        assume(size != 0);

        debug_assert!(size <= self.size_npow2);
        
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

    /// Utility function to read the bitmap at the offset in bits.
    /// 
    /// `base` is expected to be aligned to `g`'s corresponding block size.
    #[inline]
    fn read_bitflag(&self, base: *mut u8, g: usize) -> bool {
        if let Some(bmp_idx) = self.bmp_idx(base, g) {
            let index = bmp_idx >> u8::BITS.trailing_zeros();
            // SAFETY: bmp_idx should be valid (as checked in debug builds)
            let data = unsafe { *self.bitmap.get_unchecked_mut(index) };
            let bit_mask = 1 << (bmp_idx & u8::BITS as usize - 1);

            data & bit_mask != 0
        } else {
            self.lflags & (1 << g) != 0
        }
    }

    /// Utility function to toggle the bitmap at the offset in bits.
    /// 
    /// `base` is expected to be aligned to `g`'s corresponding block size.
    #[inline]
    fn toggle_bitflag(&mut self, base: *mut u8, g: usize) {
        if let Some(bmp_idx) = self.bmp_idx(base, g) {
            let index = bmp_idx >> u8::BITS.trailing_zeros();
            // SAFETY: bmp_idx should be valid (as checked in debug builds)
            let data = unsafe { self.bitmap.get_unchecked_mut(index) };
            let bit_mask = 1 << (bmp_idx & u8::BITS as usize - 1);

            unsafe { *data ^= bit_mask; }
        } else {
            self.lflags ^= 1 << g;
        }
    }

    /// Registers a block into the books, making it available for allocation.
    /// 
    /// `node` is expected to be aligned to `g`'s corresponding block size.
    /// 
    /// ### SAFETY:
    /// The block at `node` of `g` must be allocatable and not allocated.
    #[inline]
    unsafe fn add_block_next(&mut self, g: usize, node: *mut LlistNode<()>) {
        debug_assert!(g < self.llists.len());
        debug_assert!(node as usize % (self.size_npow2 >> g) == 0);

        // populating llist
        self.avails |= 1 << g;

        // add node to llist
        // SAFETY: caller guaranteed and by the above assertions
        let sentinel = self.llists.get_unchecked_mut(g);
        LlistNode::new(node, sentinel, (*sentinel).next.get(), ());

        // toggle bitmap flag, if it exists
        // SAFETY: guaranteed by caller
        self.toggle_bitflag(node.cast(), g);
    }

    /// Unregisters the next block in the free list, reserving it against 
    /// allocation, and returning the base.
    /// ### Safety:
    /// * `llists[g]` must have a nonsentinel element: 
    /// `avails` at bit `g` should be `1`.
    /// * `size` must agree with `g`'s corresponding block size.
    #[inline]
    unsafe fn remove_block_next(&mut self, g: usize) -> *mut u8 {
        let sentinel = self.llists.get_unchecked_mut(g);
        
        if (*sentinel).prev.get() == (*sentinel).next.get() {
            // last nonsentinel block in llist, toggle off avails flag
            self.avails &= !(1 << g);
        }
        
        // remove node from llist
        // SAFETY: caller guaranteed
        let node = (*sentinel).next.get();
        LlistNode::remove(node);

        // toggle bitmap flag, if it exists
        // SAFETY: caller guaranteed
        self.toggle_bitflag(node.cast(), g);

        node.cast()
    }

    /// Unregisters a known block from the free list, reserving it against allocation.
    /// 
    /// ### Safety:
    /// `node` must be a valid `LlistNode` at an allocatable, but unallocated block of `g`.
    #[inline]
    unsafe fn remove_block(&mut self, g: usize, node: *mut LlistNode<()>) {
        debug_assert!(g < self.llists.len());
        debug_assert!(node as usize % (self.size_npow2 >> g) == 0);

        if (*node).prev == (*node).next {
            // last nonsentinel block in llist, toggle off avails flag
            self.avails &= !(1 << g);
        }

        // remove node from llist
        // SAFETY: caller guaranteed
        LlistNode::remove(node);

        // toggle bitmap flag, if it exists
        // SAFETY: caller guaranteed
        self.toggle_bitflag(node.cast(), g);
    }



    const fn clamp_min_size(min_size: usize) -> usize {
        if min_size > MIN_MIN_SIZE {
            min_size.next_power_of_two()
        } else {
            MIN_MIN_SIZE
        }
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
    pub const fn new(min_size: usize, oom_handler: OomHandler<BIAS>) -> Self {

        // given all constructors call this function, min_size is always valid unless modified
        let min_size = Self::clamp_min_size(min_size);

        Self {
            min_size,
            oom_handler,

            arena: Span::empty(),
            size_npow2: 0,
            l2_sz_2np2: 0,
            lzcnt_sz_np2: 0,
            avails: 0,
            lflags: 0,
            llists: ptr::slice_from_raw_parts_mut(ptr::null_mut(), 0),
            bitmap: ptr::slice_from_raw_parts_mut(ptr::null_mut(), 0),
        }
    }

    /// Create a new Talloc for allocating memory in `arena`.
    /// 
    /// Metadata is automatically stored in `arena`.
    pub fn new_arena(arena: &mut [u8], min_size: usize) -> Talloc<BIAS> {
        unsafe {
            let mut talloc = Self::new(min_size, alloc_error);

            let _ = talloc.extend(arena.into(), MemMode::Automatic);
            
            talloc
        }
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
            let mut talloc = Talloc {
                min_size: self.min_size,
                oom_handler: self.oom_handler,
  
                arena: new_arena,
                size_npow2: new_size_npow2,
                l2_sz_2np2: new_size_npow2.ilog2() as usize + 1,
                lzcnt_sz_np2: new_size_npow2.leading_zeros(),
                llists: ptr::slice_from_raw_parts_mut(
                    meta_ptr.cast(), 
                    ll_bytes / NODE_SIZE
                ),
                bitmap: ptr::slice_from_raw_parts_mut(
                    meta_ptr.wrapping_add(ll_bytes), 
                    bm_bytes
                ),

                // initialized shortly
                avails: 0,
                lflags: 0,
            };
            
            let delta_g = talloc.llists.len() - self.llists.len();
    
            self.scan_llists_for_errors();

            // copy/init llists
            for i in 0..talloc.llists.len() {

                if i < delta_g {
                    LlistNode::new_llist(talloc.llists.get_unchecked_mut(i), ());
                } else {
                    LlistNode::mov(
                        self.llists.get_unchecked_mut(i - delta_g),
                        talloc.llists.get_unchecked_mut(i),
                    );
                }
            }

            talloc.scan_llists_for_errors();
    
            // set avails
            talloc.avails = self.avails << delta_g;
            
            // init/copy bitmap
            talloc.bitmap.as_mut_ptr().write_bytes(0, talloc.bitmap.len());
            if self.bitmap.len() != 0 {
                for old_g in 0..self.llists.len() {
                    let new_g = old_g + delta_g;
                    let size = self.size_npow2 >> old_g;
                    
                    // the pair up might not even be in the arena, in which case it's totes blank~
                    let base_pair = align_up(self.arena.base, size << 1);
                    if base_pair < self.arena.acme {
                        let old_bmp_field = 1 << old_g >> 1;

                        // if this panics, there's a serious bug in bmp_idx or base_pair or something
                        let new_bmp_idx = talloc.bmp_idx(base_pair as *mut _, new_g).unwrap();
        
                        utils::copy_slice_bits(
                            talloc.bitmap,
                            self.bitmap,
                            new_bmp_idx,
                            old_bmp_field,
                            old_bmp_field.max(1),
                        );
                        // todo optimize the above using word-size iteration where possible?
                    }
                    
                    if self.lflags & 1 << old_g != 0 {
                        debug_assert!(align_up(self.arena.base, size) as usize & size != 0);
                        talloc.toggle_bitflag(align_ptr_up(self.arena.base_ptr(), size), new_g);
                    }
                }
            }

            talloc.scan_llists_for_errors();
    
            let old_ctrl_size = self.llists.len() * NODE_SIZE + self.bitmap.len();
            let old_ctrl_mem = Span::from_ptr_size(self.llists.cast(), old_ctrl_size);
            let contained_ctrl_mem = old_ctrl_mem.align_outward(self.min_size).clamp(self.arena);
            talloc.release(contained_ctrl_mem);

            talloc.scan_llists_for_errors();

            if mem_mode.auto_release() {
                talloc.release(talloc.arena.below(self.arena.base).below(meta_mem.base));
                talloc.release(talloc.arena.below(self.arena.base).above(meta_mem.acme));
                talloc.release(talloc.arena.above(self.arena.acme).below(meta_mem.base));
                talloc.release(talloc.arena.above(self.arena.acme).above(meta_mem.acme));
            }
    
            *self = talloc;

            Ok(())
        }
    }

    fn scan_llists_for_errors(&self) {
        #[cfg(debug_assertions)]
        for i in 0..self.llists.len() {
            unsafe {
                let sentinel = self.llists.get_unchecked_mut(i);
                assert!(sentinel as usize > 0x1000);
                assert!(self.arena.contains_ptr((*sentinel).next.get().cast()));
                assert!(self.arena.contains_ptr((*sentinel).prev.get().cast()));

                for node in LlistNode::iter_mut(sentinel) {
                    assert!(self.arena.contains_ptr((*node).next.get().cast()), 
                        "node: {:p} next: {:p} prev: {:p}", node, (*node).next.get(), (*node).prev.get());
                    assert!(self.arena.contains_ptr((*node).prev.get().cast()), 
                        "node: {:p} next: {:p} prev: {:p}", node, (*node).next.get(), (*node).prev.get());
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
        let span = span.clamp(self.arena).align_inward(self.min_size);
        
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
                NonNull::new_unchecked(block_base as *mut u8), 
                Layout::from_size_align_unchecked(block_size, 1)
            );

            self.scan_llists_for_errors();
            
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

    /// Allocate memory. 
    /// 
    /// Allocations are guaranteed to be a power of two in size, *size-aligned*,
    /// not smaller than `layout.size()`.
    /// 
    /// Returns `Err` upon memory exhaustion, when the out-of-memory handler errors out.
    /// 
    /// ### Safety:
    /// * `layout.size()` must be nonzero.
    pub unsafe fn alloc(&mut self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        // SAFETY: caller guaranteed
        let block_size = self.layout_to_size(layout);

        // signal OOM until either AllocError or arena_size is large enough
        // otherwise g_of_size may crash
        while unlikely(block_size > self.size_npow2) { (self.oom_handler)(self, layout)?; }
        
        let mut block_g = self.g_of_size(block_size);

        let block_base = 'block: {
            // find free memory in a loop, OOM might occur multiple times
            let avails_big = loop {
                // allocate immediately if a block of the correct size is available
                if self.avails & 1 << block_g != 0 {
                    break 'block self.remove_block_next(block_g);
                }
                
                // find a larger block (smaller granularity) to break apart:
                // mask out the granularity availabilities of the too-small blocks
                let avails_big = self.avails & !(usize::MAX << block_g);
    
                if unlikely(avails_big == 0) {
                    // signal OOM until either AllocError or arena has the sufficient memory
                    (self.oom_handler)(self, layout)?;
                    
                    // update granularity as arena_size might have been updated
                    block_g = self.g_of_size(block_size);
    
                    continue;
                } else {
                    break avails_big;
                }
            };
    
            let g_big = utils::ilog2_nonzero(avails_big);
            let size_big = self.size_npow2 >> g_big;
    
            // 'allocate' the big block
            let base_big = self.remove_block_next(g_big);
    
            // 'deallocate' the high half of the 'allocated' block repeatedly
            // until only an appropriately sized block is allocated
            let mut size_hi = size_big >> 1;
            for g_hi in (g_big + 1)..=block_g {
                // SAFETY: given the big block was allocatable, it doesn't 
                // contain any undereferencable memory
                let base_hi = base_big.wrapping_add(size_hi);
    
                self.add_block_next(g_hi, base_hi.cast());
    
                size_hi >>= 1;
            }

            base_big
        };

        self.trim(block_base, block_size, block_g, layout.size());

        // SAFETY: the null block should never be marked allocatable
        Ok(NonNull::new_unchecked(block_base))
    }

    pub unsafe fn dealloc(&mut self, ptr: NonNull<u8>, layout: Layout) {
        let mut ptr = ptr.as_ptr();
        // SAFETY: caller-guaranteed, by requisite of alloc, as required by dealloc

        let mut size = self.layout_to_size(layout);
        let mut g = self.g_of_size(size);

        if !self.dealloc_trimmed(ptr, size, g, layout.size(), false) {
            return;
        }

        // while buddy was heterogenous - available
        while self.read_bitflag(ptr, g) {
            let (buddy_ptr, next_ptr) = if is_lower_buddy(ptr, size) {
                (ptr.wrapping_add(size), ptr)
            } else {
                (ptr.wrapping_sub(size), ptr.wrapping_sub(size))
            };
            
            // SAFETY: buddy has been confirmed to exist here, LlistNodes are not moved
            self.remove_block(g, buddy_ptr.cast());
            
            ptr = next_ptr;
            size <<= 1;
            g -= 1;
        }
        
        self.add_block_next(g, ptr.cast());
    }

    /// Shrink the block of allocated memory in-place.
    /// ### Safety:
    /// * `new_layout.size()` must be non-zero.
    /// * `ptr` must have been previously allocated, given `old_layout`.
    /// * `old_layout`'s must be smaller or equal to `new_layout`'s required size and align.
    pub unsafe fn shrink(&mut self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) {
        debug_assert!(new_layout.size() <= old_layout.size());

        // SAFETY: caller guaranteed
        let old_size = self.layout_to_size(old_layout);
        let new_size = self.layout_to_size(new_layout);
        
        let old_g = self.g_of_size(old_size);
        let new_g = self.g_of_size(new_size);

        if old_size == new_size {
            // increase the trim of the block as necessary
            self.trim_more(
                ptr.as_ptr(), 
                new_size, 
                new_g, 
                old_layout.size(), 
                new_layout.size()
            );

            return;
        }

        // deallocate the high half of the old block
        self.dealloc_trimmed(ptr.as_ptr(), old_size, old_g, old_layout.size(), true);

        // break up the block until the required size is reached
        // release high-halves while not overlapping new_layout
        // re-combining is not possible when shrinking
        // this procedure is identical to that in alloc
        let mut hi_size = old_size >> 1 + 1.min(BIAS);
        for hi_g in (old_g + 1 + 1.min(BIAS))..=new_g {
            let hi_base = ptr.as_ptr().wrapping_add(hi_size);
            self.add_block_next(hi_g, hi_base.cast());
            hi_size >>= 1;
        }

        // trim down the new block
        self.trim(ptr.as_ptr(), new_size, new_g, new_layout.size());
    }


    // TODO: the trim functions need internal documentation
    // for now, we just try to subdivide the allocated block and free some of the 
    // high blocks in a deterministic way, and reverse that process for deallocation
    // this is what BIAS actually does, it controls how finely we try to divvy up allocated blocks

    #[inline]
    unsafe fn trim(&mut self, base: *mut u8, mut size: usize, mut g: usize, layout_size: usize) {
        if BIAS == 0 { return; }

        let sub_g = BIAS.min(self.llists.len() - g - 1);
        size >>= sub_g;
        g += sub_g;

        let mut acme = align_ptr_up(base.wrapping_add(layout_size), size);
        
        for _ in 0..sub_g {
            if acme as usize & size != 0 {
                self.add_block_next(g, acme.cast());
                acme = acme.wrapping_add(size);
            }

            size <<= 1;
            g -= 1;
        }
    }

    #[inline]
    unsafe fn dealloc_trimmed(&mut self, base: *mut u8, mut size: usize, mut g: usize, layout_size: usize, half: bool) -> bool {
        if BIAS == 0 { return true;}

        let sub_g = BIAS.min(self.llists.len() - g - 1);
        size >>= sub_g;
        g += sub_g;

        let mut acme = align_ptr_up(base.wrapping_add(layout_size), size);
        let mut seeker = acme;
        let mut recombine = true;
        
        for _ in 0..(sub_g - half as usize) {
            if acme as usize & size != 0 {
                acme = acme.wrapping_sub(size);

                if !recombine {
                    self.add_block_next(g, acme.cast());
                }
            }

            if recombine && seeker as usize & size != 0 {
                if self.read_bitflag(seeker, g) {
                    self.remove_block(g, seeker.cast());
                    seeker = seeker.wrapping_add(size);
                } else {
                    recombine = false;
                    self.add_block_next(g, acme.cast());
                }
            }


            size <<= 1;
            g -= 1;
        }

        recombine
    }

    #[inline]
    unsafe fn trim_more(&mut self, base: *mut u8, mut size: usize, mut g: usize, old_layout_size: usize, new_layout_size: usize) {
        if BIAS == 0 { return;}

        // trim from old_size to new_size

        let sub_g = BIAS.min(self.llists.len() - g - 1);
        size >>= sub_g;
        g += sub_g;

        let mut old_acme = align_ptr_up(base.wrapping_add(old_layout_size), size);
        let     new_acme = align_ptr_up(base.wrapping_add(new_layout_size), size);
        let mut seeker = old_acme;
        let mut recombine = true;
        
        for _ in 0..sub_g {
            if old_acme as usize & size != 0 {
                let t_acme = old_acme.wrapping_sub(size);

                if t_acme < new_acme {
                    // puke
                    while old_acme > new_acme {
                        let s = utils::prev_pow2_nonzero((old_acme as isize - new_acme as isize) as usize);
                        old_acme = old_acme.wrapping_sub(s);
                        self.add_block_next(self.g_of_size(s), old_acme.cast())
                    }
                    break;
                }

                old_acme = t_acme;

                if !recombine {
                    self.add_block_next(g, old_acme.cast());
                }
            }

            if recombine && seeker as usize & size != 0 {
                if self.read_bitflag(seeker, g) {
                    self.remove_block(g, seeker.cast());
                    seeker = seeker.wrapping_add(size);
                } else {
                    recombine = false;
                    self.add_block_next(g, old_acme.cast());
                }
            }

            size <<= 1;
            g -= 1;
        }
    }
}


#[cfg(test)]
mod tests {

    use core::alloc::Allocator;
    use std;

    use super::*;


    #[test]
    fn it_works() {
        const ARENA_SIZE: usize = 100000000;
        const SMALL_SIZE: usize = 1 << 6;

        let arena = vec![0u8; ARENA_SIZE].into_boxed_slice();
        let arena = Box::leak(arena);

        eprintln!("{}", arena.len());

        
        let talloc = Talloc::<SPEED_BIAS>::new_arena(arena.into(), SMALL_SIZE).wrap_spin_lock();

        let layout = Layout::from_size_align(1243, 8).unwrap();
 
        let a = talloc.allocate(layout).unwrap();

        let mut x =  vec![NonNull::dangling(); 1000];

        let t1 = std::time::Instant::now();
        for _ in 0..1000 {
            for i in 0..1000 {
                x[i] = talloc.allocate(layout).unwrap().as_non_null_ptr();
            }

            for i in (0..1000).rev() {
                unsafe { talloc.deallocate(x[i], layout); }
            }
        }
        let t2 = std::time::Instant::now();
        eprintln!("duration: {:?}", (t2 - t1) / (1000 * 2000));

        unsafe {
            a.as_mut_ptr().write_bytes(255, a.len());
            talloc.deallocate(a.cast(), layout);
        }
    }
}



    // grow tends to avoid reallocation only about 1/30 of the time or so, hardly worth the complexity
    /* /// Grow the block of memory provided.
    /// 
    /// Allocations are guaranteed to be a power of two in size, *align-sized*,
    /// not smaller than `new_layout.size()`.
    /// 
    /// Returns `Err` upon memory exhaustion.
    /// ### Safety:
    /// * `ptr` must have been previously acquired, given `old_layout`.
    /// * `new_layout`'s required size must be smaller or equal to `old_layout`'s.
    pub unsafe fn grow(&mut self, ptr: NonNull<u8>, old_layout: Layout, new_layout: Layout) -> Result<NonNull<u8>, AllocError> {
        // SAFETY: caller guaranteed
        let old_size = self.layout_to_size(old_layout);
        let new_size = self.layout_to_size(new_layout);
        
        if old_size == new_size { return Ok(ptr); }

        while unlikely(new_size > self.size_npow2) { (self.oom_handler)(self, new_layout)?; }

        let old_g = self.g_of_size(old_size);
        let new_g = self.g_of_size(new_size);
        
        // Check high buddies recursively, if available, reserve them, else realloc.
        // This satisfies the requirement on Allocator::grow that the memory
        // must not be modified or reclaimed if Err is returned.

        let mut size = old_size;
        let mut g = old_g;

        while g > new_g {
            // realloc is necessary:
            // * if this is a high buddy and a larger block is required
            // * if the high buddy is not available and a larger block is required
            if !is_lower_buddy(ptr.as_ptr(), size) || !self.read_bitflag(ptr.as_ptr(), g) {
                let allocation = self.alloc(new_layout);
                if let Ok(alloc_ptr) = allocation {
                    ptr::copy_nonoverlapping(
                        ptr.as_ptr(), 
                        alloc_ptr.as_ptr(), 
                        old_layout.size()
                    );
                    self.dealloc(ptr, old_layout);
                }
                return allocation;
            }
            
            size <<= 1;
            g -= 1;
        }

        // reiterate, having confirmed there is sufficient memory available
        // remove all buddy nodes as necessary
        let mut size = old_size;
        let mut g = old_g;
        while g > new_g {
            self.remove_block(
                g,
                ptr.as_ptr().wrapping_add(size).cast()
            );

            size <<= 1;
            g -= 1;
        }

        OOGA.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        Ok(ptr)
    } */

