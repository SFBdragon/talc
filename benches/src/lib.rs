use std::{
    alloc::{GlobalAlloc, Layout},
    ptr::NonNull,
};

pub const HEAP_SIZE: usize = 1 << 27;
#[repr(align(64))] // makes initializing some allocators (e.g. galloc) easier
pub struct Heap(pub [u8; HEAP_SIZE]);
pub static mut HEAP: Heap = Heap([0u8; HEAP_SIZE]);

pub fn touch_the_whole_heap() {
    for i in (0..HEAP_SIZE).step_by(1024) {
        unsafe {
            let ptr = &raw mut HEAP.0[i];
            ptr.write_volatile(0xab);
            ptr.read_volatile();
        }
    }
}

pub struct NamedAllocator {
    pub name: &'static str,
    pub init_fn: unsafe fn() -> Box<dyn GlobalAlloc + Sync>,
}

pub const ARENA_ALLOCATORS: &[NamedAllocator] = &[
    NamedAllocator { name: "DLmalloc", init_fn: init_dlmalloc },
    NamedAllocator { name: "Talc", init_fn: init_talc },
    NamedAllocator { name: "Talc v4", init_fn: init_talc_old },
    NamedAllocator { name: "RLSF", init_fn: init_rlsf },
    NamedAllocator { name: "Galloc", init_fn: init_galloc },
    NamedAllocator { name: "Buddy Alloc", init_fn: init_buddy_alloc },
    // NamedAllocator { name: "Linked List", init_fn: init_linked_list_allocator },
];

pub const SYSTEM_ALLOCATORS: &[NamedAllocator] = &[
    NamedAllocator { name: "DLmalloc", init_fn: init_dlmalloc_sys },
    NamedAllocator { name: "Talc", init_fn: init_talc_sys },
    NamedAllocator { name: "FRuSA", init_fn: init_frusa_sys },
    NamedAllocator { name: "mimalloc", init_fn: init_mimalloc_sys },
    NamedAllocator { name: "System", init_fn: init_system },
    NamedAllocator { name: "Jemalloc", init_fn: init_jemalloc_sys },
];

/// Bias towards smaller values over larger ones.
///
/// Why not a linear distribution:
/// This concentrates most of the generated sizes around 2-3 neighboring
/// binary orders of magnitude, near the maximum.
///
/// Speculation:
/// Programs often allocate more smaller sizes than larger sizes.
///
/// Possible objection:
/// Programs may only really care about a few different orders of
/// magnitude-worth of binary orders of magnitude when it comes to
/// allocation sizes.
///
/// This could probably be improved by sampling empirical allocation data or something.
pub fn generate_size(max: usize) -> usize {
    let cap = fastrand::usize(16..max);
    fastrand::usize(4..cap)
}

/// Strongly bias towards low alignment requirements.
///
/// Most allocations don't need alignment any higher than the system pointer size.
/// (e.g. malloc doesn't guarantee a higher alignment).
pub fn generate_align() -> usize {
    // 75%    align_of::<usize>
    // 19%    align_of::<usize>*2
    //  4%    align_of::<usize>*4
    //  0.8%  align_of::<usize>*8
    //       ...
    align_of::<usize>() << fastrand::u16(..).trailing_zeros() / 2
}

pub struct AllocationWrapper<'a> {
    pub ptr: *mut u8,
    pub layout: Layout,
    pub allocator: &'a dyn GlobalAlloc,
}
impl<'a> AllocationWrapper<'a> {
    pub fn new(size: usize, align: usize, allocator: &'a dyn GlobalAlloc) -> Option<Self> {
        let layout = Layout::from_size_align(size, align).unwrap();

        let ptr = unsafe { (*allocator).alloc(layout) };

        if ptr.is_null() {
            return None;
        }

        Some(Self { ptr, layout, allocator })
    }

    pub fn realloc(&mut self, new_size: usize) -> Result<(), ()> {
        let new_ptr = unsafe { (*self.allocator).realloc(self.ptr, self.layout, new_size) };
        if new_ptr.is_null() {
            return Err(());
        }
        self.ptr = new_ptr;
        self.layout = Layout::from_size_align(new_size, self.layout.align()).unwrap();
        Ok(())
    }
}

impl<'a> Drop for AllocationWrapper<'a> {
    fn drop(&mut self) {
        unsafe { (*self.allocator).dealloc(self.ptr, self.layout) }
    }
}

unsafe fn init_talc() -> Box<dyn GlobalAlloc + Sync> {
    use talc::prelude::*;
    let talc: TalcLock<spin::Mutex<()>, _> = TalcLock::new(Manual);
    talc.lock().claim((&raw mut HEAP.0).cast(), HEAP_SIZE).unwrap();
    Box::new(talc)
}

unsafe fn init_talc_old() -> Box<dyn GlobalAlloc + Sync> {
    use prev_talc::{ErrOnOom, Talc};

    unsafe {
        let talc = Talc::new(ErrOnOom).lock::<spin::Mutex<()>>();
        talc.lock().claim((&raw mut HEAP.0).into()).unwrap();
        Box::new(talc)
    }
}

#[allow(dead_code)]
unsafe fn init_linked_list_allocator() -> Box<dyn GlobalAlloc + Sync> {
    let lla = linked_list_allocator::LockedHeap::new((&raw mut HEAP).cast(), HEAP_SIZE);
    lla.lock().init((&raw mut HEAP.0).cast(), HEAP_SIZE);
    Box::new(lla)
}

unsafe fn init_system() -> Box<dyn GlobalAlloc + Sync> {
    Box::new(std::alloc::System)
}

unsafe fn init_galloc() -> Box<dyn GlobalAlloc + Sync> {
    let galloc = good_memory_allocator::SpinLockedAllocator::<
        { good_memory_allocator::DEFAULT_SMALLBINS_AMOUNT },
        { good_memory_allocator::DEFAULT_ALIGNMENT_SUB_BINS_AMOUNT },
    >::empty();
    let boxed_galloc = Box::new(galloc);
    boxed_galloc.init(&raw mut HEAP as usize, HEAP_SIZE);
    boxed_galloc
}

unsafe fn init_rlsf() -> Box<dyn GlobalAlloc + Sync> {
    let tlsf = GlobalRLSF(spin::Mutex::new(rlsf::Tlsf::new()));
    tlsf.0.lock().insert_free_block(unsafe { std::mem::transmute(&mut HEAP.0[..]) });
    Box::new(tlsf)
}

unsafe fn init_buddy_alloc() -> Box<dyn GlobalAlloc + Sync> {
    use buddy_alloc::{BuddyAllocParam, FastAllocParam, NonThreadsafeAlloc};

    let ba = BuddyAllocWrapper(spin::Mutex::new(NonThreadsafeAlloc::new(
        FastAllocParam::new((&raw mut HEAP).cast(), HEAP_SIZE / 8),
        BuddyAllocParam::new(
            (&raw mut HEAP).cast::<u8>().add(HEAP_SIZE / 8),
            HEAP_SIZE / 8 * 7,
            64,
        ),
    )));

    Box::new(ba)
}

unsafe fn init_dlmalloc() -> Box<dyn GlobalAlloc + Sync> {
    let dl = DlMallocator(spin::Mutex::new(dlmalloc::Dlmalloc::new_with_allocator(DlmallocArena(
        std::sync::atomic::AtomicBool::new(true),
    ))));
    Box::new(dl)
}

struct BuddyAllocWrapper(pub spin::Mutex<buddy_alloc::NonThreadsafeAlloc>);

unsafe impl Send for BuddyAllocWrapper {}
unsafe impl Sync for BuddyAllocWrapper {}

unsafe impl GlobalAlloc for BuddyAllocWrapper {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0.lock().alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.lock().dealloc(ptr, layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        self.0.lock().alloc_zeroed(layout)
    }

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
struct DlmallocArena(std::sync::atomic::AtomicBool);

unsafe impl dlmalloc::Allocator for DlmallocArena {
    fn alloc(&self, _size: usize) -> (*mut u8, usize, u32) {
        let has_data = self.0.fetch_and(false, core::sync::atomic::Ordering::SeqCst);

        if has_data {
            (unsafe { &raw mut HEAP.0[0] }, HEAP_SIZE, 1)
        } else {
            (core::ptr::null_mut(), 0, 0)
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

struct GlobalRLSF<'p>(
    spin::Mutex<rlsf::Tlsf<'p, usize, usize, { usize::BITS as usize - 12 }, { usize::BITS as _ }>>,
);
unsafe impl<'a> GlobalAlloc for GlobalRLSF<'a> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0.lock().allocate(layout).map_or(std::ptr::null_mut(), |nn| nn.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.lock().deallocate(NonNull::new_unchecked(ptr), layout.align());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.0
            .lock()
            .reallocate(
                NonNull::new_unchecked(ptr),
                Layout::from_size_align_unchecked(new_size, layout.align()),
            )
            .map_or(std::ptr::null_mut(), |nn| nn.as_ptr())
    }
}

unsafe fn init_talc_sys() -> Box<dyn GlobalAlloc + Sync> {
    use talc::prelude::*;
    let talc: TalcLock<spin::Mutex<()>, _> = TalcLock::new(Os::new());

    Box::new(talc)
}

unsafe fn init_dlmalloc_sys() -> Box<dyn GlobalAlloc + Sync> {
    Box::new(dlmalloc::GlobalDlmalloc)
}

unsafe fn init_frusa_sys() -> Box<dyn GlobalAlloc + Sync> {
    Box::new(frusa::Frusa2M::new(&std::alloc::System))
}

unsafe fn init_jemalloc_sys() -> Box<dyn GlobalAlloc + Sync> {
    Box::new(jemallocator::Jemalloc)
}

unsafe fn init_mimalloc_sys() -> Box<dyn GlobalAlloc + Sync> {
    Box::new(mimalloc::MiMalloc)
}
