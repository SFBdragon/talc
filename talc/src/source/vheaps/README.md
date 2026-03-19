# Virtual Memory Management APIs Notes

## On Windows

Reserving and releasing address space: do in larger chunks:
- `VirtualAlloc(..., MEM_RESERVE, 0)`
- `VirtualFree(..., MEM_RELEASE)`

Committing memory for use
- `VirtualAlloc(..., MEM_COMMIT, PAGE_READWRITE`

Allowing memory reclaim:
- there's many ways to do this
- https://devblogs.microsoft.com/oldnewthing/20170113-00/?p=95185
- We don't care about recoverability.
- Controlling eviction priority isn't super useful.
- Removing memory from the working set is useful for accounting
- Keeping the memory accessible does simplify things a bit
- Allowing the OS to defer any expensive operations is preferrable
Main options:
- `DiscardVirtualMemory`
  - https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-discardvirtualmemory
  - "Use this function to discard memory contents that are no longer needed, while keeping the
      memory region itself committed. Discarding memory may give physical RAM back to the system.
      When the region of memory is again accessed by the application, the backing RAM is restored,
      and the contents of the memory is undefined.
- `VirtualFree(..., MEM_DECOMMIT)`
  - https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualfree
  - Reserves the pages. Reduced the commit charge of the program.

I'm not really sure about the major differences here besides affecting commit charge
and need for re-committing.
As long as we're releasing chunks of memory (which also decommits) then hopefully
too-intense pressure on the commit charge can be avoided.

`DiscardVirtualMemory` is a closer match for the Unix-y way of doing things.

https://github.com/chromium/chromium/blob/fd8a8914ca0183f0add65ae55f04e287543c7d4a/base/memory/discardable_shared_memory.cc#L423-L447
Here, the Chromium project prefers `DiscardVirtualMemory` and falls back to `MEM_RESET`.

## On Linux

Reserving and releasing address space: do in larger chunks:
- `mmap(..., libc::PROT_NONE, libc::MAP_ANONYMOUS | libc::MAP_PRIVATE, ...)`
- `munmap(...)`

Comitting memory:
- `mprotect(..., libc::PROT_READ | libc::PROT_WRITE)`

Allowing memory reclaim:
- `madvise(..., MADV_DONTNEED)` is very aggressive on Linux,
  immediately discarding the of a anonymous, private mapping
  https://man7.org/linux/man-pages/man2/madvise.2.html
- `madvise(..., MADV_FREE)` is very lazy - putting up the memory for
  reclamation if needed but effect on RSS is very delayed. Many projects
  that adopt is seem to un-adopt it to make memory usage more visible
    - mimalloc ditching MADV_FREE in 2025 https://github.com/web-infra-dev/rspack/pull/9037
    - go ditching MADV_FREE as the default in 2021 https://github.com/golang/go/issues/42330
    - jemalloc reportedly uses MADV_FREE and then eventually MADV_DONTNEED

## On macOS

See Linux for reserving and releasing address space.
See Linux for comitting memory.

Allowing memory reclaim:
- While `MADV_FREE` and `MADV_DONTNEED` do allow reclaiming memory, they tend
    to not get accounted for, leading to seemingly very high memory usage.
- `MADV_FREE_REUSABLE` and `MADV_FREE_REUSE` exist and many projects seem to
    to prefer them, largely because they are more visibly effectual (resource
    monitors don't overreport usage).

Chromium uses `MADV_FREE_REUSABLE`
https://github.com/chromium/chromium/blob/fd8a8914ca0183f0add65ae55f04e287543c7d4a/base/memory/discardable_shared_memory.cc#L407-L410
but largely because of the desire to track memory usage quite closely in tests,
otherwise it seems like `MADV_FREE_REUSABLE` behaves the same as `MADV_FREE`
but also requires `MADV_FREE_REUSE` to become accessible again.

## On BSD

See Linux for reserving and releasing address space.
See Linux for comitting memory.

Both `MADV_DONTNEED` and `MADV_FREE` are not particularly aggressive on
BSD (closer to POSIX). It's possible to `mmap` or `munmap` over
some memory to force an eviction (like other Unixes).

---

Goals:
- It's probably best to make it clear to users that we're releasing data.
  - It helps verify the the allocator is working as intended.
- We don't want to make our apps particularly attractive to OOM killers
- We don't want freeing to memory to take particularly long, all else equal.
- We want to avoid churning a lot (evicting and refilling pages repeatedly)

Windows is the odd duck out because it counts committed, discarded memory
against you. Following up with _decomitting_ after a more substantial signal
of lack-of-use is ideal.

As long as being able to release entire heaps is not too uncommon, it may be
feasible to use a single-tier physical memory release model with
reserve-commit-discard-release (discarded memory doesn't need to be recommitted).

Linux can piggy-back off of a similar mechanism by using MADV_FREE where
Windows might Discard and MADV_DONTNEED where Windows might decommit, but
this isn't one-to-one and some parameters will need to be adjusted
(MADV_FREE and MADV_DONTNEED are less comittal than Discard and MEM_DECOMMIT
respectively).

Alternatively MADV_FREE can just... not be used and MADV_DONTNEED used
for a simpler reserve-commit-discard-release model
(again discarded memory doesn't need to be recommitted).

On BSD, reserve-commit-discard-release is the only model that makes
much sense to me right now. Though the discard is going to be relatively lazy.

MacOS/Darwin is the ugly duckling which seems to have most allocators
using a reserve-commit-discard-reinstate-release model instead with
MADV_FREE_REUSABLE and MADV_FREE_REUSE.

One model to rule them all seems to be
reserve-commit-discard-reinstate-decommit-release
- track reservation size at reservation base (with the chain links)
- track comitted size at the reservation top
- track discarded size at the reservation top
- track discarded size at the top
  - Windows: discarded is the stuff that is comitted, discarded, and not decomitted
    - e.g. | committed & in-use | comitted & not in use | committed and discarded | uncomitted |
  - MacOS: discarded is the stuff that is comitted, MADV_FREE_REUSABLE, and not MADV_FREE_REUSE
    - e.g. | committed & in-use | comitted & not in use | committed and MADV_FREE_REUSABLE | uncomitted |
  - Linux: discarded is the stuff that is MADV_DONTNEED'd
    - e.g. | committed & in-use | comitted & not in use | committed and discarded | uncomitted |
  - BSD: same as linux, MADV_DONTNEED it
- Only Windows decommits, MacOS, Linux, and BSD never decommit
- Only MacOS reinstates, the others don't need to explicitly do so (noop)

A simple strategy on Windows is to decommit some memory if `committed and discarded`
gets too big. The others can noop here, I guess.
