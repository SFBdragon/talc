use core::{fmt::Debug, num::NonZeroUsize, ptr::NonNull, usize};

use crate::{Binning, base::CHUNK_UNIT, node::Node};

use super::OomHandler;

#[cfg(target_family = "unix")]
pub mod unix;
#[cfg(target_family = "windows")]
pub mod win;

#[cfg(target_family = "unix")]
pub type WithSysMem = GetSourceMemOnOom<unix::UnixMMapSource>;

#[cfg(target_family = "windows")]
pub type WithSysMem = GetSourceMemOnOom<win::Win32VirtualAllocSource>;

/// # Safety
/// Implementor must uphold [`OomHandler`]'s implementation safety requirements.
pub unsafe trait ReserveCommitDecommitRelease {
    const INIT: Self;

    /// Implementors must return well-aligned regions of memory.
    /// At the moment, alignment of at least 64 bytes is expected.
    fn reserve(&mut self, min_size: NonZeroUsize) -> Option<NonNull<[u8]>>;
    unsafe fn release(&mut self, base: NonNull<u8>, reservation_size: usize);
    unsafe fn commit(&mut self, base: NonNull<u8>, size: usize);
    unsafe fn decommit(&mut self, base: NonNull<u8>, size: usize);

    fn commit_granularity(&mut self) -> usize;
}

const RESERVATION_CACHE: usize = 2;

#[derive(Debug)]
pub struct GetSourceMemOnOom<A: ReserveCommitDecommitRelease> {
    source: A,
    uncommitted_cache: [Option<Reservation>; RESERVATION_CACHE],
    res_chain: Option<NonNull<Node>>,
}

impl<A: ReserveCommitDecommitRelease> GetSourceMemOnOom<A> {
    pub const fn new() -> Self {
        Self { source: A::INIT, uncommitted_cache: [None; RESERVATION_CACHE], res_chain: None }
    }
}

unsafe impl<A: ReserveCommitDecommitRelease + Send> Send for GetSourceMemOnOom<A> {}
unsafe impl<A: ReserveCommitDecommitRelease + Sync> Sync for GetSourceMemOnOom<A> {}

// SAFETY: TODO the backing allocator must not use `talc` whatsoever
unsafe impl<A: ReserveCommitDecommitRelease + Debug, B: Binning> OomHandler<B>
    for GetSourceMemOnOom<A>
{
    const TRACK_ARENA_END: bool = true;

    fn handle_oom(
        talc: &mut crate::base::Talc<Self, B>,
        layout: core::alloc::Layout,
    ) -> Result<(), ()> {
        let mut min_additional_size = layout.size() + crate::base::CHUNK_UNIT + layout.align() - 1;
        let commit_granularity_m1 = talc.oom_handler.source.commit_granularity() - 1;
        let additional_commit_size =
            (min_additional_size + commit_granularity_m1) & !commit_granularity_m1;

        unsafe {
            for reservation in talc.oom_handler.uncommitted_cache.iter_mut() {
                if let Some(reservation) = reservation {
                    if reservation.uncommitted >= additional_commit_size {
                        talc.oom_handler
                            .source
                            .commit(reservation.commit_end, additional_commit_size);

                        let old_arena_end = reservation.arena_end();
                        reservation.commit_end = NonNull::new_unchecked(
                            reservation.commit_end.as_ptr().wrapping_add(additional_commit_size),
                        );
                        let new_arena_end = reservation.arena_end();

                        reservation.uncommitted -= additional_commit_size;
                        new_arena_end.cast::<usize>().write(reservation.uncommitted);

                        talc.extend(old_arena_end, new_arena_end);

                        return Ok(());
                    }
                }
            }
        }

        unsafe {
            if !talc.is_metadata_established() {
                min_additional_size += crate::min_first_arena_layout::<B>().size();
            }

            if let Some(span) =
                talc.oom_handler.source.reserve(NonZeroUsize::new_unchecked(min_additional_size))
            {
                let commit_size =
                    (min_additional_size + commit_granularity_m1) & !commit_granularity_m1;

                talc.oom_handler.source.commit(span.cast(), commit_size);
                let commit_end =
                    NonNull::new_unchecked(span.cast::<u8>().as_ptr().wrapping_add(commit_size));

                debug_assert!(CHUNK_UNIT <= talc.oom_handler.source.commit_granularity());

                let res_base = span.cast::<u8>().as_ptr();

                let header = res_base.cast::<Header>();
                (*header).reserved = span.len();
                Node::link_at(
                    &raw mut (*header).node,
                    Node {
                        next: talc.oom_handler.res_chain,
                        next_of_prev: &raw mut talc.oom_handler.res_chain,
                    },
                );

                let arena_base = res_base.wrapping_add(core::mem::size_of::<Header>());
                let arena_size = commit_size - CHUNK_UNIT;
                let arena_end = talc.claim(arena_base, arena_size).unwrap_unchecked().as_ptr();

                debug_assert_eq!(arena_end, commit_end.as_ptr().wrapping_sub(CHUNK_UNIT));

                let uncommitted = span.len() - commit_size;
                arena_end.cast::<usize>().write(uncommitted);

                'overwrite_min: {
                    let mut min_uncommitted_index = usize::MAX;
                    let mut min_uncommitted_bytes = uncommitted;

                    for (index, opt_reservation) in
                        talc.oom_handler.uncommitted_cache.iter_mut().enumerate()
                    {
                        if let Some(reservation) = opt_reservation {
                            if reservation.uncommitted < min_uncommitted_bytes {
                                min_uncommitted_index = index;
                                min_uncommitted_bytes = reservation.uncommitted;
                            }
                        } else {
                            *opt_reservation = Some(Reservation { commit_end, uncommitted });

                            break 'overwrite_min;
                        }
                    }

                    if min_uncommitted_index != usize::MAX {
                        talc.oom_handler.uncommitted_cache[min_uncommitted_index] =
                            Some(Reservation { commit_end, uncommitted });
                    }
                }

                return Ok(());
            }
        }

        Err(())
    }

    unsafe fn maybe_resize_arena(
        &mut self,
        chunk_base: *mut u8,
        arena_end: *mut u8,
        is_arena_base: bool,
    ) -> *mut u8 {
        if is_arena_base {
            let reservation_base = chunk_base.wrapping_sub(CHUNK_UNIT);
            let uncommitted = arena_end.cast::<usize>().read();
            let commit_end = arena_end.wrapping_add(CHUNK_UNIT);

            for reservation in self.uncommitted_cache.iter_mut() {
                if let Some(res_commit_end) = reservation.map(|r| r.commit_end.as_ptr()) {
                    if res_commit_end == commit_end {
                        *reservation = None;
                    }
                }
            }

            Node::unlink(reservation_base.cast::<Node>().read());

            let reservation_size = uncommitted + (commit_end as usize - reservation_base as usize);

            self.source.release(NonNull::new_unchecked(reservation_base), reservation_size);

            chunk_base
        } else {
            // free size = commit_end - chunk_base - CHUNK_UNIT
            //           = (arena_end + CHUNK_UNIT) - chunk_base - CHUNK_UNIT
            //           = arena_end - chunk_base
            let free_size = arena_end as usize - chunk_base as usize;
            let commit_granularity = self.source.commit_granularity();

            if free_size >= commit_granularity * 4 {
                let uncommitted = arena_end.cast::<usize>().read();
                let commit_end = arena_end.wrapping_add(CHUNK_UNIT);

                let decommit_size = free_size & !(commit_granularity - 1);
                let new_commit_end = NonNull::new_unchecked(commit_end.wrapping_sub(decommit_size));

                self.source.decommit(new_commit_end, decommit_size);
                let new_arena_end = arena_end.wrapping_sub(decommit_size);
                let new_uncommitted = uncommitted + decommit_size;
                new_arena_end.cast::<usize>().write(new_uncommitted);

                let mut min_uncommitted_index = usize::MAX;
                let mut min_uncommitted_bytes = new_uncommitted;
                'overwrite_min: {
                    for (index, opt_reservation) in self.uncommitted_cache.iter_mut().enumerate() {
                        if let Some(reservation) = opt_reservation {
                            if reservation.commit_end.as_ptr() == commit_end {
                                reservation.uncommitted = new_uncommitted;
                                reservation.commit_end = new_commit_end;
                                break 'overwrite_min;
                            }

                            if reservation.uncommitted < min_uncommitted_bytes {
                                min_uncommitted_index = index;
                                min_uncommitted_bytes = reservation.uncommitted;
                            }
                        } else {
                            min_uncommitted_index = index;
                            min_uncommitted_bytes = 0;
                        }
                    }

                    if min_uncommitted_index != usize::MAX {
                        self.uncommitted_cache[min_uncommitted_index] = Some(Reservation {
                            commit_end: new_commit_end,
                            uncommitted: new_uncommitted,
                        });
                    }
                }

                new_arena_end
            } else {
                arena_end
            }
        }
    }
}

impl<A: ReserveCommitDecommitRelease> Drop for GetSourceMemOnOom<A> {
    fn drop(&mut self) {
        unsafe {
            for node in Node::iter_mut(self.res_chain) {
                let header = node.as_ptr().cast::<Header>();
                self.source.release(node.cast(), (*header).reserved);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Reservation {
    commit_end: NonNull<u8>,
    uncommitted: usize,
}

impl Reservation {
    pub fn arena_end(&self) -> *mut u8 {
        self.commit_end.as_ptr().wrapping_sub(CHUNK_UNIT)
    }
}

#[repr(C)]
struct Header {
    node: Node,
    reserved: usize,
}
