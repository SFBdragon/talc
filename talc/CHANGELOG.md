# Changelog

#### v5.0.3

- Update WASM examples to match current API.
- Included checks in `just check` and the GitHub CI to ensure the docs in markdown files don't break.

#### v5.0.2

Change README to avoid `<sep>` HTML tag usage as `crates.io` is not a fan.

#### v5.0.1

Fix broken `docs.rs` links due to API changes.

#### v5.0.0

Check out the [migration guide](#migrating-from-v4-to-v5)

In general, the allocator got a lot better at doing its job. Also took the opportunity to clean up the APIs, setup, and configuration.

Here are some highlights:

- Performance improvements.
- Size improvements on WebAssembly.
- `Source` (previously `OomHandler`) is now powerful enough for releasing memory automatically.
- `TalcCell` introduced: safe, `!Sync`, zero-runtime-overhead implementor of `GlobalAlloc` and `Allocator`
- The crate is now stable-by-default, with an MSRV of Rust 1.64
- Binning configuration for Talc has been added. This primarily benefitted Talc for WebAssembly performance.

Changes:
- `AssumeUnlockable` - the never-safe lock - is gone (good riddance). Instead consider `TalcCell` and `TalcSyncCell`.
- `Talc`'s heap management APIs have changed. Most notably the base of heaps are now fixed.
- The available features have changed, see [Features](#conditional-features)
- WebAssembly-specific things are all in `talc::wasm` now. `WasmHandler` became `WasmGrowAndExtend`. `WasmGrowAndClaim` is the default though.
- `Span` is gone, rest in peace.

And more.

#### v4.4.3

- [Rajas Paranjpe](https://github.com/ChocolateLoverRaj): Replaced `pub use counters` to ensure `Counters`
    is publicly accessible when the `"counters"` feature is enabled. 


#### v4.4.2

- [polarathene](https://github.com/polarathene): Replace README relative links with fully-qualified links.
- [polarathene](https://github.com/polarathene): Improve docs for `stable_examples/examples/std_global_allocator.rs`.

- Improved docs for `stable_examples/examples/stable_allocator_api.rs` and `stable_examples/examples/std_global_allocator.rs`.
- Deprecated the `Span::from*` function for converting from shared references and const pointers, as they make committing UB easy. These will be removed in v5.
- Fixed up a bunch of warnings all over the project.

#### v4.4.1

- Added utility function `except` to `Span`, which takes the set difference, potentially splitting the `Span`. Thanks [bjorn3](https://github.com/bjorn3) for the suggestion!

#### v4.4.0

- Added feature `allocator-api2` which allows using the `Allocator` trait on stable via the [`allocator-api2`](https://github.com/zakarumych/allocator-api2) crate. Thanks [jess-sol](https://github.com/jess-sol)!

#### v4.3.1

- Updated the README a little

#### v4.3.0

- Added an implementation for `Display` for the counters. Hopefully this makes your logs a bit prettier.
    - Bug me if you have opinions about the current layout, I'm open to changing it.

- Added Frusa and RLSF to the benchmarks.
    - Good showing by RLSF all around, and Frusa has particular workloads it excels at.
- Changed random actions benchmark to measure over various allocation sizes.

#### v4.2.0

- Optimized reallocation to allows other allocation operations to occur while memcopy-ing if an in-place reallocation failed.
    - As a side effect Talc now has a `grow_in_place` function that returns `Err` if growing the memory in-place isn't possible.
    - A graph of the random actions benchmark with a workload that benefits from this has been included in the [benchmarks](#benchmarks) section.

- Added `Span::from_*` and `From<>` functions for const pointers and shared references.
    - This makes creating a span in static contexts on stable much easier: `Span::from_const_array(addr_of!(MEMORY))`
- Fix: Made `Talck` derive `Debug` again.

- Contribution by [Ken Hoover](https://github.com/khoover): add Talc arena-style allocation size and perf WASM benchmarks
    - This might be a great option if you have a known dynamic memory requirement and would like to reduce your WASM size a little more.

- `wasm-size` now uses _wasm-opt_, giving more realistic size differences for users of _wasm-pack_
- Improved shell scripts
- Overhauled microbenchmarks
    - No longer simulates high-heap pressure as tolerating allocation failure is rare
    - Data is now displayed using box-and-whisker plots

#### v4.1.1

- Fix: Reset MSRV to 1.67.1 and added a check to `test.sh` for it

#### v4.1.0 (yanked, use 4.1.1)

- Added optional tracking of allocation metrics. Thanks [Ken Hoover](https://github.com/khoover) for the suggestion!
    - Enable the `"counters"` feature. Access the data via `talc.get_counters()`
    - Metrics include allocation count, bytes available, fragmentation, overhead, and more.
- Improvements to documentation
- Improved and updated benchmarks
- Integrated the WASM performance benchmark into the project. Use `wasm-bench.sh` to run (requires _wasm-pack_ and _deno_)
- Improved `wasm-size` and `wasm-size.sh`

#### v4.0.0
- Changed `Talck`'s API to be more inline with Rust norms.
    - `Talck` now hides its internal structure (no more `.0`).
    - `Talck::talc()` has been replaced by `Talck::lock()`.
    - `Talck::new()` and `Talck::into_inner(self)` have been added.
    - Removed `TalckRef` and implemented the `Allocator` trait on `Talck` directly. No need to call `talck.allocator()` anymore.
- Changed API for provided locking mechanism
    - Moved `AssumeUnlockable` into `talc::locking::AssumeUnlockable`
    - Removed `Talc::lock_assume_single_threaded`, use `.lock::<talc::locking::AssumeUnlockable>()` if necessary.
- Improvements to documentation here and there. Thanks [polarathene](https://github.com/polarathene) for the contribution!

#### v3.1.2
- Some improvements to documentation.

#### v3.1.1
- Changed the WASM OOM handler's behavior to be more robust if other code calls `memory.grow` during the allocator's use.

#### v3.1.0
- Reduced use of nightly-only features, and feature-gated the remainder (`Span::from(*mut [T])` and `Span::from_slice`) behind `nightly_api`.
- `nightly_api` feature is default-enabled
    - *WARNING:* use of `default-features = false` may cause unexpected errors if the gated functions are used. Consider adding `nightly_api` or using another function.

#### v3.0.1
- Improved documentation
- Improved and updated benchmarks
    - Increased the range of allocation sizes on Random Actions. (sorry Buddy Allocator!)
    - Increased the number of iterations the Heap Efficiency benchmark does to produce more accurate and stable values.

#### v3.0.0
- Added support for multiple discontinuous heaps! This required some major API changes
    - `new_arena` no longer exists (use `new` and then `claim`)
    - `init` has been replaced with `claim`
    - `claim`, `extend` and `truncate` now return the new heap extent
    - `InitOnOom` is now `ClaimOnOom`.
    - All of the above now have different behavior and documentation.
- Each heap now has a fixed overhead of one `usize` at the bottom.

To migrate from v2 to v3, keep in mind that you must keep track of the heaps if you want to resize them, by storing the returned `Span`s. Read [`claim`](https://docs.rs/talc/latest/talc/base/struct.Talc.html#method.claim), [`extend`](https://docs.rs/talc/latest/talc/base/struct.Talc.html#method.extend) and [`truncate`](https://docs.rs/talc/latest/talc/base/struct.Talc.html#method.truncate)'s documentation for all the details.

#### v2.2.1
- Rewrote the allocator internals to place allocation metadata above the allocation.
    - This will have the largest impact on avoiding false sharing, where previously, the allocation metadata for one allocation would infringe on the cache-line of the allocation before it, even if a sufficiently high alignment was demanded. Single-threaded performance marginally increased, too.
- Removed heap_exhaustion and replaced heap_efficiency benchmarks.
- Improved documentation and other resources.
- Changed the WASM size measurement to include slightly less overhead.

#### v2.2.0
- Added `dlmalloc` to the benchmarks.
- WASM should now be fully supported via `TalckWasm`. Let me know what breaks ;)
    - Find more details [here](https://github.com/SFBdragon/talc/README_WASM.md).


#### v2.1.0
- Tests are now passing on 32 bit targets.
- Documentation fixes and improvements for various items.
- Fixed using `lock_api` without `allocator`.
- Experimental WASM support has been added via `TalckWasm` on WASM targets.


#### v2.0.0
- Removed dependency on `spin` and switched to using `lock_api` (thanks [Stefan Lankes](https://github.com/stlankes))
    - You can specify the lock you want to use with `talc.lock::<spin::Mutex<()>>()` for example.
- Removed the requirement that the `Talc` struct must not be moved, and removed the `mov` function.
    - The arena is now used to store metadata, so extremely small arenas will result in allocation failure.
- Made the OOM handling system use generics and traits instead of a function pointer.
    - Use `ErrOnOom` to do what it says on the tin. `InitOnOom` is similar but inits to the given span if completely uninitialized. Implement `Source` on any struct to implement your own behaviour (the OOM handler state can be accessed from `handle_oom` via `talc.oom_handler`).
- Changed the API and internals of `Span` and other changes to pass `miri`'s Stacked Borrows checks.
    - Span now uses pointers exclusively and carries provenance.
- Updated the benchmarks in a number of ways, notably adding `buddy_alloc` and removing `simple_chunk_allocator`.
