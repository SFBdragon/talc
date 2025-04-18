//! Reserve, commit, decommit, release.
//!
//! [`RcdrSource`] provides a powerful [`Source`] implementation for
//! implementors of [`ReserveCommitDecommitRelease`].
//!
//! TODO

use core::{
    fmt::Debug,
    mem::{align_of, size_of},
    num::NonZeroUsize,
    ptr::{NonNull, addr_of_mut},
    usize,
};

use crate::{base::CHUNK_UNIT, base::binning::Binning, node::Node, ptr_utils};

use super::Source;

#[cfg(all(feature = "system-backed", target_family = "unix"))]
mod unix;
#[cfg(all(feature = "system-backed", target_family = "windows"))]
mod win;

#[cfg(all(feature = "system-backed", target_family = "unix"))]
pub type Os = RcdrSource<unix::UnixMMapSource>;

#[cfg(all(feature = "system-backed", target_family = "windows"))]
pub type Os = RcdrSource<win::Win32VirtualAllocSource>;

/// TODO
/// # Safety
/// Implementor must uphold [`Source`]'s implementation safety requirements.
pub unsafe trait ReserveCommitDecommitRelease {
    /// Constant initial value.
    const INIT: Self;

    /// TODO
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
pub struct RcdrSource<A: ReserveCommitDecommitRelease> {
    source: A,
    uncommitted_cache: [Option<Reservation>; RESERVATION_CACHE],
    res_chain: Option<NonNull<Option<NonNull<Node>>>>,
}

impl<A: ReserveCommitDecommitRelease> RcdrSource<A> {
    pub const fn new() -> Self {
        Self { source: A::INIT, uncommitted_cache: [None; RESERVATION_CACHE], res_chain: None }
    }
}

unsafe impl<A: ReserveCommitDecommitRelease + Send> Send for RcdrSource<A> {}
unsafe impl<A: ReserveCommitDecommitRelease + Sync> Sync for RcdrSource<A> {}

// SAFETY: TODO the backing allocator must not use `talc` whatsoever
unsafe impl<A: ReserveCommitDecommitRelease + Debug> Source for RcdrSource<A> {
    const TRACK_HEAP_END: bool = true;

    fn acquire<B: Binning>(
        talc: &mut crate::base::Talc<Self, B>,
        layout: core::alloc::Layout,
    ) -> Result<(), ()> {
        let mut min_additional_size = layout.size() + crate::base::CHUNK_UNIT + layout.align() - 1;

        let commit_granularity_m1 = talc.source.source.commit_granularity() - 1;
        let additional_commit_size =
            (min_additional_size + commit_granularity_m1) & !commit_granularity_m1;

        for reservation in talc.source.uncommitted_cache.iter_mut() {
            if let Some(reservation) = reservation {
                if reservation.uncommitted >= additional_commit_size {
                    unsafe {
                        talc.source.source.commit(reservation.commit_end, additional_commit_size);

                        let old_heap_end = reservation.heap_end();
                        reservation.commit_end = NonNull::new_unchecked(
                            reservation.commit_end.as_ptr().wrapping_add(additional_commit_size),
                        );
                        let new_heap_end = reservation.heap_end();

                        reservation.uncommitted -= additional_commit_size;
                        new_heap_end.cast::<usize>().write(reservation.uncommitted);

                        talc.extend(old_heap_end, new_heap_end);

                        return Ok(());
                    }
                }
            }
        }

        if !talc.is_metadata_established() {
            min_additional_size += crate::min_first_heap_layout::<B>().size();
            min_additional_size += size_of::<Header>();
            min_additional_size += size_of::<usize>();
        }

        unsafe {
            if let Some(span) =
                talc.source.source.reserve(NonZeroUsize::new_unchecked(min_additional_size))
            {
                let commit_size =
                    (min_additional_size + commit_granularity_m1) & !commit_granularity_m1;

                talc.source.source.commit(span.cast(), commit_size);
                let commit_end =
                    NonNull::new_unchecked(span.cast::<u8>().as_ptr().wrapping_add(commit_size));

                debug_assert!(CHUNK_UNIT <= talc.source.source.commit_granularity());

                let res_base = span.cast::<u8>().as_ptr();

                let header = res_base.cast::<Header>();
                (*header).reserved = span.len();

                let mut heap_base = res_base.wrapping_add(core::mem::size_of::<Header>());
                let chain = if let Some(chain) = talc.source.res_chain {
                    chain.as_ptr()
                } else {
                    let chain =
                        ptr_utils::align_up_by(heap_base, align_of::<Option<NonNull<Node>>>())
                            .cast::<Option<NonNull<Node>>>();

                    heap_base = chain.wrapping_add(1).cast();

                    let res_chain = NonNull::new(chain);
                    debug_assert!(res_chain.is_some());
                    talc.source.res_chain = res_chain;

                    chain
                };

                Node::link_at(
                    addr_of_mut!((*header).node),
                    Node { next: chain.read(), next_of_prev: chain },
                );

                let heap_size = commit_size - CHUNK_UNIT;
                let heap_end = talc.claim(heap_base, heap_size).unwrap_unchecked().as_ptr();

                debug_assert_eq!(heap_end, commit_end.as_ptr().wrapping_sub(CHUNK_UNIT));

                let uncommitted = span.len() - commit_size;
                heap_end.cast::<usize>().write(uncommitted);

                // Never actually repeats, but block labels are unstable on MSRV.
                'overwrite_min: loop {
                    let mut min_uncommitted_index = usize::MAX;
                    let mut min_uncommitted_bytes = uncommitted;

                    for (index, opt_reservation) in
                        talc.source.uncommitted_cache.iter_mut().enumerate()
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
                        talc.source.uncommitted_cache[min_uncommitted_index] =
                            Some(Reservation { commit_end, uncommitted });
                    }

                    break;
                }

                return Ok(());
            }
        }

        Err(())
    }

    unsafe fn resize(
        &mut self,
        chunk_base: *mut u8,
        heap_end: *mut u8,
        is_heap_base: bool,
    ) -> *mut u8 {
        if is_heap_base {
            // Because the initial chain pointer is placed in the first-established reservation,
            // AND Talc also puts its metadata into the first-established heap, `is_heap_base`
            // is never true if the given heap is the first established heap.
            // Thus, the header size is always `size_of::<Header>()` here.

            // The reservation is always CHUNK_UNIT aligned.
            // The `chunk_base` is always CHUNK_UNIT aligned.
            // We always tell Talc is can use from `base + size_of::<Header>()` onwards for non-first reservations.
            // size_of::<Header>() <= CHUNK_UNIT - 1
            // Documentation of `resize` states that the `chunk_base` of the heap base will
            // be `base + 1` aligned up to the next `CHUNK_UNIT`.
            // So we can get from the `chunk_base` to the `reservation_base` by subtracting a `CHUNK_UNIT`.
            debug_assert!(size_of::<Header>() < CHUNK_UNIT);
            let reservation_base = chunk_base.wrapping_sub(CHUNK_UNIT);
            let uncommitted = heap_end.cast::<usize>().read();
            let commit_end = heap_end.wrapping_add(CHUNK_UNIT);

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
            //           = (heap_end + CHUNK_UNIT) - chunk_base - CHUNK_UNIT
            //           = heap_end - chunk_base
            let free_size = heap_end as usize - chunk_base as usize;
            let commit_granularity = self.source.commit_granularity();

            if free_size >= commit_granularity * 4 {
                let uncommitted = heap_end.cast::<usize>().read();
                let commit_end = heap_end.wrapping_add(CHUNK_UNIT);

                let decommit_size = free_size & !(commit_granularity - 1);
                let new_commit_end = NonNull::new_unchecked(commit_end.wrapping_sub(decommit_size));

                self.source.decommit(new_commit_end, decommit_size);
                let new_heap_end = heap_end.wrapping_sub(decommit_size);
                let new_uncommitted = uncommitted + decommit_size;
                new_heap_end.cast::<usize>().write(new_uncommitted);

                let mut min_uncommitted_index = usize::MAX;
                let mut min_uncommitted_bytes = new_uncommitted;
                // this never actually loops, but block labels are unstable on MSRV
                'overwrite_min: loop {
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

                    break;
                }

                new_heap_end
            } else {
                heap_end
            }
        }
    }
}

impl<A: ReserveCommitDecommitRelease> Drop for RcdrSource<A> {
    fn drop(&mut self) {
        if let Some(chain) = self.res_chain {
            unsafe {
                for node in Node::iter_mut(chain.as_ptr().read()) {
                    let header = node.as_ptr().cast::<Header>();
                    self.source.release(node.cast(), (*header).reserved);
                }
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
    pub fn heap_end(&self) -> *mut u8 {
        self.commit_end.as_ptr().wrapping_sub(CHUNK_UNIT)
    }
}

#[repr(C)]
struct Header {
    node: Node,
    reserved: usize,
}
