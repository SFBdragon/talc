use core::{fmt::Debug, num::NonZeroUsize, ptr::{addr_of_mut, NonNull}, usize};

use crate::{base::CHUNK_UNIT, node::Node, Binning};

use super::OomHandler;

pub mod unix;
pub mod win;

#[cfg(target_family = "unix")]
pub type GetSysMemOnOom = GetSourceMemOnOom<unix::UnixMMapSource>;

#[cfg(target_family = "windows")]
pub type GetSysMemOnOom = GetSourceMemOnOom<win::Win32VirtualAllocSource>;

const MIN_RESERVATION_ALIGN: usize = 64;

/// # Safety
/// Implementor must uphold [`OomHandler`]'s implementation safety requirements.
pub unsafe trait ReserveCommitDecommitRelease {
    /// Implementors must return well-aligned regions of memory.
    /// At the moment, alignment of at least 64 bytes is expected.
    fn reserve(&mut self, min_size: NonZeroUsize) -> Option<NonNull<[u8]>>;
    unsafe fn release(&mut self, base: NonNull<u8>, reservation_size: NonZeroUsize);
    unsafe fn commit(&mut self, base: NonNull<u8>, min_size: NonZeroUsize) -> NonNull<u8>;
    unsafe fn decommit(&mut self, top: NonNull<u8>, max_size: NonZeroUsize) -> NonNull<u8>;
}

#[derive(Debug)]
pub struct GetSourceMemOnOom<A: ReserveCommitDecommitRelease> {
    source: A,
    reservations: Option<NonNull<Node>>,
}

#[repr(C)]
struct ReservationHeader {
    node: Node,
    arena_acme: NonNull<u8>,
    uncommitted: usize,
}

impl<A: ReserveCommitDecommitRelease> GetSourceMemOnOom<A> {
    pub const fn new(source: A) -> Self {
        Self {
            source,
            reservations: None,
        }
    }
}

unsafe impl<A: ReserveCommitDecommitRelease + Send> Send for GetSourceMemOnOom<A> {}
unsafe impl<A: ReserveCommitDecommitRelease + Sync> Sync for GetSourceMemOnOom<A> {}

// SAFETY: TODO the backing allocator must not use `talc` whatsoever
unsafe impl<A: ReserveCommitDecommitRelease + Debug, B: Binning> OomHandler<B> for GetSourceMemOnOom<A> {
    fn handle_oom(talc: &mut crate::base::Talc<Self, B>, layout: core::alloc::Layout) -> Result<(), ()> {
        let mut min_additional_size = layout.size() + crate::base::CHUNK_UNIT + layout.align() - 1;

        unsafe {
            for node in Node::iter_mut(talc.oom_handler.reservations).take(2) {
                let header = node.cast::<ReservationHeader>();

                if header.as_ptr().read().uncommitted > min_additional_size {
                    let arena_acme = header.as_ptr().read().arena_acme;
    
                    let new_acme = talc.oom_handler.source.commit(
                        arena_acme,
                        NonZeroUsize::new_unchecked(min_additional_size),
                    );
    
                    talc.extend_raw(arena_acme.as_ptr(), new_acme.as_ptr());

                    (*header.as_ptr()).arena_acme = new_acme;
                    (*header.as_ptr()).uncommitted -= new_acme.as_ptr() as usize - arena_acme.as_ptr() as usize;
    
                    return Ok(());
                } 
            }
        }

        unsafe {
            if !talc.is_metadata_established() {
                min_additional_size += crate::min_first_arena_layout::<B>().size();
            }

            let min_additional_size = NonZeroUsize::new_unchecked(min_additional_size);
            if let Some(span) = talc.oom_handler.source.reserve(min_additional_size) {

                let arena_acme = talc.oom_handler.source.commit(span.cast(), min_additional_size);
                let committed_size = arena_acme.as_ptr() as usize - span.as_ptr().cast::<u8>() as usize;

                let arena = talc
                    .claim(
                        span.as_ptr().cast::<u8>().wrapping_add(size_of::<ReservationHeader>()),
                        committed_size - size_of::<ReservationHeader>(),
                    )
                    .unwrap_unchecked();

                debug_assert_eq!(arena.end(), span.as_ptr().cast::<u8>().wrapping_add(committed_size));

                let node = Node {
                    next: talc.oom_handler.reservations,
                    next_of_prev: addr_of_mut!(talc.oom_handler.reservations),
                };

                Node::link_at(span.as_ptr().cast(), node);

                // todo
                let node = Node {
                    next: talc.oom_handler.reservations,
                    next_of_prev: addr_of_mut!(talc.oom_handler.reservations),
                };
                span.as_ptr().cast::<ReservationHeader>().write(ReservationHeader {
                    node,
                    arena_acme,
                    uncommitted: span.len() - committed_size,
                });
                // todo
                /* let replace_index = talc.oom_handler.best_replacement();
                talc.oom_handler.reservations[replace_index] = Some(reservation); */

                return Ok(());
            }
        }

        Err(())
    }
    
    unsafe fn handle_basereg(&mut self, arena_base: *mut u8, arena_acme: *mut u8) -> bool {
        let arena_base_offset = CHUNK_UNIT
            .max((size_of::<ReservationHeader>() + 1 + CHUNK_UNIT - 1) & !(CHUNK_UNIT - 1));

        let reservation_base = arena_base.wrapping_sub(arena_base_offset);
        let header = reservation_base
            .cast::<ReservationHeader>()
            .read();

        if arena_acme != header.arena_acme.as_ptr() {
            return false;
        }

        Node::unlink(header.node);

        let reservation_size = header.uncommitted + (arena_acme as usize - reservation_base as usize);

        self.source.release(
            NonNull::new_unchecked(reservation_base),
            NonZeroUsize::new_unchecked(reservation_size),
        );

        true
    }
}


impl<A: ReserveCommitDecommitRelease + Debug, B: Binning> crate::base::Talc<GetSourceMemOnOom<A>, B> {
    /// Release unused memory to the backing allocator.
    pub fn reclaim(&mut self) {
        let mut prev = &raw mut self.oom_handler.reservations;
        let mut curr = self.oom_handler.reservations;
        while let Some(header) = curr {
            let ReservationHeader { node, arena_acme, uncommitted }
                = unsafe { header.as_ptr().cast::<ReservationHeader>().read() };

            let span = crate::ptr_utils::nonnull_slice_from_raw_parts(header.cast(), span_size);

            let arena_base = header.cast::<u8>().as_ptr().wrapping_add(size_of::<ReservationHeader>());
            let arena = unsafe { Arena::new(arena_base, arena_end.as_ptr()) };

            let reserved = unsafe { self.reserved(&arena) };

            if self.oom_handler.source.supports_deallocate() && reserved == 0 {
                let result = unsafe {
                    self.truncate(arena, 0)
                };

                debug_assert!(result.is_none());

                unsafe {
                    self.oom_handler.source.deallocate(span);
                }

                unsafe {
                    prev.write(next);
                }
            } else if let Some(delta) = self.oom_handler.source.supports_shrink_with_delta_of() {
                let min_new_size = size_of::<ReservationHeader>() + reserved;
                if min_new_size + CHUNK_UNIT + delta.get() < span_size {
                    let gap_base = arena.base().wrapping_add(reserved);

                    unsafe {
                        let node = super::super::gap_base_to_node(gap_base).read();
                        let bin = super::super::gap_base_to_bin(gap_base).read();
                        let size = super::super::gap_base_to_size(gap_base).read();

                        if let Some(new_span_size) = self.oom_handler.source.extend(span, min_new_size) {
                            self.deregister_gap_no_touch(node, bin, size);

                            if size_of::<ReservationHeader>() + reserved + CHUNK_UNIT < new_span_size {
                                let new_gap_acme = crate::base::Talc::<super::ErrOnOom, B>::align_down(
                                    span.cast::<u8>().as_ptr().wrapping_add(new_span_size)
                                );

                                self.register_gap(gap_base, new_gap_acme);
                            }
                            
                        }
                    }
                }
            }

            prev = unsafe { &raw mut (*header.as_ptr()).next };
            curr = next;
        }
    }
}

struct Test<T: MT> {
    t1: T,
}

trait M1 {}
trait M2 {}

struct M11;
impl M1 for M11 {}

struct M22;
impl M2 for M22 {}

trait MT {
    type M11orM22;
}

impl<T: MT> Test<T> where T::M11orM22: M1 {

}
