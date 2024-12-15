use crossbeam_utils::CachePadded;

/// The minimum size and alignment that Talc will use for chunks.
pub const TALC_MIN_SIZE_ALIGN: usize = size_of::<usize>() * 4;

/// Returns the size and alignment that Talc will use for chunks given the specified over-alignment.
/// 
/// If the over-alignment is smaller than the hard minimum, the hard minimum is used.
pub const fn alloc_unit<A: ChunkAlign>() -> usize {
    if TALC_MIN_SIZE_ALIGN < A::MIN_ALIGN { A::MIN_ALIGN } else { TALC_MIN_SIZE_ALIGN }
}

pub trait ChunkAlign {
    const MIN_ALIGN: usize;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultAlign;

impl ChunkAlign for DefaultAlign { const MIN_ALIGN: usize = TALC_MIN_SIZE_ALIGN; }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetAlign<const OVERALIGN: usize>;

impl<const A: usize> ChunkAlign for SetAlign<A> { const MIN_ALIGN: usize = A; }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheAligned;

impl ChunkAlign for CacheAligned {
    const MIN_ALIGN: usize = align_of::<CachePadded<u8>>();
}


