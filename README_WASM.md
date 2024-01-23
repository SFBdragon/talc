# Talc on WASM

`Talc` provides a decent middleground by being faster than `lol_alloc` and `dlmalloc` (Rust WASM default) while inbetweening them in size. Although your mileage will vary, comparison tables are laid out below.

## Usage
Set the global allocator in your project after running `cargo add talc` as follows:

```rust
/// SAFETY:
/// The runtime environment must be single-threaded WASM.
///
/// Note: calls to memory.grow during use of the allocator is allowed.
#[global_allocator]
static ALLOCATOR: talc::TalckWasm = unsafe { talc::TalckWasm::new_global() };
```

Config features:
- If default features are disabled, make sure to enable `"lock_api"`.
- Turn on `"counters"` for allocation statistics accessible via `ALLOCATOR.lock().get_counters()`
- You can turn off default features to remove `"nightly_api"`, allowing stable Rust builds.

    `default-features = false, features = ["lock_api"]`

## Relative WASM Binary Size

Rough measurements of allocator size for relative comparison using `wasm-size.sh` + `/wasm-size`.

| Allocator | Size (bytes) - lower is better |
| --------- | ----- |
| lol_alloc | 15689 |
| talc      | 19228 |
| dlmalloc (default) | 21316 |

## WASM Benchmarks

Rough measurements of allocator speed for relative comparison using `wasm-bench.sh` + `/wasm-bench`.

| Allocator | Average Time per 100000 actions (ms) - lower is better |
|-----------|-----|
| talc | 15.86 |
| dlmalloc (default) | 18.84 |
| lol_alloc | 34.26 |



If you'd like to see comparisons to other allocators in this space, consider creating a pull request or opening an issue.