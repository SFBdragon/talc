#![feature(iter_intersperse)]

use std::{alloc::GlobalAlloc, fmt::Write, path::PathBuf, process::Command};

use benches::{AllocationWrapper, SYSTEM_ALLOCATORS, generate_align, generate_size};
use memory_stats::memory_stats;

const MAX_ALLOC_SIZE: usize = 20000;
const MAX_REALLOC_SIZE_MULTI: usize = 10;

const MAX_MEM_USAGE: usize = 2 << 30;
const REPEATS: usize = 10;

fn main() {
    let allocators = SYSTEM_ALLOCATORS.iter().filter(|a| a.name != "FRuSA");

    let mut args = std::env::args().skip(1);
    if let Some(n) = args.next() {
        let n = n.parse::<usize>().unwrap();

        let named_allocator = allocators.skip(n).next().unwrap();

        let cargo_manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let benchmark_results_dir = cargo_manifest_dir.join("../results");
        std::fs::create_dir_all(&benchmark_results_dir).unwrap();

        let allocator = unsafe { (named_allocator.init_fn)() };
        let efficiency = memory_efficiency(allocator.as_ref());

        println!("{} {} {}", efficiency.allocated, efficiency.used_phys, efficiency.used_virt,);
    } else {
        let mut allocator_results = vec![];

        for (i, named_allocator) in allocators.enumerate() {
            eprintln!("Benchmarking {}...", named_allocator.name);

            let mut allocated = 0;
            let mut used_phys = 0;
            let mut used_virt = 0;

            for _ in 0..10 {
                let output = Command::new("cargo")
                    .args(&[
                        "run",
                        "-p",
                        "benches",
                        "--quiet",
                        "--bin",
                        "memory_efficiency",
                        "--release",
                    ])
                    .arg("--")
                    .arg(i.to_string())
                    .output()
                    .unwrap();

                let stdout = String::from_utf8(output.stdout).unwrap();
                let mut results =
                    stdout.trim().split_ascii_whitespace().map(|n| n.parse::<usize>().unwrap());

                allocated += results.next().unwrap();
                used_phys += results.next().unwrap();
                used_virt += results.next().unwrap();
            }

            let m = Measurements {
                allocated: allocated / REPEATS,
                used_phys: used_phys / REPEATS,
                used_virt: used_virt / REPEATS,
            };

            allocator_results.push((named_allocator.name, m));
        }

        let mut csv = String::new();

        for results in allocator_results.iter() {
            csv += results.0;
            csv += ",";
        }

        csv.pop();
        csv += "\n";

        for results in allocator_results {
            write!(csv, "{} {} {},", results.1.allocated, results.1.used_phys, results.1.used_virt)
                .unwrap();
        }

        csv.pop();
        csv += "\n";

        let cargo_manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let benchmark_results_dir = cargo_manifest_dir.join("../results");
        std::fs::create_dir_all(&benchmark_results_dir).unwrap();
        let csv_file_path = benchmark_results_dir.join("memory-efficiency.csv");

        std::fs::write(csv_file_path, csv).unwrap();
    }
}

fn memory_efficiency(allocator: &dyn GlobalAlloc) -> Measurements {
    let mut allocated = 0;
    let mut used_phys = 0;
    let mut used_virt = 0;

    let mut v = Vec::with_capacity(100000);

    let mut running_allocated = 0;
    loop {
        for _ in 0..300 {
            let action = fastrand::usize(0..=9);

            match action {
                0..=5 => {
                    let size = generate_size(MAX_ALLOC_SIZE);
                    let align = generate_align();

                    if let Some(allocation) = AllocationWrapper::new(size, align, allocator) {
                        touch_pages(allocation.ptr, allocation.layout.size());

                        running_allocated += allocation.layout.size();

                        v.push(allocation);
                    } else {
                        panic!();
                    }
                }
                6 => {
                    if !v.is_empty() {
                        let index = fastrand::usize(0..v.len());
                        let allocation = v.swap_remove(index);

                        running_allocated -= allocation.layout.size();
                    }
                }
                7..=9 => {
                    if !v.is_empty() {
                        let index = fastrand::usize(0..v.len());
                        let new_size =
                            fastrand::usize(1..(MAX_ALLOC_SIZE * MAX_REALLOC_SIZE_MULTI));
                        let allocation = v.get_mut(index).unwrap();

                        running_allocated -= allocation.layout.size();
                        running_allocated += new_size;

                        if allocation.realloc(new_size).is_err() {
                            panic!();
                        }
                    }
                }
                _ => unreachable!(),
            }
        }

        let stats = memory_stats().unwrap();

        if running_allocated >= MAX_MEM_USAGE {
            allocated += v.iter().map(|a| a.layout.size()).sum::<usize>();
            used_phys += stats.physical_mem;
            used_virt += stats.virtual_mem;

            v.clear();
            break;
        }
    }

    Measurements { allocated, used_phys, used_virt }
}

fn touch_pages(mut ptr: *mut u8, size: usize) {
    let top = ptr.wrapping_add(size - 1);

    while ptr < top {
        // we allocated this memory, we can write to it
        unsafe {
            ptr.write_volatile(0xAB);
            ptr = ptr.wrapping_sub(ptr.wrapping_add(1).read() as _);
        }

        // 4KiB, typical page size
        ptr = ptr.wrapping_add(4 << 10);
    }
}

struct Measurements {
    allocated: usize,
    used_phys: usize,
    used_virt: usize,
}
