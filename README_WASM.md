# Talc on WASM

`Talc` provides a good middleground by being faster than `lol_alloc` and `dlmalloc` while inbetweening them in size, although your mileage will vary.

If you'd like to see comparisons to other allocators in this space, consider creating a pull request or opening an issue.

## Usage
Set the global allocator in your project after running `cargo add talc` as follows:

```rust
#[global_allocator] static TALC: talc::TalckWasm = unsafe { talc::TalckWasm::new_global() };
```

Make sure that you have the `lock_api` feature enabled! 
- e.g. using stable Rust, in your `Cargo.toml`: `talc = { version = "3", default-features = false, features = ["lock_api"] }`

Note, this disables `nightly_api`, which isn't used in this API, allowing use of stable Rust.

## Relative WASM Binary Size

Rough measurements of allocator size for relative comparison using `wasm_size.sh` and `wasm-size`.

| Allocator | Size (bytes) - lower is better |
| --------- | ----- |
| lol_alloc | 18333 |
| talc      | 22017 |
| dlmalloc  | 23935 |

## WASM Benchmarks

Rough allocator benchmarks for comparison from [this project](https://github.com/SFBdragon/wasm-alloc-bench).

| Allocator | Average Time per 100000 actions (ms) - lower is better |
|-----------|--------------|
| talc      | 14.9         |
| dlmalloc  | 17.6         |
| lol_alloc | 35.4         |

