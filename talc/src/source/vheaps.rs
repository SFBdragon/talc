//! UNFINISHED - OPEN AN ISSUE IF YOU WANT OS VIRTUAL MEMORY INTEGRATION
//!
//! Reserve, commit, decommit, release.
//!
//! [`VirtualHeaps`] defined the memory management model.
//! Read its documentation for info.
//!
//! [`VirtualHeapsSource`] implements [`Source`] using the
//!
//! [`VirtualHeapsSource`] provides a powerful [`Source`] implementation for
//! implementors of [`VirtualHeaps`].

use core::{
    fmt::Debug,
    mem::{align_of, size_of},
    num::NonZeroUsize,
    ptr::{NonNull, addr_of_mut},
    usize,
};

use crate::{base::CHUNK_UNIT, base::binning::Binning, node::Node, ptr_utils};

use super::Source;

// not sure whether to commit to making these public TODO
#[cfg(all(feature = "os", target_family = "unix"))]
mod unix;
#[cfg(all(feature = "os", target_family = "windows"))]
mod win;

#[cfg(all(feature = "os", target_family = "unix"))]
type OsSource = unix::UnixMMapSource;
#[cfg(all(feature = "os", target_family = "windows"))]
type OsSource = win::Win32VirtualAllocSource;

/// Sources heaps from the operating system, and releases memory back when unused.
///
/// Currently, the `"unix"` and `"windows"` target families are supported.
/// Feel free to PR or request other integrations.
pub type Os = VirtualHeapsSource<OsSource, DEFAULT_UNCOMMITTED_CACHE_LEN>;

#[cfg(feature = "os")]
const DEFAULT_UNCOMMITTED_CACHE_LEN: usize = 3;

/// [`VirtualHeaps`] is a trait structured around a model
/// of memory management aimed at systems with large virtual memory address
/// spaces, with clever OSs attempting to save memory by reclaiming where
/// possible.
///
/// At a high level:
/// - Reservations: Large blocks of virtual memory are reserved on-demand,
///     but without backing physical memory initially.
/// - Committing: Smaller blocks of physical memory are committed as needed.
/// - Decommitting: Blocks of physical memory no longer in-use are marked for
///     reclamation, hinting to the OS to take back the memory when it wants to.
/// - Release: Blocks of virtual memory are released entirely if no longer used.
///
/// This model is a blatant rip-off of Window's `VirtualAlloc` and `VirtualFree`
/// APIs, while remaining compatible with `mmap`-based memory management for
/// Unix-likes.
///
/// The documentation of each trait function documentation contains more details
/// about each operation.
///
/// Once implemented, [`VirtualHeapsSource`] implements the mechanisms needed to manage these
/// memory regions, and implements [`Source`], integrating this backing-memory
/// management scheme to [`Talc`](crate::base::Talc)'s [`Source`] API.
///
/// # Safety
/// Implementations must uphold [`Source`]'s implementation safety requirements.
/// In short: do not call the global allocator.
///
/// Implementations must also adhere to the guarantees stipulated on the
/// functions of [`VirtualHeaps`].
pub unsafe trait VirtualHeaps {
    /// The implementation of [`VirtualHeaps`] must
    /// be constructable in const contexts.
    ///
    /// It is allowed to lazily initialize any state as necessary
    /// once one of the following are called for the first time:
    /// - [`VirtualHeaps::reserve`]
    /// - [`VirtualHeaps::commit_granularity`]
    const INIT: Self;

    /// Reserve address space of at least `min_size`.
    ///
    /// The memory is not expected to be readable or writable
    /// until [`VirtualHeaps::commit`] is called on
    /// a subregion of the memory.
    /// Reservations are expected to occur in large quantities even if a small
    /// `min_size` is passed, and are also not expected to change the RSS
    /// (resident set size) or commit charge or equivalent.
    /// i.e. Virtual memory is reserved, but not physical memory.
    ///
    /// Implementations must guarantee that
    /// - The returned region of memory should be aligned to, and a multiple of
    ///     [`VirtualHeaps::commit_granularity`].
    /// - The returned region of memory should be at least `min_size` bytes long.
    /// - The returned region must not overlap with any reservation that has
    ///     not been passed to [`VirtualHeaps::release`] since last returned
    ///     be [`VirtualHeaps::reserve`].
    fn reserve(&mut self, min_size: NonZeroUsize) -> Option<NonNull<[u8]>>;
    /// Release reserved memory.
    ///
    /// This is allowed to be called even if the reservation is still fully
    /// or partially committed.
    ///
    /// This should release the entire virtual memory reservation back
    /// to the API/memory subsystem it was obtained from.
    /// Memory is allowed to be returned again by [`VirtualHeaps::reserve`] after
    /// being passed to [`VirtualHeaps::release`].
    ///
    /// # Safety
    ///
    /// Callers must guarantee that
    /// - `base` was returned by [`VirtualHeaps::reserve`] of the same instance.
    /// - `reservation_size` is the size of the full heap as returned by the [`VirtualHeaps::reserve`] call.
    unsafe fn release(&mut self, base: NonNull<u8>, reservation_size: usize);

    /// Commit some reserved memory.
    ///
    /// Implementations must guarantee that
    /// - When `commit` returns, the memory must be readable and writable.
    ///
    /// # Safety
    ///
    /// Callers must guarantee that
    /// - `base` is within a memory region returned by [`VirtualHeaps::reserve`]
    ///     and not yet released or committed.
    /// - `base` is aligned to [`VirtualHeaps::commit_granularity`].
    /// - `size` is a multiple of [`VirtualHeaps::commit_granularity`]
    unsafe fn commit(&mut self, base: NonNull<u8>, size: usize) -> Result<(), ()>;

    /// Hints that [`VirtualHeaps::decommit`] should be called on memory
    /// that is committed if it is expected to become unused for a while.
    /// This operation is expected to be more expensive than
    /// [`VirtualHeaps::discard`] but meaningfully reduce the amount of memory
    /// usage counted against the program over calling [`VirtualHeaps::discard`].
    const SHOULD_DECOMMIT: bool;

    /// Decommit some committed memory.
    ///
    /// Implementations must guarantee that
    /// - Memory may be re-committed later with [`VirtualHeaps::commit`].
    ///
    /// # Safety
    /// Callers must guarantee that
    /// - The memory region described by `base` and `size` is within a memory region
    ///     returned by [`VirtualHeaps::reserve`] and not yet released or decommitted.
    /// - `base` is aligned to [`VirtualHeaps::commit_granularity`]
    /// - `size` is a multiple of [`VirtualHeaps::commit_granularity`]
    unsafe fn decommit(&mut self, base: NonNull<u8>, size: usize);

    /// Allow committed memory to be discarded.
    ///
    /// The memory may become zeroed or the contents may become undefined.
    /// However reads and writes to the memory are expected to succeed
    /// unless [`VirtualHeaps::MUST_REINSTATE`] is set, in which case
    /// [`VirtualHeaps::reinstate`] will be called before reading and writing
    /// is expected.
    ///
    /// This is a hint to tell the backing memory system it can reclaim the memory for other purposes.
    ///
    /// This is allowed to do nothing.
    ///
    /// # Safety
    /// Callers must guarantee that
    /// - The memory region described by `base` and `size` is within a memory region
    ///     returned by [`VirtualHeaps::reserve`] and not yet released, discarded, or decommitted.
    /// - `base` is aligned to [`VirtualHeaps::commit_granularity`]
    /// - `size` is a multiple of [`VirtualHeaps::commit_granularity`]
    unsafe fn discard(&mut self, base: NonNull<u8>, size: usize);

    /// If set, [`VirtualHeaps::reinstate`] must be called before memory
    /// discarded with [`VirtualHeaps::discard`] may be read and written to again.
    ///
    /// See the [`VirtualHeaps::reinstate`] and [`VirtualHeaps::discard`] documentation
    /// for more information.
    const MUST_REINSTATE: bool;

    /// Allow discarded memory to be re-used.
    ///
    /// Only called if `VirtualHeaps::MUST_REINSTATE`] is set.
    ///
    /// Implementations must guarantee that
    /// - after `reinstate` returns, the memory must be readable and writeable
    /// - after `reinstate` returns, the memory must be discardable
    /// - after `reinstate` returns, the memory must be decommitable if `SHOULD_DECOMMIT` is set.
    ///
    /// If [`VirtualHeaps::MUST_REINSTATE`] is true, then the caller guarantees
    /// calling this on memory in between discarding it and reading/writing to it again.
    ///
    /// Memory is not expected to be preserved or zeroed. It may be in an undefined state.
    ///
    /// # Safety
    /// Callers must guarantee that
    /// - The memory region described by `base` and `size` is within a memory region
    ///     returned by [`VirtualHeaps::reserve`] and discarded and not yet released or decommitted.
    /// - `base` is aligned to [`VirtualHeaps::commit_granularity`]
    /// - `size` is a multiple of [`VirtualHeaps::commit_granularity`]
    unsafe fn reinstate(&mut self, base: NonNull<u8>, size: usize) -> Result<(), ()> {
        let _ = (base, size);
        Ok(())
    }

    /// The unit of size and alignment of memory regions that can be committed and decommitted.
    /// This must be a power of two.
    ///
    /// Note that the size and alignment of reservations (in contrast to committed regions) size
    /// is entirely dictated by your [`VirtualHeaps::reserve`] implementation.
    /// This doesn't affect the size and alignment of reservations.
    /// However, reservations returned by [`VirtualHeaps::reserve`]
    /// must be at least aligned and sized to a multiple of
    /// [`VirtualHeaps::commit_granularity`].
    ///
    /// [`VirtualHeaps::commit_granularity`] is expected to be at least
    /// as large as [`CHUNK_UNIT`] at the moment.
    /// This can be as large as a cache line if `"cache-aligned-allocations"` are enabled, so ~256 bytes at most.
    /// In practice, you probably want this value to be much larger;
    /// at least a page of virtual memory, if not a substantial multiple of a page.
    fn commit_granularity(&mut self) -> usize;
}

/// The block size that the implementation will decommit at a time, as a multiple
/// of the block size that the implementaton will commit.
///
/// This avoids decommitting memory too aggressively, as deallocations are
/// likely followed by more allocations that'll need the recently-freed memory
/// back. Only if there's a large amount of memory sitting around is it a great
/// idea to release it.
///
/// This number is mostly sucked out of my thumb.
/// Testing other values is likely warranted under real allocation loads.
const DECOMMIT_COMMIT_RATIO: usize = 3;

/// A [`Source`] implementing [`VirtualHeaps`]'s model
/// for managing memory.
///
/// This is the recommended way to integrate [`Talc`](crate::base::Talc)
/// with a operating system with modern virtual memory APIs.
/// Though, you'd likely prefer to use `mimalloc` or `jemalloc` on platforms
/// where this is an option.
///
/// In other cases, where you want more advances memory management, most importantly
/// capable of rescinding memory no longer in-use in a performant way,
#[derive(Debug)]
pub struct VirtualHeapsSource<V: VirtualHeaps, const UNCOMMITTED_CACHE_LEN: usize> {
    vmem: V,
    uncommitted_cache: [Option<CachedReservation>; UNCOMMITTED_CACHE_LEN],
    res_chain: Option<NonNull<Option<NonNull<Node>>>>,
}

impl<A: VirtualHeaps, const UNCOMMITTED_CACHE_LEN: usize>
    VirtualHeapsSource<A, UNCOMMITTED_CACHE_LEN>
{
    /// Create an [`VirtualHeapsSource`] instance.
    ///
    /// # Safety
    /// This [`Source`] places metadata around heaps to manage them.
    ///
    /// Therefore manual heap management (i.e. using [`Talc::claim`](crate::base::Talc::claim),
    /// [`Talc::resize`](crate::base::Talc::resize), etc.) directly is not allowed, and will cause UB.
    pub const unsafe fn new() -> Self {
        Self { vmem: A::INIT, uncommitted_cache: [None; UNCOMMITTED_CACHE_LEN], res_chain: None }
    }
}

unsafe impl<V: VirtualHeaps + Send, const C: usize> Send for VirtualHeapsSource<V, C> {}
unsafe impl<V: VirtualHeaps + Sync, const C: usize> Sync for VirtualHeapsSource<V, C> {}

// SAFETY:
// [`VirtualHeaps`]'s implementation safety constract requires upholding
// [`Source`]'s implementation safety contract.
// Other than that, this implementation does not invoke the global allocator.
unsafe impl<V: VirtualHeaps + Debug, const UNCOMMITTED_CACHE_LEN: usize> Source
    for VirtualHeapsSource<V, UNCOMMITTED_CACHE_LEN>
{
    const TRACK_HEAP_END: bool = true;

    fn acquire<B: Binning>(
        talc: &mut crate::base::Talc<Self, B>,
        layout: core::alloc::Layout,
    ) -> Result<(), ()> {
        let mut min_additional_size = layout.size() + crate::base::CHUNK_UNIT + layout.align() - 1;

        let commit_granularity_m1 = talc.source.vmem.commit_granularity() - 1;
        let additional_commit_size =
            (min_additional_size + commit_granularity_m1) & !commit_granularity_m1;

        for reservation in talc.source.uncommitted_cache.iter_mut() {
            if let Some(reservation) = reservation {
                if reservation.available >= additional_commit_size {
                    unsafe {
                        talc.source.vmem.commit(reservation.heap_end, additional_commit_size);

                        let old_heap_end = reservation.heap_end();
                        reservation.heap_end = NonNull::new_unchecked(
                            reservation.heap_end.as_ptr().wrapping_add(additional_commit_size),
                        );
                        let new_heap_end = reservation.heap_end();

                        reservation.available -= additional_commit_size;
                        new_heap_end.cast::<usize>().write(reservation.available);

                        talc.extend(old_heap_end, new_heap_end.as_ptr());

                        return Ok(());
                    }
                }
            }
        }

        if !talc.is_metadata_established() {
            min_additional_size += crate::min_first_heap_layout::<B>().size();
            min_additional_size += size_of::<ReservationHeader>();
            min_additional_size += size_of::<usize>();
        }

        unsafe {
            if let Some(span) =
                talc.source.vmem.reserve(NonZeroUsize::new_unchecked(min_additional_size))
            {
                let commit_size =
                    (min_additional_size + commit_granularity_m1) & !commit_granularity_m1;

                talc.source.vmem.commit(span.cast(), commit_size);
                let commit_end =
                    NonNull::new_unchecked(span.cast::<u8>().as_ptr().wrapping_add(commit_size));

                debug_assert!(CHUNK_UNIT <= talc.source.vmem.commit_granularity());

                let reservation_base = span.cast::<u8>().as_ptr();

                let header = reservation_base.cast::<ReservationHeader>();
                (*header).reserved = span.len();

                let mut heap_base =
                    reservation_base.wrapping_add(core::mem::size_of::<ReservationHeader>());
                let chain = if let Some(chain) = talc.source.res_chain {
                    chain.as_ptr()
                } else {
                    let chain =
                        ptr_utils::align_up_by(heap_base, align_of::<Option<NonNull<Node>>>())
                            .cast::<Option<NonNull<Node>>>();

                    heap_base = chain.wrapping_add(1).cast();

                    talc.source.res_chain = NonNull::new(chain);
                    debug_assert!(talc.source.res_chain.is_some());

                    chain
                };

                Node::link_at(
                    addr_of_mut!((*header).node),
                    Node { next: chain.read(), next_of_prev: chain },
                );

                let heap_size = commit_end.as_ptr() as usize - CHUNK_UNIT - heap_base as usize;
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
                            if reservation.available < min_uncommitted_bytes {
                                min_uncommitted_index = index;
                                min_uncommitted_bytes = reservation.available;
                            }
                        } else {
                            *opt_reservation = Some(CachedReservation {
                                heap_end: commit_end,
                                available: uncommitted,
                            });

                            break 'overwrite_min;
                        }
                    }

                    if min_uncommitted_index != usize::MAX {
                        talc.source.uncommitted_cache[min_uncommitted_index] =
                            Some(CachedReservation {
                                heap_end: commit_end,
                                available: uncommitted,
                            });
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
            // Thus, the offset between `chunk_base` and the bottom of the heap is
            // `size_of::<ReservationHeader>() + 1` rounded up by CHUNK_UNIT. i.e. CHUNK_UNIT.

            // The reservation is always CHUNK_UNIT aligned as commit_granularity >= CHUNK_ALIGN.
            // The `chunk_base` is always CHUNK_UNIT aligned.
            // We always tell Talc it can use from `base + size_of::<ReservationHeader>()` onwards for non-first reservations.
            // size_of::<ReservationHeader>() <= CHUNK_UNIT - 1
            // Documentation of `resize` states that the `chunk_base` of the heap base will
            // be `base + 1` aligned up to the next `CHUNK_UNIT`.
            // So we can get from the `chunk_base` to the `reservation_base` by subtracting a `CHUNK_UNIT`.
            debug_assert!(size_of::<ReservationHeader>() < CHUNK_UNIT);

            let reservation_base = chunk_base.wrapping_sub(CHUNK_UNIT);
            let header = reservation_base.cast::<ReservationHeader>();

            Node::unlink((*header).node);
            let reservation_size = (*header).reserved;

            let uncommitted = heap_end.cast::<usize>().read();
            let commit_end = heap_end.wrapping_add(CHUNK_UNIT);

            for reservation in self.uncommitted_cache.iter_mut() {
                if let Some(res_commit_end) = reservation.map(|r| r.heap_end.as_ptr()) {
                    if res_commit_end == commit_end {
                        *reservation = None;
                    }
                }
            }

            let reservation_size = uncommitted + (commit_end as usize - reservation_base as usize);

            self.vmem.release(NonNull::new_unchecked(reservation_base), reservation_size);

            chunk_base
        } else {
            // committed_undiscarded_size = discard_end - chunk_base - CHUNK_UNIT
            //                            = (heap_end + CHUNK_UNIT) - chunk_base - CHUNK_UNIT
            //                            = heap_end - chunk_base
            let committed_undiscarded_size = heap_end as usize - chunk_base as usize;
            let commit_granularity = self.vmem.commit_granularity();

            if committed_undiscarded_size >= commit_granularity * DISCARD_THRESSHOLD {
                let old_discarded = heap_end.cast::<usize>().read();
                let old_uncommitted = heap_end.cast::<usize>().wrapping_add(1).read();

                let comitted_undiscarded_end = heap_end.wrapping_add(CHUNK_UNIT);

                let discard_size = committed_undiscarded_size & !(commit_granularity - 1);
                let new_undiscard_end =
                    NonNull::new_unchecked(comitted_undiscarded_end.wrapping_sub(discard_size));

                self.vmem.discard(new_undiscard_end, discard_size);

                let new_heap_end = heap_end.wrapping_sub(discard_size);
                let mut new_discarded_size = discard_size + discard_size;
                let mut new_uncommitted_size = old_uncommitted;

                if V::SHOULD_DECOMMIT && old_discarded_size > DECOMMIT_OLD_DISCARDED_THRESSHOLD {
                    let to_decommit_size: usize = todo!();
                    new_discarded_size -= to_decommit_size;
                    new_uncommitted_size += to_decommit_size;

                    todo!()
                }

                new_heap_end.cast::<usize>().write(new_uncommitted_size);
                new_heap_end.cast::<usize>().add(1).write(new_uncommitted_size);

                new_heap_end.cast::<HeapFooter>().write(HeapFooter {
                    available_size: new_discarded_size + new_uncommitted_size,
                    uncomitted_size: new_uncommitted_size,
                });

                // this never actually loops, but block labels are unstable on MSRV
                'overwrite_min: loop {
                    for (index, opt_reservation) in self.uncommitted_cache.iter_mut().enumerate() {
                        if let Some(reservation) = opt_reservation {
                            if reservation.heap_end.as_ptr() == commit_end {
                                reservation.available = new_uncommitted;
                                reservation.heap_end = new_commit_end;
                                break 'overwrite_min;
                            }

                            if reservation.available < min_uncommitted_bytes {
                                min_uncommitted_index = index;
                                min_uncommitted_bytes = reservation.available;
                            }
                        } else {
                            min_uncommitted_index = index;
                            min_uncommitted_bytes = 0;
                        }
                    }

                    if min_uncommitted_index != usize::MAX {
                        self.uncommitted_cache[min_uncommitted_index] = Some(CachedReservation {
                            heap_end: new_commit_end,
                            available: new_uncommitted,
                        });
                    }

                    break;
                }
            }

            if committed_undiscarded_size >= commit_granularity * DECOMMIT_COMMIT_RATIO {
                let uncommitted = heap_end.cast::<usize>().read();

                let discarded = heap_end.cast::<usize>().add(1).read();

                let commit_end = heap_end.wrapping_add(CHUNK_UNIT);

                let decommit_size = committed_undiscarded_size & !(commit_granularity - 1);
                let new_commit_end = NonNull::new_unchecked(commit_end.wrapping_sub(decommit_size));

                self.vmem.decommit(new_commit_end, decommit_size);
                let new_heap_end = heap_end.wrapping_sub(decommit_size);
                let new_uncommitted = uncommitted + decommit_size;
                new_heap_end.cast::<usize>().write(new_uncommitted);

                let mut min_uncommitted_index = usize::MAX;
                let mut min_uncommitted_bytes = new_uncommitted;

                // this never actually loops, but block labels are unstable on MSRV
                'overwrite_min: loop {
                    for (index, opt_reservation) in self.uncommitted_cache.iter_mut().enumerate() {
                        if let Some(reservation) = opt_reservation {
                            if reservation.heap_end.as_ptr() == commit_end {
                                reservation.available = new_uncommitted;
                                reservation.heap_end = new_commit_end;
                                break 'overwrite_min;
                            }

                            if reservation.available < min_uncommitted_bytes {
                                min_uncommitted_index = index;
                                min_uncommitted_bytes = reservation.available;
                            }
                        } else {
                            min_uncommitted_index = index;
                            min_uncommitted_bytes = 0;
                        }
                    }

                    if min_uncommitted_index != usize::MAX {
                        self.uncommitted_cache[min_uncommitted_index] = Some(CachedReservation {
                            heap_end: new_commit_end,
                            available: new_uncommitted,
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

impl<V: VirtualHeaps, const C: usize> Drop for VirtualHeapsSource<V, C> {
    fn drop(&mut self) {
        if let Some(chain) = self.res_chain {
            unsafe {
                for node in Node::iter_mut(chain.as_ptr().read()) {
                    let header = node.as_ptr().cast::<ReservationHeader>();
                    self.vmem.release(node.cast(), (*header).reserved);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CachedReservation {
    heap_end: NonNull<u8>,
    available: usize,
}

impl CachedReservation {
    pub fn heap_end(&self) -> NonNull<u8> {
        // SAFETY: The commit end should never be so close to the null pointer.
        unsafe { NonNull::new_unchecked(self.heap_end.as_ptr().wrapping_sub(CHUNK_UNIT)) }
    }
}

// always less than CHUNK_UNIT in size
// as this is 3*PTR_SIZE
// while CHUNK_UNIT is 4*PTR_SIZE at minimum
#[repr(C)]
struct ReservationHeader {
    node: Node,
    reserved: usize,
}

struct HeapFooter {
    available_size: usize,
    uncomitted_size: usize,
}
