# Talc for WebAssembly

Talc is also a drop-in replacement for the default Rust WebAssembly allocator, dlmalloc. The two main configurations's usage and benchmarks are below. Both provide a decent middleground by being faster than `lol_alloc` and `dlmalloc` while inbetweening them in size.

## Usage
Set the global allocator in your project after running `cargo add talc` as follows:

```rust
/// SAFETY: The runtime environment must be single-threaded WASM.
#[global_allocator]
static ALLOCATOR: talc::TalckWasm = unsafe { talc::TalckWasm::new_global() };
```

Or if your arena size is statically known, for example 16 MiB, `0x1000000`:

```rust
#[global_allocator]
static ALLOCATOR: talc::Talck<talc::locking::AssumeUnlockable, talc::ClaimOnOom> = {
    static mut MEMORY: [u8; 0x1000000] = [0; 0x1000000];
    let span = talc::Span::from_array(std::ptr::addr_of!(MEMORY).cast_mut());
    talc::Talc::new(unsafe { talc::ClaimOnOom::new(span) }).lock()
};
```

## Configuration features for WebAssembly:
- If default features are disabled, make sure to enable `"lock_api"`.
- Turn on `"counters"` for allocation statistics accessible via `ALLOCATOR.lock().get_counters()`
- You can turn off default features to remove `"nightly_api"`, allowing stable Rust builds.

    e.g. `default-features = false, features = ["lock_api", "counters"]`

## Relative WASM Binary Size

Rough measurements of allocator size for relative comparison using `/wasm-size`.

| Allocator | WASM Size/bytes |
| --------- | --------------- |
| lol_alloc | 11655 |
| rlsf | 12242 |
| **talc** (arena\*) | 13543 |
| **talc** | 14467 |
| dlmalloc (default) | 16767 |

\* uses a static arena instead of dynamically managing the heap

## WASM Benchmarks

Rough measurements of allocator speed for relative comparison using `/wasm-bench`.

| Allocator | Average Actions/us |
|-----------|--------------------|
| **talc** | 6.7|
| **talc** (arena\*) | 6.8 |
| rlsf | 5.7 |
| dlmalloc (default) | 5.9 |
| lol_alloc | 4.4 |

\* uses a static arena instead of dynamically managing the heap


If you'd like to see comparisons to other allocators in this space, consider creating a pull request or opening an issue.
