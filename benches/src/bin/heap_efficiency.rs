use std::{alloc::GlobalAlloc, fmt::Write, path::PathBuf};

use benches::{ARENA_ALLOCATORS, AllocationWrapper, generate_align, generate_size};

const HE_MAX_ALLOC_SIZE: usize = 10000;
const HE_MAX_REALLOC_SIZE_MULTI: usize = 10;

fn main() {
    let cargo_manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let benchmark_results_dir = cargo_manifest_dir.join("../results");
    std::fs::create_dir_all(&benchmark_results_dir).unwrap();

    let mut csv = String::new();

    for named_allocator in ARENA_ALLOCATORS.iter() {
        write!(csv, "{},", named_allocator.name).unwrap();
    }

    csv.pop(); // remove trailing comma
    writeln!(csv).unwrap();

    for named_allocator in ARENA_ALLOCATORS.iter() {
        eprintln!("Benchmarking {}...", named_allocator.name);

        let allocator = unsafe { (named_allocator.init_fn)() };
        let efficiency = heap_efficiency(allocator.as_ref());

        write!(csv, "{},", efficiency).unwrap();
    }
    csv.pop(); // remove trailing comma

    let csv_file_path = benchmark_results_dir.join("heap-efficiency.csv");
    std::fs::write(csv_file_path, csv).unwrap();
}

pub fn heap_efficiency(allocator: &dyn GlobalAlloc) -> f64 {
    let mut v = Vec::with_capacity(100000);
    let mut used = 0;
    let mut total = 0;

    for _ in 0..300 {
        loop {
            let action = fastrand::usize(0..=9);

            match action {
                0..=4 => {
                    let size = generate_size(HE_MAX_ALLOC_SIZE);
                    let align = generate_align();

                    if let Some(allocation) = AllocationWrapper::new(size, align, allocator) {
                        v.push(allocation);
                    } else {
                        break;
                    }
                }
                5 => {
                    if !v.is_empty() {
                        let index = fastrand::usize(0..v.len());
                        v.swap_remove(index);
                    }
                }
                6..=9 => {
                    if !v.is_empty() {
                        let index = fastrand::usize(0..v.len());
                        let new_size =
                            fastrand::usize(1..(HE_MAX_ALLOC_SIZE * HE_MAX_REALLOC_SIZE_MULTI));
                        let allocation = v.get_mut(index).unwrap();

                        if allocation.realloc(new_size).is_err() {
                            break;
                        }
                    }
                }
                _ => unreachable!(),
            }
        }

        used += v.iter().map(|a| a.layout.size()).sum::<usize>();
        total += benches::HEAP_SIZE;
        v.clear();
    }

    used as f64 / total as f64 * 100.0
}
