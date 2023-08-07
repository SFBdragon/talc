# Talc on WASM

`Talc` provides a good middleground by being much faster than either `lol_alloc` and `dlmalloc` while inbetweening them in size, although your mileage will vary.

If you'd like to see comparisons to other allocators in this space, consider creating a pull request or opening an issue.

## Usage
Just set it to the global allocator in your project after `cargo add talc`:

```rust
#[global_allocator] struct TALC: talc::TalckWasm = unsafe { TalckWasm::new_global() };
```

## Relative WASM Binary Size

Rough measurements of allocator size for relative comparison using `wasm_size.sh` and `wasm-size`.

| Allocator | Size  |
| --------- | ----- |
| lol_alloc | 20382 |
| talc      | 23509 |
| dlmalloc  | 26011 |

## WASM Benchmarks

Rough allocator benchmarks for comparison from [this project](https://github.com/SFBdragon/wasm-alloc-bench).

| Allocator | Average (ms) |
|-----------|--------------|
| talc      | 14.9         |
| dlmalloc  | 17.6         |
| lol_alloc | 35.4         |

