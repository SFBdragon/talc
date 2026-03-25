//! This includes markdown files from the workspace as docs to test them and
//! ensure that the Rust code in them doesn't break due to internal changes.

#[doc = include_str!("../../README.md")]
pub struct RepoReadmeMd;

#[doc = include_str!("../../BENCHMARKS.md")]
pub struct BenchmarksMd;

#[doc = include_str!("../../BENCHMARKS_WASM.md")]
pub struct BenchmarksWasmMd;
